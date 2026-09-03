use revopoint_pop3_wifi::camera_control::{
    depth_exposure_range, set_depth_control, DepthAutoExposure, DepthControl, DepthControlError,
    DepthExposureRange,
};
use revopoint_pop3_wifi::http_stream::StreamLimits;
use std::error::Error;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

fn limits() -> StreamLimits {
    StreamLimits {
        connect_timeout: Duration::from_secs(1),
        idle_timeout: Duration::from_secs(1),
        total_timeout: Duration::from_secs(2),
        max_header_bytes: 1024,
        max_body_bytes: 1024,
    }
}

fn read_path(stream: &mut impl Read) -> String {
    let mut request = Vec::new();
    while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
        let mut block = [0_u8; 256];
        let count = stream.read(&mut block).expect("read request");
        assert!(count > 0);
        request.extend_from_slice(&block[..count]);
    }
    String::from_utf8(request)
        .expect("UTF-8 request")
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .expect("request path")
        .to_owned()
}

fn respond(stream: &mut impl Write, body: &[u8]) {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )
    .expect("write header");
    stream.write_all(body).expect("write body");
}

fn accept(listener: &TcpListener) -> std::net::TcpStream {
    listener
        .set_nonblocking(true)
        .expect("set fixture nonblocking");
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match listener.accept() {
            Ok((stream, _)) => return stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    Instant::now() < deadline,
                    "timed out waiting for control request"
                );
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("accept control request: {error}"),
        }
    }
}

#[test]
fn reads_and_validates_the_device_exposure_range() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture");
    let address = listener.local_addr().expect("fixture address");
    let server = thread::spawn(move || {
        let mut stream = accept(&listener);
        assert_eq!(
            read_path(&mut stream),
            "/cgi-bin/zx_cmd.cgi?cam_type=mipi&get_exposureRange"
        );
        respond(
            &mut stream,
            br#"{"min":5000,"max":65000,"step":7,"default":5007}"#,
        );
    });

    assert_eq!(
        depth_exposure_range(address, limits()).expect("valid range"),
        DepthExposureRange {
            minimum_us: 5000,
            maximum_us: 65000,
            step_us: 7,
            default_us: 5007,
        }
    );
    server.join().expect("fixture server");
}

#[test]
fn manual_exposure_disables_auto_and_sets_frame_time_before_exposure() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture");
    let address = listener.local_addr().expect("fixture address");
    let server = thread::spawn(move || {
        for expected in [
            "/cgi-bin/zx_cmd.cgi?cam_type=mipi&get_exposureRange",
            "/cgi-bin/zx_cmd.cgi?system_cmd=echo%20s%200x912%200%20%3E%20/dev/rk_preisp",
            "/cgi-bin/zx_cmd.cgi?system_cmd=echo%20s%200x910%207000%20%3E%20/dev/rk_preisp",
            "/cgi-bin/zx_cmd.cgi?system_cmd=echo%20s%200x911%205000%20%3E%20/dev/rk_preisp",
        ] {
            let mut stream = accept(&listener);
            assert_eq!(read_path(&mut stream), expected);
            if expected.ends_with("get_exposureRange") {
                respond(
                    &mut stream,
                    br#"{"min":5000,"max":65000,"step":1,"default":5000}"#,
                );
            } else {
                respond(&mut stream, b"[ok]");
            }
        }
    });

    set_depth_control(address, limits(), DepthControl::ManualExposureUs(5000))
        .expect("manual exposure");
    server.join().expect("fixture server");
}

#[test]
fn auto_exposure_modes_map_to_vendor_values() {
    for (mode, value) in [
        (DepthAutoExposure::Off, 0),
        (DepthAutoExposure::FixedFrameTime, 1),
        (DepthAutoExposure::HighQuality, 2),
        (DepthAutoExposure::Foreground, 3),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture");
        let address = listener.local_addr().expect("fixture address");
        let server = thread::spawn(move || {
            let mut stream = accept(&listener);
            assert_eq!(
                read_path(&mut stream),
                format!(
                    "/cgi-bin/zx_cmd.cgi?system_cmd=echo%20s%200x912%20{value}%20%3E%20/dev/rk_preisp"
                )
            );
            respond(&mut stream, b"[ok]");
        });
        set_depth_control(address, limits(), DepthControl::AutoExposure(mode))
            .expect("auto exposure");
        server.join().expect("fixture server");
    }
}

#[test]
fn rejects_manual_exposure_outside_the_device_range_without_writing() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture");
    let address = listener.local_addr().expect("fixture address");
    let server = thread::spawn(move || {
        let mut stream = accept(&listener);
        read_path(&mut stream);
        respond(
            &mut stream,
            br#"{"min":5000,"max":65000,"step":10,"default":5000}"#,
        );
    });

    let error = set_depth_control(address, limits(), DepthControl::ManualExposureUs(4999))
        .expect_err("below-minimum exposure must fail");
    assert!(matches!(
        error,
        DepthControlError::ExposureOutOfRange { .. }
    ));
    server.join().expect("fixture server");
}

#[test]
fn rejects_misaligned_and_malformed_ranges() {
    let range = DepthExposureRange {
        minimum_us: 5000,
        maximum_us: 65000,
        step_us: 7,
        default_us: 5000,
    };
    assert!(range.validate(5001).is_err());
    assert!(range.validate(5007).is_ok());
    assert!(range.validate(65000).is_err());

    let unit_step = DepthExposureRange {
        minimum_us: 5000,
        maximum_us: 65000,
        step_us: 1,
        default_us: 5000,
    };
    assert!(unit_step.validate(65000).is_ok());
    assert!(unit_step.validate(65001).is_err());

    for body in [
        br#"{"min":0,"max":65000,"step":1,"default":5000}"#.as_slice(),
        br#"{"min":65000,"max":5000,"step":1,"default":5000}"#.as_slice(),
        br#"{"min":5000,"max":65000,"step":0,"default":5000}"#.as_slice(),
        br#"{"min":5000,"max":65000,"step":1,"default":70000}"#.as_slice(),
        b"not-json".as_slice(),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture");
        let address = listener.local_addr().expect("fixture address");
        let body = body.to_vec();
        let server = thread::spawn(move || {
            let mut stream = accept(&listener);
            read_path(&mut stream);
            respond(&mut stream, &body);
        });
        assert!(depth_exposure_range(address, limits()).is_err());
        server.join().expect("fixture server");
    }
}

#[test]
fn errors_have_actionable_text_and_preserve_http_sources() {
    let invalid = "automatic"
        .parse::<DepthAutoExposure>()
        .expect_err("invalid mode");
    assert!(invalid.to_string().contains("fixed-frame-time"));

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture");
    let address = listener.local_addr().expect("fixture address");
    let server = thread::spawn(move || {
        let mut stream = accept(&listener);
        read_path(&mut stream);
        respond(&mut stream, b"not-json");
    });
    let malformed = depth_exposure_range(address, limits()).expect_err("malformed range");
    assert_eq!(
        malformed.to_string(),
        "scanner returned an invalid depth exposure range"
    );
    assert!(malformed.source().is_none());
    server.join().expect("fixture server");

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture");
    let address = listener.local_addr().expect("fixture address");
    let server = thread::spawn(move || {
        let mut stream = accept(&listener);
        read_path(&mut stream);
        stream
            .write_all(b"HTTP/1.1 500 Failed\r\nContent-Length: 0\r\n\r\n")
            .expect("write failure");
    });
    let http = depth_exposure_range(address, limits()).expect_err("HTTP failure");
    assert!(http.to_string().contains("query depth exposure range"));
    assert!(http.source().is_some());
    server.join().expect("fixture server");
}
