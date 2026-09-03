use revopoint_pop3_wifi::capture_archive::{
    replay_colored_ply, valid_frame_name, write_frame_archive, ArchiveFrame, ArchiveManifest,
    DepthRecord, ExtrinsicsRecord, RgbRecord,
};
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

fn manifest() -> ArchiveManifest {
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
    }
}

fn temporary_root() -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("pop3-archive-{}-{nonce}", std::process::id()))
}

#[test]
fn atomically_writes_a_self_describing_replayable_frame() {
    let root = temporary_root();
    let depth_raw = [1_000_u16.to_le_bytes(), 1_000_u16.to_le_bytes()].concat();
    let rgb_jpeg = include_bytes!("fixtures/rgb-2x1.jpg");
    let frame = ArchiveFrame {
        manifest: manifest(),
        depth_raw: &depth_raw,
        rgb_jpeg,
    };

    let directory = write_frame_archive(&root, "frame-000001", frame).expect("write archive");

    assert_eq!(directory, root.join("frame-000001"));
    assert!(!root.join(".frame-000001.partial").exists());
    assert_eq!(fs::read(directory.join("depth.z16le")).unwrap(), depth_raw);
    assert!(fs::read(directory.join("depth-mm.pgm"))
        .unwrap()
        .starts_with(b"P5\n2 1\n65535\n"));
    assert_eq!(fs::read(directory.join("rgb.jpg")).unwrap(), rgb_jpeg);
    assert!(fs::read_to_string(directory.join("manifest.json"))
        .unwrap()
        .contains("\"schema_version\": 1"));
    let replayed = replay_colored_ply(&directory).expect("offline replay");
    assert_eq!(replayed, fs::read(directory.join("colored.ply")).unwrap());

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_unsafe_names_inconsistent_metadata_and_overwrite() {
    let root = temporary_root();
    let depth_raw = [1_000_u16.to_le_bytes(), 1_000_u16.to_le_bytes()].concat();
    let frame = || ArchiveFrame {
        manifest: manifest(),
        depth_raw: &depth_raw,
        rgb_jpeg: include_bytes!("fixtures/rgb-2x1.jpg"),
    };

    assert!(write_frame_archive(&root, "../escape", frame()).is_err());
    assert!(write_frame_archive(&root, "bad name", frame()).is_err());
    write_frame_archive(&root, "frame-1", frame()).expect("first publication");
    assert!(write_frame_archive(&root, "frame-1", frame()).is_err());

    let mut invalid = manifest();
    invalid.timestamp_delta_ms = 14;
    assert!(write_frame_archive(
        &root,
        "frame-2",
        ArchiveFrame {
            manifest: invalid,
            depth_raw: &depth_raw,
            rgb_jpeg: include_bytes!("fixtures/rgb-2x1.jpg"),
        }
    )
    .is_err());

    for (index, invalid) in [
        ArchiveManifest {
            schema_version: 2,
            ..manifest()
        },
        ArchiveManifest {
            depth: DepthRecord {
                raw_file: "wrong.raw".to_owned(),
                ..manifest().depth
            },
            ..manifest()
        },
        ArchiveManifest {
            depth: DepthRecord {
                pgm_file: "wrong.pgm".to_owned(),
                ..manifest().depth
            },
            ..manifest()
        },
        ArchiveManifest {
            rgb: RgbRecord {
                jpeg_file: "wrong.jpg".to_owned(),
                ..manifest().rgb
            },
            ..manifest()
        },
        ArchiveManifest {
            colored_ply_file: "wrong.ply".to_owned(),
            ..manifest()
        },
        ArchiveManifest {
            rgb: RgbRecord {
                width: 1,
                ..manifest().rgb
            },
            ..manifest()
        },
        ArchiveManifest {
            rgb: RgbRecord {
                height: 2,
                ..manifest().rgb
            },
            ..manifest()
        },
    ]
    .into_iter()
    .enumerate()
    {
        assert!(write_frame_archive(
            &root,
            &format!("invalid-{index}"),
            ArchiveFrame {
                manifest: invalid,
                depth_raw: &depth_raw,
                rgb_jpeg: include_bytes!("fixtures/rgb-2x1.jpg"),
            }
        )
        .is_err());
    }

    fs::create_dir(root.join(".stale.partial")).unwrap();
    assert!(write_frame_archive(&root, "stale", frame()).is_err());

    let error = write_frame_archive(&root, "bad/name", frame()).unwrap_err();
    assert_eq!(error.to_string(), "archive frame name is invalid");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn frame_names_are_single_safe_path_components() {
    for valid in ["frame-000001", "angle_090", "A1"] {
        assert!(valid_frame_name(valid));
    }
    for invalid in ["", ".", "../escape", "bad name", "slash/name", "é"] {
        assert!(!valid_frame_name(invalid));
    }
}
