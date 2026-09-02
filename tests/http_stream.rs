use revopoint_pop3_wifi::http_stream::{get_chunked, StreamLimits};
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
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

fn read_request(stream: &mut impl Read) {
    let mut request = Vec::new();
    while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
        let mut block = [0_u8; 128];
        let count = stream.read(&mut block).expect("read request");
        assert!(count > 0, "request ended before its header terminator");
        request.extend_from_slice(&block[..count]);
        assert!(
            request.len() <= 1024,
            "request header exceeded fixture limit"
        );
    }
}

fn accept_fixture(listener: &TcpListener) -> Option<TcpStream> {
    listener
        .set_nonblocking(true)
        .expect("make fixture listener nonblocking");
    let deadline = Instant::now() + Duration::from_millis(250);
    loop {
        match listener.accept() {
            Ok((stream, _)) => return Some(stream),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return None;
                }
                thread::sleep(Duration::from_millis(1));
            }
            Err(error) => panic!("accept client: {error}"),
        }
    }
}

#[test]
fn reads_fragmented_chunked_response_through_tcp_boundary() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
    let address = listener.local_addr().expect("fixture address");
    let server = thread::spawn(move || {
        let Some(mut stream) = accept_fixture(&listener) else {
            return;
        };
        read_request(&mut stream);
        for fragment in [
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nAB".as_slice(),
            b"CD\r\n3\r\nX".as_slice(),
            b"YZ\r\n0\r\n\r\n".as_slice(),
        ] {
            stream.write_all(fragment).expect("write response fragment");
            thread::sleep(Duration::from_millis(5));
        }
    });

    let mut body = Vec::new();
    let received = get_chunked(address, "/stream", limits(), |chunk| {
        body.extend_from_slice(chunk)
    })
    .expect("read chunked response");

    assert_eq!(received, 7);
    assert_eq!(body, b"ABCDXYZ");
    server.join().expect("fixture server");
}

#[test]
fn rejects_non_success_response() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
    let address = listener.local_addr().expect("fixture address");
    let server = thread::spawn(move || {
        let Some(mut stream) = accept_fixture(&listener) else {
            return;
        };
        read_request(&mut stream);
        stream
            .write_all(b"HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\n\r\n")
            .expect("write response");
    });

    let error = get_chunked(address, "/stream", limits(), |_| {})
        .expect_err("non-success response must fail");

    assert!(error.to_string().contains("503"));
    server.join().expect("fixture server");
}

#[test]
fn rejects_each_invalid_path_shape_before_connecting() {
    let unavailable: SocketAddr = "127.0.0.1:1".parse().expect("socket address");

    let relative =
        get_chunked(unavailable, "stream", limits(), |_| {}).expect_err("relative path must fail");
    assert!(relative.to_string().contains("absolute"));

    let newline = get_chunked(unavailable, "/stream\nInjected: value", limits(), |_| {})
        .expect_err("newline in path must fail");
    assert!(newline.to_string().contains("newlines"));
}

#[test]
fn rejects_each_zero_or_too_small_limit_before_connecting() {
    let unavailable: SocketAddr = "127.0.0.1:1".parse().expect("socket address");
    let mut too_small_header = limits();
    too_small_header.max_header_bytes = 3;
    let error = get_chunked(unavailable, "/stream", too_small_header, |_| {})
        .expect_err("small header limit must fail validation");
    assert!(error.to_string().contains("limits"));

    let mut zero_body = limits();
    zero_body.max_body_bytes = 0;
    let error = get_chunked(unavailable, "/stream", zero_body, |_| {})
        .expect_err("zero body limit must fail validation");
    assert!(error.to_string().contains("limits"));
}

#[test]
fn rejects_chunked_token_on_the_wrong_header() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
    let address = listener.local_addr().expect("fixture address");
    let server = thread::spawn(move || {
        let Some(mut stream) = accept_fixture(&listener) else {
            return;
        };
        read_request(&mut stream);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nX-Encoding: chunked\r\n\r\n")
            .expect("write response");
    });

    let error = get_chunked(address, "/stream", limits(), |_| {})
        .expect_err("wrong transfer header must fail");
    assert!(error.to_string().contains("not chunked"));
    server.join().expect("fixture server");
}

#[test]
fn rejects_non_chunked_transfer_encoding() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
    let address = listener.local_addr().expect("fixture address");
    let server = thread::spawn(move || {
        let Some(mut stream) = accept_fixture(&listener) else {
            return;
        };
        read_request(&mut stream);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: gzip\r\n\r\n")
            .expect("write response");
    });

    let error = get_chunked(address, "/stream", limits(), |_| {})
        .expect_err("non-chunked transfer encoding must fail");
    assert!(error.to_string().contains("not chunked"));
    server.join().expect("fixture server");
}

#[test]
fn accepts_body_exactly_at_limit_and_rejects_one_byte_over() {
    for (maximum, should_pass) in [(4, true), (3, false)] {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
        let address = listener.local_addr().expect("fixture address");
        let server = thread::spawn(move || {
            let Some(mut stream) = accept_fixture(&listener) else {
                return;
            };
            read_request(&mut stream);
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nDATA\r\n0\r\n\r\n",
            );
        });
        let mut bounded = limits();
        bounded.max_body_bytes = maximum;
        let result = get_chunked(address, "/stream", bounded, |_| {});
        assert_eq!(result.is_ok(), should_pass);
        server.join().expect("fixture server");
    }
}
