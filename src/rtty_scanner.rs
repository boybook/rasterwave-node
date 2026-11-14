//! Wideband RTTY channel scanner binding.
//!
//! Same per-handle serial queue and backpressure contract as the RTTY
//! decoder: audio is copied into a bounded queue and processed in order on the
//! shared native pool; events reach the listener through a threadsafe function.

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use napi::bindgen_prelude::*;
use napi::threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode};
use napi::{JsDeferred, Result, Status};
use napi_derive::napi;
use rasterwave::rtty::{RttyChannelScanner as CoreScanner, RttyScanEvent, RttyScannerConfig};

use crate::rtty::{invalid, parse_profile};
use crate::runtime::{MAX_INPUT_OPERATIONS, MAX_OPERATIONS, error, lock_error, pool, safe_number};

type VoidResolver = Box<dyn FnOnce(Env) -> Result<()> + Send>;
type VoidDeferred = JsDeferred<(), VoidResolver>;

#[napi(object)]
#[derive(Clone)]
pub struct RttyScannerOptions {
    pub shifts_hz: Option<Vec<f64>>,
    pub min_hz: Option<f64>,
    pub max_hz: Option<f64>,
    pub max_channels: Option<u32>,
    pub min_score: Option<f64>,
    pub baud: Option<f64>,
    pub shift_hz: Option<f64>,
    pub stop_bits: Option<f64>,
    pub queue_capacity_samples: Option<u32>,
}

#[napi(object)]
pub struct RttyScanChannelInfo {
    pub id: u32,
    pub center_hz: f64,
    pub shift_hz: f64,
    pub score: f64,
    pub snr_db: f64,
    pub reversed: bool,
    pub locked: bool,
}

#[napi(object)]
pub struct RttyScanCharacter {
    pub ch: String,
    pub confidence: f64,
    pub start_sample: f64,
}

#[napi(object)]
pub struct RttyScanNotification {
    pub r#type: String,
    pub sample: Option<f64>,
    pub channels: Option<Vec<RttyScanChannelInfo>>,
    pub channel_id: Option<u32>,
    pub characters: Option<Vec<RttyScanCharacter>>,
    pub reason: Option<String>,
}

/// Without `shiftsHz` the scanner searches the profile shift (`shiftHz`, default 170 Hz).
fn scanner_config(options: Option<&RttyScannerOptions>) -> Result<RttyScannerConfig> {
    let defaults = RttyScannerConfig::default();
    let Some(options) = options else {
        return Ok(defaults);
    };
    let profile = parse_profile(options.baud, options.shift_hz, options.stop_bits)?;
    let shifts_hz = match &options.shifts_hz {
        Some(shifts) => shifts.iter().map(|&shift| shift as f32).collect(),
        None => vec![profile.shift_hz as f32],
    };
    Ok(RttyScannerConfig {
        shifts_hz,
        min_hz: options.min_hz.map_or(defaults.min_hz, |value| value as f32),
        max_hz: options.max_hz.map_or(defaults.max_hz, |value| value as f32),
        max_channels: options
            .max_channels
            .map_or(defaults.max_channels, |value| value as usize),
        min_score: options
            .min_score
            .map_or(defaults.min_score, |value| value as f32),
        profile,
    })
}

enum Notification {
    Scan(RttyScanEvent),
    Drain,
    Finished,
    Error(String),
}

impl Notification {
    fn into_js(self) -> Result<RttyScanNotification> {
        let mut output = RttyScanNotification {
            r#type: String::new(),
            sample: None,
            channels: None,
            channel_id: None,
            characters: None,
            reason: None,
        };
        match self {
            Self::Drain => output.r#type = "drain".to_owned(),
            Self::Finished => output.r#type = "finished".to_owned(),
            Self::Error(reason) => {
                output.r#type = "error".to_owned();
                output.reason = Some(reason);
            }
            Self::Scan(RttyScanEvent::Channels { sample, channels }) => {
                output.r#type = "channels".to_owned();
                output.sample = Some(safe_number(sample, "sample")?);
                output.channels = Some(
                    channels
                        .into_iter()
                        .map(|channel| RttyScanChannelInfo {
                            id: channel.id,
                            center_hz: f64::from(channel.center_hz),
                            shift_hz: f64::from(channel.shift_hz),
                            score: f64::from(channel.score),
                            snr_db: f64::from(channel.snr_db),
                            reversed: channel.reversed,
                            locked: channel.locked,
                        })
                        .collect(),
                );
            }
            Self::Scan(RttyScanEvent::Text {
                sample,
                channel_id,
                characters,
            }) => {
                output.r#type = "text".to_owned();
                output.sample = Some(safe_number(sample, "sample")?);
                output.channel_id = Some(channel_id);
                output.characters = Some(
                    characters
                        .into_iter()
                        .map(|character| {
                            Ok(RttyScanCharacter {
                                ch: character.ch.to_string(),
                                confidence: f64::from(character.confidence),
                                start_sample: safe_number(character.start_sample, "startSample")?,
                            })
                        })
                        .collect::<Result<Vec<_>>>()?,
                );
            }
            Self::Scan(RttyScanEvent::Removed { sample, channel_id }) => {
                output.r#type = "removed".to_owned();
                output.sample = Some(safe_number(sample, "sample")?);
                output.channel_id = Some(channel_id);
            }
            Self::Scan(_) => output.r#type = "unknown".to_owned(),
        }
        Ok(output)
    }
}

type EventCallback =
    ThreadsafeFunction<Notification, (), RttyScanNotification, Status, false, true, 64>;

enum Command {
    Push(Vec<f32>),
    Reset,
    Drain(VoidDeferred),
    Finish(VoidDeferred),
    Dispose(VoidDeferred),
}

struct Queue {
    commands: VecDeque<Command>,
    running: bool,
    queued_samples: usize,
}

struct Shared {
    codec: Mutex<Option<CoreScanner>>,
    queue: Mutex<Queue>,
    callback: EventCallback,
    capacity: usize,
    accepting: AtomicBool,
    disposed: AtomicBool,
    failed: Arc<AtomicBool>,
}

#[napi]
pub struct RttyScanner {
    shared: Arc<Shared>,
}

#[napi]
impl RttyScanner {
    #[napi(constructor)]
    pub fn new(
        input_sample_rate: u32,
        options: Option<RttyScannerOptions>,
        on_event: Function<'_, RttyScanNotification, ()>,
    ) -> Result<Self> {
        let capacity = options
            .as_ref()
            .and_then(|options| options.queue_capacity_samples)
            .unwrap_or_else(|| input_sample_rate.saturating_mul(5));
        if capacity == 0 {
            return Err(invalid("queueCapacitySamples must be positive"));
        }
        let codec = CoreScanner::new(input_sample_rate, scanner_config(options.as_ref())?)
            .map_err(invalid)?;
        let callback = on_event
            .build_threadsafe_function::<Notification>()
            .max_queue_size::<64>()
            .weak::<true>()
            .build_callback(|ctx| ctx.value.into_js())?;
        Ok(Self {
            shared: Arc::new(Shared {
                codec: Mutex::new(Some(codec)),
                queue: Mutex::new(Queue {
                    commands: VecDeque::new(),
                    running: false,
                    queued_samples: 0,
                }),
                callback,
                capacity: capacity as usize,
                accepting: AtomicBool::new(true),
                disposed: AtomicBool::new(false),
                failed: Arc::new(AtomicBool::new(false)),
            }),
        })
    }

    /// Queues PCM. Returns `false` (and drops nothing) when the queue is full.
    #[napi]
    pub fn push_f32(&self, input: Float32Array) -> Result<bool> {
        self.ensure_accepting()?;
        if input.len() > self.shared.capacity {
            return Err(error(
                "RASTERWAVE_CHUNK_TOO_LARGE",
                format!(
                    "chunk has {} samples; capacity is {}",
                    input.len(),
                    self.shared.capacity
                ),
            ));
        }
        if input.iter().any(|sample| !sample.is_finite()) {
            return Err(error(
                "RASTERWAVE_NON_FINITE_SAMPLE",
                "PCM contains NaN or infinity",
            ));
        }
        self.enqueue(Command::Push(input.to_vec()), input.len())
    }

    /// Forgets every channel, in order with queued audio. Channel ids keep counting up.
    #[napi]
    pub fn reset(&self) -> Result<bool> {
        self.ensure_accepting()?;
        self.enqueue(Command::Reset, 0)
    }

    #[napi]
    pub fn drain<'env>(&self, env: &'env Env) -> Result<Object<'env>> {
        self.enqueue_promise(env, Command::Drain)
    }

    #[napi]
    pub fn finish<'env>(&self, env: &'env Env) -> Result<Object<'env>> {
        self.ensure_accepting()?;
        self.shared.accepting.store(false, Ordering::Release);
        self.enqueue_promise(env, Command::Finish)
    }

    #[napi]
    pub fn dispose<'env>(&self, env: &'env Env) -> Result<Object<'env>> {
        self.shared.accepting.store(false, Ordering::Release);
        self.shared.disposed.store(true, Ordering::Release);
        let (deferred, promise) = env.create_deferred::<(), VoidResolver>()?;
        let schedule = {
            let mut queue = self.shared.queue.lock().map_err(|_| lock_error())?;
            let cancelled: Vec<Command> = queue.commands.drain(..).collect();
            for command in cancelled {
                match command {
                    // An earlier dispose() still resolves, after the codec is gone.
                    Command::Dispose(earlier) => {
                        queue.commands.push_back(Command::Dispose(earlier))
                    }
                    other => {
                        if let Some(pending) = command_deferred(other) {
                            pending.reject(error(
                                "RASTERWAVE_DISPOSED",
                                "operation was cancelled by dispose",
                            ));
                        }
                    }
                }
            }
            queue.queued_samples = 0;
            queue.commands.push_back(Command::Dispose(deferred));
            start_if_idle(&mut queue)
        };
        if schedule {
            spawn(self.shared.clone());
        }
        Ok(promise)
    }

    #[napi(getter)]
    pub fn queued_samples(&self) -> Result<f64> {
        let queue = self.shared.queue.lock().map_err(|_| lock_error())?;
        safe_number(queue.queued_samples as u64, "queuedSamples")
    }
}

impl RttyScanner {
    fn ensure_accepting(&self) -> Result<()> {
        if self.shared.disposed.load(Ordering::Acquire) {
            return Err(error("RASTERWAVE_DISPOSED", "scanner is disposed"));
        }
        if self.shared.failed.load(Ordering::Acquire) {
            return Err(error("RASTERWAVE_FAILED", "scanner session has failed"));
        }
        if !self.shared.accepting.load(Ordering::Acquire) {
            return Err(error(
                "RASTERWAVE_FINISHED",
                "scanner no longer accepts input",
            ));
        }
        Ok(())
    }

    fn enqueue(&self, command: Command, samples: usize) -> Result<bool> {
        let schedule = {
            let mut queue = self.shared.queue.lock().map_err(|_| lock_error())?;
            if queue.commands.len() >= MAX_INPUT_OPERATIONS
                || queue.queued_samples.saturating_add(samples) > self.shared.capacity
            {
                return Ok(false);
            }
            queue.queued_samples += samples;
            queue.commands.push_back(command);
            start_if_idle(&mut queue)
        };
        if schedule {
            spawn(self.shared.clone());
        }
        Ok(true)
    }

    fn enqueue_promise<'env>(
        &self,
        env: &'env Env,
        build: impl FnOnce(VoidDeferred) -> Command,
    ) -> Result<Object<'env>> {
        let (deferred, promise) = env.create_deferred::<(), VoidResolver>()?;
        let command = build(deferred);
        let schedule = {
            let mut queue = self.shared.queue.lock().map_err(|_| lock_error())?;
            if queue.commands.len() >= MAX_OPERATIONS {
                if let Some(deferred) = command_deferred(command) {
                    deferred.reject(error("RASTERWAVE_QUEUE_FULL", "operation queue is full"));
                }
                return Ok(promise);
            }
            queue.commands.push_back(command);
            start_if_idle(&mut queue)
        };
        if schedule {
            spawn(self.shared.clone());
        }
        Ok(promise)
    }
}

fn start_if_idle(queue: &mut Queue) -> bool {
    if queue.running {
        false
    } else {
        queue.running = true;
        true
    }
}

fn command_deferred(command: Command) -> Option<VoidDeferred> {
    match command {
        Command::Drain(value) | Command::Finish(value) | Command::Dispose(value) => Some(value),
        Command::Push(_) | Command::Reset => None,
    }
}

fn spawn(shared: Arc<Shared>) {
    pool().spawn(move || run(shared));
}

fn run(shared: Arc<Shared>) {
    loop {
        let command = {
            let Ok(mut queue) = shared.queue.lock() else {
                shared.failed.store(true, Ordering::Release);
                return;
            };
            match queue.commands.pop_front() {
                Some(command) => command,
                None => {
                    queue.running = false;
                    return;
                }
            }
        };
        let samples = match &command {
            Command::Push(samples) => samples.len(),
            _ => 0,
        };
        let result = catch_unwind(AssertUnwindSafe(|| process(&shared, command)));
        if samples > 0
            && let Ok(mut queue) = shared.queue.lock()
        {
            queue.queued_samples = queue.queued_samples.saturating_sub(samples);
        }
        let failure = match result {
            Ok(Ok(())) => None,
            Ok(Err(err)) => Some(err.reason.clone()),
            Err(_) => Some("RASTERWAVE_NATIVE_PANIC: scanner worker panicked".to_owned()),
        };
        if let Some(reason) = failure {
            shared.failed.store(true, Ordering::Release);
            emit(&shared, Notification::Error(reason));
        }
    }
}

fn process(shared: &Arc<Shared>, command: Command) -> Result<()> {
    let with_codec = |action: &mut dyn FnMut(&mut CoreScanner) -> Result<()>| -> Result<()> {
        let mut guard = shared.codec.lock().map_err(|_| lock_error())?;
        let codec = guard
            .as_mut()
            .ok_or_else(|| error("RASTERWAVE_DISPOSED", "scanner is disposed"))?;
        action(codec)
    };
    let mut sink = |event| emit(shared, Notification::Scan(event));
    match command {
        Command::Push(samples) => with_codec(&mut |codec| {
            codec
                .push_f32(&samples, &mut sink)
                .map_err(|err| error("RASTERWAVE_DECODE_FAILED", err))
        }),
        Command::Reset => with_codec(&mut |codec| {
            codec.reset();
            Ok(())
        }),
        Command::Drain(deferred) => {
            barrier(shared, Notification::Drain, deferred);
            Ok(())
        }
        Command::Finish(deferred) => {
            let result = with_codec(&mut |codec| {
                codec.finish(&mut sink);
                Ok(())
            });
            if let Err(err) = result {
                deferred.reject(err);
                return Ok(());
            }
            barrier(shared, Notification::Finished, deferred);
            Ok(())
        }
        Command::Dispose(deferred) => {
            if let Ok(mut guard) = shared.codec.lock() {
                guard.take();
            }
            barrier(shared, Notification::Drain, deferred);
            Ok(())
        }
    }
}

fn emit(shared: &Shared, event: Notification) {
    let failed = shared.failed.clone();
    let status = shared.callback.call_with_return_value(
        event,
        ThreadsafeFunctionCallMode::Blocking,
        move |result, _| {
            // A throwing listener fails the session, as for the decoders.
            if result.is_err() {
                failed.store(true, Ordering::Release);
            }
            Ok(())
        },
    );
    if status != Status::Ok && status != Status::Closing {
        shared.failed.store(true, Ordering::Release);
    }
}

fn barrier(shared: &Shared, event: Notification, deferred: VoidDeferred) {
    shared.callback.call_with_return_value(
        event,
        ThreadsafeFunctionCallMode::Blocking,
        move |result, _| {
            match result {
                Ok(()) => deferred.resolve(Box::new(|_| Ok(()))),
                Err(err) => deferred.reject(error("RASTERWAVE_CALLBACK_FAILED", err)),
            }
            Ok(())
        },
    );
}
