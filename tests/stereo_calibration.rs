use revopoint_pop3_wifi::http_stream::StreamLimits;
use revopoint_pop3_wifi::stereo_calibration::{
    get_reprojection_matrix, get_stereo_map_parameters, parse_map_parameters,
    parse_reprojection_matrix, rectify_y8, MapParameters,
};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

fn fixture() -> (Vec<u8>, MapParameters) {
    let expected = MapParameters {
        calibration_width: 1280,
        calibration_height: 800,
        camera_matrix: [1700.0, 0.0, 640.0, 0.0, 1710.0, 400.0, 0.0, 0.0, 1.0],
        distortion: [0.02, 0.6, 0.0002, -0.0003, -3.7],
        inverse_rectification: [
            1.0 / 1700.0,
            0.0,
            -640.0 / 1700.0,
            0.0,
            1.0 / 1710.0,
            -400.0 / 1710.0,
            0.0,
            0.0,
            1.0,
        ],
    };
    let mut bytes = Vec::with_capacity(148);
    bytes.extend_from_slice(&expected.calibration_height.to_le_bytes());
    bytes.extend_from_slice(&expected.calibration_width.to_le_bytes());
    bytes.extend_from_slice(&5_u32.to_le_bytes());
    for value in [1.0_f32, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    for value in [640.0_f32, 400.0, 1700.0, 1710.0] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    for value in expected.distortion {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    for _ in 0..7 {
        bytes.extend_from_slice(&0.0_f32.to_le_bytes());
    }
    for value in expected.inverse_rectification {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    (bytes, expected)
}

#[test]
fn parses_the_fixed_map_parameter_layout() {
    let (bytes, expected) = fixture();

    assert_eq!(bytes.len(), 148);
    assert_eq!(
        parse_map_parameters(&bytes).expect("map parameters"),
        expected
    );
}

#[test]
fn rejects_wrong_size_count_dimensions_and_nonfinite_values() {
    let (valid, _) = fixture();
    assert!(parse_map_parameters(&valid[..147]).is_err());

    for (offset, replacement) in [
        (0, 0_u32.to_le_bytes()),
        (4, 0_u32.to_le_bytes()),
        (8, 4_u32.to_le_bytes()),
        (56, 0_u32.to_le_bytes()),
        (56, (-1.0_f32).to_le_bytes()),
        (60, f32::NAN.to_le_bytes()),
        (48, f32::NAN.to_le_bytes()),
        (52, f32::INFINITY.to_le_bytes()),
        (64, f32::NAN.to_le_bytes()),
        (144, f32::INFINITY.to_le_bytes()),
    ] {
        let mut invalid = valid.clone();
        invalid[offset..offset + 4].copy_from_slice(&replacement);
        assert!(parse_map_parameters(&invalid).is_err(), "offset {offset}");
    }

    let mut singular = valid;
    singular[112..148].fill(0);
    assert!(parse_map_parameters(&singular).is_err());
}

#[test]
fn error_does_not_echo_binary_calibration_data() {
    let error = parse_map_parameters(&[0xaa; 148]).expect_err("invalid map");

    assert_eq!(
        error.to_string(),
        "scanner returned invalid stereo map parameters"
    );
    assert!(!error.to_string().contains("aa"));
}

#[test]
fn identity_rectification_preserves_a_y8_image() {
    let (_, mut parameters) = fixture();
    parameters.calibration_width = 4;
    parameters.calibration_height = 3;
    parameters.camera_matrix = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
    parameters.distortion = [0.0; 5];
    parameters.inverse_rectification = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
    let input = (0_u8..12).collect::<Vec<_>>();

    assert_eq!(
        rectify_y8(&input, 4, 3, parameters).expect("rectified image"),
        input
    );
}

#[test]
fn rectification_bilinearly_samples_subpixel_coordinates() {
    let (_, mut parameters) = fixture();
    parameters.calibration_width = 3;
    parameters.calibration_height = 3;
    parameters.camera_matrix = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
    parameters.distortion = [0.0; 5];
    parameters.inverse_rectification = [1.0, 0.0, 0.5, 0.0, 1.0, 0.5, 0.0, 0.0, 1.0];
    let input = vec![0, 10, 20, 100, 110, 120, 200, 210, 220];

    let rectified = rectify_y8(&input, 3, 3, parameters).expect("subpixel rectification");

    assert_eq!(rectified[0], 55);
    assert_eq!(rectified[1], 65);
    assert_eq!(rectified[3], 155);
    assert_eq!(rectified[2], 0, "out-of-bounds samples remain invalid");

    parameters.inverse_rectification = [1.0, 0.0, 0.5, 0.0, 1.0, -0.5, 0.0, 0.0, 1.0];
    let rectified = rectify_y8(&input, 3, 3, parameters).expect("bounded rectification");
    assert_eq!(rectified[0], 0, "negative source rows remain invalid");
}

#[test]
fn rectification_rejects_invalid_image_layout_and_projection() {
    let (_, mut parameters) = fixture();
    assert!(rectify_y8(&[0; 11], 4, 3, parameters).is_err());
    assert!(rectify_y8(&[], 0, 3, parameters).is_err());

    parameters.inverse_rectification = [0.0; 9];
    assert!(rectify_y8(&[0; 12], 4, 3, parameters).is_err());
}

#[test]
fn downloads_both_read_only_stereo_maps() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
    let address = listener.local_addr().expect("fixture address");
    let (body, expected) = fixture();
    let server = thread::spawn(move || {
        for side in ['L', 'R'] {
            let (mut stream, _) = listener.accept().expect("accept map request");
            let mut request = Vec::new();
            while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                let mut block = [0_u8; 256];
                let count = stream.read(&mut block).expect("read request");
                assert!(count > 0, "request ended before headers");
                request.extend_from_slice(&block[..count]);
            }
            assert!(String::from_utf8_lossy(&request).starts_with(&format!(
                "GET /cgi-bin/zx_cmd.cgi?download=/data/camparam/mapparam{side}.bin HTTP/1.1\r\n"
            )));
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                body.len()
            )
            .expect("write response header");
            stream.write_all(&body).expect("write response body");
        }
    });
    let limits = StreamLimits {
        connect_timeout: Duration::from_secs(1),
        idle_timeout: Duration::from_secs(1),
        total_timeout: Duration::from_secs(2),
        max_header_bytes: 1024,
        max_body_bytes: 1024,
    };

    let maps = get_stereo_map_parameters(address, limits).expect("stereo maps");
    assert_eq!(maps.left, expected);
    assert_eq!(maps.right, expected);
    server.join().expect("fixture server");
}

#[test]
fn parses_q_and_converts_scaled_disparity_to_metric_depth() {
    let mut values = [0.0_f32; 16];
    values[0] = 1.0;
    values[5] = 1.0;
    values[11] = 100.0;
    values[14] = 0.1;
    let bytes = values
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect::<Vec<_>>();

    let q = parse_reprojection_matrix(&bytes).expect("Q matrix");

    assert_eq!(q.depth_mm(10.0, 1.0), Some(100.0));
    assert_eq!(q.depth_mm(5.0, 2.0), Some(100.0));
    assert_eq!(q.depth_mm(0.0, 1.0), None);
    assert_eq!(q.depth_mm(10.0, 0.0), None);
    assert_eq!(q.depth_mm(-1.0, 1.0), None);
    assert_eq!(q.depth_mm(f32::NAN, 1.0), None);
    assert_eq!(q.depth_mm(1.0, f32::NAN), None);

    let mut full_q = q;
    full_q.values[3] = -2.0;
    full_q.values[7] = -3.0;
    full_q.values[12] = 0.01;
    full_q.values[13] = 0.02;
    full_q.values[15] = 1.0;
    let point = full_q
        .point_mm(4.0, 5.0, 4.0, 1.0, 1.0)
        .expect("reprojected point");
    assert!((point[0] - 2.0 / 1.54).abs() < 0.001);
    assert!((point[1] - 2.0 / 1.54).abs() < 0.001);
    assert!((point[2] - 100.0 / 1.54).abs() < 0.001);
    let scaled_point = full_q
        .point_mm(4.0, 5.0, 4.0, 2.0, 3.0)
        .expect("scaled reprojected point");
    assert!((scaled_point[0] - 6.0 / 2.18).abs() < 0.001);
    assert!((scaled_point[1] - 12.0 / 2.18).abs() < 0.001);
    assert!((scaled_point[2] - 100.0 / 2.18).abs() < 0.001);

    for invalid in [
        full_q.point_mm(f32::NAN, 1.0, 1.0, 1.0, 1.0),
        full_q.point_mm(-1.0, 1.0, 1.0, 1.0, 1.0),
        full_q.point_mm(1.0, f32::NAN, 1.0, 1.0, 1.0),
        full_q.point_mm(1.0, -1.0, 1.0, 1.0, 1.0),
        full_q.point_mm(1.0, 1.0, f32::NAN, 1.0, 1.0),
        full_q.point_mm(1.0, 1.0, -1.0, 1.0, 1.0),
        full_q.point_mm(1.0, 1.0, 1.0, f32::NAN, 1.0),
        full_q.point_mm(1.0, 1.0, 1.0, 0.0, 1.0),
        full_q.point_mm(1.0, 1.0, 1.0, 1.0, f32::NAN),
        full_q.point_mm(1.0, 1.0, 1.0, 1.0, 0.0),
    ] {
        assert_eq!(invalid, None);
    }

    let mut offset_q = q;
    offset_q.values[15] = 2.0;
    assert_eq!(offset_q.depth_mm(-1.0, 1.0), None);
    assert_eq!(offset_q.depth_mm(0.0, 1.0), Some(50.0));
    assert!((offset_q.depth_mm(10.0, 1.0).expect("depth") - 100.0 / 3.0).abs() < 0.001);

    let mut epsilon_q = q;
    epsilon_q.values[14] = f32::EPSILON;
    assert_eq!(epsilon_q.depth_mm(1.0, 1.0), None);
    let mut zero_depth_q = q;
    zero_depth_q.values[11] = 0.0;
    assert_eq!(zero_depth_q.depth_mm(10.0, 1.0), None);
    let mut negative_depth_q = q;
    negative_depth_q.values[15] = -2.0;
    assert_eq!(negative_depth_q.depth_mm(10.0, 1.0), None);
}

#[test]
fn rejects_malformed_or_degenerate_q() {
    assert!(parse_reprojection_matrix(&[0; 63]).is_err());
    assert!(parse_reprojection_matrix(&[0; 64]).is_err());

    let mut invalid = [0_u8; 64];
    invalid[0..4].copy_from_slice(&f32::NAN.to_le_bytes());
    assert!(parse_reprojection_matrix(&invalid).is_err());

    let mut valid = [0_u8; 64];
    for (index, value) in [(0, 1.0_f32), (5, 1.0), (11, 100.0), (14, 0.1)] {
        valid[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    for (index, value) in [(0, 0.0_f32), (5, 0.0), (11, 0.0), (14, 0.0)] {
        let mut bytes = valid;
        bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
        assert!(parse_reprojection_matrix(&bytes).is_err(), "Q[{index}]");
    }
}

#[test]
fn downloads_the_read_only_q_matrix() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
    let address = listener.local_addr().expect("fixture address");
    let mut values = [0.0_f32; 16];
    values[0] = 1.0;
    values[5] = 1.0;
    values[11] = 100.0;
    values[14] = 0.1;
    let body = values
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect::<Vec<_>>();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept Q request");
        let mut request = Vec::new();
        while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
            let mut block = [0_u8; 256];
            let count = stream.read(&mut block).expect("read request");
            assert!(count > 0, "request ended before headers");
            request.extend_from_slice(&block[..count]);
        }
        assert!(String::from_utf8_lossy(&request).starts_with(
            "GET /cgi-bin/zx_cmd.cgi?download=/data/camparam/camparamLR/Q.bin HTTP/1.1\r\n"
        ));
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .expect("write response header");
        stream.write_all(&body).expect("write response body");
    });

    let q = get_reprojection_matrix(address, limits()).expect("Q matrix");
    assert_eq!(q.values, values);
    server.join().expect("fixture server");
}

fn limits() -> StreamLimits {
    StreamLimits {
        connect_timeout: Duration::from_secs(1),
        idle_timeout: Duration::from_secs(1),
        total_timeout: Duration::from_secs(2),
        max_header_bytes: 1024,
        max_body_bytes: 1024,
    }
}
