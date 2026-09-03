use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::ops::RangeInclusive;

const INVALID_DISPARITY: u16 = u16::MAX;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisparityMap {
    pub width: u32,
    pub height: u32,
    pub values: Vec<u16>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsistentMatchParameters {
    pub disparities: RangeInclusive<u16>,
    pub radius: u32,
    pub minimum_margin_percent: u16,
    pub consistency_tolerance: u16,
}

impl DisparityMap {
    pub fn at(&self, x: u32, y: u32) -> Option<u16> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let value = self.values[y as usize * self.width as usize + x as usize];
        (value != INVALID_DISPARITY).then_some(value)
    }

    pub fn valid_count(&self) -> usize {
        self.values
            .iter()
            .filter(|value| **value != INVALID_DISPARITY)
            .count()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DisparityError;

impl Display for DisparityError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid stereo images or block-matching parameters")
    }
}

impl Error for DisparityError {}

pub fn block_match_y8(
    left: &[u8],
    right: &[u8],
    width: u32,
    height: u32,
    disparities: RangeInclusive<u16>,
    radius: u32,
) -> Result<DisparityMap, DisparityError> {
    block_match_y8_impl(left, right, width, height, disparities, radius, None)
}

pub fn block_match_y8_unique(
    left: &[u8],
    right: &[u8],
    width: u32,
    height: u32,
    disparities: RangeInclusive<u16>,
    radius: u32,
    minimum_margin_percent: u16,
) -> Result<DisparityMap, DisparityError> {
    if minimum_margin_percent == 0 {
        return Err(DisparityError);
    }
    block_match_y8_impl(
        left,
        right,
        width,
        height,
        disparities,
        radius,
        Some(minimum_margin_percent),
    )
}

fn block_match_y8_impl(
    left: &[u8],
    right: &[u8],
    width: u32,
    height: u32,
    disparities: RangeInclusive<u16>,
    radius: u32,
    minimum_margin_percent: Option<u16>,
) -> Result<DisparityMap, DisparityError> {
    let width = usize::try_from(width).map_err(|_| DisparityError)?;
    let height = usize::try_from(height).map_err(|_| DisparityError)?;
    let radius = usize::try_from(radius).map_err(|_| DisparityError)?;
    let minimum_disparity = usize::from(*disparities.start());
    let maximum_disparity = usize::from(*disparities.end());
    let pixel_count = width.checked_mul(height).ok_or(DisparityError)?;
    let window = radius
        .checked_mul(2)
        .and_then(|diameter| diameter.checked_add(1))
        .ok_or(DisparityError)?;
    if width == 0 {
        return Err(DisparityError);
    }
    if height == 0 {
        return Err(DisparityError);
    }
    if left.len() != pixel_count
        || right.len() != pixel_count
        || radius == 0
        || minimum_disparity > maximum_disparity
        || maximum_disparity + window > width
        || window > height
    {
        return Err(DisparityError);
    }

    let integral_width = width + 1;
    let mut integral = vec![0_u64; integral_width * (height + 1)];
    let mut best_costs = vec![u64::MAX; pixel_count];
    let mut second_best_costs = vec![u64::MAX; pixel_count];
    let mut values = vec![INVALID_DISPARITY; pixel_count];

    for disparity in minimum_disparity..=maximum_disparity {
        integral.fill(0);
        for y in 0..height {
            let mut row_sum = 0_u64;
            for x in 0..width {
                let difference = if x >= disparity {
                    left[y * width + x].abs_diff(right[y * width + x - disparity])
                } else {
                    0
                };
                row_sum += u64::from(difference);
                integral[(y + 1) * integral_width + x + 1] =
                    integral[y * integral_width + x + 1] + row_sum;
            }
        }

        let first_x = disparity + radius;
        for y in radius..height - radius {
            let top = y - radius;
            let bottom = y + radius + 1;
            for x in first_x..width - radius {
                let left_edge = x - radius;
                let right_edge = x + radius + 1;
                let cost = integral[bottom * integral_width + right_edge]
                    + integral[top * integral_width + left_edge]
                    - integral[top * integral_width + right_edge]
                    - integral[bottom * integral_width + left_edge];
                let index = y * width + x;
                if cost < best_costs[index] {
                    second_best_costs[index] = best_costs[index];
                    best_costs[index] = cost;
                    values[index] = disparity as u16;
                } else {
                    second_best_costs[index] = second_best_costs[index].min(cost);
                }
            }
        }
    }

    if let Some(minimum_margin_percent) = minimum_margin_percent {
        for index in 0..pixel_count {
            let best = best_costs[index];
            let second = second_best_costs[index];
            if !costs_have_minimum_margin(best, second, minimum_margin_percent) {
                values[index] = INVALID_DISPARITY;
            }
        }
    }

    Ok(DisparityMap {
        width: width as u32,
        height: height as u32,
        values,
    })
}

pub fn costs_have_minimum_margin(best: u64, second: u64, minimum_percent: u16) -> bool {
    second != u64::MAX
        && second > best
        && u128::from(second - best) * 100 >= u128::from(best.max(1)) * u128::from(minimum_percent)
}

pub fn block_match_y8_consistent(
    left: &[u8],
    right: &[u8],
    width: u32,
    height: u32,
    parameters: ConsistentMatchParameters,
) -> Result<DisparityMap, DisparityError> {
    let left_to_right = block_match_y8_unique(
        left,
        right,
        width,
        height,
        parameters.disparities.clone(),
        parameters.radius,
        parameters.minimum_margin_percent,
    )?;
    let width_usize = usize::try_from(width).map_err(|_| DisparityError)?;
    let reversed_right = reverse_rows(right, width_usize);
    let reversed_left = reverse_rows(left, width_usize);
    let reversed_right_to_left = block_match_y8_unique(
        &reversed_right,
        &reversed_left,
        width,
        height,
        parameters.disparities,
        parameters.radius,
        parameters.minimum_margin_percent,
    )?;
    let right_to_left = DisparityMap {
        width,
        height,
        values: reverse_rows(&reversed_right_to_left.values, width_usize),
    };
    enforce_left_right_consistency(
        &left_to_right,
        &right_to_left,
        parameters.consistency_tolerance,
    )
}

pub fn enforce_left_right_consistency(
    left_to_right: &DisparityMap,
    right_to_left: &DisparityMap,
    tolerance: u16,
) -> Result<DisparityMap, DisparityError> {
    if left_to_right.width != right_to_left.width || left_to_right.height != right_to_left.height {
        return Err(DisparityError);
    }
    let width = usize::try_from(left_to_right.width).map_err(|_| DisparityError)?;
    let height = usize::try_from(left_to_right.height).map_err(|_| DisparityError)?;
    let pixel_count = width.checked_mul(height).ok_or(DisparityError)?;
    if pixel_count == 0
        || left_to_right.values.len() != pixel_count
        || right_to_left.values.len() != pixel_count
    {
        return Err(DisparityError);
    }

    let mut values = vec![INVALID_DISPARITY; pixel_count];
    for y in 0..height {
        for x in 0..width {
            let index = y * width + x;
            let disparity = left_to_right.values[index];
            if disparity == INVALID_DISPARITY {
                continue;
            }
            let Some(right_x) = x.checked_sub(usize::from(disparity)) else {
                continue;
            };
            let reverse_disparity = right_to_left.values[y * width + right_x];
            if reverse_disparity != INVALID_DISPARITY
                && disparity.abs_diff(reverse_disparity) <= tolerance
            {
                values[index] = disparity;
            }
        }
    }

    Ok(DisparityMap {
        width: left_to_right.width,
        height: left_to_right.height,
        values,
    })
}

fn reverse_rows<T: Copy>(values: &[T], width: usize) -> Vec<T> {
    values
        .chunks_exact(width)
        .flat_map(|row| row.iter().rev().copied())
        .collect()
}

pub fn encode_disparity_pgm(
    map: &DisparityMap,
    maximum_disparity: u16,
) -> Result<Vec<u8>, DisparityError> {
    let expected = usize::try_from(map.width)
        .ok()
        .and_then(|width| {
            usize::try_from(map.height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .ok_or(DisparityError)?;
    if expected == 0 || map.values.len() != expected || maximum_disparity == 0 {
        return Err(DisparityError);
    }
    let header = format!("P5\n{} {}\n255\n", map.width, map.height);
    let mut pgm = Vec::with_capacity(header.len() + expected);
    pgm.extend_from_slice(header.as_bytes());
    pgm.extend(map.values.iter().map(|value| {
        if *value == INVALID_DISPARITY {
            0
        } else {
            1 + (u32::from(*value).min(u32::from(maximum_disparity)) * 254
                / u32::from(maximum_disparity)) as u8
        }
    }));
    Ok(pgm)
}
