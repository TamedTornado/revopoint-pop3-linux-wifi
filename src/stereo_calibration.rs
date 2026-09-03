use crate::http_stream::{get_bounded_body, StreamError, StreamLimits};
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::net::SocketAddr;

const MAP_PARAMETER_BYTES: usize = 148;
const REPROJECTION_MATRIX_BYTES: usize = 64;
const DISTORTION_COEFFICIENTS: u32 = 5;
const LEFT_MAP_DOWNLOAD: &str = "/cgi-bin/zx_cmd.cgi?download=/data/camparam/mapparamL.bin";
const RIGHT_MAP_DOWNLOAD: &str = "/cgi-bin/zx_cmd.cgi?download=/data/camparam/mapparamR.bin";
const REPROJECTION_MATRIX_DOWNLOAD: &str =
    "/cgi-bin/zx_cmd.cgi?download=/data/camparam/camparamLR/Q.bin";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapParameters {
    pub calibration_width: u32,
    pub calibration_height: u32,
    pub camera_matrix: [f32; 9],
    pub distortion: [f32; 5],
    pub inverse_rectification: [f32; 9],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StereoMapParameters {
    pub left: MapParameters,
    pub right: MapParameters,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReprojectionMatrix {
    pub values: [f32; 16],
}

impl ReprojectionMatrix {
    pub fn depth_mm(self, disparity_px: f32, disparity_scale: f32) -> Option<f32> {
        self.point_mm(0.0, 0.0, disparity_px, disparity_scale, disparity_scale)
            .map(|point| point[2])
    }

    pub fn point_mm(
        self,
        pixel_x: f32,
        pixel_y: f32,
        disparity_px: f32,
        horizontal_scale: f32,
        vertical_scale: f32,
    ) -> Option<[f32; 3]> {
        if !pixel_x.is_finite()
            || pixel_x < 0.0
            || !pixel_y.is_finite()
            || pixel_y < 0.0
            || !disparity_px.is_finite()
            || disparity_px < 0.0
            || !horizontal_scale.is_finite()
            || horizontal_scale <= 0.0
            || !vertical_scale.is_finite()
            || vertical_scale <= 0.0
        {
            return None;
        }
        let input = [
            pixel_x * horizontal_scale,
            pixel_y * vertical_scale,
            disparity_px * horizontal_scale,
            1.0,
        ];
        let projected = std::array::from_fn::<f32, 4, _>(|row| {
            (0..4)
                .map(|column| self.values[row * 4 + column] * input[column])
                .sum()
        });
        let homogeneous_scale = projected[3];
        if !homogeneous_scale.is_finite() || homogeneous_scale.abs() <= f32::EPSILON {
            return None;
        }
        let point = [
            projected[0] / homogeneous_scale,
            projected[1] / homogeneous_scale,
            projected[2] / homogeneous_scale,
        ];
        if !point.iter().all(|value| value.is_finite()) || point[2] <= 0.0 {
            return None;
        }
        Some(point)
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct MapParameterError;

impl Display for MapParameterError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("scanner returned invalid stereo map parameters")
    }
}

impl Error for MapParameterError {}

#[derive(Debug)]
pub enum MapParameterQueryError {
    Http {
        side: &'static str,
        source: StreamError,
    },
    Invalid {
        side: &'static str,
        source: MapParameterError,
    },
}

impl Display for MapParameterQueryError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http { side, source } => {
                write!(formatter, "download {side} stereo map: {source}")
            }
            Self::Invalid { side, source } => {
                write!(formatter, "parse {side} stereo map: {source}")
            }
        }
    }
}

impl Error for MapParameterQueryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Http { source, .. } => Some(source),
            Self::Invalid { source, .. } => Some(source),
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct RectificationError;

impl Display for RectificationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid Y8 image or stereo rectification parameters")
    }
}

impl Error for RectificationError {}

pub fn parse_map_parameters(bytes: &[u8]) -> Result<MapParameters, MapParameterError> {
    if bytes.len() != MAP_PARAMETER_BYTES || read_u32(bytes, 8) != DISTORTION_COEFFICIENTS {
        return Err(MapParameterError);
    }

    let calibration_height = read_u32(bytes, 0);
    let calibration_width = read_u32(bytes, 4);
    let cx = read_f32(bytes, 48);
    let cy = read_f32(bytes, 52);
    let fx = read_f32(bytes, 56);
    let fy = read_f32(bytes, 60);
    let distortion = read_f32_array::<5>(bytes, 64);
    let inverse_rectification = read_f32_array::<9>(bytes, 112);
    let parameters = MapParameters {
        calibration_width,
        calibration_height,
        camera_matrix: [fx, 0.0, cx, 0.0, fy, cy, 0.0, 0.0, 1.0],
        distortion,
        inverse_rectification,
    };

    if calibration_width == 0 || calibration_height == 0 {
        return Err(MapParameterError);
    }
    if !fx.is_finite() || fx <= 0.0 || !fy.is_finite() || fy <= 0.0 {
        return Err(MapParameterError);
    }
    if !cx.is_finite() || !cy.is_finite() {
        return Err(MapParameterError);
    }
    if !distortion.iter().all(|value| value.is_finite()) {
        return Err(MapParameterError);
    }
    if !inverse_rectification.iter().all(|value| value.is_finite()) {
        return Err(MapParameterError);
    }
    if determinant(inverse_rectification).abs() <= f32::EPSILON {
        return Err(MapParameterError);
    }
    Ok(parameters)
}

pub fn parse_reprojection_matrix(bytes: &[u8]) -> Result<ReprojectionMatrix, MapParameterError> {
    if bytes.len() != REPROJECTION_MATRIX_BYTES {
        return Err(MapParameterError);
    }
    let values = read_f32_array::<16>(bytes, 0);
    if !values.iter().all(|value| value.is_finite()) {
        return Err(MapParameterError);
    }
    if values[0] == 0.0 || values[5] == 0.0 {
        return Err(MapParameterError);
    }
    if values[11] <= 0.0 || values[14] == 0.0 {
        return Err(MapParameterError);
    }
    Ok(ReprojectionMatrix { values })
}

pub fn get_stereo_map_parameters(
    address: SocketAddr,
    limits: StreamLimits,
) -> Result<StereoMapParameters, MapParameterQueryError> {
    let left = get_bounded_body(address, LEFT_MAP_DOWNLOAD, limits).map_err(|source| {
        MapParameterQueryError::Http {
            side: "left",
            source,
        }
    })?;
    let right = get_bounded_body(address, RIGHT_MAP_DOWNLOAD, limits).map_err(|source| {
        MapParameterQueryError::Http {
            side: "right",
            source,
        }
    })?;
    Ok(StereoMapParameters {
        left: parse_map_parameters(&left).map_err(|source| MapParameterQueryError::Invalid {
            side: "left",
            source,
        })?,
        right: parse_map_parameters(&right).map_err(|source| MapParameterQueryError::Invalid {
            side: "right",
            source,
        })?,
    })
}

pub fn get_reprojection_matrix(
    address: SocketAddr,
    limits: StreamLimits,
) -> Result<ReprojectionMatrix, MapParameterQueryError> {
    let bytes = get_bounded_body(address, REPROJECTION_MATRIX_DOWNLOAD, limits)
        .map_err(|source| MapParameterQueryError::Http { side: "Q", source })?;
    parse_reprojection_matrix(&bytes)
        .map_err(|source| MapParameterQueryError::Invalid { side: "Q", source })
}

pub fn rectify_y8(
    input: &[u8],
    width: u32,
    height: u32,
    parameters: MapParameters,
) -> Result<Vec<u8>, RectificationError> {
    let pixel_count = usize::try_from(width)
        .ok()
        .and_then(|width| {
            usize::try_from(height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .filter(|pixel_count| *pixel_count != 0 && *pixel_count == input.len())
        .ok_or(RectificationError)?;
    let [fx, _, cx, _, fy, cy, _, _, _] = parameters.camera_matrix;
    if parameters.calibration_width == 0
        || parameters.calibration_height == 0
        || !fx.is_finite()
        || fx <= 0.0
        || !fy.is_finite()
        || fy <= 0.0
        || !cx.is_finite()
        || !cy.is_finite()
        || !parameters.distortion.iter().all(|value| value.is_finite())
        || !parameters
            .inverse_rectification
            .iter()
            .all(|value| value.is_finite())
        || determinant(parameters.inverse_rectification).abs() <= f32::EPSILON
    {
        return Err(RectificationError);
    }

    let source_scale_x = width as f32 / parameters.calibration_width as f32;
    let source_scale_y = height as f32 / parameters.calibration_height as f32;
    let target_scale_x = parameters.calibration_width as f32 / width as f32;
    let target_scale_y = parameters.calibration_height as f32 / height as f32;
    let [h00, h01, h02, h10, h11, h12, h20, h21, h22] = parameters.inverse_rectification;
    let [k1, k2, p1, p2, k3] = parameters.distortion;
    let mut output = vec![0_u8; pixel_count];

    for target_y in 0..height {
        for target_x in 0..width {
            let rectified_x = target_x as f32 * target_scale_x;
            let rectified_y = target_y as f32 * target_scale_y;
            let denominator = h20 * rectified_x + h21 * rectified_y + h22;
            if !denominator.is_finite() || denominator.abs() <= f32::EPSILON {
                continue;
            }
            let x = (h00 * rectified_x + h01 * rectified_y + h02) / denominator;
            let y = (h10 * rectified_x + h11 * rectified_y + h12) / denominator;
            let radius_squared = x * x + y * y;
            let radial = 1.0
                + k1 * radius_squared
                + k2 * radius_squared * radius_squared
                + k3 * radius_squared * radius_squared * radius_squared;
            let distorted_x = x * radial + 2.0 * p1 * x * y + p2 * (radius_squared + 2.0 * x * x);
            let distorted_y = y * radial + p1 * (radius_squared + 2.0 * y * y) + 2.0 * p2 * x * y;
            let source_x = (fx * distorted_x + cx) * source_scale_x;
            let source_y = (fy * distorted_y + cy) * source_scale_y;
            if !source_x.is_finite() || !source_y.is_finite() {
                continue;
            }
            let source_x0 = source_x.floor() as i64;
            let source_y0 = source_y.floor() as i64;
            let source_x1 = source_x.ceil() as i64;
            let source_y1 = source_y.ceil() as i64;
            if source_x0 < 0
                || source_y0 < 0
                || source_x1 >= i64::from(width)
                || source_y1 >= i64::from(height)
            {
                continue;
            }
            let fraction_x = source_x - source_x0 as f32;
            let fraction_y = source_y - source_y0 as f32;
            let sample = |x: i64, y: i64| input[y as usize * width as usize + x as usize] as f32;
            let top = sample(source_x0, source_y0) * (1.0 - fraction_x)
                + sample(source_x1, source_y0) * fraction_x;
            let bottom = sample(source_x0, source_y1) * (1.0 - fraction_x)
                + sample(source_x1, source_y1) * fraction_x;
            let target_index = target_y as usize * width as usize + target_x as usize;
            output[target_index] = (top * (1.0 - fraction_y) + bottom * fraction_y).round() as u8;
        }
    }
    Ok(output)
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("four-byte field"),
    )
}

fn read_f32(bytes: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("four-byte field"),
    )
}

fn read_f32_array<const N: usize>(bytes: &[u8], offset: usize) -> [f32; N] {
    std::array::from_fn(|index| read_f32(bytes, offset + index * 4))
}

fn determinant(matrix: [f32; 9]) -> f32 {
    matrix[0] * (matrix[4] * matrix[8] - matrix[5] * matrix[7])
        - matrix[1] * (matrix[3] * matrix[8] - matrix[5] * matrix[6])
        + matrix[2] * (matrix[3] * matrix[7] - matrix[4] * matrix[6])
}
