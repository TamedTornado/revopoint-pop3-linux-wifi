use revopoint_pop3_wifi::frame_envelope::FrameEnvelopeParser;

const MAGIC: [u8; 4] = 0x1122_3344_u32.to_le_bytes();

fn envelope(payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::from(MAGIC);
    bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    bytes.extend_from_slice(payload);
    bytes
}

#[test]
fn recovers_one_complete_frame() {
    let mut parser = FrameEnvelopeParser::new(1024, 4);
    let frames = parser.push(&envelope(b"depth")).expect("parse frame");

    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].declared_payload_len, 5);
    assert_eq!(frames[0].payload, b"depth");
    parser.finish().expect("complete stream");
}

#[test]
fn recovers_two_concatenated_frames() {
    let mut bytes = envelope(b"first");
    bytes.extend_from_slice(&envelope(b"second"));
    let mut parser = FrameEnvelopeParser::new(1024, 4);

    let frames = parser.push(&bytes).expect("parse frames");

    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].payload, b"first");
    assert_eq!(frames[1].payload, b"second");
}

#[test]
fn accepts_every_split_position_across_the_header() {
    let bytes = envelope(b"payload");
    for split in 0..=8 {
        let mut parser = FrameEnvelopeParser::new(1024, 4);
        assert!(parser
            .push(&bytes[..split])
            .expect("first fragment")
            .is_empty());
        let frames = parser.push(&bytes[split..]).expect("second fragment");
        assert_eq!(frames.len(), 1, "split={split}");
        assert_eq!(frames[0].payload, b"payload", "split={split}");
    }
}

#[test]
fn accepts_the_four_byte_preamble_observed_on_hardware() {
    let mut bytes = Vec::from(b"\r\n\r\n".as_slice());
    bytes.extend_from_slice(&envelope(b"depth"));
    let mut parser = FrameEnvelopeParser::new(1024, 4);

    let frames = parser.push(&bytes).expect("parse preamble and frame");

    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].payload, b"depth");
}

#[test]
fn rejects_bad_magic_after_synchronization() {
    let mut bytes = envelope(b"first");
    bytes.extend_from_slice(b"BAD!");
    let mut parser = FrameEnvelopeParser::new(1024, 4);

    let error = parser.push(&bytes).expect_err("bad magic must fail");

    assert!(error.to_string().contains("magic"));
}

#[test]
fn rejects_zero_and_oversized_payload_lengths() {
    for length in [0_u32, 1025] {
        let mut bytes = Vec::from(MAGIC);
        bytes.extend_from_slice(&length.to_le_bytes());
        let mut parser = FrameEnvelopeParser::new(1024, 4);
        let error = parser.push(&bytes).expect_err("invalid length must fail");
        assert!(error.to_string().contains("length"));
    }
}

#[test]
fn accepts_a_payload_exactly_at_the_configured_limit() {
    let mut parser = FrameEnvelopeParser::new(5, 4);

    let frames = parser.push(&envelope(b"12345")).expect("boundary frame");

    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].payload, b"12345");
}

#[test]
fn rejects_missing_magic_as_soon_as_the_preamble_limit_is_exhausted() {
    let mut parser = FrameEnvelopeParser::new(1024, 4);

    let error = parser
        .push(b"12345678")
        .expect_err("eight non-magic bytes exceed four-byte preamble allowance");

    assert!(error.to_string().contains("magic"));
}

#[test]
fn reports_a_truncated_frame_at_end_of_stream() {
    let bytes = envelope(b"payload");
    let mut parser = FrameEnvelopeParser::new(1024, 4);
    parser
        .push(&bytes[..bytes.len() - 1])
        .expect("partial frame");

    let error = parser.finish().expect_err("truncation must fail");

    assert!(error.to_string().contains("truncated"));
}
