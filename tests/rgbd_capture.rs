use revopoint_pop3_wifi::camera_control::{DepthAutoExposure, DepthControl};
use revopoint_pop3_wifi::http_stream::StreamLimits;
use revopoint_pop3_wifi::rgbd_stream::RgbdStreamError;
use revopoint_pop3_wifi::rgbd_stream::{capture_rgbd_until, capture_rgbd_until_with_control};
use std::error::Error;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

fn limits() -> StreamLimits {
    StreamLimits {
        connect_timeout: Duration::from_secs(1),
        idle_timeout: Duration::from_secs(1),
        total_timeout: Duration::from_secs(4),
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
fn configures_both_sensors_captures_both_endpoints_and_cleans_up() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
    let address = listener.local_addr().expect("fixture address");
    let server = thread::spawn(move || {
        for (path, body) in [
            ("/cgi-bin/zx_cmd.cgi?close_stream_all", b"{result:0}".as_slice()),
            ("/cgi-bin/zx_cmd.cgi?cam_type=mipi&set_display_reso=1&&set_display_width=640&&set_display_height=400&&set_display_type=4", br#"{"result":0}"#.as_slice()),
            ("/cgi-bin/zx_cmd.cgi?cam_type=mipi&set_depth_output_fmt=3", br#"{"result":0}"#.as_slice()),
            ("/cgi-bin/zx_cmd.cgi?cam_type=usb&set_resolution=1&width=1280&height=800", br#"{"result":0}"#.as_slice()),
            ("/cgi-bin/zx_cmd.cgi?cam_type=mipi&set_trigger_mode=0", br#"{"result":0}"#.as_slice()),
            ("/cgi-bin/zx_cmd.cgi?system_cmd=echo%20s%200x912%203%20%3E%20/dev/rk_preisp", b"[ok]".as_slice()),
            ("/cgi-bin/zx_cmd.cgi?system_cmd=echo%20s%200xb00%201%20%3E%20/dev/rk_preisp", b"[ok]".as_slice()),
            ("/cgi-bin/zx_cmd.cgi?system_cmd=echo%20s%200xb01%201%20%3E%20/dev/rk_preisp", b"[ok]".as_slice()),
            ("/cgi-bin/zx_cmd.cgi?system_cmd=echo%20s%200xb02%201%20%3E%20/dev/rk_preisp", b"[ok]".as_slice()),
        ] {
            expect_control(&listener, path, body);
        }

        let mut media_streams = Vec::new();
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().expect("accept media request");
            let path = read_path(&mut stream);
            let body = match path.as_str() {
                "/cgi-bin/zx_media.cgi?camera_id=21" => b"depth".as_slice(),
                "/cgi-bin/zx_media.cgi?camera_id=50&type_id=20" => b"color".as_slice(),
                _ => panic!("unexpected media path {path}"),
            };
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n",
                body.len()
            )
            .expect("write media header");
            stream.write_all(body).expect("write media body");
            media_streams.push((stream, body));
        }
        thread::sleep(Duration::from_millis(10));
        for (mut stream, body) in media_streams {
            let _ = write!(stream, "\r\n{:x}\r\n", body.len());
            let _ = stream.write_all(body);
        }

        for (path, body) in [
            (
                "/cgi-bin/zx_cmd.cgi?close_stream_all",
                br#"{"result":0}"#.as_slice(),
            ),
            (
                "/cgi-bin/zx_cmd.cgi?system_cmd=echo%20s%200xb02%200%20%3E%20/dev/rk_preisp",
                b"[ok]".as_slice(),
            ),
            (
                "/cgi-bin/zx_cmd.cgi?system_cmd=echo%20s%200xb01%200%20%3E%20/dev/rk_preisp",
                b"[ok]".as_slice(),
            ),
            (
                "/cgi-bin/zx_cmd.cgi?system_cmd=echo%20s%200xb00%200%20%3E%20/dev/rk_preisp",
                b"[ok]".as_slice(),
            ),
        ] {
            expect_control(&listener, path, body);
        }
    });
    let mut depth = Vec::new();
    let mut rgb = Vec::new();

    let received = capture_rgbd_until_with_control(
        address,
        limits(),
        DepthControl::AutoExposure(DepthAutoExposure::Foreground),
        |chunk| {
            depth.extend_from_slice(chunk);
            depth.len() >= 5
        },
        |chunk| {
            rgb.extend_from_slice(chunk);
            rgb.len() >= 5
        },
    )
    .expect("concurrent RGB-D capture");

    assert!(received.0 >= 5);
    assert!(received.1 >= 5);
    assert!(depth.starts_with(b"depth"));
    assert!(rgb.starts_with(b"color"));
    server.join().expect("fixture server");
}

#[test]
fn concurrent_stream_errors_preserve_stage_and_source_context() {
    let error = capture_rgbd_until(
        "127.0.0.1:1".parse().expect("socket address"),
        limits(),
        |_| true,
        |_| true,
    )
    .expect_err("unreachable scanner must fail");
    assert!(error.to_string().contains("close existing streams"));
    assert!(error.source().is_some());

    let rejected = RgbdStreamError::Rejected("test control");
    assert_eq!(rejected.to_string(), "scanner rejected test control");
    assert!(rejected.source().is_none());

    let panicked = RgbdStreamError::WorkerPanicked;
    assert_eq!(panicked.to_string(), "RGB-D capture worker panicked");
    assert!(panicked.source().is_none());
}
