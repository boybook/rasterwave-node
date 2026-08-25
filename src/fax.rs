use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use napi::bindgen_prelude::*;
use napi::threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode};
use napi::{Error, JsDeferred, Result, Status};
use napi_derive::napi;
use rasterwave::fax::{
    FaxClockCalibrationPoint, FaxClockRecoveryMode, FaxClockSource, FaxClockStatus, FaxDecodeEvent,
    FaxDecodeEventRef, FaxDecoder as CoreDecoder, FaxDecoderConfig, FaxEncodeOptions,
    FaxEncoder as CoreEncoder, FaxIoc, FaxLpm, FaxModulation, FaxPaperCorrection, FaxPolarity,
    FaxRasterBasis, FaxSpec, correct_fax_paper as core_correct_fax_paper,
};
use rasterwave::{
    FaxPaperConfig, FaxPaperDecoder, FaxPaperEvent, FaxPaperEventRef, FaxPaperMode, GrayImage,
    PaperBoundaryKind,
};

use crate::runtime::{
    MAX_INPUT_OPERATIONS, MAX_OPERATIONS, MAX_READ_SAMPLES, error, lock_error, pool, safe_number,
};
use crate::types::{
    FaxClockCalibrationOptions, FaxDecoderOptions, FaxEncoderOptions, FaxModulationOptions,
    FaxPaperCorrectionOptions, FaxSpecOptions, JsFaxIoc, JsFaxPolarity,
};

type VoidResolver = Box<dyn FnOnce(Env) -> Result<()> + Send>;
type VoidDeferred = JsDeferred<(), VoidResolver>;
type SamplesResolver = Box<dyn FnOnce(Env) -> Result<Float32Array> + Send>;
type SamplesDeferred = JsDeferred<Float32Array, SamplesResolver>;
type BytesResolver = Box<dyn FnOnce(Env) -> Result<Uint8Array> + Send>;
type BytesDeferred = JsDeferred<Uint8Array, BytesResolver>;

#[napi(object)]
pub struct FaxDecodeNotification {
    pub r#type: String,
    pub page_id: Option<f64>,
    pub paper_id: Option<f64>,
    pub boundary_id: Option<f64>,
    pub ioc: Option<JsFaxIoc>,
    pub lpm: Option<u32>,
    pub width: Option<u32>,
    pub active_width: Option<u32>,
    pub modulation: Option<String>,
    pub line_index: Option<f64>,
    pub segment_line_index: Option<u32>,
    pub pixels: Option<Uint8Array>,
    pub lines: Option<u32>,
    pub partial: Option<bool>,
    pub reason: Option<String>,
    pub boundary_kind: Option<String>,
    pub trusted: Option<bool>,
    pub start_line: Option<f64>,
    pub end_line: Option<f64>,
    pub basis: Option<String>,
    pub revision: Option<u32>,
    pub reference_line: Option<f64>,
    pub phase_pixels: Option<f64>,
    pub clock_ppm: Option<f64>,
    pub confidence: Option<f64>,
    pub clock_source: Option<String>,
    pub clock_status: Option<String>,
}

#[napi(object)]
pub struct FaxClockStateSnapshot {
    pub revision: u32,
    pub reference_line: f64,
    pub phase_pixels: f64,
    pub clock_ppm: f64,
    pub confidence: f64,
    pub source: String,
    pub status: String,
}

enum CodecEvent {
    Framed(FaxDecodeEvent),
    Paper(FaxPaperEvent),
}

enum OwnedNotification {
    Codec(CodecEvent),
    Drain,
    Finished,
    Error(String),
}

impl OwnedNotification {
    fn into_js(self) -> Result<FaxDecodeNotification> {
        let mut output = FaxDecodeNotification {
            r#type: String::new(),
            page_id: None,
            paper_id: None,
            boundary_id: None,
            ioc: None,
            lpm: None,
            width: None,
            active_width: None,
            modulation: None,
            line_index: None,
            segment_line_index: None,
            pixels: None,
            lines: None,
            partial: None,
            reason: None,
            boundary_kind: None,
            trusted: None,
            start_line: None,
            end_line: None,
            basis: None,
            revision: None,
            reference_line: None,
            phase_pixels: None,
            clock_ppm: None,
            confidence: None,
            clock_source: None,
            clock_status: None,
        };
        match self {
            Self::Drain => output.r#type = "drain".to_owned(),
            Self::Finished => output.r#type = "finished".to_owned(),
            Self::Error(reason) => {
                output.r#type = "error".to_owned();
                output.reason = Some(reason);
            }
            Self::Codec(CodecEvent::Framed(event)) => match event {
                FaxDecodeEvent::AptDetected { ioc } => {
                    output.r#type = "aptDetected".to_owned();
                    output.ioc = Some(ioc.into());
                }
                FaxDecodeEvent::PhasingLocked {
                    ioc,
                    lpm,
                    width,
                    clock,
                } => {
                    output.r#type = "phasingLocked".to_owned();
                    output.ioc = Some(ioc.into());
                    output.lpm = Some(u32::from(lpm.get()));
                    output.width = Some(width);
                    apply_clock(&mut output, clock)?;
                }
                FaxDecodeEvent::PageStarted {
                    page_id,
                    spec,
                    clock,
                } => {
                    output.r#type = "pageStarted".to_owned();
                    output.page_id = Some(safe_number(page_id, "pageId")?);
                    output.ioc = Some(spec.ioc.into());
                    output.lpm = Some(u32::from(spec.lpm.get()));
                    output.width = Some(spec.width());
                    output.active_width = Some(spec.active_width());
                    output.modulation = Some(modulation_name(spec.modulation).to_owned());
                    apply_clock(&mut output, clock)?;
                }
                FaxDecodeEvent::LineReady {
                    page_id,
                    line_index,
                    pixels,
                    basis,
                } => {
                    output.r#type = "lineReady".to_owned();
                    output.page_id = Some(safe_number(page_id, "pageId")?);
                    output.line_index = Some(f64::from(line_index));
                    output.pixels = Some(Uint8Array::new(pixels));
                    output.basis = Some(raster_basis_name(basis).to_owned());
                }
                FaxDecodeEvent::PageCompleted {
                    page_id,
                    lines,
                    partial,
                } => {
                    output.r#type = "pageCompleted".to_owned();
                    output.page_id = Some(safe_number(page_id, "pageId")?);
                    output.lines = Some(lines);
                    output.partial = Some(partial);
                }
                FaxDecodeEvent::SignalRejected { reason } => {
                    output.r#type = "signalRejected".to_owned();
                    output.reason = Some(reason.to_owned());
                }
                _ => {
                    output.r#type = "error".to_owned();
                    output.reason = Some("unsupported fax decoder event".to_owned());
                }
            },
            Self::Codec(CodecEvent::Paper(event)) => match event {
                FaxPaperEvent::PaperStarted { paper_id, spec } => {
                    output.r#type = "paperStarted".to_owned();
                    output.paper_id = Some(safe_number(paper_id, "paperId")?);
                    apply_spec(&mut output, spec);
                }
                FaxPaperEvent::Boundary {
                    paper_id,
                    boundary_id,
                    line_index,
                    spec,
                    kind,
                    trusted,
                } => {
                    output.r#type = "rasterBoundary".to_owned();
                    output.paper_id = Some(safe_number(paper_id, "paperId")?);
                    output.boundary_id = Some(safe_number(boundary_id, "boundaryId")?);
                    output.line_index = Some(safe_number(line_index, "lineIndex")?);
                    apply_spec(&mut output, spec);
                    output.boundary_kind = Some(paper_boundary_name(kind).to_owned());
                    output.trusted = Some(trusted);
                }
                FaxPaperEvent::AptDetected { ioc } => {
                    output.r#type = "aptDetected".to_owned();
                    output.ioc = Some(ioc.into());
                }
                FaxPaperEvent::ClockCalibration {
                    paper_id,
                    boundary_id,
                    calibration,
                } => {
                    output.r#type = "clockCalibration".to_owned();
                    output.paper_id = Some(safe_number(paper_id, "paperId")?);
                    output.boundary_id = Some(safe_number(boundary_id, "boundaryId")?);
                    apply_clock(&mut output, calibration)?;
                }
                FaxPaperEvent::LineReady {
                    paper_id,
                    boundary_id,
                    line_index,
                    segment_line_index,
                    spec,
                    pixels,
                    basis,
                } => {
                    output.r#type = "rasterLineReady".to_owned();
                    output.paper_id = Some(safe_number(paper_id, "paperId")?);
                    output.boundary_id = Some(safe_number(boundary_id, "boundaryId")?);
                    output.line_index = Some(safe_number(line_index, "lineIndex")?);
                    output.segment_line_index = Some(segment_line_index);
                    apply_spec(&mut output, spec);
                    output.pixels = Some(Uint8Array::new(pixels));
                    output.basis = Some(raster_basis_name(basis).to_owned());
                }
                FaxPaperEvent::TransmissionCompleted {
                    paper_id,
                    boundary_id,
                    start_line,
                    end_line,
                    spec,
                    lines,
                } => {
                    output.r#type = "transmissionCompleted".to_owned();
                    output.paper_id = Some(safe_number(paper_id, "paperId")?);
                    output.boundary_id = Some(safe_number(boundary_id, "boundaryId")?);
                    output.start_line = Some(safe_number(start_line, "startLine")?);
                    output.end_line = Some(safe_number(end_line, "endLine")?);
                    apply_spec(&mut output, spec);
                    output.lines = Some(lines);
                    output.partial = Some(false);
                }
                FaxPaperEvent::ProtocolObserved { spec, trusted } => {
                    output.r#type = "protocolObserved".to_owned();
                    apply_spec(&mut output, spec);
                    output.trusted = Some(trusted);
                }
                FaxPaperEvent::SignalRejected { reason } => {
                    output.r#type = "signalRejected".to_owned();
                    output.reason = Some(reason.to_owned());
                }
                _ => {
                    output.r#type = "error".to_owned();
                    output.reason = Some("unsupported fax paper event".to_owned());
                }
            },
        }
        Ok(output)
    }
}

fn apply_spec(output: &mut FaxDecodeNotification, spec: FaxSpec) {
    output.ioc = Some(spec.ioc.into());
    output.lpm = Some(u32::from(spec.lpm.get()));
    output.width = Some(spec.width());
    output.active_width = Some(spec.active_width());
    output.modulation = Some(modulation_name(spec.modulation).to_owned());
}

fn apply_clock(output: &mut FaxDecodeNotification, clock: FaxClockCalibrationPoint) -> Result<()> {
    output.revision = Some(clock.revision);
    output.reference_line = Some(safe_number(clock.reference_line, "referenceLine")?);
    output.phase_pixels = Some(f64::from(clock.phase_pixels));
    output.clock_ppm = Some(f64::from(clock.clock_ppm));
    output.confidence = Some(f64::from(clock.confidence));
    output.clock_source = Some(clock_source_name(clock.source).to_owned());
    output.clock_status = Some(clock_status_name(clock.status).to_owned());
    Ok(())
}

fn clock_source_name(source: FaxClockSource) -> &'static str {
    match source {
        FaxClockSource::Nominal => "nominal",
        FaxClockSource::Phasing => "phasing",
        FaxClockSource::DeadSector => "deadSector",
        FaxClockSource::Manual => "manual",
    }
}

fn clock_status_name(status: FaxClockStatus) -> &'static str {
    match status {
        FaxClockStatus::Nominal => "nominal",
        FaxClockStatus::Acquiring => "acquiring",
        FaxClockStatus::Locked => "locked",
        FaxClockStatus::Tracking => "tracking",
        FaxClockStatus::Degraded => "degraded",
    }
}

fn raster_basis_name(basis: FaxRasterBasis) -> &'static str {
    match basis {
        FaxRasterBasis::Calibrated => "calibrated",
        FaxRasterBasis::NominalPaper => "nominalPaper",
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

/// Correct an owned nominal-grid fax paper without blocking the Node event loop.
#[napi]
pub fn correct_fax_paper<'env>(
    env: &'env Env,
    pixels: Uint8Array,
    width: u32,
    height: u32,
    start_line: f64,
    calibration: Vec<FaxClockCalibrationOptions>,
    adjustment: Option<FaxPaperCorrectionOptions>,
) -> Result<Object<'env>> {
    let start_line = safe_u64_input(start_line, "startLine")?;
    let points = calibration
        .into_iter()
        .map(|point| {
            Ok(FaxClockCalibrationPoint {
                revision: safe_u32_input(point.revision, "revision")?,
                reference_line: safe_u64_input(point.reference_line, "referenceLine")?,
                phase_pixels: finite_f32(point.phase_pixels, "phasePixels")?,
                clock_ppm: finite_f32(point.clock_ppm, "clockPpm")?,
                confidence: finite_f32(point.confidence, "confidence")?.clamp(0.0, 1.0),
                source: parse_clock_source(&point.source)?,
                status: parse_clock_status(&point.status)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let adjustment = adjustment.unwrap_or_default();
    let adjustment = FaxPaperCorrection {
        phase_pixels: finite_f32(adjustment.phase_pixels.unwrap_or(0.0), "phasePixels")?,
        clock_ppm: finite_f32(adjustment.clock_ppm.unwrap_or(0.0), "clockPpm")?,
    };
    let owned = pixels.to_vec();
    let (deferred, promise): (BytesDeferred, _) =
        env.create_deferred::<Uint8Array, BytesResolver>()?;
    pool().spawn(move || {
        let result = catch_unwind(AssertUnwindSafe(|| {
            core_correct_fax_paper(&owned, width, height, start_line, &points, adjustment)
        }));
        match result {
            Ok(Ok(output)) => deferred.resolve(Box::new(move |_| Ok(Uint8Array::new(output)))),
            Ok(Err(err)) => deferred.reject(error("RASTERWAVE_CORRECTION_FAILED", err)),
            Err(_) => deferred.reject(error(
                "RASTERWAVE_NATIVE_PANIC",
                "fax paper correction panicked",
            )),
        }
    });
    Ok(promise)
}

fn safe_u64_input(value: f64, field: &str) -> Result<u64> {
    if !value.is_finite() || value < 0.0 || value.fract() != 0.0 || value > 9_007_199_254_740_991.0
    {
        return Err(error(
            "RASTERWAVE_INVALID_CONFIG",
            format!("{field} must be a non-negative safe integer"),
        ));
    }
    Ok(value as u64)
}

fn safe_u32_input(value: f64, field: &str) -> Result<u32> {
    let value = safe_u64_input(value, field)?;
    u32::try_from(value).map_err(|_| {
        error(
            "RASTERWAVE_INVALID_CONFIG",
            format!("{field} exceeds the supported range"),
        )
    })
}

fn finite_f32(value: f64, field: &str) -> Result<f32> {
    let converted = value as f32;
    if !value.is_finite() || !converted.is_finite() {
        return Err(error(
            "RASTERWAVE_INVALID_CONFIG",
            format!("{field} must be finite"),
        ));
    }
    Ok(converted)
}

fn parse_clock_source(value: &str) -> Result<FaxClockSource> {
    match value {
        "nominal" => Ok(FaxClockSource::Nominal),
        "phasing" => Ok(FaxClockSource::Phasing),
        "deadSector" => Ok(FaxClockSource::DeadSector),
        "manual" => Ok(FaxClockSource::Manual),
        _ => Err(error(
            "RASTERWAVE_INVALID_CONFIG",
            "clock source is invalid",
        )),
    }
}

fn parse_clock_status(value: &str) -> Result<FaxClockStatus> {
    match value {
        "nominal" => Ok(FaxClockStatus::Nominal),
        "acquiring" => Ok(FaxClockStatus::Acquiring),
        "locked" => Ok(FaxClockStatus::Locked),
        "tracking" => Ok(FaxClockStatus::Tracking),
        "degraded" => Ok(FaxClockStatus::Degraded),
        _ => Err(error(
            "RASTERWAVE_INVALID_CONFIG",
            "clock status is invalid",
        )),
    }
}

type EventCallback =
    ThreadsafeFunction<OwnedNotification, (), FaxDecodeNotification, Status, false, true, 64>;

enum DecoderCommand {
    Push(Vec<f32>),
    Reset,
    SignalLost,
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
    clock_state: Mutex<FaxClockCalibrationPoint>,
}

enum DecoderBackend {
    Framed(Box<CoreDecoder>),
    Paper(Box<FaxPaperDecoder>),
}

#[napi]
pub struct FaxDecoder {
    shared: Arc<DecoderShared>,
}

#[napi]
impl FaxDecoder {
    #[napi(constructor)]
    pub fn new(
        input_sample_rate: u32,
        options: Option<FaxDecoderOptions>,
        on_event: Function<'_, FaxDecodeNotification, ()>,
    ) -> Result<Self> {
        let options = options.unwrap_or(FaxDecoderOptions {
            output_mode: None,
            clock_recovery: None,
            continuous_auto: None,
            auto_am_modulation: None,
            immediate_decode: None,
            ioc: None,
            lpm: None,
            modulation: None,
            max_lines: None,
            am_full_scale: None,
            expected_phasing_seconds: None,
            apt_confirm_seconds: None,
            acquisition_timeout_seconds: None,
            stop_confirm_seconds: None,
            signal_loss_seconds: None,
            minimum_signal_level: None,
            minimum_carrier_coherence: None,
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
        if output_mode == "continuousPaper" && options.max_lines.is_some() {
            return Err(error(
                "RASTERWAVE_INVALID_CONFIG",
                "maxLines cannot be combined with continuousPaper",
            ));
        }
        let parsed_ioc = options.ioc.map(Into::into);
        let clock_recovery = match options.clock_recovery.as_deref().unwrap_or("auto") {
            "auto" => FaxClockRecoveryMode::Auto,
            "off" => FaxClockRecoveryMode::Off,
            _ => {
                return Err(error(
                    "RASTERWAVE_INVALID_CONFIG",
                    "clockRecovery must be 'auto' or 'off'",
                ));
            }
        };
        let parsed_lpm = options.lpm.map(fax_lpm).transpose()?;
        let parsed_modulation = options
            .modulation
            .as_ref()
            .map(fax_modulation)
            .transpose()?;
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
            let mut fallback = FaxSpec::standard(
                parsed_ioc.unwrap_or(FaxIoc::Ioc576),
                parsed_lpm.unwrap_or(FaxLpm::LPM_120),
            );
            fallback.modulation = parsed_modulation.unwrap_or(FaxModulation::WMO_FM);
            let mode = if options.continuous_auto.unwrap_or(true) {
                FaxPaperMode::Auto { fallback }
            } else {
                FaxPaperMode::Manual { spec: fallback }
            };
            let mut paper = FaxPaperConfig {
                mode,
                clock_recovery,
                ..FaxPaperConfig::default()
            };
            if let Some(value) = options.auto_am_modulation.as_ref() {
                paper.auto_am_modulation = fax_modulation(value)?;
            }
            if let Some(value) = options.am_full_scale {
                paper.am_full_scale = value as f32;
            }
            if options.expected_phasing_seconds.is_some() {
                paper.expected_phasing_seconds =
                    options.expected_phasing_seconds.map(|value| value as f32);
            }
            if let Some(value) = options.apt_confirm_seconds {
                paper.apt_confirm_seconds = value as f32;
            }
            if let Some(value) = options.acquisition_timeout_seconds {
                paper.acquisition_timeout_seconds = value as f32;
            }
            if let Some(value) = options.stop_confirm_seconds {
                paper.stop_confirm_seconds = value as f32;
            }
            if let Some(value) = options.signal_loss_seconds {
                paper.signal_loss_seconds = value as f32;
            }
            if let Some(value) = options.minimum_signal_level {
                paper.minimum_signal_level = value as f32;
            }
            if let Some(value) = options.minimum_carrier_coherence {
                paper.minimum_carrier_coherence = value as f32;
            }
            DecoderBackend::Paper(Box::new(
                FaxPaperDecoder::new(input_sample_rate, paper)
                    .map_err(|err| error("RASTERWAVE_INVALID_CONFIG", err))?,
            ))
        } else {
            let mut config = FaxDecoderConfig {
                immediate_decode: options.immediate_decode.unwrap_or(false),
                clock_recovery,
                ioc: parsed_ioc,
                lpm: parsed_lpm,
                modulation: parsed_modulation.unwrap_or(FaxModulation::WMO_FM),
                max_lines: options.max_lines,
                ..FaxDecoderConfig::default()
            };
            if let Some(value) = options.am_full_scale {
                config.am_full_scale = value as f32;
            }
            if options.expected_phasing_seconds.is_some() {
                config.expected_phasing_seconds =
                    options.expected_phasing_seconds.map(|value| value as f32);
            }
            if let Some(value) = options.apt_confirm_seconds {
                config.apt_confirm_seconds = value as f32;
            }
            if let Some(value) = options.acquisition_timeout_seconds {
                config.acquisition_timeout_seconds = value as f32;
            }
            if let Some(value) = options.stop_confirm_seconds {
                config.stop_confirm_seconds = value as f32;
            }
            if let Some(value) = options.signal_loss_seconds {
                config.signal_loss_seconds = value as f32;
            }
            if let Some(value) = options.minimum_signal_level {
                config.minimum_signal_level = value as f32;
            }
            if let Some(value) = options.minimum_carrier_coherence {
                config.minimum_carrier_coherence = value as f32;
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
                clock_state: Mutex::new(FaxClockCalibrationPoint::default()),
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
    pub fn mark_signal_lost(&self) -> Result<bool> {
        self.ensure_accepting()?;
        self.enqueue_bounded(DecoderCommand::SignalLost, 0)
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
    pub fn clock_state(&self) -> Result<FaxClockStateSnapshot> {
        let clock = *self.shared.clock_state.lock().map_err(|_| lock_error())?;
        Ok(FaxClockStateSnapshot {
            revision: clock.revision,
            reference_line: safe_number(clock.reference_line, "referenceLine")?,
            phase_pixels: f64::from(clock.phase_pixels),
            clock_ppm: f64::from(clock.clock_ppm),
            confidence: f64::from(clock.confidence),
            source: clock_source_name(clock.source).to_owned(),
            status: clock_status_name(clock.status).to_owned(),
        })
    }
}

impl FaxDecoder {
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
                        "RASTERWAVE_NATIVE_PANIC: fax decoder worker panicked".to_owned(),
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
            match codec {
                DecoderBackend::Framed(codec) => codec
                    .push_f32(&samples, &mut |event: FaxDecodeEventRef<'_>| {
                        emit_notification(
                            &callback_shared,
                            OwnedNotification::Codec(CodecEvent::Framed(event.to_owned())),
                        );
                    })
                    .map_err(|err| error("RASTERWAVE_DECODE_FAILED", err))?,
                DecoderBackend::Paper(codec) => codec
                    .push_f32(&samples, &mut |event: FaxPaperEventRef<'_>| {
                        emit_notification(
                            &callback_shared,
                            OwnedNotification::Codec(CodecEvent::Paper(event.to_owned())),
                        );
                    })
                    .map_err(|err| error("RASTERWAVE_DECODE_FAILED", err))?,
            };
        }
        DecoderCommand::Reset => {
            let mut guard = shared.codec.lock().map_err(|_| lock_error())?;
            let codec = guard
                .as_mut()
                .ok_or_else(|| error("RASTERWAVE_DISPOSED", "decoder is disposed"))?;
            match codec {
                DecoderBackend::Framed(codec) => codec.reset(),
                DecoderBackend::Paper(codec) => {
                    let callback_shared = shared.clone();
                    codec
                        .reset(&mut |event: FaxPaperEventRef<'_>| {
                            emit_notification(
                                &callback_shared,
                                OwnedNotification::Codec(CodecEvent::Paper(event.to_owned())),
                            );
                        })
                        .map_err(|err| error("RASTERWAVE_DECODE_FAILED", err))?;
                }
            }
        }
        DecoderCommand::SignalLost => {
            let mut guard = shared.codec.lock().map_err(|_| lock_error())?;
            let codec = guard
                .as_mut()
                .ok_or_else(|| error("RASTERWAVE_DISPOSED", "decoder is disposed"))?;
            let callback_shared = shared.clone();
            match codec {
                DecoderBackend::Framed(codec) => codec
                    .mark_signal_lost(&mut |event: FaxDecodeEventRef<'_>| {
                        emit_notification(
                            &callback_shared,
                            OwnedNotification::Codec(CodecEvent::Framed(event.to_owned())),
                        );
                    })
                    .map_err(|err| error("RASTERWAVE_DECODE_FAILED", err))?,
                DecoderBackend::Paper(codec) => codec
                    .mark_signal_lost(&mut |event: FaxPaperEventRef<'_>| {
                        emit_notification(
                            &callback_shared,
                            OwnedNotification::Codec(CodecEvent::Paper(event.to_owned())),
                        );
                    })
                    .map_err(|err| error("RASTERWAVE_DECODE_FAILED", err))?,
            };
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
                DecoderBackend::Framed(codec) => {
                    codec.finish(&mut |event: FaxDecodeEventRef<'_>| {
                        emit_notification(
                            &callback_shared,
                            OwnedNotification::Codec(CodecEvent::Framed(event.to_owned())),
                        );
                    })
                }
                DecoderBackend::Paper(codec) => codec.finish(&mut |event: FaxPaperEventRef<'_>| {
                    emit_notification(
                        &callback_shared,
                        OwnedNotification::Codec(CodecEvent::Paper(event.to_owned())),
                    );
                }),
            };
            if let Err(err) = result {
                deferred.reject(error("RASTERWAVE_DECODE_FAILED", err));
                return Ok(());
            }
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

fn emit_notification(shared: &DecoderShared, event: OwnedNotification) {
    if let Some(clock) = notification_clock(&event)
        && let Ok(mut state) = shared.clock_state.lock()
    {
        *state = clock;
    }
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

fn notification_clock(event: &OwnedNotification) -> Option<FaxClockCalibrationPoint> {
    match event {
        OwnedNotification::Codec(CodecEvent::Framed(FaxDecodeEvent::PhasingLocked {
            clock,
            ..
        }))
        | OwnedNotification::Codec(CodecEvent::Framed(FaxDecodeEvent::PageStarted {
            clock, ..
        })) => Some(*clock),
        OwnedNotification::Codec(CodecEvent::Paper(FaxPaperEvent::ClockCalibration {
            calibration,
            ..
        })) => Some(*calibration),
        _ => None,
    }
}

fn emit_barrier(shared: &DecoderShared, event: OwnedNotification, deferred: VoidDeferred) {
    let _ = shared.callback.call_with_return_value(
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

#[napi(object)]
pub struct FaxEncoderProgressSnapshot {
    pub samples_emitted: f64,
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
    finished: AtomicBool,
}

#[napi]
pub struct FaxEncoder {
    shared: Arc<EncoderShared>,
}

#[napi]
impl FaxEncoder {
    #[napi(constructor)]
    pub fn new(
        pixels: Uint8Array,
        width: u32,
        height: u32,
        spec: FaxSpecOptions,
        sample_rate: u32,
        options: Option<FaxEncoderOptions>,
    ) -> Result<Self> {
        let image = GrayImage::new(width, height, pixels.to_vec())
            .map_err(|err| error("RASTERWAVE_INVALID_IMAGE", err))?;
        let spec = fax_spec(&spec)?;
        let options = options.unwrap_or(FaxEncoderOptions {
            amplitude: None,
            include_apt: None,
            include_phasing: None,
        });
        let mut encode_options = FaxEncodeOptions::default();
        if let Some(value) = options.amplitude {
            encode_options.amplitude = value as f32;
        }
        if let Some(value) = options.include_apt {
            encode_options.include_apt = value;
        }
        if let Some(value) = options.include_phasing {
            encode_options.include_phasing = value;
        }
        let codec = CoreEncoder::new(image, spec, sample_rate, encode_options)
            .map_err(|err| error("RASTERWAVE_INVALID_CONFIG", err))?;
        Ok(Self {
            shared: Arc::new(EncoderShared {
                codec: Mutex::new(Some(codec)),
                queue: Mutex::new(EncoderQueue {
                    commands: VecDeque::new(),
                    running: false,
                }),
                accepting: AtomicBool::new(true),
                samples_emitted: AtomicU64::new(0),
                finished: AtomicBool::new(false),
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
    pub fn progress(&self) -> Result<FaxEncoderProgressSnapshot> {
        Ok(FaxEncoderProgressSnapshot {
            samples_emitted: safe_number(
                self.shared.samples_emitted.load(Ordering::Acquire),
                "samplesEmitted",
            )?,
            finished: self.shared.finished.load(Ordering::Acquire),
        })
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
                        shared
                            .samples_emitted
                            .store(codec.samples_emitted(), Ordering::Release);
                        shared
                            .finished
                            .store(codec.is_finished(), Ordering::Release);
                        Ok::<_, Error>(output)
                    }));
                    match result {
                        Ok(Ok(output)) => {
                            deferred.resolve(Box::new(move |_| Ok(Float32Array::new(output))))
                        }
                        Ok(Err(err)) => deferred.reject(err),
                        Err(_) => deferred.reject(error(
                            "RASTERWAVE_NATIVE_PANIC",
                            "fax encoder worker panicked",
                        )),
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

fn fax_lpm(value: u32) -> Result<FaxLpm> {
    let value = u16::try_from(value)
        .map_err(|_| error("RASTERWAVE_INVALID_CONFIG", "LPM does not fit u16"))?;
    FaxLpm::new(value).map_err(|err| error("RASTERWAVE_INVALID_CONFIG", err))
}

fn fax_modulation(options: &FaxModulationOptions) -> Result<FaxModulation> {
    match options.kind.as_str() {
        "fm" | "fmSubcarrier" => Ok(FaxModulation::FmSubcarrier {
            center_hz: options.center_hz.unwrap_or(1900.0) as f32,
            deviation_hz: options.deviation_hz.unwrap_or(400.0) as f32,
            polarity: options.polarity.unwrap_or(JsFaxPolarity::Normal).into(),
        }),
        "am" | "amSubcarrier" => Ok(FaxModulation::AmSubcarrier {
            carrier_hz: options.carrier_hz.unwrap_or(1900.0) as f32,
            black_level: options.black_level.unwrap_or(0.0) as f32,
            white_level: options.white_level.unwrap_or(1.0) as f32,
        }),
        other => Err(error(
            "RASTERWAVE_INVALID_CONFIG",
            format!("unsupported fax modulation: {other}"),
        )),
    }
}

fn fax_spec(options: &FaxSpecOptions) -> Result<FaxSpec> {
    let ioc: FaxIoc = options.ioc.into();
    let lpm = fax_lpm(options.lpm)?;
    let mut spec = FaxSpec::standard(ioc, lpm);
    if let Some(value) = &options.modulation {
        spec.modulation = fax_modulation(value)?;
    }
    if let Some(value) = options.phasing_seconds {
        spec.phasing_seconds = value as f32;
    }
    if let Some(value) = options.start_seconds {
        spec.start_seconds = value as f32;
    }
    if let Some(value) = options.stop_seconds {
        spec.stop_seconds = value as f32;
    }
    if let Some(value) = options.trailing_black_seconds {
        spec.trailing_black_seconds = value as f32;
    }
    if let Some(value) = options.dead_sector_fraction {
        spec.dead_sector_fraction = value as f32;
    }
    Ok(spec)
}

fn modulation_name(value: FaxModulation) -> &'static str {
    match value {
        FaxModulation::FmSubcarrier { .. } => "fm",
        FaxModulation::AmSubcarrier { .. } => "am",
    }
}

impl From<JsFaxIoc> for FaxIoc {
    fn from(value: JsFaxIoc) -> Self {
        match value {
            JsFaxIoc::Ioc288 => Self::Ioc288,
            JsFaxIoc::Ioc576 => Self::Ioc576,
        }
    }
}

impl From<FaxIoc> for JsFaxIoc {
    fn from(value: FaxIoc) -> Self {
        match value {
            FaxIoc::Ioc288 => Self::Ioc288,
            FaxIoc::Ioc576 => Self::Ioc576,
        }
    }
}

impl From<JsFaxPolarity> for FaxPolarity {
    fn from(value: JsFaxPolarity) -> Self {
        match value {
            JsFaxPolarity::Normal => Self::Normal,
            JsFaxPolarity::Inverted => Self::Inverted,
        }
    }
}
