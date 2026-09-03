use revopoint_pop3_wifi::depth_decode::{DepthEncoding, DepthPlane};
use revopoint_pop3_wifi::stereo_calibration::ReprojectionMatrix;
use revopoint_pop3_wifi::stereo_depth::{depth_z_statistics, encode_z16_pgm, reproject_z16};
use revopoint_pop3_wifi::stereo_match::DisparityMap;

fn q_fixture() -> ReprojectionMatrix {
    let mut values = [0.0_f32; 16];
    values[0] = 1.0;
    values[5] = 1.0;
    values[11] = 100.0;
    values[14] = 0.1;
    ReprojectionMatrix { values }
}

#[test]
fn reprojects_valid_disparities_as_millimeter_z16() {
    let disparity = DisparityMap {
        width: 3,
        height: 1,
        values: vec![10, u16::MAX, 5],
    };

    let depth = reproject_z16(&disparity, q_fixture(), 3, 1).expect("metric depth");

    assert_eq!(depth.width, 3);
    assert_eq!(depth.height, 1);
    assert_eq!(depth.stride_bytes, 6);
    assert_eq!(depth.millimeters_per_unit, 1.0);
    assert_eq!(
        depth.bytes,
        [100_u16, 0, 200].map(u16::to_le_bytes).concat()
    );
}

#[test]
fn applies_calibration_resolution_scale_to_disparity() {
    let disparity = DisparityMap {
        width: 2,
        height: 1,
        values: vec![5, 5],
    };

    let depth = reproject_z16(&disparity, q_fixture(), 4, 2).expect("scaled depth");

    assert_eq!(depth.bytes, [100_u16, 100].map(u16::to_le_bytes).concat());
}

#[test]
fn rejects_invalid_layout_and_scale() {
    let malformed = DisparityMap {
        width: 2,
        height: 1,
        values: vec![5],
    };
    let valid = DisparityMap {
        width: 2,
        height: 1,
        values: vec![5, 5],
    };

    assert!(reproject_z16(&malformed, q_fixture(), 2, 1).is_err());
    assert!(reproject_z16(&valid, q_fixture(), 0, 1).is_err());
    assert!(reproject_z16(&valid, q_fixture(), 2, 0).is_err());
    assert_eq!(
        reproject_z16(&malformed, q_fixture(), 2, 1)
            .expect_err("invalid disparity")
            .to_string(),
        "disparity cannot be represented as metric Z16 depth"
    );
}

#[test]
fn uses_both_coordinates_and_each_disparity_during_reprojection() {
    let disparity = DisparityMap {
        width: 2,
        height: 2,
        values: vec![10, 10, 5, 5],
    };
    let mut q = q_fixture();
    q.values[13] = 1.0;

    let depth = reproject_z16(&disparity, q, 2, 6).expect("two-dimensional depth");

    assert_eq!(
        depth.bytes,
        [100_u16, 100, 29, 29].map(u16::to_le_bytes).concat()
    );
}

#[test]
fn maps_unrepresentable_positive_depths_to_invalid_zero() {
    let disparity = DisparityMap {
        width: 1,
        height: 1,
        values: vec![0],
    };
    let mut too_small = q_fixture();
    too_small.values[11] = 0.4;
    too_small.values[15] = 1.0;
    let mut too_large = too_small;
    too_large.values[11] = 70_000.0;

    assert_eq!(
        reproject_z16(&disparity, too_small, 1, 1)
            .expect("small depth")
            .bytes,
        [0, 0]
    );
    assert_eq!(
        reproject_z16(&disparity, too_large, 1, 1)
            .expect("large depth")
            .bytes,
        [0, 0]
    );
}

#[test]
fn encodes_big_endian_sixteen_bit_pgm_for_linux_viewers() {
    let disparity = DisparityMap {
        width: 2,
        height: 1,
        values: vec![10, u16::MAX],
    };
    let depth = reproject_z16(&disparity, q_fixture(), 2, 1).expect("metric depth");

    let pgm = encode_z16_pgm(&depth).expect("depth PGM");

    assert_eq!(&pgm[..13], b"P5\n2 1\n65535\n");
    assert_eq!(&pgm[13..], &[0, 100, 0, 0]);

    for invalid in [
        DepthPlane {
            width: 0,
            height: 1,
            stride_bytes: 0,
            encoding: DepthEncoding::Z16LittleEndian,
            millimeters_per_unit: 1.0,
            bytes: Vec::new(),
        },
        DepthPlane {
            width: 2,
            height: 0,
            stride_bytes: 4,
            encoding: DepthEncoding::Z16LittleEndian,
            millimeters_per_unit: 1.0,
            bytes: Vec::new(),
        },
        DepthPlane {
            width: 2,
            height: 1,
            stride_bytes: 2,
            encoding: DepthEncoding::Z16LittleEndian,
            millimeters_per_unit: 1.0,
            bytes: vec![0; 4],
        },
        DepthPlane {
            width: 2,
            height: 1,
            stride_bytes: 4,
            encoding: DepthEncoding::Z16LittleEndian,
            millimeters_per_unit: 1.0,
            bytes: vec![0; 2],
        },
        DepthPlane {
            width: 2,
            height: 1,
            stride_bytes: 4,
            encoding: DepthEncoding::Z16LittleEndian,
            millimeters_per_unit: 2.0,
            bytes: vec![0; 4],
        },
    ] {
        assert!(encode_z16_pgm(&invalid).is_err());
    }
}

#[test]
fn reports_robust_planar_depth_statistics_without_invalid_zero() {
    let depth = DepthPlane {
        width: 5,
        height: 1,
        stride_bytes: 10,
        encoding: DepthEncoding::Z16LittleEndian,
        millimeters_per_unit: 1.0,
        bytes: [0_u16, 100, 101, 102, 200].map(u16::to_le_bytes).concat(),
    };

    let statistics = depth_z_statistics(&depth).expect("depth statistics");

    assert_eq!(statistics.valid_samples, 4);
    assert_eq!(statistics.median_mm, 102.0);
    assert_eq!(statistics.median_absolute_deviation_mm, 2.0);
    assert_eq!(statistics.p10_mm, 100.0);
    assert_eq!(statistics.p90_mm, 102.0);

    let scaled = DepthPlane {
        width: 5,
        height: 1,
        stride_bytes: 10,
        encoding: DepthEncoding::Z16LittleEndian,
        millimeters_per_unit: 2.0,
        bytes: [0_u16, 100, 101, 102, 200].map(u16::to_le_bytes).concat(),
    };
    let scaled_statistics = depth_z_statistics(&scaled).expect("scaled statistics");
    assert_eq!(scaled_statistics.median_mm, 204.0);
    assert_eq!(scaled_statistics.median_absolute_deviation_mm, 4.0);
    assert_eq!(scaled_statistics.p10_mm, 200.0);
    assert_eq!(scaled_statistics.p90_mm, 204.0);

    let empty = DepthPlane {
        bytes: vec![0; 10],
        ..depth
    };
    assert!(depth_z_statistics(&empty).is_err());

    for invalid in [
        DepthPlane {
            stride_bytes: 8,
            ..scaled
        },
        DepthPlane {
            width: 5,
            height: 1,
            stride_bytes: 10,
            encoding: DepthEncoding::Z16LittleEndian,
            millimeters_per_unit: 2.0,
            bytes: vec![1; 8],
        },
        DepthPlane {
            width: 5,
            height: 1,
            stride_bytes: 10,
            encoding: DepthEncoding::Z16LittleEndian,
            millimeters_per_unit: f32::NAN,
            bytes: vec![1; 10],
        },
        DepthPlane {
            width: 5,
            height: 1,
            stride_bytes: 10,
            encoding: DepthEncoding::Z16LittleEndian,
            millimeters_per_unit: 0.0,
            bytes: vec![1; 10],
        },
    ] {
        assert!(depth_z_statistics(&invalid).is_err());
    }
}
