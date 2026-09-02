use revopoint_pop3_wifi::depth_stream::{get_depth_scale_mm, parse_depth_scale_mm};
use revopoint_pop3_wifi::http_stream::StreamLimits;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

fn property_response(divisor: u32, value_type: u8) -> [u8; 60] {
    let mut response = [0_u8; 60];
    response[0] = 0xff;
    response[1..5].copy_from_slice(&divisor.to_le_bytes());
    response[59] = value_type;
    response
}

#[test]
fn parses_the_device_depth_unit_divisor_as_millimeters() {
    assert_eq!(
        parse_depth_scale_mm(&property_response(10, 4)).expect("depth scale"),
        0.1
    );
}

#[test]
fn rejects_wrong_length_marker_type_and_zero_divisor() {
    let valid = property_response(10, 4);
    assert!(parse_depth_scale_mm(&valid[..59]).is_err());

    let mut wrong_marker = valid;
    wrong_marker[0] = 0;
    assert!(parse_depth_scale_mm(&wrong_marker).is_err());

    assert!(parse_depth_scale_mm(&property_response(10, 3)).is_err());
    assert!(parse_depth_scale_mm(&property_response(0, 4)).is_err());
}

#[test]
fn explains_an_invalid_scale_without_echoing_the_response() {
    let error =
        parse_depth_scale_mm(&property_response(0, 4)).expect_err("zero divisor must be rejected");

    assert_eq!(error.to_string(), "scanner returned an invalid depth scale");
}

#[test]
fn queries_the_read_only_depth_scale_property() {
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
        assert!(String::from_utf8_lossy(&request).starts_with(
            "GET /cgi-bin/zx_cmd.cgi?cam_type=mipi&algo_get_cmd_buf=2328 HTTP/1.1\r\n"
        ));
        let body = property_response(10, 4);
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
        get_depth_scale_mm(address, limits).expect("query depth scale"),
        0.1
    );
    server.join().expect("fixture server");
}
