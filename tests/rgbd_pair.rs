use revopoint_pop3_wifi::rgbd_pair::{pair_timestamps, select_closest_pair, PairingPolicy};

#[test]
fn accepts_an_rgb_frame_just_after_depth() {
    let pair = pair_timestamps(23_709_703, 23_709_718, PairingPolicy::default())
        .expect("15 ms device-clock separation should pair");

    assert_eq!(pair.signed_delta_ms, 15);
    assert_eq!(pair.absolute_delta_ms, 15);
}

#[test]
fn rejects_a_frame_outside_the_bounded_pairing_window() {
    assert!(pair_timestamps(1_000, 1_050, PairingPolicy::default()).is_some());
    assert!(pair_timestamps(1_000, 1_051, PairingPolicy::default()).is_none());
}

#[test]
fn requires_rgb_to_follow_depth_by_default() {
    assert!(pair_timestamps(1_000, 1_000, PairingPolicy::default()).is_some());
    assert!(pair_timestamps(1_000, 999, PairingPolicy::default()).is_none());
}

#[test]
fn applies_the_configured_rgb_clock_offset() {
    let policy = PairingPolicy {
        rgb_offset_ms: 10,
        maximum_delta_ms: 5,
        require_rgb_after_depth: false,
    };
    let pair = pair_timestamps(1_000, 1_012, policy).expect("offset-adjusted clocks pair");

    assert_eq!(pair.signed_delta_ms, 2);
    assert_eq!(pair.absolute_delta_ms, 2);
}

#[test]
fn compares_device_timestamps_across_u32_wraparound() {
    let pair = pair_timestamps(u32::MAX - 4, 5, PairingPolicy::default())
        .expect("ten milliseconds across wrap should pair");

    assert_eq!(pair.signed_delta_ms, 10);
    assert_eq!(pair.absolute_delta_ms, 10);
}

#[test]
fn selects_the_closest_valid_candidate_from_overlapping_streams() {
    let depths = [1_000, 1_047, 1_094, 1_141, 1_188];
    let rgbs = [1_171, 1_203, 1_265];

    let selected = select_closest_pair(&depths, &rgbs, PairingPolicy::default())
        .expect("overlapping streams contain valid pairs");

    assert_eq!(selected.depth_index, 4);
    assert_eq!(selected.rgb_index, 1);
    assert_eq!(selected.timestamps.signed_delta_ms, 15);
}

#[test]
fn closest_selection_rejects_nonoverlapping_streams_and_empty_inputs() {
    assert!(select_closest_pair(&[], &[1_000], PairingPolicy::default()).is_none());
    assert!(select_closest_pair(&[1_000], &[], PairingPolicy::default()).is_none());
    assert!(select_closest_pair(&[1_000], &[1_100], PairingPolicy::default()).is_none());
}
