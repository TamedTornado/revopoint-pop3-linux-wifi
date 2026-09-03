use revopoint_pop3_wifi::capture_archive::RotationDirection;
use revopoint_pop3_wifi::turntable_schedule::{
    generate_schedule, write_schedule, TurntableScheduleSpec,
};
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

fn specification() -> TurntableScheduleSpec {
    TurntableScheduleSpec {
        session_id: "car-rotation".to_owned(),
        frame_count: 4,
        direction: RotationDirection::CounterclockwiseViewedFromAxisTip,
        axis_depth_camera: [0.0, 1.0, 0.0],
        center_mm_depth_camera: [10.0, 20.0, 300.0],
    }
}

fn temporary_directory() -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("pop3-schedule-{}-{nonce}", std::process::id()))
}

#[test]
fn generates_one_complete_rotation_with_deterministic_angles() {
    let records = generate_schedule(&specification()).expect("valid schedule");

    assert_eq!(records.len(), 4);
    assert_eq!(
        records
            .iter()
            .map(|record| record.frame_index)
            .collect::<Vec<_>>(),
        [0, 1, 2, 3]
    );
    assert_eq!(
        records
            .iter()
            .map(|record| record.commanded_angle_degrees)
            .collect::<Vec<_>>(),
        [0.0, 90.0, 180.0, 270.0]
    );
    assert!(records.iter().all(|record| {
        record.session_id == "car-rotation"
            && record.expected_frame_count == 4
            && record.observed_angle_degrees.is_none()
            && record.axis_depth_camera == [0.0, 1.0, 0.0]
            && record.center_mm_depth_camera == [10.0, 20.0, 300.0]
    }));
}

#[test]
fn rejects_unbounded_counts_and_invalid_shared_geometry() {
    assert_eq!(
        generate_schedule(&TurntableScheduleSpec {
            frame_count: 3_600,
            ..specification()
        })
        .expect("maximum bounded frame count")
        .len(),
        3_600
    );
    for frame_count in [0, 3_601, u32::MAX] {
        assert_eq!(
            generate_schedule(&TurntableScheduleSpec {
                frame_count,
                ..specification()
            })
            .expect_err("invalid frame count")
            .to_string(),
            "turntable frame count must be between 1 and 3600"
        );
    }
    for invalid in [
        TurntableScheduleSpec {
            session_id: "bad session".to_owned(),
            ..specification()
        },
        TurntableScheduleSpec {
            axis_depth_camera: [0.0; 3],
            ..specification()
        },
        TurntableScheduleSpec {
            center_mm_depth_camera: [0.0, f32::NAN, 0.0],
            ..specification()
        },
    ] {
        assert_eq!(
            generate_schedule(&invalid)
                .expect_err("invalid shared geometry")
                .to_string(),
            "turntable schedule specification is invalid"
        );
    }
}

#[test]
fn writes_zero_padded_individual_capture_manifests_and_refuses_overwrite() {
    let directory = temporary_directory();

    let paths = write_schedule(&directory, &specification()).expect("write schedule");

    assert_eq!(paths.len(), 4);
    assert_eq!(
        paths[0].file_name().and_then(|name| name.to_str()),
        Some("frame-000000.json")
    );
    assert_eq!(
        paths[3].file_name().and_then(|name| name.to_str()),
        Some("frame-000003.json")
    );
    let first: serde_json::Value =
        serde_json::from_slice(&fs::read(&paths[0]).expect("read first frame"))
            .expect("parse first frame");
    assert_eq!(first["commanded_angle_degrees"], 0.0);
    assert_eq!(first["expected_frame_count"], 4);
    assert_eq!(
        write_schedule(&directory, &specification())
            .expect_err("refuse overwrite")
            .to_string(),
        "turntable schedule output already exists"
    );

    fs::remove_dir_all(directory).expect("remove fixture");
}
