use revopoint_pop3_wifi::http_stream::{get_chunked, StreamLimits};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

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

#[test]
fn reads_fragmented_chunked_response_through_tcp_boundary() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
    let address = listener.local_addr().expect("fixture address");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept client");
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

    server.join().expect("fixture server");
    assert_eq!(received, 7);
    assert_eq!(body, b"ABCDXYZ");
}

#[test]
fn rejects_non_success_response() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
    let address = listener.local_addr().expect("fixture address");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept client");
        read_request(&mut stream);
        stream
            .write_all(b"HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\n\r\n")
            .expect("write response");
    });

    let error = get_chunked(address, "/stream", limits(), |_| {})
        .expect_err("non-success response must fail");

    server.join().expect("fixture server");
    assert!(error.to_string().contains("503"));
}
