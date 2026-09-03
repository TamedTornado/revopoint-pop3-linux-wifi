use revopoint_pop3_wifi::depth_stream::{
    capture_depth_prefix, capture_pair_prefix, DepthStreamError,
};
use revopoint_pop3_wifi::frame_envelope::FrameEnvelopeParser;
use revopoint_pop3_wifi::http_stream::StreamLimits;
use std::error::Error;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

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
fn requests_vendor_verified_z16y8y8_selector_for_depth() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
    let address = listener.local_addr().expect("fixture address");
    let server = thread::spawn(move || {
        let (mut close, _) = listener.accept().expect("accept initial close request");
        assert_eq!(
            read_path(&mut close),
            "/cgi-bin/zx_cmd.cgi?close_stream_all"
        );
        close
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\n\r\n{\"result\":0}")
            .expect("write initial close response");

        let (mut profile, _) = listener.accept().expect("accept profile request");
        assert_eq!(
            read_path(&mut profile),
            "/cgi-bin/zx_cmd.cgi?cam_type=mipi&set_display_reso=1&&set_display_width=640&&set_display_height=400&&set_display_type=4"
        );
        profile
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\n\r\n{\"result\":0}")
            .expect("write profile response");

        let (mut selector, _) = listener.accept().expect("accept selector request");
        assert_eq!(
            read_path(&mut selector),
            "/cgi-bin/zx_cmd.cgi?cam_type=mipi&set_depth_output_fmt=3"
        );
        selector
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\n\r\n{\"result\":0}")
            .expect("write selector response");

        let (mut trigger, _) = listener.accept().expect("accept trigger-mode request");
        assert_eq!(
            read_path(&mut trigger),
            "/cgi-bin/zx_cmd.cgi?cam_type=mipi&set_trigger_mode=0"
        );
        trigger
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\n\r\n{\"result\":0}")
            .expect("write trigger-mode response");

        expect_control(&listener, LED_MASTER_ON_PATH);
        expect_control(&listener, LED_IR_ON_PATH);

        let (mut media, _) = listener.accept().expect("accept media request");
        assert_eq!(read_path(&mut media), "/cgi-bin/zx_media.cgi?camera_id=21");
        media
            .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\ndepth\r\n")
            .expect("write media response");

        let (mut close, _) = listener.accept().expect("accept close request");
        assert_eq!(
            read_path(&mut close),
            "/cgi-bin/zx_cmd.cgi?close_stream_all"
        );
        close
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\n\r\n{\"result\":0}")
            .expect("write close response");
        expect_control(&listener, LED_IR_OFF_PATH);
        expect_control(&listener, LED_MASTER_OFF_PATH);
    });

    let mut body = Vec::new();
    let received =
        capture_depth_prefix(address, limits(), 5, |chunk| body.extend_from_slice(chunk))
            .expect("capture Z16Y8Y8 prefix");

    assert_eq!(received, 5);
    assert_eq!(body, b"depth");
    server.join().expect("fixture server");
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

        expect_control(&listener, LED_MASTER_ON_PATH);
        expect_control(&listener, LED_IR_ON_PATH);

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
        expect_control(&listener, LED_IR_OFF_PATH);
        expect_control(&listener, LED_MASTER_OFF_PATH);
    });

    let mut body = Vec::new();
    let received = capture_pair_prefix(address, limits(), 5, |chunk| body.extend_from_slice(chunk))
        .expect("capture PAIR prefix");

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

        expect_control(&listener, LED_MASTER_ON_PATH);
        expect_control(&listener, LED_IR_ON_PATH);

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
        expect_control(&listener, LED_IR_OFF_PATH);
        expect_control(&listener, LED_MASTER_OFF_PATH);
    });

    let error = capture_pair_prefix(address, limits(), 5, |_| {})
        .expect_err("media failure must be reported");

    assert!(error.to_string().contains("capture PAIR media"));
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

        expect_control(&listener, LED_MASTER_ON_PATH);
        expect_control(&listener, LED_IR_ON_PATH);

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
        expect_control(&listener, LED_IR_OFF_PATH);
        expect_control(&listener, LED_MASTER_OFF_PATH);
    });
    let mut parser = FrameEnvelopeParser::new(1024, 4);
    let mut payloads = Vec::new();

    capture_pair_prefix(address, limits(), capture_bytes, |chunk| {
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

#[test]
fn retries_a_rejected_depth_selector_without_power_cycling() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
    let address = listener.local_addr().expect("fixture address");
    let server = thread::spawn(move || {
        let (mut profile, _) = listener.accept().expect("accept profile request");
        read_path(&mut profile);
        profile
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\n\r\n{\"result\":0}")
            .expect("write profile response");

        let (mut first, _) = listener.accept().expect("accept first selector request");
        assert_eq!(read_path(&mut first), SET_DEPTH_SELECTOR_PATH);
        first
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\n\r\n{\"result\":1}")
            .expect("reject first selector request");

        listener
            .set_nonblocking(true)
            .expect("make retry listener nonblocking");
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut second = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "selector was not retried");
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("accept selector retry: {error}"),
            }
        };
        assert_eq!(read_path(&mut second), SET_DEPTH_SELECTOR_PATH);
        second
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\n\r\n{\"result\":0}")
            .expect("accept second selector request");

        expect_control(&listener, LED_MASTER_ON_PATH);
        expect_control(&listener, LED_IR_ON_PATH);

        let mut media = accept_before(&listener, deadline, "media request");
        read_path(&mut media);
        media
            .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n1\r\nx\r\n")
            .expect("write media response");

        let mut close = accept_before(&listener, deadline, "close request");
        read_path(&mut close);
        close
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\n\r\n{\"result\":0}")
            .expect("write close response");
        expect_control(&listener, LED_IR_OFF_PATH);
        expect_control(&listener, LED_MASTER_OFF_PATH);
    });

    let result = capture_pair_prefix(address, limits(), 1, |_| {});
    let server_result = server.join();
    assert!(
        server_result.is_ok(),
        "fixture server failed: {server_result:?}"
    );
    assert_eq!(result.expect("capture after selector retry"), 1);
}

#[test]
fn stops_after_three_firmware_rejections() {
    let (result, attempts) = capture_with_repeated_selector_response(
        b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\n\r\n{\"result\":1}",
    );

    assert_eq!(attempts, 3);
    assert!(result
        .expect_err("three rejected selectors must fail")
        .to_string()
        .contains("rejected the depth output configuration"));
}

#[test]
fn stops_after_three_selector_http_failures_and_preserves_the_cause() {
    let (result, attempts) = capture_with_repeated_selector_response(
        b"HTTP/1.1 500 Failed\r\nContent-Length: 0\r\n\r\n",
    );

    assert_eq!(attempts, 3);
    let error = result.expect_err("three failed selector requests must fail");
    assert!(error.to_string().contains("configure depth output"));
    assert!(error.to_string().contains("500"));
}

#[test]
fn rejected_infrared_enable_still_turns_both_emitter_controls_off() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
    let address = listener.local_addr().expect("fixture address");
    let server = thread::spawn(move || {
        let (mut profile, _) = listener.accept().expect("accept profile request");
        read_path(&mut profile);
        profile
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\n\r\n{\"result\":0}")
            .expect("write profile response");

        let (mut selector, _) = listener.accept().expect("accept selector request");
        assert_eq!(read_path(&mut selector), SET_DEPTH_SELECTOR_PATH);
        selector
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\n\r\n{\"result\":0}")
            .expect("write selector response");

        expect_control(&listener, LED_MASTER_ON_PATH);
        let (mut infrared, _) = listener.accept().expect("accept infrared request");
        assert_eq!(read_path(&mut infrared), LED_IR_ON_PATH);
        infrared
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\n\r\n[failed]")
            .expect("reject infrared enable");

        expect_control(&listener, LED_IR_OFF_PATH);
        expect_control(&listener, LED_MASTER_OFF_PATH);
    });

    let error = capture_pair_prefix(address, limits(), 1, |_| {})
        .expect_err("rejected infrared enable must abort capture");

    assert_eq!(
        error.to_string(),
        "scanner rejected enable infrared projector"
    );
    server.join().expect("fixture server");
}

fn capture_with_repeated_selector_response(
    selector_response: &'static [u8],
) -> (Result<usize, DepthStreamError>, usize) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
    let address = listener.local_addr().expect("fixture address");
    let (done_tx, done_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut profile, _) = listener.accept().expect("accept profile request");
        read_path(&mut profile);
        profile
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\n\r\n{\"result\":0}")
            .expect("write profile response");
        listener
            .set_nonblocking(true)
            .expect("make selector listener nonblocking");

        let mut attempts = 0;
        loop {
            match listener.accept() {
                Ok((mut selector, _)) => {
                    assert_eq!(read_path(&mut selector), SET_DEPTH_SELECTOR_PATH);
                    selector
                        .write_all(selector_response)
                        .expect("write repeated selector response");
                    attempts += 1;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if done_rx.try_recv().is_ok() {
                        return attempts;
                    }
                    thread::sleep(Duration::from_millis(1));
                }
                Err(error) => panic!("accept selector request: {error}"),
            }
        }
    });

    let result = capture_pair_prefix(address, limits(), 1, |_| {});
    done_tx.send(()).expect("notify fixture server");
    let attempts = server.join().expect("fixture server");
    (result, attempts)
}

const SET_DEPTH_SELECTOR_PATH: &str = "/cgi-bin/zx_cmd.cgi?cam_type=mipi&set_depth_output_fmt=1";
const LED_MASTER_ON_PATH: &str =
    "/cgi-bin/zx_cmd.cgi?system_cmd=echo%20s%200xb00%201%20%3E%20/dev/rk_preisp";
const LED_IR_ON_PATH: &str =
    "/cgi-bin/zx_cmd.cgi?system_cmd=echo%20s%200xb01%201%20%3E%20/dev/rk_preisp";
const LED_IR_OFF_PATH: &str =
    "/cgi-bin/zx_cmd.cgi?system_cmd=echo%20s%200xb01%200%20%3E%20/dev/rk_preisp";
const LED_MASTER_OFF_PATH: &str =
    "/cgi-bin/zx_cmd.cgi?system_cmd=echo%20s%200xb00%200%20%3E%20/dev/rk_preisp";

fn expect_control(listener: &TcpListener, path: &str) {
    let mut stream = accept_before(
        listener,
        Instant::now() + Duration::from_secs(1),
        "control request",
    );
    assert_eq!(read_path(&mut stream), path);
    stream
        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\n[ok]")
        .expect("write control response");
}

fn accept_before(listener: &TcpListener, deadline: Instant, stage: &str) -> std::net::TcpStream {
    loop {
        match listener.accept() {
            Ok((stream, _)) => return stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "timed out waiting for {stage}");
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("accept {stage}: {error}"),
        }
    }
}
