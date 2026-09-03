use revopoint_pop3_wifi::http_stream::StreamLimits;
use revopoint_pop3_wifi::rgb_stream::capture_rgb_prefix;
use std::error::Error;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

fn limits() -> StreamLimits {
    StreamLimits {
        connect_timeout: Duration::from_secs(1),
        idle_timeout: Duration::from_secs(1),
        total_timeout: Duration::from_secs(3),
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

fn expect_control(listener: &TcpListener, path: &str, body: &[u8]) {
    let (mut stream, _) = listener.accept().expect("accept control request");
    assert_eq!(read_path(&mut stream), path);
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )
    .expect("write response header");
    stream.write_all(body).expect("write response body");
}

#[test]
fn configures_captures_and_cleans_up_the_rgb_stream() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
    let address = listener.local_addr().expect("fixture address");
    let server = thread::spawn(move || {
        expect_control(
            &listener,
            "/cgi-bin/zx_cmd.cgi?close_stream_all",
            b"{result:0}",
        );
        expect_control(
            &listener,
            "/cgi-bin/zx_cmd.cgi?cam_type=usb&set_resolution=1&width=1280&height=800",
            br#"{"result":0}"#,
        );
        expect_control(
            &listener,
            "/cgi-bin/zx_cmd.cgi?system_cmd=echo%20s%200xb00%201%20%3E%20/dev/rk_preisp",
            b"[ok]",
        );
        expect_control(
            &listener,
            "/cgi-bin/zx_cmd.cgi?system_cmd=echo%20s%200xb02%201%20%3E%20/dev/rk_preisp",
            b"[ok]",
        );
        expect_control(
            &listener,
            "/cgi-bin/zx_cmd.cgi?cam_type=mipi&set_trigger_mode=0",
            br#"{"result":0}"#,
        );

        let (mut media, _) = listener.accept().expect("accept RGB media request");
        assert_eq!(
            read_path(&mut media),
            "/cgi-bin/zx_media.cgi?camera_id=50&type_id=20"
        );
        media
            .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nimage\r\n")
            .expect("write RGB media");

        expect_control(
            &listener,
            "/cgi-bin/zx_cmd.cgi?close_stream_all",
            br#"{"result":0}"#,
        );
        expect_control(
            &listener,
            "/cgi-bin/zx_cmd.cgi?system_cmd=echo%20s%200xb02%200%20%3E%20/dev/rk_preisp",
            b"[ok]",
        );
        expect_control(
            &listener,
            "/cgi-bin/zx_cmd.cgi?system_cmd=echo%20s%200xb00%200%20%3E%20/dev/rk_preisp",
            b"[ok]",
        );
    });
    let mut body = Vec::new();

    let received = capture_rgb_prefix(address, limits(), 5, |chunk| body.extend_from_slice(chunk))
        .expect("capture RGB prefix");

    assert_eq!(received, 5);
    assert_eq!(body, b"image");
    server.join().expect("fixture server");
}

#[test]
fn preserves_stage_and_source_for_rgb_http_failures() {
    let error = capture_rgb_prefix(
        "127.0.0.1:1".parse().expect("socket address"),
        limits(),
        1,
        |_| {},
    )
    .expect_err("unreachable fixture must fail");

    assert!(error.to_string().contains("close existing streams"));
    assert!(error.source().is_some());
}

#[test]
fn describes_rejected_rgb_controls_without_a_source() {
    let error = revopoint_pop3_wifi::rgb_stream::RgbStreamError::Rejected("RGB test control");

    assert_eq!(error.to_string(), "scanner rejected RGB test control");
    assert!(error.source().is_none());
}
