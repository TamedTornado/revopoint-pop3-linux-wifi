use revopoint_pop3_wifi::depth_stream::capture_depth_prefix;
use revopoint_pop3_wifi::http_stream::StreamLimits;
use std::error::Error;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

fn read_path(stream: &mut impl Read) -> String {
    let mut request = Vec::new();
    while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
        let mut block = [0_u8; 256];
        let count = stream.read(&mut block).expect("read request");
        assert!(count > 0, "request ended before header terminator");
        request.extend_from_slice(&block[..count]);
    }
    let request = String::from_utf8(request).expect("request is UTF-8");
    request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .expect("request path")
        .to_owned()
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

#[test]
fn configures_captures_a_prefix_and_closes_the_stream() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
    let address = listener.local_addr().expect("fixture address");
    let server = thread::spawn(move || {
        let (mut start, _) = listener.accept().expect("accept start request");
        assert_eq!(
            read_path(&mut start),
            "/cgi-bin/zx_cmd.cgi?cam_type=mipi&set_depth_output_fmt=1"
        );
        start
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 14\r\n\r\n{\"result\":0}\r\n")
            .expect("write start response");

        let (mut media, _) = listener.accept().expect("accept media request");
        assert_eq!(read_path(&mut media), "/cgi-bin/zx_media.cgi?camera_id=21");
        media
            .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n8\r\n12345678\r\n")
            .expect("write media response");

        let (mut close, _) = listener.accept().expect("accept close request");
        assert_eq!(
            read_path(&mut close),
            "/cgi-bin/zx_cmd.cgi?close_stream_all"
        );
        close
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\n\r\n{\"result\":0}")
            .expect("write close response");
    });

    let mut body = Vec::new();
    let received =
        capture_depth_prefix(address, limits(), 5, |chunk| body.extend_from_slice(chunk))
            .expect("capture depth prefix");

    assert_eq!(received, 5);
    assert_eq!(body, b"12345");
    server.join().expect("fixture server");
}

#[test]
fn closes_the_scanner_stream_after_a_capture_error() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
    let address = listener.local_addr().expect("fixture address");
    let server = thread::spawn(move || {
        let (mut start, _) = listener.accept().expect("accept start request");
        assert_eq!(
            read_path(&mut start),
            "/cgi-bin/zx_cmd.cgi?cam_type=mipi&set_depth_output_fmt=1"
        );
        start
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\n\r\n{\"result\":0}")
            .expect("write start response");

        let (mut media, _) = listener.accept().expect("accept media request");
        assert_eq!(read_path(&mut media), "/cgi-bin/zx_media.cgi?camera_id=21");
        media
            .write_all(b"HTTP/1.1 500 Failed\r\nContent-Length: 0\r\n\r\n")
            .expect("write media failure");

        let (mut close, _) = listener.accept().expect("accept close request");
        assert_eq!(
            read_path(&mut close),
            "/cgi-bin/zx_cmd.cgi?close_stream_all"
        );
        close
            .write_all(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nc\r\n{\"result\":0}\r\n0\r\n\r\n",
            )
            .expect("write close response");
    });

    let error = capture_depth_prefix(address, limits(), 5, |_| {})
        .expect_err("media failure must be reported");

    assert!(error.to_string().contains("capture depth media"));
    assert!(error.to_string().contains("500"));
    assert!(
        error.source().is_some(),
        "HTTP cause must remain inspectable"
    );
    server.join().expect("fixture server");
}
