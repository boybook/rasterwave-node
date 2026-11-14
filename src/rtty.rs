//! RTTY bindings.
//!
//! The encoder is synchronous: one 100 ms frame costs microseconds, and running
//! on the caller's thread keeps `pushText`, `retractUnsent` and `readSamples`
//! strictly in program order, which interactive retraction depends on. The
//! decoder uses the same per-handle serial queue and backpressure contract as
//! the SSTV decoder.

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use napi::bindgen_prelude::*;
use napi::threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode};
use napi::{JsDeferred, Result, Status};
use napi_derive::napi;
use rasterwave::rtty::{
    CharacterSet, RttyDecodeEvent, RttyDecoder as CoreDecoder, RttyDecoderConfig,
    RttyEncoder as CoreEncoder, RttyEncoderConfig, RttyIdle, RttyLane, RttyLaneSet, RttyProfile,
    RttyTones, StopBits,
};

use crate::runtime::{
    MAX_INPUT_OPERATIONS, MAX_OPERATIONS, MAX_READ_SAMPLES, error, lock_error, pool, safe_number,
};

type VoidResolver = Box<dyn FnOnce(Env) -> Result<()> + Send>;
type VoidDeferred = JsDeferred<(), VoidResolver>;

#[napi(object)]
#[derive(Clone)]
pub struct RttyEncoderOptions {
    pub baud: Option<f64>,
    pub shift_hz: Option<f64>,
    pub stop_bits: Option<f64>,
    pub mark_hz: Option<f64>,
    pub space_hz: Option<f64>,
    pub character_set: Option<String>,
    pub amplitude: Option<f64>,
    pub preamble_mark_ms: Option<f64>,
    pub preamble_ltrs: Option<u32>,
    pub idle: Option<String>,
    pub tail_ltrs: Option<u32>,
    pub tail_mark_ms: Option<f64>,
    pub transition_ms: Option<f64>,
    pub envelope_ms: Option<f64>,
    pub expand_newline: Option<bool>,
}

#[napi(object)]
#[derive(Clone)]
pub struct RttyDecoderOptions {
    pub baud: Option<f64>,
    pub shift_hz: Option<f64>,
    pub stop_bits: Option<f64>,
    pub mark_hz: Option<f64>,
    pub space_hz: Option<f64>,
    pub character_set: Option<String>,
    pub usos: Option<bool>,
    pub afc: Option<bool>,
    pub afc_range_hz: Option<f64>,
    pub squelch: Option<f64>,
    pub lanes: Option<Vec<String>>,
    pub auto_reverse: Option<bool>,
    pub signal_report_ms: Option<f64>,
    pub noise_blanker: Option<bool>,
    pub queue_capacity_samples: Option<u32>,
}

#[napi(object)]
pub struct RttyProfileInfo {
    pub id: String,
    pub baud: f64,
    pub shift_hz: f64,
    pub stop_bits: f64,
}

#[napi]
pub fn rtty_profiles() -> Vec<RttyProfileInfo> {
    [
        ("45/170", RttyProfile::RTTY_45_170),
        ("50/170", RttyProfile::RTTY_50_170),
        ("75/170", RttyProfile::RTTY_75_170),
        ("50/425", RttyProfile::RTTY_50_425),
        ("50/850", RttyProfile::RTTY_50_850),
    ]
    .into_iter()
    .map(|(id, profile)| RttyProfileInfo {
        id: id.to_owned(),
        baud: profile.baud,
        shift_hz: profile.shift_hz,
        stop_bits: profile.stop_bits.bits(),
    })
    .collect()
}

pub(crate) fn invalid(message: impl std::fmt::Display) -> Error {
    error("RASTERWAVE_INVALID_CONFIG", message)
}

fn invalid_argument(message: impl std::fmt::Display) -> Error {
    error("RASTERWAVE_INVALID_ARGUMENT", message)
}

pub(crate) fn parse_profile(
    baud: Option<f64>,
    shift: Option<f64>,
    stop: Option<f64>,
) -> Result<RttyProfile> {
    let defaults = RttyProfile::default();
    let stop_bits = match stop {
        None => defaults.stop_bits,
        Some(1.0) => StopBits::One,
        Some(1.5) => StopBits::OnePointFive,
        Some(2.0) => StopBits::Two,
        Some(_) => return Err(invalid("stopBits must be 1, 1.5 or 2")),
    };
    Ok(RttyProfile::new(
        baud.unwrap_or(defaults.baud),
        shift.unwrap_or(defaults.shift_hz),
        stop_bits,
    ))
}

/// Without explicit tones, the pair is centred on 2210 Hz with mark below
/// space (lower-sideband AFSK convention) and separated by the profile shift.
fn parse_tones(mark: Option<f64>, space: Option<f64>, profile: &RttyProfile) -> Result<RttyTones> {
    match (mark, space) {
        (None, None) => Ok(RttyTones::from_center(2_210.0, profile.shift_hz, false)),
        (Some(mark), Some(space)) => Ok(RttyTones::new(mark, space)),
        _ => Err(invalid("markHz and spaceHz must be given together")),
    }
}

/// Option parsers return the bare message; constructors report it as
/// `RASTERWAVE_INVALID_CONFIG`, control methods as `RASTERWAVE_INVALID_ARGUMENT`.
fn parse_character_set(value: Option<&str>) -> std::result::Result<CharacterSet, &'static str> {
    match value {
        None | Some("usTty") => Ok(CharacterSet::UsTty),
        Some("ita2") => Ok(CharacterSet::Ita2),
        Some(_) => Err("characterSet must be 'usTty' or 'ita2'"),
    }
}

fn seconds(ms: Option<f64>, default: f64) -> f64 {
    ms.map_or(default, |value| value / 1000.0)
}

fn small_count(value: Option<u32>, default: u8, field: &str) -> Result<u8> {
    match value {
        None => Ok(default),
        Some(value) => u8::try_from(value)
            .ok()
            .filter(|value| *value <= 32)
            .ok_or_else(|| invalid(format!("{field} must be between 0 and 32"))),
    }
}

fn encoder_config(options: Option<RttyEncoderOptions>) -> Result<RttyEncoderConfig> {
    let defaults = RttyEncoderConfig::default();
    let Some(options) = options else {
        return Ok(defaults);
    };
    let profile = parse_profile(options.baud, options.shift_hz, options.stop_bits)?;
    Ok(RttyEncoderConfig {
        profile,
        tones: parse_tones(options.mark_hz, options.space_hz, &profile)?,
        character_set: parse_character_set(options.character_set.as_deref()).map_err(invalid)?,
        amplitude: options
            .amplitude
            .map_or(defaults.amplitude, |value| value as f32),
        preamble_mark_seconds: seconds(options.preamble_mark_ms, defaults.preamble_mark_seconds),
        preamble_ltrs: small_count(
            options.preamble_ltrs,
            defaults.preamble_ltrs,
            "preambleLtrs",
        )?,
        idle: match options.idle.as_deref() {
            None | Some("diddle") => RttyIdle::Diddle,
            Some("mark") => RttyIdle::Mark,
            Some(_) => return Err(invalid("idle must be 'diddle' or 'mark'")),
        },
        tail_ltrs: small_count(options.tail_ltrs, defaults.tail_ltrs, "tailLtrs")?,
        tail_mark_seconds: seconds(options.tail_mark_ms, defaults.tail_mark_seconds),
        transition_seconds: seconds(options.transition_ms, defaults.transition_seconds),
        envelope_seconds: seconds(options.envelope_ms, defaults.envelope_seconds),
        expand_newline: options.expand_newline.unwrap_or(defaults.expand_newline),
    })
}

fn parse_lanes(names: &[String]) -> std::result::Result<RttyLaneSet, &'static str> {
    let mut lanes = RttyLaneSet {
        atc: false,
        kahn: false,
        mark_only: false,
        space_only: false,
    };
    for name in names {
        match name.as_str() {
            "atc" => lanes.atc = true,
            "kahn" => lanes.kahn = true,
            "markOnly" => lanes.mark_only = true,
            "spaceOnly" => lanes.space_only = true,
            _ => return Err("lanes may contain 'atc', 'kahn', 'markOnly' and 'spaceOnly'"),
        }
    }
    Ok(lanes)
}

fn decoder_config(options: &RttyDecoderOptions) -> Result<RttyDecoderConfig> {
    let defaults = RttyDecoderConfig::default();
    let lanes = match &options.lanes {
        None => defaults.lanes,
        Some(names) => parse_lanes(names).map_err(invalid)?,
    };
    let profile = parse_profile(options.baud, options.shift_hz, options.stop_bits)?;
    Ok(RttyDecoderConfig {
        profile,
        tones: parse_tones(options.mark_hz, options.space_hz, &profile)?,
        character_set: parse_character_set(options.character_set.as_deref()).map_err(invalid)?,
        usos: options.usos.unwrap_or(defaults.usos),
        afc: options.afc.unwrap_or(defaults.afc),
        afc_range_hz: options.afc_range_hz.unwrap_or(defaults.afc_range_hz),
        squelch: options
            .squelch
            .map_or(defaults.squelch, |value| value as f32),
        lanes,
        auto_reverse: options.auto_reverse.unwrap_or(defaults.auto_reverse),
        signal_report_seconds: seconds(options.signal_report_ms, defaults.signal_report_seconds),
        noise_blanker: options.noise_blanker.unwrap_or(defaults.noise_blanker),
    })
}

fn lane_name(lane: RttyLane) -> &'static str {
    match lane {
        RttyLane::Atc => "atc",
        RttyLane::Kahn => "kahn",
        RttyLane::MarkOnly => "markOnly",
        RttyLane::SpaceOnly => "spaceOnly",
        _ => "unknown",
    }
}

// ---------------------------------------------------------------- encoder

#[napi(object)]
pub struct RttyPushResult {
    pub accepted: u32,
    pub first_seq: Option<f64>,
    pub rejected: String,
}

#[napi(object)]
pub struct RttyRenderedCharacter {
    pub seq: f64,
    pub ch: String,
    pub start_sample: f64,
    pub end_sample: f64,
}

#[napi(object)]
pub struct RttyReadResult {
    pub samples: Float32Array,
    pub rendered: Vec<RttyRenderedCharacter>,
    pub finished: bool,
}

#[napi]
pub struct RttyEncoder {
    codec: Option<CoreEncoder>,
}

#[napi]
impl RttyEncoder {
    #[napi(constructor)]
    pub fn new(sample_rate: u32, options: Option<RttyEncoderOptions>) -> Result<Self> {
        let codec = CoreEncoder::new(sample_rate, encoder_config(options)?).map_err(invalid)?;
        Ok(Self { codec: Some(codec) })
    }

    fn codec(&mut self) -> Result<&mut CoreEncoder> {
        self.codec
            .as_mut()
            .ok_or_else(|| error("RASTERWAVE_DISPOSED", "encoder is disposed"))
    }

    fn codec_ref(&self) -> Result<&CoreEncoder> {
        self.codec
            .as_ref()
            .ok_or_else(|| error("RASTERWAVE_DISPOSED", "encoder is disposed"))
    }

    #[napi]
    pub fn push_text(&mut self, text: String) -> Result<RttyPushResult> {
        let report = self
            .codec()?
            .push_text(&text)
            .map_err(|err| error("RASTERWAVE_ENCODER_CLOSED", err))?;
        // Rejected characters are reported, never silently dropped.
        Ok(RttyPushResult {
            accepted: u32::try_from(report.accepted).unwrap_or(u32::MAX),
            first_seq: report
                .first_seq
                .map(|seq| safe_number(seq, "firstSeq"))
                .transpose()?,
            rejected: report.rejected.into_iter().collect(),
        })
    }

    #[napi]
    pub fn retract_unsent(&mut self, count: f64) -> Result<u32> {
        if !count.is_finite() || count < 0.0 || count.fract() != 0.0 {
            return Err(error(
                "RASTERWAVE_INVALID_ARGUMENT",
                "count must be a non-negative integer",
            ));
        }
        let removed = self
            .codec()?
            .retract_unsent(count.min(f64::from(u32::MAX)) as usize);
        Ok(u32::try_from(removed).unwrap_or(u32::MAX))
    }

    #[napi]
    pub fn clear_unsent(&mut self) -> Result<u32> {
        let removed = self.codec()?.clear_unsent();
        Ok(u32::try_from(removed).unwrap_or(u32::MAX))
    }

    #[napi]
    pub fn finish(&mut self) -> Result<()> {
        self.codec()?.finish();
        Ok(())
    }

    #[napi]
    pub fn abort(&mut self) -> Result<()> {
        self.codec()?.abort();
        Ok(())
    }

    #[napi]
    pub fn read_samples(&mut self, max_samples: u32) -> Result<RttyReadResult> {
        if max_samples == 0 || max_samples > MAX_READ_SAMPLES {
            return Err(error(
                "RASTERWAVE_INVALID_ARGUMENT",
                format!("maxSamples must be in 1..={MAX_READ_SAMPLES}"),
            ));
        }
        let codec = self.codec()?;
        let mut output = vec![0.0_f32; max_samples as usize];
        let written = catch_unwind(AssertUnwindSafe(|| codec.read_samples(&mut output)))
            .map_err(|_| error("RASTERWAVE_NATIVE_PANIC", "encoder panicked"))?;
        output.truncate(written);
        let mut rendered = Vec::new();
        codec.drain_rendered(&mut rendered);
        let rendered = rendered
            .into_iter()
            .map(|item| {
                Ok(RttyRenderedCharacter {
                    seq: safe_number(item.seq, "seq")?,
                    ch: item.ch.to_string(),
                    start_sample: safe_number(item.start_sample, "startSample")?,
                    end_sample: safe_number(item.end_sample, "endSample")?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(RttyReadResult {
            samples: Float32Array::new(output),
            rendered,
            finished: codec.is_finished(),
        })
    }

    #[napi]
    pub fn dispose(&mut self) {
        self.codec = None;
    }

    #[napi(getter)]
    pub fn pending_text(&self) -> Result<String> {
        Ok(self.codec_ref()?.pending_text())
    }

    #[napi(getter)]
    pub fn pending_length(&self) -> Result<u32> {
        Ok(u32::try_from(self.codec_ref()?.pending_len()).unwrap_or(u32::MAX))
    }

    #[napi(getter)]
    pub fn is_finished(&self) -> bool {
        self.codec.as_ref().is_none_or(CoreEncoder::is_finished)
    }

    #[napi(getter)]
    pub fn is_finishing(&self) -> bool {
        self.codec.as_ref().is_none_or(CoreEncoder::is_finishing)
    }

    #[napi(getter)]
    pub fn samples_rendered(&self) -> Result<f64> {
        safe_number(self.codec_ref()?.samples_rendered(), "samplesRendered")
    }

    #[napi(getter)]
    pub fn sample_rate(&self) -> Result<u32> {
        Ok(self.codec_ref()?.sample_rate())
    }
}

// ---------------------------------------------------------------- decoder

#[napi(object)]
pub struct RttyDecoderSettings {
    pub afc: Option<bool>,
    pub usos: Option<bool>,
    pub character_set: Option<String>,
    pub lanes: Option<Vec<String>>,
    pub auto_reverse: Option<bool>,
}

#[derive(Clone, Copy)]
struct DecoderSettings {
    afc: Option<bool>,
    usos: Option<bool>,
    character_set: Option<CharacterSet>,
    lanes: Option<RttyLaneSet>,
    auto_reverse: Option<bool>,
}

#[napi(object)]
pub struct RttyDecodeNotification {
    pub r#type: String,
    pub ch: Option<String>,
    pub code: Option<u32>,
    pub confidence: Option<f64>,
    pub start_sample: Option<f64>,
    pub lane: Option<String>,
    pub reversed: Option<bool>,
    pub sample: Option<f64>,
    pub locked: Option<bool>,
    pub snr_db: Option<f64>,
    pub mark_db: Option<f64>,
    pub space_db: Option<f64>,
    pub frequency_offset_hz: Option<f64>,
    pub reason: Option<String>,
}

enum Notification {
    Codec(RttyDecodeEvent),
    Drain,
    Finished,
    Error(String),
}

impl Notification {
    fn into_js(self) -> Result<RttyDecodeNotification> {
        let mut output = RttyDecodeNotification {
            r#type: String::new(),
            ch: None,
            code: None,
            confidence: None,
            start_sample: None,
            lane: None,
            reversed: None,
            sample: None,
            locked: None,
            snr_db: None,
            mark_db: None,
            space_db: None,
            frequency_offset_hz: None,
            reason: None,
        };
        match self {
            Self::Drain => output.r#type = "drain".to_owned(),
            Self::Finished => output.r#type = "finished".to_owned(),
            Self::Error(reason) => {
                output.r#type = "error".to_owned();
                output.reason = Some(reason);
            }
            Self::Codec(RttyDecodeEvent::Character(character)) => {
                output.r#type = "character".to_owned();
                output.ch = Some(character.ch.to_string());
                output.code = Some(u32::from(character.code));
                output.confidence = Some(f64::from(character.confidence));
                output.start_sample = Some(safe_number(character.start_sample, "startSample")?);
                output.lane = Some(lane_name(character.lane).to_owned());
                output.reversed = Some(character.reversed);
            }
            Self::Codec(RttyDecodeEvent::Signal(signal)) => {
                output.r#type = "signal".to_owned();
                output.sample = Some(safe_number(signal.sample, "sample")?);
                output.locked = Some(signal.locked);
                output.snr_db = Some(f64::from(signal.snr_db));
                output.mark_db = Some(f64::from(signal.mark_db));
                output.space_db = Some(f64::from(signal.space_db));
                output.frequency_offset_hz = Some(f64::from(signal.frequency_offset_hz));
                output.reversed = Some(signal.reversed);
            }
            Self::Codec(_) => output.r#type = "unknown".to_owned(),
        }
        Ok(output)
    }
}

type EventCallback =
    ThreadsafeFunction<Notification, (), RttyDecodeNotification, Status, false, true, 64>;

enum Command {
    Push(Vec<f32>),
    Retune(RttyTones),
    Squelch(f32),
    Settings(DecoderSettings),
    Reset,
    /// Input samples lost at this point of the queue. Consecutive calls merge.
    Discontinuity(u64),
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
    codec: Mutex<Option<CoreDecoder>>,
    sample_rate: u32,
    /// Profile the decoder was created with. Baud and stop bits never change,
    /// so a retune is fully validated from this and the new shift.
    profile: RttyProfile,
    queue: Mutex<Queue>,
    callback: EventCallback,
    capacity: usize,
    accepting: AtomicBool,
    disposed: AtomicBool,
    failed: Arc<AtomicBool>,
}

#[napi]
pub struct RttyDecoder {
    shared: Arc<Shared>,
}

#[napi]
impl RttyDecoder {
    #[napi(constructor)]
    pub fn new(
        input_sample_rate: u32,
        options: Option<RttyDecoderOptions>,
        on_event: Function<'_, RttyDecodeNotification, ()>,
    ) -> Result<Self> {
        let options = options.unwrap_or(RttyDecoderOptions {
            baud: None,
            shift_hz: None,
            stop_bits: None,
            mark_hz: None,
            space_hz: None,
            character_set: None,
            usos: None,
            afc: None,
            afc_range_hz: None,
            squelch: None,
            lanes: None,
            auto_reverse: None,
            signal_report_ms: None,
            noise_blanker: None,
            queue_capacity_samples: None,
        });
        let capacity = options
            .queue_capacity_samples
            .unwrap_or_else(|| input_sample_rate.saturating_mul(5));
        if capacity == 0 {
            return Err(invalid("queueCapacitySamples must be positive"));
        }
        let config = decoder_config(&options)?;
        let codec = CoreDecoder::new(input_sample_rate, config).map_err(invalid)?;
        let callback = on_event
            .build_threadsafe_function::<Notification>()
            .max_queue_size::<64>()
            .weak::<true>()
            .build_callback(|ctx| ctx.value.into_js())?;
        Ok(Self {
            shared: Arc::new(Shared {
                codec: Mutex::new(Some(codec)),
                sample_rate: input_sample_rate,
                profile: config.profile,
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

    /// Validated in full before queueing, so the worker cannot reject it.
    #[napi]
    pub fn retune(&self, mark_hz: f64, space_hz: f64) -> Result<bool> {
        self.ensure_accepting()?;
        let tones = RttyTones::new(mark_hz, space_hz);
        tones
            .validate(self.shared.sample_rate)
            .map_err(invalid_argument)?;
        // A different shift becomes the decoder's profile shift.
        RttyProfile {
            shift_hz: tones.shift_hz(),
            ..self.shared.profile
        }
        .validate()
        .map_err(invalid_argument)?;
        self.enqueue(Command::Retune(tones), 0)
    }

    #[napi]
    pub fn set_squelch(&self, squelch: f64) -> Result<bool> {
        self.ensure_accepting()?;
        if !squelch.is_finite() || !(0.0..=1.0).contains(&squelch) {
            return Err(error(
                "RASTERWAVE_INVALID_ARGUMENT",
                "squelch must be between 0 and 1",
            ));
        }
        self.enqueue(Command::Squelch(squelch as f32), 0)
    }

    /// Changes receive settings in order with queued audio. Omitted fields keep their value.
    #[napi]
    pub fn update_settings(&self, settings: RttyDecoderSettings) -> Result<bool> {
        self.ensure_accepting()?;
        let lanes = settings
            .lanes
            .as_deref()
            .map(parse_lanes)
            .transpose()
            .map_err(invalid_argument)?;
        if lanes
            .is_some_and(|lanes| !(lanes.atc || lanes.kahn || lanes.mark_only || lanes.space_only))
        {
            return Err(invalid_argument(
                "lanes must enable at least one decision rule",
            ));
        }
        let character_set = settings
            .character_set
            .as_deref()
            .map(|value| parse_character_set(Some(value)))
            .transpose()
            .map_err(invalid_argument)?;
        self.enqueue(
            Command::Settings(DecoderSettings {
                afc: settings.afc,
                usos: settings.usos,
                character_set,
                lanes,
                auto_reverse: settings.auto_reverse,
            }),
            0,
        )
    }

    #[napi]
    pub fn reset(&self) -> Result<bool> {
        self.ensure_accepting()?;
        self.enqueue(Command::Reset, 0)
    }

    /// Declares that `droppedInputSamples` samples were lost between the audio
    /// queued before this call and the audio queued after it.
    ///
    /// Always accepted: the count is queued in order with the audio, merged
    /// into a marker already at the end of the queue, and allowed past the
    /// operation limit otherwise (merging keeps that bounded), so a full queue
    /// never loses timeline information.
    #[napi]
    pub fn mark_discontinuity(&self, dropped_input_samples: f64) -> Result<bool> {
        self.ensure_accepting()?;
        if !dropped_input_samples.is_finite()
            || !(0.0..=9_007_199_254_740_991.0).contains(&dropped_input_samples)
            || dropped_input_samples.fract() != 0.0
        {
            return Err(error(
                "RASTERWAVE_INVALID_ARGUMENT",
                "droppedInputSamples must be a non-negative safe integer",
            ));
        }
        let dropped = dropped_input_samples as u64;
        if dropped == 0 {
            return Ok(true);
        }
        let schedule = {
            let mut queue = self.shared.queue.lock().map_err(|_| lock_error())?;
            if let Some(Command::Discontinuity(total)) = queue.commands.back_mut() {
                *total = total.saturating_add(dropped);
            } else {
                queue.commands.push_back(Command::Discontinuity(dropped));
            }
            start_if_idle(&mut queue)
        };
        if schedule {
            spawn(self.shared.clone());
        }
        Ok(true)
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

impl RttyDecoder {
    fn ensure_accepting(&self) -> Result<()> {
        if self.shared.disposed.load(Ordering::Acquire) {
            return Err(error("RASTERWAVE_DISPOSED", "decoder is disposed"));
        }
        if self.shared.failed.load(Ordering::Acquire) {
            return Err(error("RASTERWAVE_FAILED", "decoder session has failed"));
        }
        if !self.shared.accepting.load(Ordering::Acquire) {
            return Err(error(
                "RASTERWAVE_FINISHED",
                "decoder no longer accepts input",
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
        _ => None,
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
        let control = matches!(
            command,
            Command::Retune(_) | Command::Squelch(_) | Command::Settings(_)
        );
        let result = catch_unwind(AssertUnwindSafe(|| process(&shared, command)));
        if samples > 0
            && let Ok(mut queue) = shared.queue.lock()
        {
            queue.queued_samples = queue.queued_samples.saturating_sub(samples);
        }
        let failure = match result {
            Ok(Ok(())) => None,
            Ok(Err(err)) if control => {
                // Control commands are validated before they are queued; one
                // the codec still rejects leaves the previous settings in
                // place and does not end the session.
                emit(&shared, Notification::Error(err.reason.clone()));
                None
            }
            Ok(Err(err)) => Some(err.reason.clone()),
            Err(_) => Some("RASTERWAVE_NATIVE_PANIC: decoder worker panicked".to_owned()),
        };
        if let Some(reason) = failure {
            shared.failed.store(true, Ordering::Release);
            emit(&shared, Notification::Error(reason));
        }
    }
}

fn process(shared: &Arc<Shared>, command: Command) -> Result<()> {
    let with_codec = |action: &mut dyn FnMut(&mut CoreDecoder) -> Result<()>| -> Result<()> {
        let mut guard = shared.codec.lock().map_err(|_| lock_error())?;
        let codec = guard
            .as_mut()
            .ok_or_else(|| error("RASTERWAVE_DISPOSED", "decoder is disposed"))?;
        action(codec)
    };
    let sink = |event| emit(shared, Notification::Codec(event));
    match command {
        Command::Push(samples) => with_codec(&mut |codec| {
            codec
                .push_f32(&samples, &mut sink.clone())
                .map_err(|err| error("RASTERWAVE_DECODE_FAILED", err))
        }),
        Command::Retune(tones) => with_codec(&mut |codec| {
            codec
                .retune(tones, &mut sink.clone())
                .map_err(|err| error("RASTERWAVE_INVALID_ARGUMENT", err))
        }),
        Command::Squelch(value) => with_codec(&mut |codec| {
            codec
                .set_squelch(value)
                .map_err(|err| error("RASTERWAVE_INVALID_ARGUMENT", err))
        }),
        Command::Settings(settings) => with_codec(&mut |codec| {
            if let Some(afc) = settings.afc {
                codec.set_afc(afc);
            }
            if let Some(usos) = settings.usos {
                codec.set_usos(usos);
            }
            if let Some(character_set) = settings.character_set {
                codec.set_character_set(character_set);
            }
            let config = *codec.config();
            let lanes = settings.lanes.unwrap_or(config.lanes);
            let auto_reverse = settings.auto_reverse.unwrap_or(config.auto_reverse);
            codec
                .set_lanes(lanes, auto_reverse, &mut sink.clone())
                .map_err(|err| error("RASTERWAVE_INVALID_ARGUMENT", err))
        }),
        Command::Reset => with_codec(&mut |codec| {
            codec.reset();
            Ok(())
        }),
        Command::Discontinuity(dropped) => with_codec(&mut |codec| {
            codec.mark_discontinuity(dropped, &mut sink.clone());
            Ok(())
        }),
        Command::Drain(deferred) => {
            barrier(shared, Notification::Drain, deferred);
            Ok(())
        }
        Command::Finish(deferred) => {
            let result = with_codec(&mut |codec| {
                codec.finish(&mut sink.clone());
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
            // A throwing listener fails the session, as for the SSTV decoder.
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
