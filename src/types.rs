use napi::bindgen_prelude::*;
use napi_derive::napi;
use rasterwave::{ColorLayout, ModeStatus, SSTV_MODES, ScanLayout, SstvMode};

use crate::runtime::error;

#[napi(js_name = "SstvMode", string_enum = "camelCase")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JsSstvMode {
    Robot8Bw,
    Robot12Bw,
    Robot24Bw,
    Robot36Bw,
    Robot12,
    Robot24,
    Robot36,
    Robot72,
    Martin1,
    Martin2,
    Martin3,
    Martin4,
    Scottie1,
    Scottie2,
    Scottie3,
    Scottie4,
    ScottieDx,
    Pd50,
    Pd90,
    Pd120,
    Pd160,
    Pd180,
    Pd240,
    Pd290,
    #[napi(value = "wraaseSc2_30")]
    WraaseSc2_30,
    #[napi(value = "wraaseSc2_60")]
    WraaseSc2_60,
    #[napi(value = "wraaseSc2_120")]
    WraaseSc2_120,
    #[napi(value = "wraaseSc2_180")]
    WraaseSc2_180,
    Pasokon3,
    Pasokon5,
    Pasokon7,
}

impl From<JsSstvMode> for SstvMode {
    fn from(value: JsSstvMode) -> Self {
        match value {
            JsSstvMode::Robot8Bw => Self::Robot8Bw,
            JsSstvMode::Robot12Bw => Self::Robot12Bw,
            JsSstvMode::Robot24Bw => Self::Robot24Bw,
            JsSstvMode::Robot36Bw => Self::Robot36Bw,
            JsSstvMode::Robot12 => Self::Robot12,
            JsSstvMode::Robot24 => Self::Robot24,
            JsSstvMode::Robot36 => Self::Robot36,
            JsSstvMode::Robot72 => Self::Robot72,
            JsSstvMode::Martin1 => Self::Martin1,
            JsSstvMode::Martin2 => Self::Martin2,
            JsSstvMode::Martin3 => Self::Martin3,
            JsSstvMode::Martin4 => Self::Martin4,
            JsSstvMode::Scottie1 => Self::Scottie1,
            JsSstvMode::Scottie2 => Self::Scottie2,
            JsSstvMode::Scottie3 => Self::Scottie3,
            JsSstvMode::Scottie4 => Self::Scottie4,
            JsSstvMode::ScottieDx => Self::ScottieDx,
            JsSstvMode::Pd50 => Self::Pd50,
            JsSstvMode::Pd90 => Self::Pd90,
            JsSstvMode::Pd120 => Self::Pd120,
            JsSstvMode::Pd160 => Self::Pd160,
            JsSstvMode::Pd180 => Self::Pd180,
            JsSstvMode::Pd240 => Self::Pd240,
            JsSstvMode::Pd290 => Self::Pd290,
            JsSstvMode::WraaseSc2_30 => Self::WraaseSc2_30,
            JsSstvMode::WraaseSc2_60 => Self::WraaseSc2_60,
            JsSstvMode::WraaseSc2_120 => Self::WraaseSc2_120,
            JsSstvMode::WraaseSc2_180 => Self::WraaseSc2_180,
            JsSstvMode::Pasokon3 => Self::Pasokon3,
            JsSstvMode::Pasokon5 => Self::Pasokon5,
            JsSstvMode::Pasokon7 => Self::Pasokon7,
        }
    }
}

impl TryFrom<SstvMode> for JsSstvMode {
    type Error = napi::Error;

    fn try_from(value: SstvMode) -> Result<Self> {
        Ok(match value {
            SstvMode::Robot8Bw => Self::Robot8Bw,
            SstvMode::Robot12Bw => Self::Robot12Bw,
            SstvMode::Robot24Bw => Self::Robot24Bw,
            SstvMode::Robot36Bw => Self::Robot36Bw,
            SstvMode::Robot12 => Self::Robot12,
            SstvMode::Robot24 => Self::Robot24,
            SstvMode::Robot36 => Self::Robot36,
            SstvMode::Robot72 => Self::Robot72,
            SstvMode::Martin1 => Self::Martin1,
            SstvMode::Martin2 => Self::Martin2,
            SstvMode::Martin3 => Self::Martin3,
            SstvMode::Martin4 => Self::Martin4,
            SstvMode::Scottie1 => Self::Scottie1,
            SstvMode::Scottie2 => Self::Scottie2,
            SstvMode::Scottie3 => Self::Scottie3,
            SstvMode::Scottie4 => Self::Scottie4,
            SstvMode::ScottieDx => Self::ScottieDx,
            SstvMode::Pd50 => Self::Pd50,
            SstvMode::Pd90 => Self::Pd90,
            SstvMode::Pd120 => Self::Pd120,
            SstvMode::Pd160 => Self::Pd160,
            SstvMode::Pd180 => Self::Pd180,
            SstvMode::Pd240 => Self::Pd240,
            SstvMode::Pd290 => Self::Pd290,
            SstvMode::WraaseSc2_30 => Self::WraaseSc2_30,
            SstvMode::WraaseSc2_60 => Self::WraaseSc2_60,
            SstvMode::WraaseSc2_120 => Self::WraaseSc2_120,
            SstvMode::WraaseSc2_180 => Self::WraaseSc2_180,
            SstvMode::Pasokon3 => Self::Pasokon3,
            SstvMode::Pasokon5 => Self::Pasokon5,
            SstvMode::Pasokon7 => Self::Pasokon7,
            _ => return Err(error("RASTERWAVE_UNSUPPORTED_MODE", "unknown SSTV mode")),
        })
    }
}

#[napi(object)]
pub struct SstvModeInfo {
    pub mode: JsSstvMode,
    pub name: String,
    pub vis_code: u32,
    pub width: u32,
    pub height: u32,
    pub color_layout: String,
    pub scan_layout: String,
    pub line_seconds: f64,
    pub rows_per_line: u32,
    pub status: String,
}

#[napi]
pub fn sstv_modes() -> Result<Vec<SstvModeInfo>> {
    SSTV_MODES
        .iter()
        .map(|spec| {
            Ok(SstvModeInfo {
                mode: spec.mode.try_into()?,
                name: spec.name.to_owned(),
                vis_code: u32::from(spec.vis_code),
                width: spec.width,
                height: spec.height,
                color_layout: match spec.color {
                    ColorLayout::Monochrome => "monochrome",
                    ColorLayout::Rgb => "rgb",
                    ColorLayout::Yuv => "yuv",
                }
                .to_owned(),
                scan_layout: match spec.layout {
                    ScanLayout::Monochrome { .. } => "monochrome",
                    ScanLayout::Martin { .. } => "martin",
                    ScanLayout::Scottie { .. } => "scottie",
                    ScanLayout::Robot { .. } => "robot",
                    ScanLayout::Pd { .. } => "pd",
                    ScanLayout::Wraase { .. } => "wraase",
                    ScanLayout::Pasokon { .. } => "pasokon",
                }
                .to_owned(),
                line_seconds: spec.line_seconds,
                rows_per_line: u32::from(spec.rows_per_line),
                status: match spec.status() {
                    ModeStatus::Canonical => "canonical",
                    ModeStatus::Compatibility => "compatibility",
                }
                .to_owned(),
            })
        })
        .collect()
}

#[napi(object)]
#[derive(Clone)]
pub struct SstvDecoderOptions {
    pub immediate_decode: Option<bool>,
    pub detect_vis: Option<bool>,
    pub detect_sync_timing: Option<bool>,
    pub manual_mode: Option<JsSstvMode>,
    pub minimum_signal_level: Option<f64>,
    pub queue_capacity_samples: Option<u32>,
}

#[napi(object)]
#[derive(Clone)]
pub struct SstvEncoderOptions {
    pub amplitude: Option<f64>,
    pub tone_offset_hz: Option<f64>,
    pub include_vis_header: Option<bool>,
}

#[napi(js_name = "FaxIoc", string_enum = "camelCase")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JsFaxIoc {
    Ioc288,
    Ioc576,
}

#[napi(js_name = "FaxPolarity", string_enum = "camelCase")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JsFaxPolarity {
    Normal,
    Inverted,
}

#[napi(object)]
#[derive(Clone)]
pub struct FaxModulationOptions {
    pub kind: String,
    pub center_hz: Option<f64>,
    pub deviation_hz: Option<f64>,
    pub polarity: Option<JsFaxPolarity>,
    pub carrier_hz: Option<f64>,
    pub black_level: Option<f64>,
    pub white_level: Option<f64>,
}

#[napi(object)]
#[derive(Clone)]
pub struct FaxSpecOptions {
    pub ioc: JsFaxIoc,
    pub lpm: u32,
    pub modulation: Option<FaxModulationOptions>,
    pub phasing_seconds: Option<f64>,
    pub start_seconds: Option<f64>,
    pub stop_seconds: Option<f64>,
    pub trailing_black_seconds: Option<f64>,
    pub dead_sector_fraction: Option<f64>,
}

#[napi(object)]
#[derive(Clone)]
pub struct FaxEncoderOptions {
    pub amplitude: Option<f64>,
    pub include_apt: Option<bool>,
    pub include_phasing: Option<bool>,
}

#[napi(object)]
#[derive(Clone)]
pub struct FaxDecoderOptions {
    pub immediate_decode: Option<bool>,
    pub ioc: Option<JsFaxIoc>,
    pub lpm: Option<u32>,
    pub modulation: Option<FaxModulationOptions>,
    pub max_lines: Option<u32>,
    pub am_full_scale: Option<f64>,
    pub expected_phasing_seconds: Option<f64>,
    pub apt_confirm_seconds: Option<f64>,
    pub acquisition_timeout_seconds: Option<f64>,
    pub stop_confirm_seconds: Option<f64>,
    pub signal_loss_seconds: Option<f64>,
    pub minimum_signal_level: Option<f64>,
    pub minimum_carrier_coherence: Option<f64>,
    pub queue_capacity_samples: Option<u32>,
}
