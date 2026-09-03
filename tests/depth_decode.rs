use revopoint_pop3_wifi::depth_decode::{decode_quicklz, DecodedDepth, DepthEncoding};
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

#[test]
fn wraps_an_exact_decoded_buffer_as_explicit_metric_z16() {
    let decoded = DecodedDepth {
        flags: 0x47,
        compressed_len: 9,
        decompressed_len: 12,
        bytes: (0_u8..12).collect(),
    };

    let plane = decoded.into_z16_plane(3, 2, 0.1).expect("valid Z16 plane");

    assert_eq!(plane.width, 3);
    assert_eq!(plane.height, 2);
    assert_eq!(plane.stride_bytes, 6);
    assert_eq!(plane.encoding, DepthEncoding::Z16LittleEndian);
    assert_eq!(plane.millimeters_per_unit, 0.1);
    assert_eq!(plane.bytes, (0_u8..12).collect::<Vec<_>>());
}

#[test]
fn rejects_inconsistent_z16_layout_and_invalid_scale() {
    for (width, height, scale) in [
        (4, 2, 0.1),
        (0, 2, 0.1),
        (3, 0, 0.1),
        (3, 2, 0.0),
        (3, 2, f32::NAN),
    ] {
        let decoded = DecodedDepth {
            flags: 0x47,
            compressed_len: 9,
            decompressed_len: 12,
            bytes: vec![0; 12],
        };

        assert!(decoded.into_z16_plane(width, height, scale).is_err());
    }

    let inconsistent_header = DecodedDepth {
        flags: 0x47,
        compressed_len: 9,
        decompressed_len: 10,
        bytes: vec![0; 12],
    };
    assert!(inconsistent_header.into_z16_plane(3, 2, 0.1).is_err());

    for (width, height) in [(0, 2), (3, 0)] {
        let decoded = DecodedDepth {
            flags: 0x47,
            compressed_len: 9,
            decompressed_len: 12,
            bytes: vec![0; 12],
        };
        assert_eq!(
            decoded
                .into_z16_plane(width, height, 0.1)
                .expect_err("zero dimension must fail as metadata")
                .to_string(),
            "invalid Z16 plane metadata"
        );
    }
}

#[test]
fn splits_z16y8y8_into_depth_and_two_infrared_planes() {
    let decoded = DecodedDepth {
        flags: 3,
        compressed_len: 8,
        decompressed_len: 8,
        bytes: vec![1, 0, 2, 0, 10, 20, 30, 40],
    };

    let frame = decoded
        .into_z16y8y8(2, 1, 0.1)
        .expect("decode composite depth frame");

    assert_eq!(frame.depth.bytes, [1, 0, 2, 0]);
    assert_eq!(frame.left, [10, 20]);
    assert_eq!(frame.right, [30, 40]);
    assert_eq!(frame.depth.millimeters_per_unit, 0.1);
}

#[test]
fn rejects_each_independent_z16y8y8_length_mismatch() {
    for (decompressed_len, bytes) in [(7, vec![0; 8]), (8, vec![0; 7])] {
        let decoded = DecodedDepth {
            flags: 3,
            compressed_len: 8,
            decompressed_len,
            bytes,
        };

        assert!(decoded.into_z16y8y8(2, 1, 0.1).is_err());
    }
}

#[test]
fn reports_z16_statistics_in_raw_units_and_millimeters() {
    let decoded = DecodedDepth {
        flags: 0x47,
        compressed_len: 9,
        decompressed_len: 6,
        bytes: vec![0, 0, 10, 0, 20, 0],
    };
    let plane = decoded.into_z16_plane(3, 1, 0.1).expect("Z16 plane");

    let statistics = plane.statistics();

    assert_eq!(statistics.samples, 3);
    assert_eq!(statistics.nonzero_samples, 2);
    assert_eq!(statistics.minimum_nonzero_raw, Some(10));
    assert_eq!(statistics.maximum_raw, 20);
    assert!((statistics.mean_nonzero_mm.expect("nonzero mean") - 1.5).abs() < 0.000_001);
}
