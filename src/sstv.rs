use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use napi::bindgen_prelude::*;
use napi::threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode};
use napi::{Error, JsDeferred, Result, Status};
use napi_derive::napi;
use rasterwave::{
    AbortReason, DecodeEvent, DecodeEventRef, DecoderConfig, DetectionSource, EncodeOptions,
    EncoderStage, LineCompleteness, PaperBoundaryKind, RgbImage, SstvDecoder as CoreDecoder,
    SstvEncoder as CoreEncoder, SstvPaperConfig, SstvPaperDecoder, SstvPaperEvent,
    SstvPaperEventRef, SstvPaperMode, SstvStationId, SstvTransmissionEnvelope, SyncState,
};

use crate::runtime::{
    MAX_INPUT_OPERATIONS, MAX_OPERATIONS, MAX_READ_SAMPLES, error, lock_error, pool, safe_number,
};
use crate::types::{JsSstvMode, SstvDecoderOptions, SstvEncoderOptions};

type VoidResolver = Box<dyn FnOnce(Env) -> Result<()> + Send>;
type VoidDeferred = JsDeferred<(), VoidResolver>;
type SamplesResolver = Box<dyn FnOnce(Env) -> Result<Float32Array> + Send>;
type SamplesDeferred = JsDeferred<Float32Array, SamplesResolver>;

#[napi(object)]
pub struct SstvDecodeNotification {
    pub r#type: String,
    pub image_id: Option<f64>,
    pub paper_id: Option<f64>,
    pub boundary_id: Option<f64>,
    pub mode: Option<JsSstvMode>,
    pub candidates: Option<Vec<JsSstvMode>>,
    pub confidence: Option<f64>,
    pub detection: Option<String>,
    pub vis_code: Option<u32>,
    pub ambiguous: Option<bool>,
    pub candidate_count: Option<u32>,
    pub frequency_offset_hz: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub line_index: Option<f64>,
    pub mode_line_index: Option<u32>,
    pub revision: Option<u32>,
    pub completeness: Option<String>,
    pub pixels: Option<Uint8Array>,
    pub lines: Option<u32>,
    pub last_line: Option<u32>,
    pub reason: Option<String>,
    pub boundary_kind: Option<String>,
    pub trusted: Option<bool>,
    pub nominal_height: Option<u32>,
    pub start_line: Option<f64>,
    pub end_line: Option<f64>,
}

enum CodecEvent {
    Framed(DecodeEvent),
    Paper(SstvPaperEvent),
}

enum OwnedNotification {
    Codec(CodecEvent),
    Drain,
    Finished,
    Error(String),
}

impl OwnedNotification {
    fn into_js(self) -> Result<SstvDecodeNotification> {
        let mut output = SstvDecodeNotification {
            r#type: String::new(),
            image_id: None,
            paper_id: None,
            boundary_id: None,
            mode: None,
            candidates: None,
            confidence: None,
            detection: None,
            vis_code: None,
            ambiguous: None,
            candidate_count: None,
            frequency_offset_hz: None,
            width: None,
            height: None,
            line_index: None,
            mode_line_index: None,
            revision: None,
            completeness: None,
            pixels: None,
            lines: None,
            last_line: None,
            reason: None,
            boundary_kind: None,
            trusted: None,
            nominal_height: None,
            start_line: None,
            end_line: None,
        };
        match self {
            Self::Drain => output.r#type = "drain".to_owned(),
            Self::Finished => output.r#type = "finished".to_owned(),
            Self::Error(reason) => {
                output.r#type = "error".to_owned();
                output.reason = Some(reason);
            }
            Self::Codec(CodecEvent::Framed(event)) => match event {
                DecodeEvent::ModeCandidate {
                    candidates,
                    confidence,
                } => {
                    output.r#type = "modeCandidate".to_owned();
                    output.candidates = Some(
                        candidates
                            .into_iter()
                            .map(JsSstvMode::try_from)
                            .collect::<Result<_>>()?,
                    );
                    output.confidence = Some(f64::from(confidence));
                }
                DecodeEvent::ImageStarted {
                    image_id,
                    mode,
                    detection,
                    frequency_offset_hz,
                    width,
                    height,
                } => {
                    output.r#type = "imageStarted".to_owned();
                    output.image_id = Some(safe_number(image_id, "imageId")?);
                    output.mode = Some(mode.try_into()?);
                    match detection {
                        DetectionSource::Vis { code } => {
                            output.detection = Some("vis".to_owned());
                            output.vis_code = Some(u32::from(code));
                        }
                        DetectionSource::SyncTiming {
                            ambiguous,
                            candidate_count,
                        } => {
                            output.detection = Some("syncTiming".to_owned());
                            output.ambiguous = Some(ambiguous);
                            output.candidate_count = Some(u32::from(candidate_count));
                        }
                        DetectionSource::Manual => {
                            output.detection = Some("manual".to_owned());
                        }
                        _ => output.detection = Some("unknown".to_owned()),
                    }
                    output.frequency_offset_hz = Some(f64::from(frequency_offset_hz));
                    output.width = Some(width);
                    output.height = Some(height);
                }
                DecodeEvent::LineReady {
                    image_id,
                    mode,
                    line_index,
                    revision,
                    completeness,
                    pixels,
                } => {
                    output.r#type = "lineReady".to_owned();
                    output.image_id = Some(safe_number(image_id, "imageId")?);
                    output.mode = Some(mode.try_into()?);
                    output.line_index = Some(f64::from(line_index));
                    output.revision = Some(revision);
                    output.completeness = Some(
                        match completeness {
                            LineCompleteness::Provisional => "provisional",
                            LineCompleteness::Final => "final",
                        }
                        .to_owned(),
                    );
                    let mut bytes = Vec::with_capacity(pixels.len() * 3);
                    for pixel in pixels {
                        bytes.extend_from_slice(&[pixel.r, pixel.g, pixel.b]);
                    }
                    output.pixels = Some(Uint8Array::new(bytes));
                }
                DecodeEvent::ImageCompleted {
                    image_id,
                    mode,
                    lines,
                } => {
                    output.r#type = "imageCompleted".to_owned();
                    output.image_id = Some(safe_number(image_id, "imageId")?);
                    output.mode = Some(mode.try_into()?);
                    output.lines = Some(lines);
                }
                DecodeEvent::ImageAborted {
                    image_id,
                    mode,
                    last_line,
                    reason,
                } => {
                    output.r#type = "imageAborted".to_owned();
                    output.image_id = Some(safe_number(image_id, "imageId")?);
                    output.mode = Some(mode.try_into()?);
                    output.last_line = last_line;
                    output.reason = Some(
                        match reason {
                            AbortReason::InputDiscontinuity => "inputDiscontinuity",
                            AbortReason::EndOfInput => "endOfInput",
                            AbortReason::Reset => "reset",
                            AbortReason::SyncLost => "syncLost",
                            _ => "unknown",
                        }
                        .to_owned(),
                    );
                }
                DecodeEvent::SignalRejected { reason } => {
                    output.r#type = "signalRejected".to_owned();
                    output.reason = Some(reason.to_owned());
                }
                _ => {
                    output.r#type = "error".to_owned();
                    output.reason = Some("unsupported decoder event".to_owned());
                }
            },
            Self::Codec(CodecEvent::Paper(event)) => match event {
                SstvPaperEvent::PaperStarted {
                    paper_id,
                    mode,
                    width,
                } => {
                    output.r#type = "paperStarted".to_owned();
                    output.paper_id = Some(safe_number(paper_id, "paperId")?);
                    output.mode = Some(mode.try_into()?);
                    output.width = Some(width);
                }
                SstvPaperEvent::Boundary {
                    paper_id,
                    boundary_id,
                    line_index,
                    mode,
                    detection,
                    kind,
                    trusted,
                    width,
                    nominal_height,
                } => {
                    output.r#type = "rasterBoundary".to_owned();
                    output.paper_id = Some(safe_number(paper_id, "paperId")?);
                    output.boundary_id = Some(safe_number(boundary_id, "boundaryId")?);
                    output.line_index = Some(safe_number(line_index, "lineIndex")?);
                    output.mode = Some(mode.try_into()?);
                    if let Some(detection) = detection {
                        apply_detection(&mut output, detection);
                    }
                    output.boundary_kind = Some(paper_boundary_name(kind).to_owned());
                    output.trusted = Some(trusted);
                    output.width = Some(width);
                    output.nominal_height = Some(nominal_height);
                }
                SstvPaperEvent::ModeCandidate {
                    candidates,
                    confidence,
                } => {
                    output.r#type = "modeCandidate".to_owned();
                    output.candidates = Some(
                        candidates
                            .into_iter()
                            .map(JsSstvMode::try_from)
                            .collect::<Result<_>>()?,
                    );
                    output.confidence = Some(f64::from(confidence));
                }
                SstvPaperEvent::LineReady {
                    paper_id,
                    boundary_id,
                    line_index,
                    mode_line_index,
                    mode,
                    revision,
                    completeness,
                    pixels,
                } => {
                    output.r#type = "rasterLineReady".to_owned();
                    output.paper_id = Some(safe_number(paper_id, "paperId")?);
                    output.boundary_id = Some(safe_number(boundary_id, "boundaryId")?);
                    output.line_index = Some(safe_number(line_index, "lineIndex")?);
                    output.mode_line_index = Some(mode_line_index);
                    output.mode = Some(mode.try_into()?);
                    output.revision = Some(revision);
                    output.completeness = Some(line_completeness_name(completeness).to_owned());
                    let mut bytes = Vec::with_capacity(pixels.len() * 3);
                    for pixel in pixels {
                        bytes.extend_from_slice(&[pixel.r, pixel.g, pixel.b]);
                    }
                    output.pixels = Some(Uint8Array::new(bytes));
                }
                SstvPaperEvent::TransmissionCompleted {
                    paper_id,
                    boundary_id,
                    start_line,
                    end_line,
                    mode,
                    lines,
                } => {
                    output.r#type = "transmissionCompleted".to_owned();
                    output.paper_id = Some(safe_number(paper_id, "paperId")?);
                    output.boundary_id = Some(safe_number(boundary_id, "boundaryId")?);
                    output.start_line = Some(safe_number(start_line, "startLine")?);
                    output.end_line = Some(safe_number(end_line, "endLine")?);
                    output.mode = Some(mode.try_into()?);
                    output.lines = Some(lines);
                }
                SstvPaperEvent::ProtocolObserved {
                    mode,
                    detection,
                    trusted,
                } => {
                    output.r#type = "protocolObserved".to_owned();
                    output.mode = Some(mode.try_into()?);
                    apply_detection(&mut output, detection);
                    output.trusted = Some(trusted);
                }
                SstvPaperEvent::SignalRejected { reason } => {
                    output.r#type = "signalRejected".to_owned();
                    output.reason = Some(reason.to_owned());
                }
                _ => {
                    output.r#type = "error".to_owned();
                    output.reason = Some("unsupported SSTV paper event".to_owned());
                }
            },
        }
        Ok(output)
    }
}

fn apply_detection(output: &mut SstvDecodeNotification, detection: DetectionSource) {
    match detection {
        DetectionSource::Vis { code } => {
            output.detection = Some("vis".to_owned());
            output.vis_code = Some(u32::from(code));
        }
        DetectionSource::SyncTiming {
            ambiguous,
            candidate_count,
        } => {
            output.detection = Some("syncTiming".to_owned());
            output.ambiguous = Some(ambiguous);
            output.candidate_count = Some(u32::from(candidate_count));
        }
        DetectionSource::Manual => output.detection = Some("manual".to_owned()),
        _ => output.detection = Some("unknown".to_owned()),
    }
}

fn paper_boundary_name(kind: PaperBoundaryKind) -> &'static str {
    match kind {
        PaperBoundaryKind::Initial => "initial",
        PaperBoundaryKind::Vis => "vis",
        PaperBoundaryKind::SyncTiming => "syncTiming",
        PaperBoundaryKind::AptPhasing => "aptPhasing",
        PaperBoundaryKind::ProtocolEnd => "protocolEnd",
        PaperBoundaryKind::Discontinuity => "discontinuity",
        PaperBoundaryKind::Reset => "reset",
        _ => "unknown",
    }
}

fn line_completeness_name(value: LineCompleteness) -> &'static str {
    match value {
        LineCompleteness::Provisional => "provisional",
        LineCompleteness::Final => "final",
    }
}

type EventCallback =
    ThreadsafeFunction<OwnedNotification, (), SstvDecodeNotification, Status, false, true, 64>;

enum DecoderCommand {
    Push(Vec<f32>),
    Reset,
    Discontinuity(u64),
    Drain(VoidDeferred),
    Finish(VoidDeferred),
    Dispose(VoidDeferred),
}

struct DecoderQueue {
    commands: VecDeque<DecoderCommand>,
    running: bool,
    queued_samples: usize,
}

struct DecoderShared {
    codec: Mutex<Option<DecoderBackend>>,
    queue: Mutex<DecoderQueue>,
    callback: EventCallback,
    queue_capacity_samples: usize,
    accepting: AtomicBool,
    disposed: AtomicBool,
    failed: Arc<AtomicBool>,
    sync_state: AtomicU64,
}

enum DecoderBackend {
    Framed(Box<CoreDecoder>),
    Paper(Box<SstvPaperDecoder>),
}

#[napi]
pub struct SstvDecoder {
    shared: Arc<DecoderShared>,
}

#[napi]
impl SstvDecoder {
    #[napi(constructor)]
    pub fn new(
        input_sample_rate: u32,
        options: Option<SstvDecoderOptions>,
        on_event: Function<'_, SstvDecodeNotification, ()>,
    ) -> Result<Self> {
        let options = options.unwrap_or(SstvDecoderOptions {
            output_mode: None,
            fallback_mode: None,
            immediate_decode: None,
            detect_vis: None,
            detect_sync_timing: None,
            manual_mode: None,
            minimum_signal_level: None,
            queue_capacity_samples: None,
        });
        let output_mode = options.output_mode.as_deref().unwrap_or("framed");
        if output_mode != "framed" && output_mode != "continuousPaper" {
            return Err(error(
                "RASTERWAVE_INVALID_CONFIG",
                "outputMode must be 'framed' or 'continuousPaper'",
            ));
        }
        if output_mode == "continuousPaper" && options.immediate_decode == Some(true) {
            return Err(error(
                "RASTERWAVE_INVALID_CONFIG",
                "immediateDecode cannot be combined with continuousPaper",
            ));
        }
        let capacity = options
            .queue_capacity_samples
            .unwrap_or_else(|| input_sample_rate.saturating_mul(5));
        if capacity == 0 {
            return Err(error(
                "RASTERWAVE_INVALID_CONFIG",
                "queueCapacitySamples must be positive",
            ));
        }
        let codec = if output_mode == "continuousPaper" {
            let mode = match options.manual_mode {
                Some(mode) => SstvPaperMode::Manual { mode: mode.into() },
                None => SstvPaperMode::Auto {
                    fallback: options.fallback_mode.unwrap_or(JsSstvMode::Robot36).into(),
                },
            };
            DecoderBackend::Paper(Box::new(
                SstvPaperDecoder::new(
                    input_sample_rate,
                    SstvPaperConfig {
                        mode,
                        detect_vis: options.detect_vis.unwrap_or(true),
                        detect_sync_timing: options.detect_sync_timing.unwrap_or(true),
                        minimum_signal_level: options.minimum_signal_level.unwrap_or(0.002) as f32,
                    },
                )
                .map_err(|err| error("RASTERWAVE_INVALID_CONFIG", err))?,
            ))
        } else {
            let mut config = DecoderConfig::default();
            if let Some(value) = options.immediate_decode {
                config.immediate_decode = value;
            }
            if let Some(value) = options.detect_vis {
                config.detect_vis = value;
            }
            if let Some(value) = options.detect_sync_timing {
                config.detect_sync_timing = value;
            }
            config.manual_mode = options.manual_mode.map(Into::into);
            if let Some(value) = options.minimum_signal_level {
                config.minimum_signal_level = value as f32;
            }
            DecoderBackend::Framed(Box::new(
                CoreDecoder::new(input_sample_rate, config)
                    .map_err(|err| error("RASTERWAVE_INVALID_CONFIG", err))?,
            ))
        };
        let callback = on_event
            .build_threadsafe_function::<OwnedNotification>()
            .max_queue_size::<64>()
            .weak::<true>()
            .build_callback(|ctx| ctx.value.into_js())?;
        Ok(Self {
            shared: Arc::new(DecoderShared {
                codec: Mutex::new(Some(codec)),
                queue: Mutex::new(DecoderQueue {
                    commands: VecDeque::new(),
                    running: false,
                    queued_samples: 0,
                }),
                callback,
                queue_capacity_samples: capacity as usize,
                accepting: AtomicBool::new(true),
                disposed: AtomicBool::new(false),
                failed: Arc::new(AtomicBool::new(false)),
                sync_state: AtomicU64::new(0),
            }),
        })
    }

    #[napi]
    pub fn push_f32(&self, input: Float32Array) -> Result<bool> {
        self.ensure_accepting()?;
        if input.len() > self.shared.queue_capacity_samples {
            return Err(error(
                "RASTERWAVE_CHUNK_TOO_LARGE",
                format!(
                    "chunk has {} samples; capacity is {}",
                    input.len(),
                    self.shared.queue_capacity_samples
                ),
            ));
        }
        if input.iter().any(|sample| !sample.is_finite()) {
            return Err(error(
                "RASTERWAVE_NON_FINITE_SAMPLE",
                "PCM contains NaN or infinity",
            ));
        }
        self.enqueue_bounded(DecoderCommand::Push(input.to_vec()), input.len())
    }

    #[napi]
    pub fn reset(&self) -> Result<bool> {
        self.ensure_accepting()?;
        self.enqueue_bounded(DecoderCommand::Reset, 0)
    }

    #[napi]
    pub fn mark_discontinuity(&self, dropped_input_samples: f64) -> Result<bool> {
        self.ensure_accepting()?;
        if !dropped_input_samples.is_finite()
            || !(0.0..=9_007_199_254_740_991.0).contains(&dropped_input_samples)
        {
            return Err(error(
                "RASTERWAVE_INVALID_ARGUMENT",
                "droppedInputSamples must be a non-negative safe integer",
            ));
        }
        self.enqueue_bounded(
            DecoderCommand::Discontinuity(dropped_input_samples as u64),
            0,
        )
    }

    #[napi]
    pub fn drain<'env>(&self, env: &'env Env) -> Result<Object<'env>> {
        self.enqueue_void_promise(env, DecoderCommand::Drain)
    }

    #[napi]
    pub fn finish<'env>(&self, env: &'env Env) -> Result<Object<'env>> {
        self.ensure_accepting()?;
        self.shared.accepting.store(false, Ordering::Release);
        self.enqueue_void_promise(env, DecoderCommand::Finish)
    }

    #[napi]
    pub fn dispose<'env>(&self, env: &'env Env) -> Result<Object<'env>> {
        self.shared.accepting.store(false, Ordering::Release);
        self.shared.disposed.store(true, Ordering::Release);
        let (deferred, promise) = env.create_deferred::<(), VoidResolver>()?;
        let should_schedule = {
            let mut queue = self.shared.queue.lock().map_err(|_| lock_error())?;
            for command in queue.commands.drain(..) {
                reject_decoder_command(command);
            }
            queue.queued_samples = 0;
            queue.commands.push_back(DecoderCommand::Dispose(deferred));
            if queue.running {
                false
            } else {
                queue.running = true;
                true
            }
        };
        if should_schedule {
            schedule_decoder(self.shared.clone());
        }
        Ok(promise)
    }

    #[napi(getter)]
    pub fn queued_samples(&self) -> Result<f64> {
        let queue = self.shared.queue.lock().map_err(|_| lock_error())?;
        safe_number(queue.queued_samples as u64, "queuedSamples")
    }

    #[napi(getter)]
    pub fn sync_state(&self) -> String {
        match self.shared.sync_state.load(Ordering::Acquire) {
            1 => "readingVis",
            2 => "confirming",
            3 => "locked",
            4 => "finished",
            _ => "searching",
        }
        .to_owned()
    }
}

impl SstvDecoder {
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

    fn enqueue_bounded(&self, command: DecoderCommand, sample_count: usize) -> Result<bool> {
        let should_schedule = {
            let mut queue = self.shared.queue.lock().map_err(|_| lock_error())?;
            if queue.commands.len() >= MAX_INPUT_OPERATIONS
                || queue.queued_samples.saturating_add(sample_count)
                    > self.shared.queue_capacity_samples
            {
                return Ok(false);
            }
            queue.queued_samples += sample_count;
            queue.commands.push_back(command);
            if queue.running {
                false
            } else {
                queue.running = true;
                true
            }
        };
        if should_schedule {
            schedule_decoder(self.shared.clone());
        }
        Ok(true)
    }

    fn enqueue_void_promise<'env, F>(&self, env: &'env Env, build: F) -> Result<Object<'env>>
    where
        F: FnOnce(VoidDeferred) -> DecoderCommand,
    {
        let (deferred, promise) = env.create_deferred::<(), VoidResolver>()?;
        let command = build(deferred);
        let should_schedule = {
            let mut queue = self.shared.queue.lock().map_err(|_| lock_error())?;
            if queue.commands.len() >= MAX_OPERATIONS {
                if let Some(deferred) = command_deferred(command) {
                    deferred.reject(error("RASTERWAVE_QUEUE_FULL", "operation queue is full"));
                }
                return Ok(promise);
            }
            queue.commands.push_back(command);
            if queue.running {
                false
            } else {
                queue.running = true;
                true
            }
        };
        if should_schedule {
            schedule_decoder(self.shared.clone());
        }
        Ok(promise)
    }
}

fn command_deferred(command: DecoderCommand) -> Option<VoidDeferred> {
    match command {
        DecoderCommand::Drain(value)
        | DecoderCommand::Finish(value)
        | DecoderCommand::Dispose(value) => Some(value),
        _ => None,
    }
}

fn reject_decoder_command(command: DecoderCommand) {
    if let Some(deferred) = command_deferred(command) {
        deferred.reject(error(
            "RASTERWAVE_DISPOSED",
            "operation was cancelled by dispose",
        ));
    }
}

fn schedule_decoder(shared: Arc<DecoderShared>) {
    pool().spawn(move || drain_decoder(shared));
}

fn drain_decoder(shared: Arc<DecoderShared>) {
    loop {
        let command = {
            let mut queue = match shared.queue.lock() {
                Ok(value) => value,
                Err(_) => {
                    shared.failed.store(true, Ordering::Release);
                    return;
                }
            };
            match queue.commands.pop_front() {
                Some(command) => command,
                None => {
                    queue.running = false;
                    return;
                }
            }
        };
        let sample_count = match &command {
            DecoderCommand::Push(samples) => samples.len(),
            _ => 0,
        };
        let result = catch_unwind(AssertUnwindSafe(|| {
            process_decoder_command(&shared, command)
        }));
        if sample_count > 0 {
            if let Ok(mut queue) = shared.queue.lock() {
                queue.queued_samples = queue.queued_samples.saturating_sub(sample_count);
            }
        }
        match result {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                shared.failed.store(true, Ordering::Release);
                emit_notification(&shared, OwnedNotification::Error(err.to_string()));
            }
            Err(_) => {
                shared.failed.store(true, Ordering::Release);
                emit_notification(
                    &shared,
                    OwnedNotification::Error(
                        "RASTERWAVE_NATIVE_PANIC: decoder worker panicked".to_owned(),
                    ),
                );
            }
        }
    }
}

fn process_decoder_command(shared: &Arc<DecoderShared>, command: DecoderCommand) -> Result<()> {
    match command {
        DecoderCommand::Push(samples) => {
            let mut guard = shared.codec.lock().map_err(|_| lock_error())?;
            let codec = guard
                .as_mut()
                .ok_or_else(|| error("RASTERWAVE_DISPOSED", "decoder is disposed"))?;
            let callback_shared = shared.clone();
            let state = match codec {
                DecoderBackend::Framed(codec) => {
                    codec
                        .push_f32(&samples, &mut |event: DecodeEventRef<'_>| {
                            emit_notification(
                                &callback_shared,
                                OwnedNotification::Codec(CodecEvent::Framed(event.to_owned())),
                            );
                        })
                        .map_err(|err| error("RASTERWAVE_DECODE_FAILED", err))?;
                    codec.sync_state()
                }
                DecoderBackend::Paper(codec) => {
                    codec
                        .push_f32(&samples, &mut |event: SstvPaperEventRef<'_>| {
                            emit_notification(
                                &callback_shared,
                                OwnedNotification::Codec(CodecEvent::Paper(event.to_owned())),
                            );
                        })
                        .map_err(|err| error("RASTERWAVE_DECODE_FAILED", err))?;
                    codec.sync_state()
                }
            };
            update_sync_state(shared, state);
        }
        DecoderCommand::Reset => {
            let mut guard = shared.codec.lock().map_err(|_| lock_error())?;
            let codec = guard
                .as_mut()
                .ok_or_else(|| error("RASTERWAVE_DISPOSED", "decoder is disposed"))?;
            let callback_shared = shared.clone();
            let state = match codec {
                DecoderBackend::Framed(codec) => {
                    codec.reset_with_sink(&mut |event: DecodeEventRef<'_>| {
                        emit_notification(
                            &callback_shared,
                            OwnedNotification::Codec(CodecEvent::Framed(event.to_owned())),
                        );
                    });
                    codec.sync_state()
                }
                DecoderBackend::Paper(codec) => {
                    codec
                        .reset(&mut |event: SstvPaperEventRef<'_>| {
                            emit_notification(
                                &callback_shared,
                                OwnedNotification::Codec(CodecEvent::Paper(event.to_owned())),
                            );
                        })
                        .map_err(|err| error("RASTERWAVE_DECODE_FAILED", err))?;
                    codec.sync_state()
                }
            };
            update_sync_state(shared, state);
        }
        DecoderCommand::Discontinuity(dropped) => {
            let mut guard = shared.codec.lock().map_err(|_| lock_error())?;
            let codec = guard
                .as_mut()
                .ok_or_else(|| error("RASTERWAVE_DISPOSED", "decoder is disposed"))?;
            let callback_shared = shared.clone();
            let state = match codec {
                DecoderBackend::Framed(codec) => {
                    codec
                        .mark_discontinuity(dropped, &mut |event: DecodeEventRef<'_>| {
                            emit_notification(
                                &callback_shared,
                                OwnedNotification::Codec(CodecEvent::Framed(event.to_owned())),
                            );
                        })
                        .map_err(|err| error("RASTERWAVE_DECODE_FAILED", err))?;
                    codec.sync_state()
                }
                DecoderBackend::Paper(codec) => {
                    codec
                        .mark_discontinuity(dropped, &mut |event: SstvPaperEventRef<'_>| {
                            emit_notification(
                                &callback_shared,
                                OwnedNotification::Codec(CodecEvent::Paper(event.to_owned())),
                            );
                        })
                        .map_err(|err| error("RASTERWAVE_DECODE_FAILED", err))?;
                    codec.sync_state()
                }
            };
            update_sync_state(shared, state);
        }
        DecoderCommand::Drain(deferred) => {
            emit_barrier(shared, OwnedNotification::Drain, deferred);
        }
        DecoderCommand::Finish(deferred) => {
            let mut guard = shared.codec.lock().map_err(|_| lock_error())?;
            let codec = guard
                .as_mut()
                .ok_or_else(|| error("RASTERWAVE_DISPOSED", "decoder is disposed"))?;
            let callback_shared = shared.clone();
            let result = match codec {
                DecoderBackend::Framed(codec) => codec.finish(&mut |event: DecodeEventRef<'_>| {
                    emit_notification(
                        &callback_shared,
                        OwnedNotification::Codec(CodecEvent::Framed(event.to_owned())),
                    );
                }),
                DecoderBackend::Paper(codec) => {
                    codec.finish(&mut |event: SstvPaperEventRef<'_>| {
                        emit_notification(
                            &callback_shared,
                            OwnedNotification::Codec(CodecEvent::Paper(event.to_owned())),
                        );
                    })
                }
            };
            if let Err(err) = result {
                deferred.reject(error("RASTERWAVE_DECODE_FAILED", err));
                return Ok(());
            }
            update_sync_state(shared, SyncState::Finished);
            drop(guard);
            emit_barrier(shared, OwnedNotification::Finished, deferred);
        }
        DecoderCommand::Dispose(deferred) => {
            let mut guard = shared.codec.lock().map_err(|_| lock_error())?;
            guard.take();
            drop(guard);
            emit_barrier(shared, OwnedNotification::Drain, deferred);
        }
    }
    Ok(())
}

fn update_sync_state(shared: &DecoderShared, state: SyncState) {
    let value = match state {
        SyncState::Searching => 0,
        SyncState::ReadingVis => 1,
        SyncState::Confirming => 2,
        SyncState::Locked => 3,
        SyncState::Finished => 4,
    };
    shared.sync_state.store(value, Ordering::Release);
}

fn emit_notification(shared: &DecoderShared, event: OwnedNotification) {
    let failed = shared.failed.clone();
    let status = shared.callback.call_with_return_value(
        event,
        ThreadsafeFunctionCallMode::Blocking,
        move |result, _| {
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

fn emit_barrier(shared: &DecoderShared, event: OwnedNotification, deferred: VoidDeferred) {
    let status = shared.callback.call_with_return_value(
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
    if status != Status::Ok {
        deferred_reject_unavailable(status);
    }
}

fn deferred_reject_unavailable(_status: Status) {
    // The deferred is owned by the TSFN callback when the call was accepted.
}

#[napi(object)]
pub struct EncoderProgressSnapshot {
    pub samples_emitted: f64,
    pub estimated_total_samples: f64,
    pub current_row: Option<u32>,
    pub stage: String,
    pub raster_start_sample: f64,
    pub raster_end_sample: f64,
    pub finished: bool,
}

enum EncoderCommand {
    Read(u32, SamplesDeferred),
    Dispose(VoidDeferred),
}

struct EncoderQueue {
    commands: VecDeque<EncoderCommand>,
    running: bool,
}

struct EncoderShared {
    codec: Mutex<Option<CoreEncoder>>,
    queue: Mutex<EncoderQueue>,
    accepting: AtomicBool,
    samples_emitted: AtomicU64,
    estimated_total_samples: u64,
    raster_start_sample: u64,
    raster_end_sample: u64,
    current_row: AtomicI64,
    stage: AtomicU8,
    finished: AtomicBool,
}

#[napi]
pub struct SstvEncoder {
    shared: Arc<EncoderShared>,
}

#[napi]
impl SstvEncoder {
    #[napi(constructor)]
    pub fn new(
        pixels: Uint8Array,
        mode: JsSstvMode,
        sample_rate: u32,
        options: Option<SstvEncoderOptions>,
    ) -> Result<Self> {
        let core_mode: rasterwave::SstvMode = mode.into();
        let spec = core_mode.spec();
        let image = RgbImage::from_rgb8(spec.width, spec.height, pixels.as_ref())
            .map_err(|err| error("RASTERWAVE_INVALID_IMAGE", err))?;
        let options = options.unwrap_or(SstvEncoderOptions {
            amplitude: None,
            tone_offset_hz: None,
            include_vis_header: None,
            enhanced_preamble: None,
            station_id: None,
            post_image_gap_ms: None,
            end_guard_ms: None,
        });
        let mut encode_options = EncodeOptions::default();
        if let Some(value) = options.amplitude {
            encode_options.amplitude = value as f32;
        }
        if let Some(value) = options.tone_offset_hz {
            encode_options.tone_offset_hz = value as f32;
        }
        if let Some(value) = options.include_vis_header {
            encode_options.include_vis_header = value;
        }
        let station_id = match options.station_id {
            None => SstvStationId::None,
            Some(value) if value.kind == "none" => SstvStationId::None,
            Some(value) if value.kind == "fsk" => SstvStationId::Fsk {
                callsign: value.callsign.ok_or_else(|| {
                    error(
                        "RASTERWAVE_INVALID_CONFIG",
                        "FSK stationId requires callsign",
                    )
                })?,
            },
            Some(value) if value.kind == "cw" => SstvStationId::Cw {
                callsign: value.callsign.ok_or_else(|| {
                    error(
                        "RASTERWAVE_INVALID_CONFIG",
                        "CW stationId requires callsign",
                    )
                })?,
                wpm: u16::try_from(value.wpm.unwrap_or(20)).map_err(|_| {
                    error(
                        "RASTERWAVE_INVALID_CONFIG",
                        "CW stationId WPM is out of range",
                    )
                })?,
                tone_hz: value.tone_hz.unwrap_or(800.0) as f32,
            },
            Some(_) => {
                return Err(error(
                    "RASTERWAVE_INVALID_CONFIG",
                    "stationId.kind must be none, fsk, or cw",
                ));
            }
        };
        let envelope = SstvTransmissionEnvelope {
            enhanced_preamble: options.enhanced_preamble.unwrap_or(false),
            station_id,
            post_image_gap_seconds: options.post_image_gap_ms.unwrap_or(0.0) / 1000.0,
            end_guard_seconds: options.end_guard_ms.unwrap_or(0.0) / 1000.0,
        };
        let codec =
            CoreEncoder::new_with_envelope(image, core_mode, sample_rate, encode_options, envelope)
                .map_err(|err| error("RASTERWAVE_INVALID_CONFIG", err))?;
        let progress = codec.progress();
        Ok(Self {
            shared: Arc::new(EncoderShared {
                codec: Mutex::new(Some(codec)),
                queue: Mutex::new(EncoderQueue {
                    commands: VecDeque::new(),
                    running: false,
                }),
                accepting: AtomicBool::new(true),
                samples_emitted: AtomicU64::new(progress.samples_emitted),
                estimated_total_samples: progress.estimated_total_samples,
                raster_start_sample: progress.raster_start_sample,
                raster_end_sample: progress.raster_end_sample,
                current_row: AtomicI64::new(progress.current_row.map(i64::from).unwrap_or(-1)),
                stage: AtomicU8::new(encoder_stage_code(progress.stage)),
                finished: AtomicBool::new(progress.finished),
            }),
        })
    }

    #[napi]
    pub fn read_samples<'env>(&self, env: &'env Env, max_samples: u32) -> Result<Object<'env>> {
        if !self.shared.accepting.load(Ordering::Acquire) {
            return Err(error("RASTERWAVE_DISPOSED", "encoder is disposed"));
        }
        if max_samples == 0 || max_samples > MAX_READ_SAMPLES {
            return Err(error(
                "RASTERWAVE_INVALID_ARGUMENT",
                format!("maxSamples must be in 1..={MAX_READ_SAMPLES}"),
            ));
        }
        let (deferred, promise) = env.create_deferred::<Float32Array, SamplesResolver>()?;
        let should_schedule = {
            let mut queue = self.shared.queue.lock().map_err(|_| lock_error())?;
            if queue.commands.len() >= MAX_OPERATIONS {
                deferred.reject(error("RASTERWAVE_QUEUE_FULL", "operation queue is full"));
                return Ok(promise);
            }
            queue
                .commands
                .push_back(EncoderCommand::Read(max_samples, deferred));
            if queue.running {
                false
            } else {
                queue.running = true;
                true
            }
        };
        if should_schedule {
            schedule_encoder(self.shared.clone());
        }
        Ok(promise)
    }

    #[napi]
    pub fn dispose<'env>(&self, env: &'env Env) -> Result<Object<'env>> {
        self.shared.accepting.store(false, Ordering::Release);
        let (deferred, promise) = env.create_deferred::<(), VoidResolver>()?;
        let should_schedule = {
            let mut queue = self.shared.queue.lock().map_err(|_| lock_error())?;
            for command in queue.commands.drain(..) {
                match command {
                    EncoderCommand::Read(_, pending) => pending.reject(error(
                        "RASTERWAVE_DISPOSED",
                        "read was cancelled by dispose",
                    )),
                    EncoderCommand::Dispose(pending) => {
                        pending.reject(error("RASTERWAVE_DISPOSED", "dispose was superseded"))
                    }
                }
            }
            queue.commands.push_back(EncoderCommand::Dispose(deferred));
            if queue.running {
                false
            } else {
                queue.running = true;
                true
            }
        };
        if should_schedule {
            schedule_encoder(self.shared.clone());
        }
        Ok(promise)
    }

    #[napi(getter)]
    pub fn is_finished(&self) -> bool {
        self.shared.finished.load(Ordering::Acquire)
    }

    #[napi(getter)]
    pub fn progress(&self) -> Result<EncoderProgressSnapshot> {
        Ok(EncoderProgressSnapshot {
            samples_emitted: safe_number(
                self.shared.samples_emitted.load(Ordering::Acquire),
                "samplesEmitted",
            )?,
            estimated_total_samples: safe_number(
                self.shared.estimated_total_samples,
                "estimatedTotalSamples",
            )?,
            current_row: match self.shared.current_row.load(Ordering::Acquire) {
                -1 => None,
                value => Some(value as u32),
            },
            stage: encoder_stage_name_from_code(self.shared.stage.load(Ordering::Acquire))
                .to_owned(),
            raster_start_sample: safe_number(self.shared.raster_start_sample, "rasterStartSample")?,
            raster_end_sample: safe_number(self.shared.raster_end_sample, "rasterEndSample")?,
            finished: self.shared.finished.load(Ordering::Acquire),
        })
    }
}

fn encoder_stage_code(stage: EncoderStage) -> u8 {
    match stage {
        EncoderStage::Preamble => 0,
        EncoderStage::Vis => 1,
        EncoderStage::Raster => 2,
        EncoderStage::StationId => 3,
        EncoderStage::Guard => 4,
        EncoderStage::Finished => 5,
    }
}

fn encoder_stage_name_from_code(code: u8) -> &'static str {
    match code {
        0 => "preamble",
        1 => "vis",
        2 => "raster",
        3 => "stationId",
        4 => "guard",
        _ => "finished",
    }
}

fn schedule_encoder(shared: Arc<EncoderShared>) {
    pool().spawn(move || {
        loop {
            let command = {
                let mut queue = match shared.queue.lock() {
                    Ok(value) => value,
                    Err(_) => return,
                };
                match queue.commands.pop_front() {
                    Some(command) => command,
                    None => {
                        queue.running = false;
                        return;
                    }
                }
            };
            match command {
                EncoderCommand::Read(max_samples, deferred) => {
                    let result = catch_unwind(AssertUnwindSafe(|| {
                        let mut guard = shared.codec.lock().map_err(|_| lock_error())?;
                        let codec = guard
                            .as_mut()
                            .ok_or_else(|| error("RASTERWAVE_DISPOSED", "encoder is disposed"))?;
                        let mut output = vec![0.0_f32; max_samples as usize];
                        let written = codec.read_samples(&mut output);
                        output.truncate(written);
                        let progress = codec.progress();
                        shared
                            .samples_emitted
                            .store(progress.samples_emitted, Ordering::Release);
                        shared.current_row.store(
                            progress.current_row.map(i64::from).unwrap_or(-1),
                            Ordering::Release,
                        );
                        shared
                            .stage
                            .store(encoder_stage_code(progress.stage), Ordering::Release);
                        shared.finished.store(progress.finished, Ordering::Release);
                        Ok::<_, Error>(output)
                    }));
                    match result {
                        Ok(Ok(output)) => {
                            deferred.resolve(Box::new(move |_| Ok(Float32Array::new(output))))
                        }
                        Ok(Err(err)) => deferred.reject(err),
                        Err(_) => deferred
                            .reject(error("RASTERWAVE_NATIVE_PANIC", "encoder worker panicked")),
                    }
                }
                EncoderCommand::Dispose(deferred) => {
                    if let Ok(mut guard) = shared.codec.lock() {
                        guard.take();
                    }
                    deferred.resolve(Box::new(|_| Ok(())));
                }
            }
        }
    });
}
