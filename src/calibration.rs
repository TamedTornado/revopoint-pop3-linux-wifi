use crate::http_stream::{get_bounded_body, StreamError, StreamLimits};
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::net::SocketAddr;

const DEPTH_INTRINSICS_DOWNLOAD: &str = "/cgi-bin/zx_cmd.cgi?download=/data/camparam/Pl.bin";
const INTRINSICS_BYTES: usize = 40;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DepthIntrinsics {
    pub calibration_width: u16,
    pub calibration_height: u16,
    pub fx: f32,
    pub fy: f32,
    pub cx: f32,
    pub cy: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScaledDepthIntrinsics {
    pub width: u32,
    pub height: u32,
    pub fx: f32,
    pub fy: f32,
    pub cx: f32,
    pub cy: f32,
}

#[derive(Debug, Eq, PartialEq)]
pub struct CalibrationError;

impl Display for CalibrationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("scanner returned invalid depth intrinsics")
    }
}

impl Error for CalibrationError {}

#[derive(Debug)]
pub enum CalibrationQueryError {
    Http(StreamError),
    Invalid(CalibrationError),
}

impl Display for CalibrationQueryError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http(error) => write!(formatter, "download depth intrinsics: {error}"),
            Self::Invalid(error) => Display::fmt(error, formatter),
        }
    }
}

impl Error for CalibrationQueryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Http(error) => Some(error),
            Self::Invalid(error) => Some(error),
        }
    }
}

impl DepthIntrinsics {
    pub fn for_resolution(
        self,
        width: u32,
        height: u32,
    ) -> Result<ScaledDepthIntrinsics, CalibrationError> {
        if width == 0 || height == 0 || !self.is_valid() {
            return Err(CalibrationError);
        }
        let scale_x = width as f32 / f32::from(self.calibration_width);
        let scale_y = height as f32 / f32::from(self.calibration_height);
        Ok(ScaledDepthIntrinsics {
            width,
            height,
            fx: self.fx * scale_x,
            fy: self.fy * scale_y,
            cx: self.cx * scale_x,
            cy: self.cy * scale_y,
        })
    }

    fn is_valid(self) -> bool {
        self.calibration_width != 0
            && self.calibration_height != 0
            && self.fx.is_finite()
            && self.fx > 0.0
            && self.fy.is_finite()
            && self.fy > 0.0
            && self.cx.is_finite()
            && self.cy.is_finite()
    }
}

pub fn parse_depth_intrinsics(bytes: &[u8]) -> Result<DepthIntrinsics, CalibrationError> {
    if bytes.len() != INTRINSICS_BYTES {
        return Err(CalibrationError);
    }
    let matrix = std::array::from_fn::<_, 9, _>(|index| {
        let start = 4 + index * 4;
        f32::from_le_bytes(bytes[start..start + 4].try_into().expect("four-byte field"))
    });
    if matrix[1] != 0.0
        || matrix[3] != 0.0
        || matrix[6] != 0.0
        || matrix[7] != 0.0
        || matrix[8] != 1.0
    {
        return Err(CalibrationError);
    }
    let intrinsics = DepthIntrinsics {
        calibration_width: u16::from_le_bytes(bytes[0..2].try_into().expect("two-byte width")),
        calibration_height: u16::from_le_bytes(bytes[2..4].try_into().expect("two-byte height")),
        fx: matrix[0],
        fy: matrix[4],
        cx: matrix[2],
        cy: matrix[5],
    };
    intrinsics
        .is_valid()
        .then_some(intrinsics)
        .ok_or(CalibrationError)
}

pub fn get_depth_intrinsics(
    address: SocketAddr,
    limits: StreamLimits,
) -> Result<DepthIntrinsics, CalibrationQueryError> {
    let response = get_bounded_body(address, DEPTH_INTRINSICS_DOWNLOAD, limits)
        .map_err(CalibrationQueryError::Http)?;
    parse_depth_intrinsics(&response).map_err(CalibrationQueryError::Invalid)
}
