use crate::capture_archive::{validate_turntable_record, RotationDirection, TurntableRecord};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs;
use std::path::{Path, PathBuf};

const MAXIMUM_FRAME_COUNT: u32 = 3_600;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TurntableScheduleSpec {
    pub session_id: String,
    pub frame_count: u32,
    pub direction: RotationDirection,
    pub axis_depth_camera: [f32; 3],
    pub center_mm_depth_camera: [f32; 3],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TurntableScheduleError(String);

impl Display for TurntableScheduleError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for TurntableScheduleError {}

pub fn generate_schedule(
    spec: &TurntableScheduleSpec,
) -> Result<Vec<TurntableRecord>, TurntableScheduleError> {
    if spec.frame_count == 0 || spec.frame_count > MAXIMUM_FRAME_COUNT {
        return Err(fail("turntable frame count must be between 1 and 3600"));
    }
    let angle_step = 360.0_f32 / spec.frame_count as f32;
    let mut records = Vec::new();
    records
        .try_reserve_exact(spec.frame_count as usize)
        .map_err(|_| fail("turntable schedule cannot be allocated"))?;
    for frame_index in 0..spec.frame_count {
        let record = TurntableRecord {
            session_id: spec.session_id.clone(),
            frame_index,
            expected_frame_count: spec.frame_count,
            commanded_angle_degrees: frame_index as f32 * angle_step,
            observed_angle_degrees: None,
            direction: spec.direction,
            axis_depth_camera: spec.axis_depth_camera,
            center_mm_depth_camera: spec.center_mm_depth_camera,
        };
        validate_turntable_record(&record)
            .map_err(|_| fail("turntable schedule specification is invalid"))?;
        records.push(record);
    }
    Ok(records)
}

pub fn write_schedule(
    output_directory: &Path,
    spec: &TurntableScheduleSpec,
) -> Result<Vec<PathBuf>, TurntableScheduleError> {
    let records = generate_schedule(spec)?;
    if output_directory.exists() {
        return Err(fail("turntable schedule output already exists"));
    }
    fs::create_dir(output_directory).map_err(|error| {
        fail(format!(
            "turntable schedule directory cannot be created: {error}"
        ))
    })?;
    let result: Result<Vec<PathBuf>, TurntableScheduleError> = records
        .into_iter()
        .map(|record| {
            let path = output_directory.join(format!("frame-{:06}.json", record.frame_index));
            let mut json = serde_json::to_vec_pretty(&record)
                .map_err(|error| fail(format!("turntable schedule JSON failed: {error}")))?;
            json.push(b'\n');
            fs::write(&path, json).map_err(|error| {
                fail(format!(
                    "turntable schedule frame cannot be written: {error}"
                ))
            })?;
            Ok(path)
        })
        .collect();
    if result.is_err() {
        let _ = fs::remove_dir_all(output_directory);
    }
    result
}

fn fail(message: impl Into<String>) -> TurntableScheduleError {
    TurntableScheduleError(message.into())
}
