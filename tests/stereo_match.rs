use revopoint_pop3_wifi::stereo_match::{
    block_match_y8, block_match_y8_consistent, block_match_y8_unique, costs_have_minimum_margin,
    encode_disparity_pgm, enforce_left_right_consistency, filter_disparity_consensus,
    global_sad_disparity_y8, ConsistentMatchParameters, DisparityError, DisparityMap,
    GlobalMatchParameters,
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
fn global_match_recovers_the_dominant_horizontal_shift() {
    let width = 24_u32;
    let height = 9_u32;
    let shift = 3_usize;
    let left = (0..width as usize * height as usize)
        .map(|index| ((index * 37 + index / width as usize * 19) % 251) as u8)
        .collect::<Vec<_>>();
    let mut right = vec![0_u8; left.len()];
    for y in 0..height as usize {
        for x in 0..width as usize - shift {
            right[y * width as usize + x] = left[y * width as usize + x + shift];
        }
    }

    let disparity = global_sad_disparity_y8(
        &left,
        &right,
        width,
        height,
        GlobalMatchParameters {
            disparities: 0..=6,
            border: 1,
        },
    )
    .expect("global disparity");

    assert_eq!(disparity, 3);
}

#[test]
fn global_match_rejects_invalid_layout_and_search_region() {
    let image = vec![0_u8; 8 * 6];
    let reversed_start = 4;
    let reversed_end = 3;
    let parameters = GlobalMatchParameters {
        disparities: 0..=3,
        border: 1,
    };
    assert!(global_sad_disparity_y8(&image[..47], &image, 8, 6, parameters.clone()).is_err());
    assert!(global_sad_disparity_y8(&image, &image[..47], 8, 6, parameters.clone()).is_err());
    assert!(global_sad_disparity_y8(&image, &image, 0, 6, parameters.clone()).is_err());
    assert!(global_sad_disparity_y8(
        &image,
        &image,
        8,
        6,
        GlobalMatchParameters {
            disparities: reversed_start..=reversed_end,
            border: 1,
        },
    )
    .is_err());
    assert!(global_sad_disparity_y8(
        &image,
        &image,
        8,
        6,
        GlobalMatchParameters {
            disparities: 0..=7,
            border: 1,
        },
    )
    .is_err());
    assert!(global_sad_disparity_y8(
        &image,
        &image,
        8,
        6,
        GlobalMatchParameters {
            disparities: 0..=3,
            border: 3,
        },
    )
    .is_err());
    let wide_image = vec![0_u8; 20 * 6];
    assert!(global_sad_disparity_y8(
        &wide_image,
        &wide_image,
        20,
        6,
        GlobalMatchParameters {
            disparities: 0..=3,
            border: 3,
        },
    )
    .is_err());

    let flat = vec![42_u8; 20 * 9];
    assert_eq!(
        global_sad_disparity_y8(
            &flat,
            &flat,
            20,
            9,
            GlobalMatchParameters {
                disparities: 0..=4,
                border: 1,
            },
        )
        .expect("flat global match"),
        0,
        "equal normalized costs retain the lowest disparity"
    );
}

#[test]
fn global_match_agrees_with_an_independent_direct_reference() {
    let width = 17_usize;
    let height = 9_usize;
    for seed in 1..=24_usize {
        let left = (0..width * height)
            .map(|index| ((index * index * (seed + 3) + index * 43 + seed * 29) % 251) as u8)
            .collect::<Vec<_>>();
        let right = (0..left.len())
            .map(|index| ((index * index * 31 + index * (seed + 5) + 101) % 253) as u8)
            .collect::<Vec<_>>();

        let actual = global_sad_disparity_y8(
            &left,
            &right,
            width as u32,
            height as u32,
            GlobalMatchParameters {
                disparities: 1..=5,
                border: 2,
            },
        )
        .expect("global match");
        let expected = direct_global_match(&left, &right, width, height, 1..=5, 2);

        assert_eq!(actual, expected, "seed {seed}");
    }
}

fn direct_global_match(
    left: &[u8],
    right: &[u8],
    width: usize,
    height: usize,
    disparities: std::ops::RangeInclusive<usize>,
    border: usize,
) -> u16 {
    disparities
        .map(|disparity| {
            let differences = (border..height - border).flat_map(|y| {
                (disparity + border..width - border).map(move |x| {
                    u64::from(left[y * width + x].abs_diff(right[y * width + x - disparity]))
                })
            });
            let (sum, count) = differences.fold((0_u64, 0_u64), |(sum, count), difference| {
                (sum + difference, count + 1)
            });
            (sum as f64 / count as f64, disparity as u16)
        })
        .min_by(|left, right| left.0.total_cmp(&right.0))
        .expect("non-empty disparity range")
        .1
}

#[test]
fn rejects_disparities_that_do_not_survive_the_reverse_match() {
    let invalid = u16::MAX;
    let left_to_right = DisparityMap {
        width: 6,
        height: 1,
        values: vec![invalid, 2, 2, 2, 1, 0],
    };
    let right_to_left = DisparityMap {
        width: 6,
        height: 1,
        values: vec![2, 3, invalid, 1, invalid, invalid],
    };

    let exact =
        enforce_left_right_consistency(&left_to_right, &right_to_left, 0).expect("consistent map");
    assert_eq!(exact.values, vec![invalid, invalid, 2, invalid, 1, invalid]);

    let tolerant =
        enforce_left_right_consistency(&left_to_right, &right_to_left, 1).expect("tolerant map");
    assert_eq!(tolerant.at(3, 0), Some(2));
}

#[test]
fn bidirectional_match_retains_a_known_shift() {
    let width = 32_u32;
    let height = 9_u32;
    let shift = 4_usize;
    let left = (0..width as usize * height as usize)
        .map(|index| ((index * 37 + index / width as usize * 19) % 251) as u8)
        .collect::<Vec<_>>();
    let mut right = vec![0_u8; left.len()];
    for y in 0..height as usize {
        for x in 0..width as usize - shift {
            right[y * width as usize + x] = left[y * width as usize + x + shift];
        }
    }

    let map = block_match_y8_consistent(
        &left,
        &right,
        width,
        height,
        ConsistentMatchParameters {
            disparities: 0..=8,
            radius: 1,
            minimum_margin_percent: 10,
            consistency_tolerance: 0,
        },
    )
    .expect("consistent disparity map");

    for y in 1..height - 1 {
        for x in 9..width - 5 {
            assert_eq!(map.at(x, y), Some(4), "pixel ({x},{y})");
        }
    }
}

#[test]
fn consistency_rejects_incompatible_maps() {
    let valid = DisparityMap {
        width: 2,
        height: 1,
        values: vec![0, 0],
    };
    let wrong_dimensions = DisparityMap {
        width: 1,
        height: 2,
        values: vec![0, 0],
    };
    let wrong_width = DisparityMap {
        width: 1,
        height: 1,
        values: vec![0, 0],
    };
    let wrong_length = DisparityMap {
        width: 2,
        height: 1,
        values: vec![0],
    };

    assert_eq!(
        enforce_left_right_consistency(&valid, &wrong_dimensions, 0),
        Err(DisparityError)
    );
    assert_eq!(
        enforce_left_right_consistency(&valid, &wrong_width, 0),
        Err(DisparityError)
    );
    assert_eq!(
        enforce_left_right_consistency(&valid, &wrong_length, 0),
        Err(DisparityError)
    );
    assert_eq!(
        enforce_left_right_consistency(&wrong_length, &valid, 0),
        Err(DisparityError)
    );
}

#[test]
fn marks_pixels_without_a_complete_search_window_invalid() {
    let image = vec![42_u8; 12 * 8];
    let map = block_match_y8(&image, &image, 12, 8, 0..=4, 2).expect("disparity map");

    assert_eq!(map.at(0, 0), None);
    assert_eq!(map.at(5, 3), Some(0));
    assert_eq!(map.at(12, 3), None);
    assert_eq!(map.at(5, 8), None);
    assert_eq!(map.valid_count(), 32);
}

#[test]
fn uniqueness_filter_rejects_an_ambiguous_flat_image() {
    let image = vec![42_u8; 20 * 7];

    let ordinary = block_match_y8(&image, &image, 20, 7, 0..=4, 1).expect("ordinary map");
    let unique = block_match_y8_unique(&image, &image, 20, 7, 0..=4, 1, 10).expect("unique map");

    assert_eq!(ordinary.at(8, 3), Some(0));
    assert_eq!(unique.at(8, 3), None);
}

#[test]
fn uniqueness_margin_includes_its_exact_boundary() {
    assert!(costs_have_minimum_margin(100, 110, 10));
    assert!(!costs_have_minimum_margin(100, 109, 10));
    assert!(!costs_have_minimum_margin(100, 100, 10));
    assert!(!costs_have_minimum_margin(100, 100, 0));
    assert!(!costs_have_minimum_margin(100, u64::MAX, 10));
}

#[test]
fn uniqueness_filter_retains_a_distinct_match() {
    let width = 24_u32;
    let height = 9_u32;
    let shift = 3_usize;
    let left = (0..width as usize * height as usize)
        .map(|index| ((index * 37 + index / width as usize * 19) % 251) as u8)
        .collect::<Vec<_>>();
    let mut right = vec![0_u8; left.len()];
    for y in 0..height as usize {
        for x in 0..width as usize - shift {
            right[y * width as usize + x] = left[y * width as usize + x + shift];
        }
    }

    let map =
        block_match_y8_unique(&left, &right, width, height, 0..=6, 1, 10).expect("unique map");

    assert_eq!(map.at(10, 4), Some(3));
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

    let actual_unique = block_match_y8_unique(&left, &right, width, height, 1..=4, 2, 25)
        .expect("optimized unique disparity map");
    let expected_unique = direct_match_unique(
        &left,
        &right,
        (width as usize, height as usize),
        1..=4,
        2,
        25,
    );
    assert_eq!(actual_unique.values, expected_unique);
    assert!(block_match_y8_unique(&left, &right, width, height, 1..=4, 2, 0).is_err());
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

fn direct_match_unique(
    left: &[u8],
    right: &[u8],
    dimensions: (usize, usize),
    disparities: std::ops::RangeInclusive<usize>,
    radius: usize,
    minimum_margin_percent: u16,
) -> Vec<u16> {
    let (width, height) = dimensions;
    let mut output = vec![u16::MAX; width * height];
    for y in radius..height - radius {
        for x in radius..width - radius {
            let mut candidates = Vec::new();
            for disparity in disparities.clone() {
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
                candidates.push((cost, disparity));
            }
            candidates.sort_by_key(|candidate| candidate.0);
            let [best, second, ..] = candidates.as_slice() else {
                continue;
            };
            let required_margin = u128::from(best.0.max(1)) * u128::from(minimum_margin_percent);
            if second.0 > best.0 && u128::from(second.0 - best.0) * 100 >= required_margin {
                output[y * width + x] = best.1 as u16;
            }
        }
    }
    output
}

#[test]
fn local_consensus_removes_isolated_disparity_outliers() {
    let invalid = u16::MAX;
    let map = DisparityMap {
        width: 5,
        height: 3,
        values: vec![
            invalid, invalid, 10, invalid, invalid, //
            invalid, 10, 11, 10, invalid, //
            50, invalid, invalid, invalid, invalid,
        ],
    };

    let filtered = filter_disparity_consensus(&map, 1, 2, 2).expect("consensus map");

    assert_eq!(filtered.at(2, 1), Some(10));
    assert_eq!(filtered.at(0, 2), None);
    assert_eq!(filtered.valid_count(), 4);

    let one_neighbor = DisparityMap {
        width: 3,
        height: 1,
        values: vec![10, 10, invalid],
    };
    let rejected = filter_disparity_consensus(&one_neighbor, 1, 2, 0).expect("one-neighbor map");
    assert_eq!(rejected.valid_count(), 0);

    let out_of_tolerance = DisparityMap {
        width: 3,
        height: 2,
        values: vec![50, 60, invalid, 10, invalid, invalid],
    };
    let rejected = filter_disparity_consensus(&out_of_tolerance, 1, 2, 2).expect("outlier map");
    assert_eq!(rejected.at(0, 0), None);

    let median_fixture = DisparityMap {
        width: 2,
        height: 2,
        values: vec![8, 9, 10, 11],
    };
    let smoothed = filter_disparity_consensus(&median_fixture, 1, 3, 3).expect("median fixture");
    assert_eq!(smoothed.values, vec![10; 4]);
}

#[test]
fn local_consensus_rejects_invalid_layout_and_parameters() {
    let valid = DisparityMap {
        width: 3,
        height: 3,
        values: vec![0; 9],
    };
    let malformed = DisparityMap {
        values: vec![0; 8],
        ..valid.clone()
    };

    assert!(filter_disparity_consensus(&malformed, 1, 1, 1).is_err());
    assert!(filter_disparity_consensus(&valid, 0, 1, 1).is_err());
    assert!(filter_disparity_consensus(&valid, 1, 0, 1).is_err());
    assert!(filter_disparity_consensus(&valid, 1, 9, 1).is_err());
    let exact_maximum = filter_disparity_consensus(&valid, 1, 8, 0).expect("maximum support map");
    assert_eq!(exact_maximum.valid_count(), 1);
    assert_eq!(exact_maximum.at(1, 1), Some(0));
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
