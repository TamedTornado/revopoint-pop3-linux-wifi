use crate::depth_decode::{DepthEncoding, DepthPlane};
use crate::stereo_calibration::ReprojectionMatrix;
use crate::stereo_match::DisparityMap;
use std::error::Error;
use std::fmt::{self, Display, Formatter};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StereoDepthError;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlanarDepthStatistics {
    pub valid_samples: usize,
    pub median_mm: f32,
    pub median_absolute_deviation_mm: f32,
    pub p10_mm: f32,
    pub p90_mm: f32,
}

impl Display for StereoDepthError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("disparity cannot be represented as metric Z16 depth")
    }
}

impl Error for StereoDepthError {}

pub fn reproject_z16(
    disparity: &DisparityMap,
    reprojection: ReprojectionMatrix,
    calibration_width: u32,
    calibration_height: u32,
) -> Result<DepthPlane, StereoDepthError> {
    let width = usize::try_from(disparity.width).map_err(|_| StereoDepthError)?;
    let height = usize::try_from(disparity.height).map_err(|_| StereoDepthError)?;
    let pixel_count = width.checked_mul(height).ok_or(StereoDepthError)?;
    if pixel_count == 0
        || disparity.values.len() != pixel_count
        || calibration_width == 0
        || calibration_height == 0
    {
        return Err(StereoDepthError);
    }
    let horizontal_scale = calibration_width as f32 / disparity.width as f32;
    let vertical_scale = calibration_height as f32 / disparity.height as f32;
    let stride_bytes = width.checked_mul(2).ok_or(StereoDepthError)?;
    let mut bytes = Vec::with_capacity(pixel_count * 2);

    for y in 0..height {
        for x in 0..width {
            let disparity_px = disparity.values[y * width + x];
            let depth_mm = (disparity_px != u16::MAX)
                .then(|| {
                    reprojection.point_mm(
                        x as f32,
                        y as f32,
                        f32::from(disparity_px),
                        horizontal_scale,
                        vertical_scale,
                    )
                })
                .flatten()
                .map(|point| point[2]);
            let sample = depth_mm
                .map(f32::round)
                .filter(|value| *value >= 1.0 && *value <= f32::from(u16::MAX))
                .map_or(0, |value| value as u16);
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
    }

    Ok(DepthPlane {
        width: disparity.width,
        height: disparity.height,
        stride_bytes,
        encoding: DepthEncoding::Z16LittleEndian,
        millimeters_per_unit: 1.0,
        bytes,
    })
}

pub fn encode_z16_pgm(depth: &DepthPlane) -> Result<Vec<u8>, StereoDepthError> {
    let stride = usize::try_from(depth.width)
        .ok()
        .and_then(|width| width.checked_mul(2))
        .ok_or(StereoDepthError)?;
    let expected = usize::try_from(depth.height)
        .ok()
        .and_then(|height| height.checked_mul(stride))
        .ok_or(StereoDepthError)?;
    if depth.width == 0
        || depth.height == 0
        || depth.stride_bytes != stride
        || depth.bytes.len() != expected
        || !depth.millimeters_per_unit.is_finite()
        || depth.millimeters_per_unit <= 0.0
    {
        return Err(StereoDepthError);
    }

    let header = format!("P5\n{} {}\n65535\n", depth.width, depth.height);
    let mut pgm = Vec::with_capacity(header.len() + expected);
    pgm.extend_from_slice(header.as_bytes());
    for sample in depth.bytes.as_chunks::<2>().0 {
        let raw = u16::from_le_bytes(*sample);
        let millimeters = (f32::from(raw) * depth.millimeters_per_unit).round();
        let metric_sample = if millimeters <= f32::from(u16::MAX) {
            millimeters as u16
        } else {
            0
        };
        pgm.extend_from_slice(&metric_sample.to_be_bytes());
    }
    Ok(pgm)
}

pub fn depth_z_statistics(depth: &DepthPlane) -> Result<PlanarDepthStatistics, StereoDepthError> {
    let stride = usize::try_from(depth.width)
        .ok()
        .and_then(|width| width.checked_mul(2))
        .ok_or(StereoDepthError)?;
    let expected = usize::try_from(depth.height)
        .ok()
        .and_then(|height| height.checked_mul(stride))
        .ok_or(StereoDepthError)?;
    if depth.stride_bytes != stride
        || depth.bytes.len() != expected
        || !depth.millimeters_per_unit.is_finite()
        || depth.millimeters_per_unit <= 0.0
    {
        return Err(StereoDepthError);
    }
    let mut samples = depth
        .bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|sample| u16::from_le_bytes(*sample))
        .filter(|sample| *sample != 0)
        .map(|sample| f32::from(sample) * depth.millimeters_per_unit)
        .collect::<Vec<_>>();
    if samples.is_empty() {
        return Err(StereoDepthError);
    }
    samples.sort_by(f32::total_cmp);
    let median_mm = samples[samples.len() / 2];
    let mut absolute_deviations = samples
        .iter()
        .map(|sample| (sample - median_mm).abs())
        .collect::<Vec<_>>();
    absolute_deviations.sort_by(f32::total_cmp);
    let median_absolute_deviation_mm = absolute_deviations[absolute_deviations.len() / 2];
    let last = samples.len() - 1;
    Ok(PlanarDepthStatistics {
        valid_samples: samples.len(),
        median_mm,
        median_absolute_deviation_mm,
        p10_mm: samples[last / 10],
        p90_mm: samples[last * 9 / 10],
    })
}
