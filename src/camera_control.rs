use crate::http_stream::{get_bounded_body, StreamError, StreamLimits};
use serde::Deserialize;
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::net::SocketAddr;
use std::str::FromStr;

const GET_DEPTH_EXPOSURE_RANGE: &str = "/cgi-bin/zx_cmd.cgi?cam_type=mipi&get_exposureRange";
const DEPTH_EXPOSURE_REGISTER: u16 = 0x911;
const DEPTH_AUTO_EXPOSURE_REGISTER: u16 = 0x912;
const DEPTH_FRAME_TIME_REGISTER: u16 = 0x910;
const EXPOSURE_FRAME_MARGIN_US: u32 = 2_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DepthAutoExposure {
    Off,
    FixedFrameTime,
    HighQuality,
    Foreground,
}

impl DepthAutoExposure {
    const fn vendor_value(self) -> u8 {
        match self {
            Self::Off => 0,
            Self::FixedFrameTime => 1,
            Self::HighQuality => 2,
            Self::Foreground => 3,
        }
    }
}

impl Display for DepthAutoExposure {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Off => "off",
            Self::FixedFrameTime => "fixed-frame-time",
            Self::HighQuality => "high-quality",
            Self::Foreground => "foreground",
        })
    }
}

impl FromStr for DepthAutoExposure {
    type Err = DepthControlError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "off" => Ok(Self::Off),
            "fixed-frame-time" => Ok(Self::FixedFrameTime),
            "high-quality" => Ok(Self::HighQuality),
            "foreground" => Ok(Self::Foreground),
            _ => Err(DepthControlError::InvalidAutoExposureMode(value.to_owned())),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DepthControl {
    ManualExposureUs(u32),
    AutoExposure(DepthAutoExposure),
}

impl Default for DepthControl {
    fn default() -> Self {
        Self::AutoExposure(DepthAutoExposure::Foreground)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
pub struct DepthExposureRange {
    #[serde(rename = "min")]
    pub minimum_us: u32,
    #[serde(rename = "max")]
    pub maximum_us: u32,
    #[serde(rename = "step")]
    pub step_us: u32,
    #[serde(rename = "default")]
    pub default_us: u32,
}

impl DepthExposureRange {
    pub fn validate(self, exposure_us: u32) -> Result<(), DepthControlError> {
        if !self.is_valid() {
            return Err(DepthControlError::InvalidExposureRange);
        }
        if exposure_us < self.minimum_us
            || exposure_us > self.maximum_us
            || !(exposure_us - self.minimum_us).is_multiple_of(self.step_us)
        {
            return Err(DepthControlError::ExposureOutOfRange {
                exposure_us,
                range: self,
            });
        }
        Ok(())
    }

    fn is_valid(self) -> bool {
        self.minimum_us > 0
            && self.minimum_us <= self.maximum_us
            && self.step_us > 0
            && self.default_us >= self.minimum_us
            && self.default_us <= self.maximum_us
            && (self.default_us - self.minimum_us).is_multiple_of(self.step_us)
    }
}

#[derive(Debug)]
pub enum DepthControlError {
    Http {
        stage: &'static str,
        source: StreamError,
    },
    InvalidExposureRange,
    ExposureOutOfRange {
        exposure_us: u32,
        range: DepthExposureRange,
    },
    InvalidAutoExposureMode(String),
    Rejected(&'static str),
    FrameTimeOverflow,
}

impl Display for DepthControlError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http { stage, source } => write!(formatter, "{stage}: {source}"),
            Self::InvalidExposureRange => {
                formatter.write_str("scanner returned an invalid depth exposure range")
            }
            Self::ExposureOutOfRange { exposure_us, range } => write!(
                formatter,
                "depth exposure {exposure_us} us is outside {}..={} us in {} us steps",
                range.minimum_us, range.maximum_us, range.step_us
            ),
            Self::InvalidAutoExposureMode(mode) => write!(
                formatter,
                "invalid depth auto-exposure mode {mode:?}; expected off, fixed-frame-time, high-quality, or foreground"
            ),
            Self::Rejected(stage) => write!(formatter, "scanner rejected {stage}"),
            Self::FrameTimeOverflow => formatter.write_str("depth exposure frame time overflowed"),
        }
    }
}

impl Error for DepthControlError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Http { source, .. } => Some(source),
            _ => None,
        }
    }
}

pub fn depth_exposure_range(
    address: SocketAddr,
    limits: StreamLimits,
) -> Result<DepthExposureRange, DepthControlError> {
    let response =
        get_bounded_body(address, GET_DEPTH_EXPOSURE_RANGE, limits).map_err(|source| {
            DepthControlError::Http {
                stage: "query depth exposure range",
                source,
            }
        })?;
    let range: DepthExposureRange =
        serde_json::from_slice(&response).map_err(|_| DepthControlError::InvalidExposureRange)?;
    if !range.is_valid() {
        return Err(DepthControlError::InvalidExposureRange);
    }
    Ok(range)
}

pub fn set_depth_control(
    address: SocketAddr,
    limits: StreamLimits,
    control: DepthControl,
) -> Result<(), DepthControlError> {
    match control {
        DepthControl::AutoExposure(mode) => set_register(
            address,
            limits,
            DEPTH_AUTO_EXPOSURE_REGISTER,
            u32::from(mode.vendor_value()),
            "depth auto exposure",
        ),
        DepthControl::ManualExposureUs(exposure_us) => {
            depth_exposure_range(address, limits)?.validate(exposure_us)?;
            let frame_time_us = exposure_us
                .checked_add(EXPOSURE_FRAME_MARGIN_US)
                .ok_or(DepthControlError::FrameTimeOverflow)?;
            set_register(
                address,
                limits,
                DEPTH_AUTO_EXPOSURE_REGISTER,
                u32::from(DepthAutoExposure::Off.vendor_value()),
                "disable depth auto exposure",
            )?;
            set_register(
                address,
                limits,
                DEPTH_FRAME_TIME_REGISTER,
                frame_time_us,
                "depth frame time",
            )?;
            set_register(
                address,
                limits,
                DEPTH_EXPOSURE_REGISTER,
                exposure_us,
                "depth exposure",
            )
        }
    }
}

fn set_register(
    address: SocketAddr,
    limits: StreamLimits,
    register: u16,
    value: u32,
    stage: &'static str,
) -> Result<(), DepthControlError> {
    let path = format!(
        "/cgi-bin/zx_cmd.cgi?system_cmd=echo%20s%200x{register:x}%20{value}%20%3E%20/dev/rk_preisp"
    );
    let response = get_bounded_body(address, &path, limits)
        .map_err(|source| DepthControlError::Http { stage, source })?;
    if !response.trim_ascii().ends_with(b"[ok]") {
        return Err(DepthControlError::Rejected(stage));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_displays_auto_exposure_modes() {
        for (text, mode) in [
            ("off", DepthAutoExposure::Off),
            ("fixed-frame-time", DepthAutoExposure::FixedFrameTime),
            ("high-quality", DepthAutoExposure::HighQuality),
            ("foreground", DepthAutoExposure::Foreground),
        ] {
            assert_eq!(text.parse::<DepthAutoExposure>().expect("valid mode"), mode);
            assert_eq!(mode.to_string(), text);
        }
        assert!("automatic".parse::<DepthAutoExposure>().is_err());
    }
}
