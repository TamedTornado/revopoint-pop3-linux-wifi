#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PairingPolicy {
    pub rgb_offset_ms: i32,
    pub maximum_delta_ms: u32,
    pub require_rgb_after_depth: bool,
}

impl Default for PairingPolicy {
    fn default() -> Self {
        Self {
            rgb_offset_ms: 0,
            maximum_delta_ms: 50,
            require_rgb_after_depth: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimestampPair {
    pub signed_delta_ms: i32,
    pub absolute_delta_ms: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PairSelection {
    pub depth_index: usize,
    pub rgb_index: usize,
    pub timestamps: TimestampPair,
}

pub fn pair_timestamps(
    depth_timestamp_ms: u32,
    rgb_timestamp_ms: u32,
    policy: PairingPolicy,
) -> Option<TimestampPair> {
    let adjusted_depth = depth_timestamp_ms.wrapping_add_signed(policy.rgb_offset_ms);
    let signed_delta_ms = rgb_timestamp_ms.wrapping_sub(adjusted_depth) as i32;
    let absolute_delta_ms = signed_delta_ms.unsigned_abs();

    if absolute_delta_ms > policy.maximum_delta_ms
        || (policy.require_rgb_after_depth && signed_delta_ms < 0)
    {
        return None;
    }

    Some(TimestampPair {
        signed_delta_ms,
        absolute_delta_ms,
    })
}

pub fn select_closest_pair(
    depth_timestamps_ms: &[u32],
    rgb_timestamps_ms: &[u32],
    policy: PairingPolicy,
) -> Option<PairSelection> {
    depth_timestamps_ms
        .iter()
        .enumerate()
        .flat_map(|(depth_index, &depth_timestamp_ms)| {
            rgb_timestamps_ms.iter().enumerate().filter_map(
                move |(rgb_index, &rgb_timestamp_ms)| {
                    pair_timestamps(depth_timestamp_ms, rgb_timestamp_ms, policy).map(
                        |timestamps| PairSelection {
                            depth_index,
                            rgb_index,
                            timestamps,
                        },
                    )
                },
            )
        })
        .min_by_key(|selection| selection.timestamps.absolute_delta_ms)
}
