use revopoint_pop3_wifi::depth_decode::decode_quicklz;
use revopoint_pop3_wifi::frame_envelope::CompressedFrame;

fn frame(payload: &[u8]) -> CompressedFrame {
    CompressedFrame {
        declared_payload_len: payload.len() as u32,
        payload: payload.to_vec(),
    }
}

#[test]
fn decodes_a_bounded_independent_quicklz_fixture() {
    let encoded = [
        0x47, 0x17, 0x00, 0x00, 0x00, 0x0a, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0x00, 0x01,
        0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09,
    ];

    let decoded = decode_quicklz(&frame(&encoded), 10).expect("decode fixture at exact limit");

    assert_eq!(decoded.flags, 0x47);
    assert_eq!(decoded.compressed_len, 23);
    assert_eq!(decoded.decompressed_len, 10);
    assert_eq!(decoded.bytes, (0_u8..10).collect::<Vec<_>>());
}

#[test]
fn rejects_a_short_quicklz_header_flag_even_when_other_bits_are_set() {
    let encoded = [0x45, 0x09, 0, 0, 0, 0x01, 0, 0, 0];

    let error = decode_quicklz(&frame(&encoded), 1024).expect_err("short header must fail");

    assert!(error.to_string().contains("long header"));
}

#[test]
fn rejects_a_compressed_length_that_disagrees_with_the_envelope() {
    let encoded = [0x47, 0x09, 0, 0, 0, 0, 0, 0, 0];
    let mut framed = frame(&encoded);
    framed.declared_payload_len += 1;

    let error = decode_quicklz(&framed, 1024).expect_err("length mismatch must fail");

    assert!(error.to_string().contains("length"));
}

#[test]
fn rejects_decompressed_output_above_the_limit_before_allocation() {
    let encoded = [0x47, 0x09, 0, 0, 0, 0x01, 0x00, 0x10, 0x00];

    let error = decode_quicklz(&frame(&encoded), 1024).expect_err("oversize must fail");

    assert!(error.to_string().contains("limit"));
}
