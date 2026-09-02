use revopoint_pop3_wifi::depth_stream::capture_depth_prefix;
use revopoint_pop3_wifi::frame_envelope::FrameEnvelopeParser;
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

fn envelope(payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::from(0x1122_3344_u32.to_le_bytes());
    bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    bytes.extend_from_slice(payload);
    bytes
}

#[test]
fn configures_captures_a_prefix_and_closes_the_stream() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
    let address = listener.local_addr().expect("fixture address");
    let server = thread::spawn(move || {
        let (mut profile, _) = listener.accept().expect("accept profile request");
        assert_eq!(
            read_path(&mut profile),
            "/cgi-bin/zx_cmd.cgi?cam_type=mipi&set_display_reso=1&&set_display_width=640&&set_display_height=400&&set_display_type=2"
        );
        profile
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\n\r\n{\"result\":0}")
            .expect("write profile response");

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
        let (mut profile, _) = listener.accept().expect("accept profile request");
        assert_eq!(
            read_path(&mut profile),
            "/cgi-bin/zx_cmd.cgi?cam_type=mipi&set_display_reso=1&&set_display_width=640&&set_display_height=400&&set_display_type=2"
        );
        profile
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\n\r\n{\"result\":0}")
            .expect("write profile response");

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

#[test]
fn carries_fragmented_http_bytes_into_complete_frame_envelopes() {
    let mut media_body = Vec::from(b"\r\n\r\n".as_slice());
    media_body.extend_from_slice(&envelope(b"first"));
    media_body.extend_from_slice(&envelope(b"second"));
    let capture_bytes = media_body.len();
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
    let address = listener.local_addr().expect("fixture address");
    let server = thread::spawn(move || {
        let (mut profile, _) = listener.accept().expect("accept profile request");
        assert_eq!(
            read_path(&mut profile),
            "/cgi-bin/zx_cmd.cgi?cam_type=mipi&set_display_reso=1&&set_display_width=640&&set_display_height=400&&set_display_type=2"
        );
        profile
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\n\r\n{\"result\":0}")
            .expect("write profile response");

        let (mut start, _) = listener.accept().expect("accept start request");
        read_path(&mut start);
        start
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\n\r\n{\"result\":0}")
            .expect("write start response");

        let (mut media, _) = listener.accept().expect("accept media request");
        read_path(&mut media);
        write!(
            media,
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n",
            media_body.len()
        )
        .expect("write media header");
        for fragment in media_body.chunks(3) {
            media.write_all(fragment).expect("write media fragment");
        }

        let (mut close, _) = listener.accept().expect("accept close request");
        read_path(&mut close);
        close
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\n\r\n{\"result\":0}")
            .expect("write close response");
    });
    let mut parser = FrameEnvelopeParser::new(1024, 4);
    let mut payloads = Vec::new();

    capture_depth_prefix(address, limits(), capture_bytes, |chunk| {
        payloads.extend(
            parser
                .push(chunk)
                .expect("parse HTTP fragment")
                .into_iter()
                .map(|frame| frame.payload),
        );
    })
    .expect("capture framed stream");

    assert_eq!(payloads, [b"first".to_vec(), b"second".to_vec()]);
    parser.finish().expect("complete frame stream");
    server.join().expect("fixture server");
}
