use revopoint_pop3_wifi::capture_archive::{
    write_frame_archive, ArchiveFrame, ArchiveManifest, DepthRecord, ExtrinsicsRecord, RgbRecord,
    RotationDirection, TurntableRecord,
};
use revopoint_pop3_wifi::rgb_registration::ColoredPoint;
use revopoint_pop3_wifi::turntable_merge::{
    align_point_to_reference, merge_archive_session, merge_frames,
};
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

fn record(index: u32, angle: f32, direction: RotationDirection) -> TurntableRecord {
    TurntableRecord {
        session_id: "car-rotation".to_owned(),
        frame_index: index,
        expected_frame_count: 2,
        commanded_angle_degrees: angle,
        observed_angle_degrees: None,
        direction,
        axis_depth_camera: [0.0, 0.0, 1.0],
        center_mm_depth_camera: [10.0, 0.0, 0.0],
    }
}

fn point(position_mm: [f32; 3], rgb: [u8; 3]) -> ColoredPoint {
    ColoredPoint { position_mm, rgb }
}

fn assert_position(actual: [f32; 3], expected: [f32; 3]) {
    for (actual, expected) in actual.into_iter().zip(expected) {
        assert!((actual - expected).abs() < 0.0001, "{actual} != {expected}");
    }
}

fn archive_manifest(turntable: TurntableRecord) -> ArchiveManifest {
    ArchiveManifest {
        schema_version: 1,
        depth_timestamp_ms: 1_000,
        rgb_timestamp_ms: 1_015,
        timestamp_delta_ms: 15,
        depth: DepthRecord {
            width: 2,
            height: 1,
            millimeters_per_unit: 0.1,
            intrinsics: [1.0, 1.0, 0.0, 0.0],
            raw_file: "depth.z16le".to_owned(),
            pgm_file: "depth-mm.pgm".to_owned(),
        },
        rgb: RgbRecord {
            width: 2,
            height: 1,
            calibration_width: 2,
            calibration_height: 1,
            intrinsics: [1.0, 1.0, 0.0, 0.0],
            distortion: [0.0; 5],
            jpeg_file: "rgb.jpg".to_owned(),
        },
        depth_to_rgb: ExtrinsicsRecord {
            rotation: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            translation_mm: [0.0; 3],
        },
        colored_ply_file: "colored.ply".to_owned(),
        turntable: Some(turntable),
    }
}

fn temporary_root() -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("pop3-turntable-{}-{nonce}", std::process::id()))
}

#[test]
fn reverses_counterclockwise_object_motion_around_an_offset_axis() {
    let aligned = align_point_to_reference(
        point([10.0, 1.0, 2.0], [1, 2, 3]),
        &record(
            1,
            90.0,
            RotationDirection::CounterclockwiseViewedFromAxisTip,
        ),
    )
    .expect("align point");

    assert_position(aligned.position_mm, [11.0, 0.0, 2.0]);
    assert_eq!(aligned.rgb, [1, 2, 3]);
}

#[test]
fn reverses_clockwise_motion_and_prefers_an_observed_angle() {
    let mut metadata = record(1, 40.0, RotationDirection::ClockwiseViewedFromAxisTip);
    metadata.observed_angle_degrees = Some(90.0);
    let aligned = align_point_to_reference(point([10.0, -1.0, 2.0], [4, 5, 6]), &metadata)
        .expect("align point");

    assert_position(aligned.position_mm, [11.0, 0.0, 2.0]);
}

#[test]
fn rotates_around_every_component_of_an_offset_center() {
    let mut metadata = record(
        1,
        90.0,
        RotationDirection::CounterclockwiseViewedFromAxisTip,
    );
    metadata.center_mm_depth_camera = [10.0, 20.0, 30.0];

    let aligned = align_point_to_reference(point([10.0, 21.0, 32.0], [7, 8, 9]), &metadata)
        .expect("align around offset center");

    assert_position(aligned.position_mm, [11.0, 20.0, 32.0]);
}

#[test]
fn applies_full_rodrigues_rotation_around_an_arbitrary_axis() {
    let mut metadata = record(
        1,
        120.0,
        RotationDirection::CounterclockwiseViewedFromAxisTip,
    );
    let inverse_square_root_of_three = 1.0_f32 / 3.0_f32.sqrt();
    metadata.axis_depth_camera = [inverse_square_root_of_three; 3];
    metadata.center_mm_depth_camera = [10.0, 20.0, 30.0];

    let aligned = align_point_to_reference(point([11.0, 22.0, 33.0], [10, 20, 30]), &metadata)
        .expect("align around arbitrary axis");

    // Inverse 120-degree rotation around (1, 1, 1) maps (x, y, z)
    // to (y, z, x), before translating back from the offset center.
    assert_position(aligned.position_mm, [12.0, 23.0, 31.0]);
}

#[test]
fn merges_one_complete_session_in_frame_index_order() {
    let frames = vec![
        (
            record(
                1,
                90.0,
                RotationDirection::CounterclockwiseViewedFromAxisTip,
            ),
            vec![point([10.0, 1.0, 0.0], [2, 0, 0])],
        ),
        (
            record(0, 0.0, RotationDirection::CounterclockwiseViewedFromAxisTip),
            vec![point([11.0, 0.0, 0.0], [1, 0, 0])],
        ),
    ];

    let merged = merge_frames(frames).expect("complete session");

    assert_eq!(merged.len(), 2);
    assert_eq!(merged[0].rgb, [1, 0, 0]);
    assert_eq!(merged[1].rgb, [2, 0, 0]);
    assert_position(merged[0].position_mm, [11.0, 0.0, 0.0]);
    assert_position(merged[1].position_mm, [11.0, 0.0, 0.0]);
}

#[test]
fn rejects_incomplete_duplicate_or_inconsistent_sessions() {
    let first = record(0, 0.0, RotationDirection::CounterclockwiseViewedFromAxisTip);
    assert!(merge_frames(vec![(first.clone(), vec![point([0.0; 3], [0; 3])])]).is_err());

    let duplicate = vec![
        (first.clone(), vec![point([0.0; 3], [0; 3])]),
        (first.clone(), vec![point([0.0; 3], [0; 3])]),
    ];
    assert!(merge_frames(duplicate).is_err());

    let mut other_session = record(
        1,
        180.0,
        RotationDirection::CounterclockwiseViewedFromAxisTip,
    );
    other_session.session_id = "different".to_owned();
    assert!(merge_frames(vec![
        (first.clone(), vec![point([0.0; 3], [0; 3])]),
        (other_session, vec![point([0.0; 3], [0; 3])]),
    ])
    .is_err());

    let mut other_axis = record(
        1,
        180.0,
        RotationDirection::CounterclockwiseViewedFromAxisTip,
    );
    other_axis.axis_depth_camera = [0.0, 1.0, 0.0];
    assert!(merge_frames(vec![
        (first, vec![point([0.0; 3], [0; 3])]),
        (other_axis, vec![point([0.0; 3], [0; 3])]),
    ])
    .is_err());
}

#[test]
fn replays_and_merges_a_complete_archive_session_without_scanner_hardware() {
    let root = temporary_root();
    let depth_raw = [1_000_u16.to_le_bytes(), 1_000_u16.to_le_bytes()].concat();
    let rgb_jpeg = include_bytes!("fixtures/rgb-2x1.jpg");
    for (index, angle) in [(0, 0.0), (1, 90.0)] {
        write_frame_archive(
            &root,
            &format!("car-rotation-{index:06}"),
            ArchiveFrame {
                manifest: archive_manifest(record(
                    index,
                    angle,
                    RotationDirection::CounterclockwiseViewedFromAxisTip,
                )),
                depth_raw: &depth_raw,
                rgb_jpeg,
            },
        )
        .expect("archive frame");
    }
    fs::write(root.join("operator-notes.txt"), b"ignored non-frame entry")
        .expect("write harmless unrelated file");
    fs::create_dir(root.join("unrelated-directory")).expect("create unrelated directory");

    let merged = merge_archive_session(&root, "car-rotation").expect("merge archive session");

    assert_eq!(merged.len(), 4);
    assert!(merged
        .iter()
        .all(|point| point.position_mm.into_iter().all(f32::is_finite)));
    fs::remove_dir_all(root).expect("remove fixture");
}

#[test]
fn rejects_missing_archive_frames_and_unsafe_session_ids() {
    let root = temporary_root();
    let depth_raw = [1_000_u16.to_le_bytes(), 1_000_u16.to_le_bytes()].concat();
    write_frame_archive(
        &root,
        "car-rotation-000000",
        ArchiveFrame {
            manifest: archive_manifest(record(
                0,
                0.0,
                RotationDirection::CounterclockwiseViewedFromAxisTip,
            )),
            depth_raw: &depth_raw,
            rgb_jpeg: include_bytes!("fixtures/rgb-2x1.jpg"),
        },
    )
    .expect("archive frame");

    assert_eq!(
        merge_archive_session(&root, "car-rotation")
            .expect_err("missing frame")
            .to_string(),
        "turntable session is incomplete"
    );
    assert_eq!(
        merge_archive_session(&root, "../escape")
            .expect_err("unsafe ID")
            .to_string(),
        "turntable session ID is invalid"
    );
    fs::remove_dir_all(root).expect("remove fixture");
}
