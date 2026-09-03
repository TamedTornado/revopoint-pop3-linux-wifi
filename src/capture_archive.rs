use crate::calibration::ScaledDepthIntrinsics;
use crate::depth_decode::{DepthEncoding, DepthPlane};
use crate::rgb_calibration::{LeftToRgbExtrinsics, RgbCalibration, RgbDistortion, RgbIntrinsics};
use crate::rgb_registration::{colorize_depth, decode_jpeg_rgb, encode_binary_ply};
use crate::rgbd_pair::{pair_timestamps, PairingPolicy};
use crate::stereo_depth::encode_z16_pgm;
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs;
use std::path::{Path, PathBuf};

const MANIFEST_FILE: &str = "manifest.json";
const DEPTH_FILE: &str = "depth.z16le";
const DEPTH_PGM_FILE: &str = "depth-mm.pgm";
const RGB_FILE: &str = "rgb.jpg";
const COLORED_FILE: &str = "colored.ply";
const AXIS_SQUARED_TOLERANCE: f32 = 0.015625;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArchiveManifest {
    pub schema_version: u32,
    pub depth_timestamp_ms: u32,
    pub rgb_timestamp_ms: u32,
    pub timestamp_delta_ms: u32,
    pub depth: DepthRecord,
    pub rgb: RgbRecord,
    pub depth_to_rgb: ExtrinsicsRecord,
    pub colored_ply_file: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turntable: Option<TurntableRecord>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TurntableRecord {
    pub session_id: String,
    pub frame_index: u32,
    pub expected_frame_count: u32,
    pub commanded_angle_degrees: f32,
    pub observed_angle_degrees: Option<f32>,
    pub direction: RotationDirection,
    pub axis_depth_camera: [f32; 3],
    pub center_mm_depth_camera: [f32; 3],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RotationDirection {
    ClockwiseViewedFromAxisTip,
    CounterclockwiseViewedFromAxisTip,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DepthRecord {
    pub width: u32,
    pub height: u32,
    pub millimeters_per_unit: f32,
    pub intrinsics: [f32; 4],
    pub raw_file: String,
    pub pgm_file: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RgbRecord {
    pub width: u32,
    pub height: u32,
    pub calibration_width: u16,
    pub calibration_height: u16,
    pub intrinsics: [f32; 4],
    pub distortion: [f32; 5],
    pub jpeg_file: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExtrinsicsRecord {
    pub rotation: [f32; 9],
    pub translation_mm: [f32; 3],
}

pub struct ArchiveFrame<'a> {
    pub manifest: ArchiveManifest,
    pub depth_raw: &'a [u8],
    pub rgb_jpeg: &'a [u8],
}

#[derive(Debug)]
pub struct ArchiveError(String);

impl Display for ArchiveError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for ArchiveError {}

pub fn write_frame_archive(
    root: &Path,
    name: &str,
    frame: ArchiveFrame<'_>,
) -> Result<PathBuf, ArchiveError> {
    if !valid_frame_name(name) {
        return Err(fail("archive frame name is invalid"));
    }
    let (colored_ply, depth_pgm) = build_outputs(&frame.manifest, frame.depth_raw, frame.rgb_jpeg)?;
    fs::create_dir_all(root).map_err(io_fail)?;
    let destination = root.join(name);
    let partial = root.join(format!(".{name}.partial"));
    if destination.exists() {
        return Err(fail("archive frame already exists"));
    }
    if partial.exists() {
        return Err(fail("archive frame already exists"));
    }
    fs::create_dir(&partial).map_err(io_fail)?;
    let publication = (|| {
        fs::write(partial.join(DEPTH_FILE), frame.depth_raw).map_err(io_fail)?;
        fs::write(partial.join(DEPTH_PGM_FILE), depth_pgm).map_err(io_fail)?;
        fs::write(partial.join(RGB_FILE), frame.rgb_jpeg).map_err(io_fail)?;
        fs::write(partial.join(COLORED_FILE), colored_ply).map_err(io_fail)?;
        let manifest = serde_json::to_vec_pretty(&frame.manifest).map_err(json_fail)?;
        fs::write(partial.join(MANIFEST_FILE), manifest).map_err(io_fail)?;
        fs::rename(&partial, &destination).map_err(io_fail)
    })();
    if publication.is_err() {
        let _ = fs::remove_dir_all(&partial);
    }
    publication.map(|()| destination)
}

pub fn replay_colored_ply(directory: &Path) -> Result<Vec<u8>, ArchiveError> {
    let manifest = fs::read(directory.join(MANIFEST_FILE)).map_err(io_fail)?;
    let manifest: ArchiveManifest = serde_json::from_slice(&manifest).map_err(json_fail)?;
    validate_manifest(&manifest)?;
    let depth_raw = fs::read(directory.join(&manifest.depth.raw_file)).map_err(io_fail)?;
    let rgb_jpeg = fs::read(directory.join(&manifest.rgb.jpeg_file)).map_err(io_fail)?;
    build_outputs(&manifest, &depth_raw, &rgb_jpeg).map(|(colored_ply, _)| colored_ply)
}

fn build_outputs(
    manifest: &ArchiveManifest,
    depth_raw: &[u8],
    rgb_jpeg: &[u8],
) -> Result<(Vec<u8>, Vec<u8>), ArchiveError> {
    validate_manifest(manifest)?;
    let depth = DepthPlane {
        width: manifest.depth.width,
        height: manifest.depth.height,
        stride_bytes: usize::try_from(manifest.depth.width)
            .ok()
            .and_then(|width| width.checked_mul(2))
            .ok_or_else(|| fail("archive depth stride is invalid"))?,
        encoding: DepthEncoding::Z16LittleEndian,
        millimeters_per_unit: manifest.depth.millimeters_per_unit,
        bytes: depth_raw.to_vec(),
    };
    let depth_intrinsics = ScaledDepthIntrinsics {
        width: manifest.depth.width,
        height: manifest.depth.height,
        fx: manifest.depth.intrinsics[0],
        fy: manifest.depth.intrinsics[1],
        cx: manifest.depth.intrinsics[2],
        cy: manifest.depth.intrinsics[3],
    };
    let rgb = decode_jpeg_rgb(rgb_jpeg).map_err(|_| fail("archive RGB JPEG is invalid"))?;
    if rgb.width != manifest.rgb.width || rgb.height != manifest.rgb.height {
        return Err(fail("archive RGB dimensions disagree with its manifest"));
    }
    let calibration = RgbCalibration {
        intrinsics: RgbIntrinsics {
            calibration_width: manifest.rgb.calibration_width,
            calibration_height: manifest.rgb.calibration_height,
            fx: manifest.rgb.intrinsics[0],
            fy: manifest.rgb.intrinsics[1],
            cx: manifest.rgb.intrinsics[2],
            cy: manifest.rgb.intrinsics[3],
        },
        distortion: RgbDistortion {
            coefficients: manifest.rgb.distortion,
        },
        left_to_rgb: LeftToRgbExtrinsics {
            rotation: manifest.depth_to_rgb.rotation,
            translation_mm: manifest.depth_to_rgb.translation_mm,
        },
    };
    let points = colorize_depth(&depth, depth_intrinsics, &rgb, calibration)
        .map_err(|_| fail("archive RGB-D registration is invalid"))?;
    if points.is_empty() {
        return Err(fail("archive RGB-D frame has no registered points"));
    }
    let depth_pgm = encode_z16_pgm(&depth).map_err(|_| fail("archive depth PGM is invalid"))?;
    Ok((encode_binary_ply(&points), depth_pgm))
}

fn validate_manifest(manifest: &ArchiveManifest) -> Result<(), ArchiveError> {
    let pair = pair_timestamps(
        manifest.depth_timestamp_ms,
        manifest.rgb_timestamp_ms,
        PairingPolicy::default(),
    )
    .ok_or_else(|| fail("archive timestamps do not form a valid pair"))?;
    if manifest.schema_version != 1
        || manifest.timestamp_delta_ms != pair.absolute_delta_ms
        || manifest.depth.raw_file != DEPTH_FILE
        || manifest.depth.pgm_file != DEPTH_PGM_FILE
        || manifest.rgb.jpeg_file != RGB_FILE
        || manifest.colored_ply_file != COLORED_FILE
    {
        return Err(fail("archive manifest is inconsistent"));
    }
    if let Some(turntable) = &manifest.turntable {
        validate_turntable_record(turntable)?;
    }
    Ok(())
}

pub fn validate_turntable_record(turntable: &TurntableRecord) -> Result<(), ArchiveError> {
    let angle_is_valid = |angle: f32| angle.is_finite() && (0.0..360.0).contains(&angle);
    let axis_squared = turntable
        .axis_depth_camera
        .iter()
        .map(|component| component * component)
        .sum::<f32>();
    if !valid_frame_name(&turntable.session_id)
        || turntable.expected_frame_count == 0
        || turntable.frame_index >= turntable.expected_frame_count
        || !angle_is_valid(turntable.commanded_angle_degrees)
        || turntable
            .observed_angle_degrees
            .is_some_and(|angle| !angle_is_valid(angle))
        || !axis_squared.is_finite()
        || (axis_squared - 1.0).abs() > AXIS_SQUARED_TOLERANCE
        || !turntable
            .center_mm_depth_camera
            .iter()
            .all(|component| component.is_finite())
    {
        return Err(fail("archive turntable metadata is invalid"));
    }
    Ok(())
}

pub fn valid_frame_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn fail(message: impl Into<String>) -> ArchiveError {
    ArchiveError(message.into())
}

fn io_fail(error: std::io::Error) -> ArchiveError {
    fail(format!("archive I/O failed: {error}"))
}

fn json_fail(error: serde_json::Error) -> ArchiveError {
    fail(format!("archive manifest JSON failed: {error}"))
}
