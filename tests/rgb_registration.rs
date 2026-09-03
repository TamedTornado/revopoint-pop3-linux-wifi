use revopoint_pop3_wifi::calibration::ScaledDepthIntrinsics;
use revopoint_pop3_wifi::depth_decode::{DepthEncoding, DepthPlane};
use revopoint_pop3_wifi::rgb_calibration::{
    LeftToRgbExtrinsics, RgbCalibration, RgbDistortion, RgbIntrinsics,
};
use revopoint_pop3_wifi::rgb_registration::{
    colorize_depth, decode_jpeg_rgb, encode_binary_ply, project_depth_point, RgbImage,
};

fn calibration(translation_mm: [f32; 3]) -> RgbCalibration {
    RgbCalibration {
        intrinsics: RgbIntrinsics {
            calibration_width: 2,
            calibration_height: 1,
            fx: 1.0,
            fy: 1.0,
            cx: 0.0,
            cy: 0.0,
        },
        distortion: RgbDistortion {
            coefficients: [0.0; 5],
        },
        left_to_rgb: LeftToRgbExtrinsics {
            rotation: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            translation_mm,
        },
    }
}

fn depth() -> DepthPlane {
    DepthPlane {
        width: 2,
        height: 1,
        stride_bytes: 4,
        encoding: DepthEncoding::Z16LittleEndian,
        millimeters_per_unit: 0.1,
        bytes: [1_000_u16.to_le_bytes(), 1_000_u16.to_le_bytes()].concat(),
    }
}

#[test]
fn projects_depth_points_into_the_rgb_camera_and_samples_color() {
    let image = RgbImage {
        width: 2,
        height: 1,
        pixels: vec![255, 0, 0, 0, 255, 0],
    };
    let intrinsics = ScaledDepthIntrinsics {
        width: 2,
        height: 1,
        fx: 1.0,
        fy: 1.0,
        cx: 0.0,
        cy: 0.0,
    };

    let points = colorize_depth(&depth(), intrinsics, &image, calibration([0.0; 3]))
        .expect("identity registration");

    assert_eq!(points.len(), 2);
    assert_eq!(points[0].position_mm, [0.0, 0.0, 100.0]);
    assert_eq!(points[0].rgb, [255, 0, 0]);
    assert_eq!(points[1].position_mm, [100.0, 0.0, 100.0]);
    assert_eq!(points[1].rgb, [0, 255, 0]);
}

#[test]
fn omits_invalid_depth_and_points_outside_the_rgb_image() {
    let mut plane = depth();
    plane.bytes[..2].fill(0);
    let image = RgbImage {
        width: 2,
        height: 1,
        pixels: vec![255; 6],
    };
    let intrinsics = ScaledDepthIntrinsics {
        width: 2,
        height: 1,
        fx: 1.0,
        fy: 1.0,
        cx: 0.0,
        cy: 0.0,
    };

    let points = colorize_depth(&plane, intrinsics, &image, calibration([1_000.0, 0.0, 0.0]))
        .expect("valid but non-overlapping cameras");

    assert!(points.is_empty());
}

#[test]
fn writes_a_standard_binary_little_endian_colored_ply() {
    let image = RgbImage {
        width: 2,
        height: 1,
        pixels: vec![255, 0, 0, 0, 255, 0],
    };
    let intrinsics = ScaledDepthIntrinsics {
        width: 2,
        height: 1,
        fx: 1.0,
        fy: 1.0,
        cx: 0.0,
        cy: 0.0,
    };
    let points = colorize_depth(&depth(), intrinsics, &image, calibration([0.0; 3])).unwrap();

    let ply = encode_binary_ply(&points);
    let header_end = ply
        .windows(11)
        .position(|bytes| bytes == b"end_header\n")
        .expect("PLY header terminator")
        + 11;
    let header = std::str::from_utf8(&ply[..header_end]).expect("ASCII PLY header");

    assert!(header.starts_with("ply\nformat binary_little_endian 1.0\n"));
    assert!(header.contains("element vertex 2\n"));
    assert_eq!(ply.len() - header_end, 2 * 15);
}

#[test]
fn follows_the_published_projection_without_applying_distortion_twice() {
    let mut calibration = calibration([60.0, 0.0, 0.0]);
    calibration.distortion.coefficients = [100.0; 5];
    let mut plane = depth();
    plane.width = 1;
    plane.stride_bytes = 2;
    plane.bytes.truncate(2);
    let image = RgbImage {
        width: 2,
        height: 1,
        pixels: vec![255, 0, 0, 0, 255, 0],
    };
    let intrinsics = ScaledDepthIntrinsics {
        width: 1,
        height: 1,
        fx: 1.0,
        fy: 1.0,
        cx: 0.0,
        cy: 0.0,
    };

    let points = colorize_depth(&plane, intrinsics, &image, calibration)
        .expect("published SDK projection convention");

    assert_eq!(points[0].rgb, [255, 0, 0]);
}

#[test]
fn projection_uses_every_row_major_rotation_and_translation_term() {
    let calibration = RgbCalibration {
        intrinsics: RgbIntrinsics {
            calibration_width: 100,
            calibration_height: 200,
            fx: 100.0,
            fy: 200.0,
            cx: 10.0,
            cy: 20.0,
        },
        distortion: RgbDistortion {
            coefficients: [0.0; 5],
        },
        left_to_rgb: LeftToRgbExtrinsics {
            rotation: [2.0, 3.0, 5.0, 7.0, 11.0, 13.0, 17.0, 19.0, 23.0],
            translation_mm: [29.0, 31.0, 37.0],
        },
    };

    let pixel = project_depth_point(
        [41.0, 43.0, 47.0],
        100,
        200,
        [100.0, 200.0, 10.0, 20.0],
        calibration,
    );

    assert_eq!(pixel, Some([27, 125]));
}

#[test]
fn decodes_a_bounded_rgb_jpeg_fixture() {
    let image = decode_jpeg_rgb(include_bytes!("fixtures/rgb-2x1.jpg")).expect("RGB JPEG");

    assert_eq!((image.width, image.height), (2, 1));
    assert_eq!(image.pixels.len(), 6);
    assert!(image.pixels[0] > image.pixels[1]);
    assert!(image.pixels[4] > image.pixels[3]);
    assert_eq!(
        decode_jpeg_rgb(b"not a jpeg").unwrap_err().to_string(),
        "RGB-D data cannot be registered"
    );
}

#[test]
fn projection_rejects_each_camera_boundary() {
    let calibration = calibration([0.0; 3]);
    let intrinsics = [1.0, 1.0, 0.0, 0.0];
    assert_eq!(
        project_depth_point([0.0, 0.0, 100.0], 1, 1, intrinsics, calibration),
        Some([0, 0])
    );
    for (point, width, height, camera) in [
        ([0.0, 0.0, -100.0], 1, 1, intrinsics),
        ([0.0, 0.0, 0.0], 1, 1, intrinsics),
        ([0.0, 0.0, f32::NAN], 1, 1, intrinsics),
        ([-100.0, 0.0, 100.0], 1, 1, intrinsics),
        ([100.0, 0.0, 100.0], 1, 1, intrinsics),
        ([0.0, -100.0, 100.0], 1, 1, intrinsics),
        ([0.0, 100.0, 100.0], 1, 1, intrinsics),
        ([0.0, 0.0, 100.0], 1, 1, [f32::NAN, 1.0, 0.0, 0.0]),
        ([0.0, 0.0, 100.0], 1, 1, [1.0, f32::NAN, 0.0, 0.0]),
    ] {
        assert_eq!(
            project_depth_point(point, width, height, camera, calibration),
            None
        );
    }
}

#[test]
fn registration_rejects_each_inconsistent_input_contract() {
    let image = RgbImage {
        width: 2,
        height: 1,
        pixels: vec![255; 6],
    };
    let intrinsics = ScaledDepthIntrinsics {
        width: 2,
        height: 1,
        fx: 1.0,
        fy: 1.0,
        cx: 0.0,
        cy: 0.0,
    };
    let assert_rejected = |plane: DepthPlane, camera: ScaledDepthIntrinsics, rgb: &RgbImage| {
        assert!(colorize_depth(&plane, camera, rgb, calibration([0.0; 3])).is_err());
    };

    let mut plane = depth();
    plane.width = 0;
    assert_rejected(plane, intrinsics, &image);
    let mut plane = depth();
    plane.height = 0;
    plane.bytes.clear();
    assert_rejected(
        plane,
        ScaledDepthIntrinsics {
            height: 0,
            ..intrinsics
        },
        &image,
    );
    let mut plane = depth();
    plane.stride_bytes = 3;
    assert_rejected(plane, intrinsics, &image);
    let mut plane = depth();
    plane.bytes.pop();
    assert_rejected(plane, intrinsics, &image);
    for scale in [0.0, f32::NAN] {
        let mut plane = depth();
        plane.millimeters_per_unit = scale;
        assert_rejected(plane, intrinsics, &image);
    }
    for camera in [
        ScaledDepthIntrinsics {
            width: 1,
            ..intrinsics
        },
        ScaledDepthIntrinsics {
            height: 2,
            ..intrinsics
        },
        ScaledDepthIntrinsics {
            fx: 0.0,
            ..intrinsics
        },
        ScaledDepthIntrinsics {
            fx: f32::NAN,
            ..intrinsics
        },
        ScaledDepthIntrinsics {
            fy: 0.0,
            ..intrinsics
        },
        ScaledDepthIntrinsics {
            fy: f32::NAN,
            ..intrinsics
        },
        ScaledDepthIntrinsics {
            cx: f32::NAN,
            ..intrinsics
        },
        ScaledDepthIntrinsics {
            cy: f32::NAN,
            ..intrinsics
        },
    ] {
        assert_rejected(depth(), camera, &image);
    }
    for rgb in [
        RgbImage {
            width: 0,
            ..image.clone()
        },
        RgbImage {
            height: 0,
            ..image.clone()
        },
        RgbImage {
            pixels: vec![0; 5],
            ..image.clone()
        },
    ] {
        assert_rejected(depth(), intrinsics, &rgb);
    }
    for invalid in [
        RgbIntrinsics {
            fx: 0.0,
            ..calibration([0.0; 3]).intrinsics
        },
        RgbIntrinsics {
            fy: 0.0,
            ..calibration([0.0; 3]).intrinsics
        },
    ] {
        assert!(colorize_depth(
            &depth(),
            intrinsics,
            &image,
            RgbCalibration {
                intrinsics: invalid,
                ..calibration([0.0; 3])
            }
        )
        .is_err());
    }
}

#[test]
fn registration_scales_rgb_intrinsics_to_the_actual_image() {
    let mut pixels = vec![0; 4 * 4 * 3];
    pixels[0..3].copy_from_slice(&[255, 0, 0]);
    pixels[6..9].copy_from_slice(&[0, 0, 255]);
    pixels[24..27].copy_from_slice(&[255, 255, 0]);
    pixels[30..33].copy_from_slice(&[0, 255, 255]);
    let image = RgbImage {
        width: 4,
        height: 4,
        pixels,
    };
    let plane = DepthPlane {
        width: 2,
        height: 2,
        stride_bytes: 4,
        encoding: DepthEncoding::Z16LittleEndian,
        millimeters_per_unit: 0.1,
        bytes: [1_000_u16.to_le_bytes(); 4].concat(),
    };
    let intrinsics = ScaledDepthIntrinsics {
        width: 2,
        height: 2,
        fx: 2.0,
        fy: 2.0,
        cx: 0.5,
        cy: 0.5,
    };
    let registration = RgbCalibration {
        intrinsics: RgbIntrinsics {
            calibration_width: 2,
            calibration_height: 2,
            fx: 2.0,
            fy: 2.0,
            cx: 0.5,
            cy: 0.5,
        },
        ..calibration([0.0; 3])
    };

    let points = colorize_depth(&plane, intrinsics, &image, registration).unwrap();

    assert_eq!(points[0].rgb, [255, 0, 0]);
    assert_eq!(points[1].rgb, [0, 0, 255]);
    assert_eq!(points[2].rgb, [255, 255, 0]);
    assert_eq!(points[3].rgb, [0, 255, 255]);
}

#[test]
fn registration_uses_both_depth_pixel_coordinates_and_intrinsics() {
    let plane = DepthPlane {
        width: 2,
        height: 2,
        stride_bytes: 4,
        encoding: DepthEncoding::Z16LittleEndian,
        millimeters_per_unit: 0.1,
        bytes: [1_000_u16.to_le_bytes(); 4].concat(),
    };
    let image = RgbImage {
        width: 2,
        height: 2,
        pixels: vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0],
    };
    let intrinsics = ScaledDepthIntrinsics {
        width: 2,
        height: 2,
        fx: 2.0,
        fy: 4.0,
        cx: 0.5,
        cy: 0.5,
    };
    let registration = RgbCalibration {
        intrinsics: RgbIntrinsics {
            calibration_width: 2,
            calibration_height: 2,
            fx: 2.0,
            fy: 4.0,
            cx: 0.5,
            cy: 0.5,
        },
        ..calibration([0.0; 3])
    };

    let points = colorize_depth(&plane, intrinsics, &image, registration).unwrap();

    assert_eq!(points.len(), 4);
    assert_eq!(points[0].position_mm, [-25.0, -12.5, 100.0]);
    assert_eq!(points[1].position_mm, [25.0, -12.5, 100.0]);
    assert_eq!(points[2].position_mm, [-25.0, 12.5, 100.0]);
    assert_eq!(points[3].position_mm, [25.0, 12.5, 100.0]);
    assert_eq!(
        points.iter().map(|point| point.rgb).collect::<Vec<_>>(),
        [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0]]
    );
}
