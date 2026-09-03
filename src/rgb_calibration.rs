use crate::http_stream::{get_bounded_body, StreamError, StreamLimits};
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::net::SocketAddr;

const RGB_INTRINSICS_DOWNLOAD: &str = "/cgi-bin/zx_cmd.cgi?download=/data/camparam/Prgb.bin";
const RGB_DISTORTION_DOWNLOAD: &str = "/cgi-bin/zx_cmd.cgi?download=/data/camparam/Distort.bin";
const LEFT_TO_RGB_EXTRINSICS_DOWNLOAD: &str =
    "/cgi-bin/zx_cmd.cgi?download=/data/camparam/LC_RT.bin";
const INTRINSICS_BYTES: usize = 40;
const DISTORTION_BYTES: usize = 20;
const EXTRINSICS_BYTES: usize = 48;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RgbIntrinsics {
    pub calibration_width: u16,
    pub calibration_height: u16,
    pub fx: f32,
    pub fy: f32,
    pub cx: f32,
    pub cy: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RgbDistortion {
    pub coefficients: [f32; 5],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LeftToRgbExtrinsics {
    pub rotation: [f32; 9],
    pub translation_mm: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RgbCalibration {
    pub intrinsics: RgbIntrinsics,
    pub distortion: RgbDistortion,
    pub left_to_rgb: LeftToRgbExtrinsics,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RgbCalibrationError;

impl Display for RgbCalibrationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("scanner returned invalid RGB calibration")
    }
}

impl Error for RgbCalibrationError {}

#[derive(Debug)]
pub enum RgbCalibrationQueryError {
    Http {
        component: &'static str,
        source: StreamError,
    },
    Invalid {
        component: &'static str,
        source: RgbCalibrationError,
    },
}

impl Display for RgbCalibrationQueryError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http { component, source } => {
                write!(formatter, "download RGB {component}: {source}")
            }
            Self::Invalid { component, source } => {
                write!(formatter, "parse RGB {component}: {source}")
            }
        }
    }
}

impl Error for RgbCalibrationQueryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Http { source, .. } => Some(source),
            Self::Invalid { source, .. } => Some(source),
        }
    }
}

pub fn parse_rgb_intrinsics(bytes: &[u8]) -> Result<RgbIntrinsics, RgbCalibrationError> {
    if bytes.len() != INTRINSICS_BYTES {
        return Err(RgbCalibrationError);
    }
    let matrix = read_f32_array::<9>(bytes, 4);
    if matrix[1] != 0.0
        || matrix[3] != 0.0
        || matrix[6] != 0.0
        || matrix[7] != 0.0
        || matrix[8] != 1.0
    {
        return Err(RgbCalibrationError);
    }
    let intrinsics = RgbIntrinsics {
        calibration_width: u16::from_le_bytes(bytes[0..2].try_into().expect("two-byte width")),
        calibration_height: u16::from_le_bytes(bytes[2..4].try_into().expect("two-byte height")),
        fx: matrix[0],
        fy: matrix[4],
        cx: matrix[2],
        cy: matrix[5],
    };
    (intrinsics.calibration_width != 0
        && intrinsics.calibration_height != 0
        && intrinsics.fx.is_finite()
        && intrinsics.fx > 0.0
        && intrinsics.fy.is_finite()
        && intrinsics.fy > 0.0
        && intrinsics.cx.is_finite()
        && intrinsics.cy.is_finite())
    .then_some(intrinsics)
    .ok_or(RgbCalibrationError)
}

pub fn parse_rgb_distortion(bytes: &[u8]) -> Result<RgbDistortion, RgbCalibrationError> {
    if bytes.len() != DISTORTION_BYTES {
        return Err(RgbCalibrationError);
    }
    let distortion = RgbDistortion {
        coefficients: read_f32_array::<5>(bytes, 0),
    };
    distortion
        .coefficients
        .iter()
        .all(|value| value.is_finite())
        .then_some(distortion)
        .ok_or(RgbCalibrationError)
}

pub fn parse_left_to_rgb_extrinsics(
    bytes: &[u8],
) -> Result<LeftToRgbExtrinsics, RgbCalibrationError> {
    if bytes.len() != EXTRINSICS_BYTES {
        return Err(RgbCalibrationError);
    }
    let extrinsics = LeftToRgbExtrinsics {
        rotation: read_f32_array::<9>(bytes, 0),
        translation_mm: read_f32_array::<3>(bytes, 36),
    };
    if !extrinsics.rotation.iter().all(|value| value.is_finite())
        || !extrinsics
            .translation_mm
            .iter()
            .all(|value| value.is_finite())
        || determinant(extrinsics.rotation).abs() <= f32::EPSILON
    {
        return Err(RgbCalibrationError);
    }
    Ok(extrinsics)
}

pub fn get_rgb_calibration(
    address: SocketAddr,
    limits: StreamLimits,
) -> Result<RgbCalibration, RgbCalibrationQueryError> {
    let intrinsics = download(address, limits, RGB_INTRINSICS_DOWNLOAD, "intrinsics")?;
    let distortion = download(address, limits, RGB_DISTORTION_DOWNLOAD, "distortion")?;
    let extrinsics = download(
        address,
        limits,
        LEFT_TO_RGB_EXTRINSICS_DOWNLOAD,
        "left-to-RGB extrinsics",
    )?;
    Ok(RgbCalibration {
        intrinsics: parse_rgb_intrinsics(&intrinsics).map_err(|source| {
            RgbCalibrationQueryError::Invalid {
                component: "intrinsics",
                source,
            }
        })?,
        distortion: parse_rgb_distortion(&distortion).map_err(|source| {
            RgbCalibrationQueryError::Invalid {
                component: "distortion",
                source,
            }
        })?,
        left_to_rgb: parse_left_to_rgb_extrinsics(&extrinsics).map_err(|source| {
            RgbCalibrationQueryError::Invalid {
                component: "left-to-RGB extrinsics",
                source,
            }
        })?,
    })
}

fn download(
    address: SocketAddr,
    limits: StreamLimits,
    path: &'static str,
    component: &'static str,
) -> Result<Vec<u8>, RgbCalibrationQueryError> {
    get_bounded_body(address, path, limits)
        .map_err(|source| RgbCalibrationQueryError::Http { component, source })
}

fn read_f32_array<const N: usize>(bytes: &[u8], offset: usize) -> [f32; N] {
    std::array::from_fn(|index| {
        let start = offset + index * 4;
        f32::from_le_bytes(bytes[start..start + 4].try_into().expect("four-byte float"))
    })
}

fn determinant(matrix: [f32; 9]) -> f32 {
    matrix[0] * (matrix[4] * matrix[8] - matrix[5] * matrix[7])
        - matrix[1] * (matrix[3] * matrix[8] - matrix[5] * matrix[6])
        + matrix[2] * (matrix[3] * matrix[7] - matrix[4] * matrix[6])
}
