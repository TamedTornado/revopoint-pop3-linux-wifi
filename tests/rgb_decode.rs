use revopoint_pop3_wifi::rgb_decode::inspect_jpeg;

fn jpeg(width: u16, height: u16) -> Vec<u8> {
    let mut bytes = vec![
        0xff, 0xd8, 0xff, 0xe0, 0x00, 0x02, 0xff, 0xc0, 0x00, 0x11, 8,
    ];
    bytes.extend_from_slice(&height.to_be_bytes());
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&[3, 1, 0x11, 0, 2, 0x11, 0, 3, 0x11, 0]);
    bytes.extend_from_slice(&[0xff, 0xd9]);
    bytes
}

#[test]
fn reads_dimensions_from_a_bounded_jpeg_frame() {
    let information = inspect_jpeg(&jpeg(1280, 800)).expect("valid JPEG envelope");

    assert_eq!(information.width, 1280);
    assert_eq!(information.height, 800);
    assert_eq!(information.encoded_len, jpeg(1280, 800).len());
    assert_eq!(information.device_timestamp_ms, None);
}

#[test]
fn separates_the_observed_four_byte_transport_trailer() {
    let mut frame = jpeg(1280, 800);
    let jpeg_len = frame.len();
    frame.extend_from_slice(&19_235_652_u32.to_le_bytes());

    let information = inspect_jpeg(&frame).expect("JPEG with transport trailer");

    assert_eq!(information.encoded_len, jpeg_len);
    assert_eq!(information.device_timestamp_ms, Some(19_235_652));
}

#[test]
fn rejects_truncated_missing_and_zero_sized_jpeg_frames() {
    for bytes in [
        vec![0xff, 0xd8, 0xff],
        vec![0xff, 0xd8, 0xff, 0xd9],
        jpeg(0, 800),
        jpeg(1280, 0),
    ] {
        let error = inspect_jpeg(&bytes).expect_err("invalid JPEG must fail");
        assert_eq!(
            error.to_string(),
            "RGB frame is not a complete bounded JPEG image"
        );
    }
}

#[test]
fn accepts_each_standalone_marker_before_the_frame_dimensions() {
    for marker in [0x01, 0xd0] {
        let mut bytes = jpeg(1280, 800);
        bytes.splice(2..2, [0xff, marker]);

        assert_eq!(inspect_jpeg(&bytes).expect("standalone marker").width, 1280);
    }
}

#[test]
fn enforces_the_minimum_start_of_frame_segment_length() {
    let valid = vec![0xff, 0xd8, 0xff, 0xc0, 0, 8, 8, 0, 1, 0, 1, 0, 0xff, 0xd9];
    let invalid = vec![0xff, 0xd8, 0xff, 0xc0, 0, 7, 8, 0, 1, 0, 1, 0xff, 0xd9];

    assert_eq!(inspect_jpeg(&valid).expect("minimum SOF").width, 1);
    assert!(inspect_jpeg(&invalid).is_err());
}
