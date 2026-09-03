use revopoint_pop3_wifi::http_stream::StreamLimits;
use revopoint_pop3_wifi::rgb_calibration::{
    get_rgb_calibration, parse_left_to_rgb_extrinsics, parse_rgb_distortion, parse_rgb_intrinsics,
    LeftToRgbExtrinsics, RgbCalibration, RgbCalibrationError, RgbCalibrationQueryError,
    RgbDistortion, RgbIntrinsics,
};
use std::error::Error;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

fn intrinsics() -> RgbIntrinsics {
    RgbIntrinsics {
        calibration_width: 1280,
        calibration_height: 800,
        fx: 1750.0,
        fy: 1745.0,
        cx: 396.0,
        cy: 397.0,
    }
}

fn intrinsics_bytes(value: RgbIntrinsics) -> [u8; 40] {
    let mut bytes = [0_u8; 40];
    bytes[0..2].copy_from_slice(&value.calibration_width.to_le_bytes());
    bytes[2..4].copy_from_slice(&value.calibration_height.to_le_bytes());
    for (index, component) in [
        value.fx, 0.0, value.cx, 0.0, value.fy, value.cy, 0.0, 0.0, 1.0,
    ]
    .into_iter()
    .enumerate()
    {
        let offset = 4 + index * 4;
        bytes[offset..offset + 4].copy_from_slice(&component.to_le_bytes());
    }
    bytes
}

fn float_bytes<const N: usize>(values: [f32; N]) -> Vec<u8> {
    values
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect::<Vec<_>>()
}

fn fixture_calibration() -> RgbCalibration {
    RgbCalibration {
        intrinsics: intrinsics(),
        distortion: RgbDistortion {
            coefficients: [0.07, 0.24, 0.0007, -0.00009, -2.9],
        },
        left_to_rgb: LeftToRgbExtrinsics {
            rotation: [0.99, -0.01, -0.02, 0.01, 0.99, -0.001, 0.02, 0.001, 0.99],
            translation_mm: [-14.7, -0.18, -1.15],
        },
    }
}

#[test]
fn parses_rgb_intrinsics_distortion_and_left_to_rgb_transform() {
    let expected = fixture_calibration();

    assert_eq!(
        parse_rgb_intrinsics(&intrinsics_bytes(expected.intrinsics)).expect("RGB intrinsics"),
        expected.intrinsics
    );
    assert_eq!(
        parse_rgb_distortion(&float_bytes(expected.distortion.coefficients))
            .expect("RGB distortion"),
        expected.distortion
    );
    let mut transform = expected.left_to_rgb.rotation.to_vec();
    transform.extend(expected.left_to_rgb.translation_mm);
    let transform: [f32; 12] = transform.try_into().expect("12 floats");
    assert_eq!(
        parse_left_to_rgb_extrinsics(&float_bytes(transform)).expect("left-to-RGB transform"),
        expected.left_to_rgb
    );
}

#[test]
fn rejects_wrong_sizes_nonfinite_values_and_degenerate_calibration() {
    let expected = fixture_calibration();
    assert!(parse_rgb_intrinsics(&intrinsics_bytes(expected.intrinsics)[..39]).is_err());
    assert!(parse_rgb_distortion(&[0_u8; 19]).is_err());
    assert!(parse_left_to_rgb_extrinsics(&[0_u8; 47]).is_err());

    for invalid in [
        RgbIntrinsics {
            calibration_width: 0,
            ..expected.intrinsics
        },
        RgbIntrinsics {
            calibration_height: 0,
            ..expected.intrinsics
        },
        RgbIntrinsics {
            fx: 0.0,
            ..expected.intrinsics
        },
        RgbIntrinsics {
            fy: 0.0,
            ..expected.intrinsics
        },
        RgbIntrinsics {
            cx: f32::NAN,
            ..expected.intrinsics
        },
        RgbIntrinsics {
            cy: f32::NAN,
            ..expected.intrinsics
        },
    ] {
        assert!(parse_rgb_intrinsics(&intrinsics_bytes(invalid)).is_err());
    }
    for matrix_index in [1, 3, 6, 7, 8] {
        let mut noncanonical = intrinsics_bytes(expected.intrinsics);
        let offset = 4 + matrix_index * 4;
        noncanonical[offset..offset + 4].copy_from_slice(&2.0_f32.to_le_bytes());
        assert!(parse_rgb_intrinsics(&noncanonical).is_err());
    }

    let mut invalid_distortion = expected.distortion.coefficients;
    invalid_distortion[4] = f32::INFINITY;
    assert!(parse_rgb_distortion(&float_bytes(invalid_distortion)).is_err());

    let mut invalid_transform = [0.0_f32; 12];
    invalid_transform[9..12].copy_from_slice(&expected.left_to_rgb.translation_mm);
    assert!(parse_left_to_rgb_extrinsics(&float_bytes(invalid_transform)).is_err());

    for invalid_component in [0, 9] {
        let mut transform = expected.left_to_rgb.rotation.to_vec();
        transform.extend(expected.left_to_rgb.translation_mm);
        transform[invalid_component] = f32::NAN;
        assert!(parse_left_to_rgb_extrinsics(&float_bytes::<12>(
            transform.try_into().expect("12 floats")
        ))
        .is_err());
    }

    let singular = [
        1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, -14.7, -0.18, -1.15,
    ];
    assert!(parse_left_to_rgb_extrinsics(&float_bytes(singular)).is_err());
}

#[test]
fn calibration_errors_preserve_component_context_and_sources() {
    let invalid = RgbCalibrationError;
    assert_eq!(
        invalid.to_string(),
        "scanner returned invalid RGB calibration"
    );
    let query = RgbCalibrationQueryError::Invalid {
        component: "test component",
        source: invalid,
    };
    assert_eq!(
        query.to_string(),
        "parse RGB test component: scanner returned invalid RGB calibration"
    );
    assert_eq!(
        query
            .source()
            .expect("calibration error source")
            .to_string(),
        invalid.to_string()
    );

    let http = get_rgb_calibration("127.0.0.1:1".parse().expect("socket address"), limits())
        .expect_err("unreachable fixture must fail");
    assert!(http.to_string().contains("download RGB intrinsics"));
    assert!(http.source().is_some());
}

fn limits() -> StreamLimits {
    StreamLimits {
        connect_timeout: Duration::from_secs(1),
        idle_timeout: Duration::from_secs(1),
        total_timeout: Duration::from_secs(3),
        max_header_bytes: 1024,
        max_body_bytes: 1024,
    }
}

fn serve_file(listener: &TcpListener, path: &str, body: &[u8]) {
    let (mut stream, _) = listener.accept().expect("accept calibration request");
    let mut request = Vec::new();
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        let mut block = [0_u8; 256];
        let count = stream.read(&mut block).expect("read calibration request");
        assert!(count > 0);
        request.extend_from_slice(&block[..count]);
    }
    assert!(String::from_utf8_lossy(&request).starts_with(&format!("GET {path} HTTP/1.1\r\n")));
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )
    .expect("write calibration header");
    stream.write_all(body).expect("write calibration body");
}

#[test]
fn downloads_the_three_read_only_rgb_calibration_files() {
    let expected = fixture_calibration();
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
    let address = listener.local_addr().expect("fixture address");
    let server = thread::spawn(move || {
        serve_file(
            &listener,
            "/cgi-bin/zx_cmd.cgi?download=/data/camparam/Prgb.bin",
            &intrinsics_bytes(expected.intrinsics),
        );
        serve_file(
            &listener,
            "/cgi-bin/zx_cmd.cgi?download=/data/camparam/Distort.bin",
            &float_bytes(expected.distortion.coefficients),
        );
        let mut transform = expected.left_to_rgb.rotation.to_vec();
        transform.extend(expected.left_to_rgb.translation_mm);
        let transform: [f32; 12] = transform.try_into().expect("12 floats");
        serve_file(
            &listener,
            "/cgi-bin/zx_cmd.cgi?download=/data/camparam/LC_RT.bin",
            &float_bytes(transform),
        );
    });

    assert_eq!(
        get_rgb_calibration(address, limits()).expect("RGB calibration"),
        expected
    );
    server.join().expect("fixture server");
}
