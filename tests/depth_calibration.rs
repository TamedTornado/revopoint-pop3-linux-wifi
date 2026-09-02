use revopoint_pop3_wifi::calibration::{
    get_depth_intrinsics, parse_depth_intrinsics, CalibrationError, CalibrationQueryError,
    DepthIntrinsics,
};
use revopoint_pop3_wifi::http_stream::StreamLimits;
use std::error::Error;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

fn intrinsics_bytes(intrinsics: DepthIntrinsics) -> [u8; 40] {
    let mut bytes = [0_u8; 40];
    bytes[0..2].copy_from_slice(&intrinsics.calibration_width.to_le_bytes());
    bytes[2..4].copy_from_slice(&intrinsics.calibration_height.to_le_bytes());
    for (index, value) in [
        intrinsics.fx,
        0.0,
        intrinsics.cx,
        0.0,
        intrinsics.fy,
        intrinsics.cy,
        0.0,
        0.0,
        1.0,
    ]
    .into_iter()
    .enumerate()
    {
        let start = 4 + index * 4;
        bytes[start..start + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn fixture_intrinsics() -> DepthIntrinsics {
    DepthIntrinsics {
        calibration_width: 1280,
        calibration_height: 800,
        fx: 1600.0,
        fy: 1500.0,
        cx: 600.0,
        cy: 390.0,
    }
}

#[test]
fn parses_the_fixed_depth_intrinsics_layout() {
    assert_eq!(
        parse_depth_intrinsics(&intrinsics_bytes(fixture_intrinsics())).expect("intrinsics"),
        fixture_intrinsics()
    );
}

#[test]
fn scales_intrinsics_to_the_selected_depth_resolution() {
    let scaled = fixture_intrinsics()
        .for_resolution(640, 400)
        .expect("scaled intrinsics");

    assert_eq!(scaled.width, 640);
    assert_eq!(scaled.height, 400);
    assert_eq!(scaled.fx, 800.0);
    assert_eq!(scaled.fy, 750.0);
    assert_eq!(scaled.cx, 300.0);
    assert_eq!(scaled.cy, 195.0);
}

#[test]
fn rejects_malformed_or_degenerate_intrinsics() {
    let valid = intrinsics_bytes(fixture_intrinsics());
    assert!(parse_depth_intrinsics(&valid[..39]).is_err());

    for matrix_index in [1, 3, 6, 7, 8] {
        let mut noncanonical_matrix = valid;
        let start = 4 + matrix_index * 4;
        noncanonical_matrix[start..start + 4].copy_from_slice(&2.0_f32.to_le_bytes());
        assert!(parse_depth_intrinsics(&noncanonical_matrix).is_err());
    }

    for invalid in [
        DepthIntrinsics {
            calibration_width: 0,
            ..fixture_intrinsics()
        },
        DepthIntrinsics {
            fx: 0.0,
            ..fixture_intrinsics()
        },
        DepthIntrinsics {
            fy: 0.0,
            ..fixture_intrinsics()
        },
        DepthIntrinsics {
            cy: f32::NAN,
            ..fixture_intrinsics()
        },
    ] {
        assert!(parse_depth_intrinsics(&intrinsics_bytes(invalid)).is_err());
        assert!(invalid.for_resolution(640, 400).is_err());
    }
    assert!(fixture_intrinsics().for_resolution(0, 400).is_err());
    assert!(fixture_intrinsics().for_resolution(640, 0).is_err());
}

#[test]
fn calibration_errors_preserve_context_and_sources() {
    let invalid = CalibrationError;
    assert_eq!(
        invalid.to_string(),
        "scanner returned invalid depth intrinsics"
    );
    let query = CalibrationQueryError::Invalid(invalid);
    assert_eq!(
        query.to_string(),
        "scanner returned invalid depth intrinsics"
    );
    assert_eq!(
        query.source().expect("calibration source").to_string(),
        "scanner returned invalid depth intrinsics"
    );
}

#[test]
fn downloads_the_read_only_depth_intrinsics_file() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
    let address = listener.local_addr().expect("fixture address");
    let expected = fixture_intrinsics();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept query");
        let mut request = Vec::new();
        while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
            let mut block = [0_u8; 64];
            let count = stream.read(&mut block).expect("read query");
            assert!(count > 0, "query ended before its headers");
            request.extend_from_slice(&block[..count]);
        }
        assert!(String::from_utf8_lossy(&request)
            .starts_with("GET /cgi-bin/zx_cmd.cgi?download=/data/camparam/Pl.bin HTTP/1.1\r\n"));
        let body = intrinsics_bytes(expected);
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .expect("write response header");
        stream.write_all(&body).expect("write response body");
    });
    let limits = StreamLimits {
        connect_timeout: Duration::from_secs(1),
        idle_timeout: Duration::from_secs(1),
        total_timeout: Duration::from_secs(2),
        max_header_bytes: 1024,
        max_body_bytes: 1024,
    };

    assert_eq!(
        get_depth_intrinsics(address, limits).expect("query intrinsics"),
        expected
    );
    server.join().expect("fixture server");
}
