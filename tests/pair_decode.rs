use revopoint_pop3_wifi::pair_decode::{decode_y8_pair, encode_y8_pgm, PairDecodeError};

#[test]
fn splits_pair_into_contiguous_left_and_right_y8_planes() {
    let decoded = vec![1, 2, 3, 4, 10, 20, 30, 40];

    let pair = decode_y8_pair(decoded, 2, 2).expect("valid PAIR frame");

    assert_eq!(pair.width, 2);
    assert_eq!(pair.height, 2);
    assert_eq!(pair.left, [1, 2, 3, 4]);
    assert_eq!(pair.right, [10, 20, 30, 40]);
}

#[test]
fn rejects_a_buffer_that_is_not_exactly_two_y8_planes() {
    assert_eq!(
        decode_y8_pair(vec![0; 7], 2, 2),
        Err(PairDecodeError::InvalidLayout)
    );
    assert_eq!(
        decode_y8_pair(vec![0; 9], 2, 2),
        Err(PairDecodeError::InvalidLayout)
    );
}

#[test]
fn rejects_zero_dimensions_and_size_overflow() {
    assert_eq!(
        decode_y8_pair(Vec::new(), 0, 2),
        Err(PairDecodeError::InvalidLayout)
    );
    assert_eq!(
        decode_y8_pair(Vec::new(), u32::MAX, u32::MAX),
        Err(PairDecodeError::InvalidLayout)
    );
}

#[test]
fn encodes_a_binary_pgm_for_stock_linux_image_viewers() {
    assert_eq!(
        encode_y8_pgm(2, 2, &[0, 127, 128, 255]).expect("valid Y8 image"),
        b"P5\n2 2\n255\n\x00\x7f\x80\xff"
    );
}

#[test]
fn rejects_inconsistent_pgm_dimensions() {
    assert_eq!(
        encode_y8_pgm(2, 2, &[0; 3]),
        Err(PairDecodeError::InvalidLayout)
    );
    assert_eq!(
        encode_y8_pgm(0, 2, &[]),
        Err(PairDecodeError::InvalidLayout)
    );
}

#[test]
fn explains_an_invalid_pair_without_echoing_frame_data() {
    assert_eq!(
        PairDecodeError::InvalidLayout.to_string(),
        "decoded buffer does not contain two contiguous Y8 planes"
    );
}
