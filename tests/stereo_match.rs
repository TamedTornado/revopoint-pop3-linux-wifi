use revopoint_pop3_wifi::stereo_match::{
    block_match_y8, encode_disparity_pgm, DisparityError, DisparityMap,
};

#[test]
fn recovers_a_known_horizontal_shift() {
    let width = 24_u32;
    let height = 9_u32;
    let disparity = 3_usize;
    let left = (0..width as usize * height as usize)
        .map(|index| ((index * 37 + index / width as usize * 19) % 251) as u8)
        .collect::<Vec<_>>();
    let mut right = vec![0_u8; left.len()];
    for y in 0..height as usize {
        for x in 0..width as usize - disparity {
            right[y * width as usize + x] = left[y * width as usize + x + disparity];
        }
    }

    let map = block_match_y8(&left, &right, width, height, 0..=6, 1).expect("disparity map");

    for y in 1..height - 1 {
        for x in 5..width - 1 {
            assert_eq!(map.at(x, y), Some(3), "pixel ({x},{y})");
        }
    }
}

#[test]
fn marks_pixels_without_a_complete_search_window_invalid() {
    let image = vec![42_u8; 12 * 8];
    let map = block_match_y8(&image, &image, 12, 8, 0..=4, 2).expect("disparity map");

    assert_eq!(map.at(0, 0), None);
    assert_eq!(map.at(5, 3), Some(0));
    assert_eq!(map.at(12, 3), None);
    assert_eq!(map.at(5, 8), None);
}

#[test]
fn rejects_invalid_layout_range_and_window() {
    let image = vec![0_u8; 12 * 8];
    let reversed_start = 5;
    let reversed_end = 4;
    for result in [
        block_match_y8(&image[..95], &image, 12, 8, 0..=4, 1),
        block_match_y8(&image, &image[..95], 12, 8, 0..=4, 1),
        block_match_y8(&[], &[], 0, 8, 0..=4, 1),
        block_match_y8(&[], &[], 12, 0, 0..=4, 1),
        block_match_y8(&image, &image, 12, 8, reversed_start..=reversed_end, 1),
        block_match_y8(&image, &image, 12, 8, 0..=11, 1),
        block_match_y8(&image, &image, 12, 8, 0..=4, 4),
        block_match_y8(&image, &image, 12, 8, 0..=4, 0),
    ] {
        let error = result.expect_err("invalid request");
        assert_eq!(error, DisparityError);
        assert_eq!(
            error.to_string(),
            "invalid stereo images or block-matching parameters"
        );
    }
}

#[test]
fn optimized_costs_match_a_direct_reference_implementation() {
    let width = 13_u32;
    let height = 8_u32;
    let left = (0..width as usize * height as usize)
        .map(|index| ((index * index * 17 + index * 43 + 29) % 251) as u8)
        .collect::<Vec<_>>();
    let right = (0..left.len())
        .map(|index| ((index * index * 31 + index * 7 + 101) % 253) as u8)
        .collect::<Vec<_>>();

    let actual =
        block_match_y8(&left, &right, width, height, 1..=4, 2).expect("optimized disparity map");
    let expected = direct_match(&left, &right, width as usize, height as usize, 1, 4, 2);

    assert_eq!(actual.values, expected);
}

fn direct_match(
    left: &[u8],
    right: &[u8],
    width: usize,
    height: usize,
    minimum_disparity: usize,
    maximum_disparity: usize,
    radius: usize,
) -> Vec<u16> {
    let mut output = vec![u16::MAX; width * height];
    for y in radius..height - radius {
        for x in radius..width - radius {
            let mut best = None;
            for disparity in minimum_disparity..=maximum_disparity {
                if x < disparity + radius {
                    continue;
                }
                let mut cost = 0_u64;
                for window_y in y - radius..=y + radius {
                    for window_x in x - radius..=x + radius {
                        cost += u64::from(
                            left[window_y * width + window_x]
                                .abs_diff(right[window_y * width + window_x - disparity]),
                        );
                    }
                }
                if best.is_none_or(|(best_cost, _)| cost < best_cost) {
                    best = Some((cost, disparity));
                }
            }
            if let Some((_, disparity)) = best {
                output[y * width + x] = disparity as u16;
            }
        }
    }
    output
}

#[test]
fn encodes_valid_disparities_for_a_stock_image_viewer() {
    let left = [10, 20, 30, 40, 50, 60, 70, 80, 90];
    let map = block_match_y8(&left, &left, 3, 3, 0..=0, 1).expect("disparity map");

    let pgm = encode_disparity_pgm(&map, 8).expect("PGM");

    assert_eq!(&pgm[..11], b"P5\n3 3\n255\n");
    assert_eq!(pgm.len(), 20);
    assert_eq!(
        pgm[15], 1,
        "a valid zero disparity is distinct from invalid"
    );
    assert_eq!(pgm[11], 0, "invalid border is black");
    assert!(encode_disparity_pgm(&map, 0).is_err());

    let scale_map = DisparityMap {
        width: 5,
        height: 1,
        values: vec![u16::MAX, 0, 4, 8, 9],
    };
    let scaled = encode_disparity_pgm(&scale_map, 8).expect("scaled PGM");
    assert_eq!(&scaled[11..], &[0, 1, 128, 255, 255]);

    let empty = DisparityMap {
        width: 0,
        height: 0,
        values: Vec::new(),
    };
    assert!(encode_disparity_pgm(&empty, 8).is_err());
}
