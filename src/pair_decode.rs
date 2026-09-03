use std::error::Error;
use std::fmt::{self, Display, Formatter};

#[derive(Debug, Eq, PartialEq)]
pub enum PairDecodeError {
    InvalidLayout,
}

impl Display for PairDecodeError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("decoded buffer does not contain two contiguous Y8 planes")
    }
}

impl Error for PairDecodeError {}

#[derive(Debug, Eq, PartialEq)]
pub struct Y8Pair {
    pub width: u32,
    pub height: u32,
    pub left: Vec<u8>,
    pub right: Vec<u8>,
}

fn image_bytes(width: u32, height: u32) -> Result<usize, PairDecodeError> {
    usize::try_from(width)
        .ok()
        .filter(|width| *width != 0)
        .and_then(|width| {
            usize::try_from(height)
                .ok()
                .filter(|height| *height != 0)
                .and_then(|height| width.checked_mul(height))
        })
        .ok_or(PairDecodeError::InvalidLayout)
}

pub fn decode_y8_pair(
    mut bytes: Vec<u8>,
    width: u32,
    height: u32,
) -> Result<Y8Pair, PairDecodeError> {
    let plane_bytes = image_bytes(width, height)?;
    let frame_bytes = plane_bytes
        .checked_mul(2)
        .ok_or(PairDecodeError::InvalidLayout)?;
    if bytes.len() != frame_bytes {
        return Err(PairDecodeError::InvalidLayout);
    }

    let right = bytes.split_off(plane_bytes);
    let left = bytes;
    Ok(Y8Pair {
        width,
        height,
        left,
        right,
    })
}

pub fn encode_y8_pgm(width: u32, height: u32, pixels: &[u8]) -> Result<Vec<u8>, PairDecodeError> {
    if pixels.len() != image_bytes(width, height)? {
        return Err(PairDecodeError::InvalidLayout);
    }
    let mut pgm = format!("P5\n{width} {height}\n255\n").into_bytes();
    pgm.extend_from_slice(pixels);
    Ok(pgm)
}
