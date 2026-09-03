use crate::capture_archive::{
    read_manifest, replay_colored_points, valid_frame_name, validate_turntable_record,
    RotationDirection, TurntableRecord,
};
use crate::rgb_registration::ColoredPoint;
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs;
use std::path::Path;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TurntableMergeError(String);

impl Display for TurntableMergeError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for TurntableMergeError {}

pub fn align_point_to_reference(
    point: ColoredPoint,
    metadata: &TurntableRecord,
) -> Result<ColoredPoint, TurntableMergeError> {
    validate_turntable_record(metadata).map_err(|_| fail("turntable frame metadata is invalid"))?;
    align_point_to_validated_reference(point, metadata)
}

fn align_point_to_validated_reference(
    point: ColoredPoint,
    metadata: &TurntableRecord,
) -> Result<ColoredPoint, TurntableMergeError> {
    if !point.position_mm.into_iter().all(f32::is_finite) {
        return Err(fail("turntable frame contains a non-finite point"));
    }
    let angle_degrees = metadata
        .observed_angle_degrees
        .unwrap_or(metadata.commanded_angle_degrees);
    let object_angle = match metadata.direction {
        RotationDirection::CounterclockwiseViewedFromAxisTip => angle_degrees,
        RotationDirection::ClockwiseViewedFromAxisTip => -angle_degrees,
    };

    // The camera is stationary, so undo the object's recorded motion to place
    // every frame in the coordinate system of the zero-degree frame.
    let radians = -object_angle.to_radians();
    let axis = metadata.axis_depth_camera;
    let center = metadata.center_mm_depth_camera;
    let relative = [
        point.position_mm[0] - center[0],
        point.position_mm[1] - center[1],
        point.position_mm[2] - center[2],
    ];
    let cosine = radians.cos();
    let sine = radians.sin();
    let dot = axis[0] * relative[0] + axis[1] * relative[1] + axis[2] * relative[2];
    let cross = [
        axis[1] * relative[2] - axis[2] * relative[1],
        axis[2] * relative[0] - axis[0] * relative[2],
        axis[0] * relative[1] - axis[1] * relative[0],
    ];
    let one_minus_cosine = 1.0 - cosine;
    let position_mm = std::array::from_fn(|index| {
        center[index]
            + relative[index] * cosine
            + cross[index] * sine
            + axis[index] * dot * one_minus_cosine
    });
    if !position_mm.into_iter().all(f32::is_finite) {
        return Err(fail("turntable alignment produced a non-finite point"));
    }
    Ok(ColoredPoint {
        position_mm,
        rgb: point.rgb,
    })
}

pub fn merge_frames(
    mut frames: Vec<(TurntableRecord, Vec<ColoredPoint>)>,
) -> Result<Vec<ColoredPoint>, TurntableMergeError> {
    let first = frames
        .first()
        .ok_or_else(|| fail("turntable session contains no frames"))?
        .0
        .clone();
    validate_turntable_record(&first).map_err(|_| fail("turntable frame metadata is invalid"))?;
    if usize::try_from(first.expected_frame_count).ok() != Some(frames.len()) {
        return Err(fail("turntable session is incomplete"));
    }
    frames.sort_by_key(|(metadata, _)| metadata.frame_index);
    let point_count = frames.iter().try_fold(0_usize, |total, (_, points)| {
        total.checked_add(points.len())
    });
    let mut merged = Vec::new();
    merged
        .try_reserve(point_count.ok_or_else(|| fail("turntable point count is too large"))?)
        .map_err(|_| fail("turntable point cloud cannot be allocated"))?;

    for (expected_index, (metadata, points)) in frames.into_iter().enumerate() {
        validate_turntable_record(&metadata)
            .map_err(|_| fail("turntable frame metadata is invalid"))?;
        if metadata.session_id != first.session_id
            || metadata.expected_frame_count != first.expected_frame_count
            || metadata.frame_index
                != u32::try_from(expected_index)
                    .map_err(|_| fail("turntable frame index is too large"))?
            || metadata.direction != first.direction
            || metadata.axis_depth_camera != first.axis_depth_camera
            || metadata.center_mm_depth_camera != first.center_mm_depth_camera
        {
            return Err(fail("turntable session metadata is inconsistent"));
        }
        for point in points {
            merged.push(align_point_to_validated_reference(point, &metadata)?);
        }
    }
    Ok(merged)
}

pub fn merge_archive_session(
    archive_root: &Path,
    session_id: &str,
) -> Result<Vec<ColoredPoint>, TurntableMergeError> {
    if !valid_frame_name(session_id) {
        return Err(fail("turntable session ID is invalid"));
    }
    let entries = fs::read_dir(archive_root)
        .map_err(|error| fail(format!("turntable archive cannot be read: {error}")))?;
    let mut frames = Vec::new();
    for entry in entries {
        let entry = entry
            .map_err(|error| fail(format!("turntable archive entry cannot be read: {error}")))?;
        let file_type = entry.file_type().map_err(|error| {
            fail(format!(
                "turntable archive entry cannot be inspected: {error}"
            ))
        })?;
        if !file_type.is_dir() || !entry.path().join("manifest.json").is_file() {
            continue;
        }
        let manifest = read_manifest(&entry.path())
            .map_err(|error| fail(format!("turntable archive manifest is invalid: {error}")))?;
        let Some(metadata) = manifest.turntable else {
            continue;
        };
        if metadata.session_id != session_id {
            continue;
        }
        let (_, points) = replay_colored_points(&entry.path()).map_err(|error| {
            fail(format!(
                "turntable archive frame cannot be replayed: {error}"
            ))
        })?;
        frames.push((metadata, points));
    }
    merge_frames(frames)
}

fn fail(message: impl Into<String>) -> TurntableMergeError {
    TurntableMergeError(message.into())
}
