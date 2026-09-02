use revopoint_pop3_wifi::depth_stream::{
    get_current_depth_resolution, parse_current_resolution, DepthResolution,
};
use revopoint_pop3_wifi::http_stream::StreamLimits;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

#[test]
fn parses_current_resolution_from_the_concatenated_device_response() {
    let response = concat!(
        r#"{"curr-resolution":"640x800x2"}"#,
        r#"{"depth-resolution":"1280x800x2-640x400x2"}"#,
        r#"{"depth+ir-resolution":"640x400x4"}"#,
    );

    let resolution = parse_current_resolution(response.as_bytes()).expect("current resolution");

    assert_eq!(
        resolution,
        DepthResolution {
            width: 640,
            height: 800,
            bytes_per_pixel: 2,
        }
    );
    assert_eq!(resolution.stride_bytes().expect("stride"), 1280);
    assert_eq!(resolution.frame_bytes().expect("frame bytes"), 1_024_000);
}

#[test]
fn rejects_missing_zero_extra_and_overflowing_components() {
    for response in [
        br#"{"depth-resolution":"640x400x2"}"#.as_slice(),
        br#"{"curr-resolution":"0x800x2"}"#,
        br#"{"curr-resolution":"640x800x2x1"}"#,
        br#"{"curr-resolution":"4294967295x4294967295x255"}"#,
    ] {
        assert!(
            parse_current_resolution(response).is_err(),
            "invalid response accepted: {}",
            String::from_utf8_lossy(response)
        );
    }
}

#[test]
fn explains_an_invalid_resolution_without_echoing_the_response() {
    let error = parse_current_resolution(br#"{"curr-resolution":"invalid"}"#)
        .expect_err("invalid resolution must fail");

    assert_eq!(
        error.to_string(),
        "scanner returned an invalid current depth resolution"
    );
}

#[test]
fn queries_the_read_only_current_resolution_endpoint() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
    let address = listener.local_addr().expect("fixture address");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept query");
        let mut request = Vec::new();
        while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
            let mut block = [0_u8; 64];
            let count = stream.read(&mut block).expect("read query");
            assert!(count > 0, "query ended before its headers");
            request.extend_from_slice(&block[..count]);
        }
        let request = String::from_utf8_lossy(&request);
        assert!(request
            .starts_with("GET /cgi-bin/zx_cmd.cgi?cam_type=mipi&get_depth_reso HTTP/1.1\r\n"));
        let body = br#"{"curr-resolution":"640x800x2"}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .expect("write response header");
        stream.write_all(body).expect("write response body");
    });
    let limits = StreamLimits {
        connect_timeout: Duration::from_secs(1),
        idle_timeout: Duration::from_secs(1),
        total_timeout: Duration::from_secs(2),
        max_header_bytes: 1024,
        max_body_bytes: 1024,
    };

    let resolution =
        get_current_depth_resolution(address, limits).expect("query current resolution");

    assert_eq!(resolution.frame_bytes(), Some(1_024_000));
    server.join().expect("fixture server");
}
