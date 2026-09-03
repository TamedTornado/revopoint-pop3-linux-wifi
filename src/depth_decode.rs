use crate::frame_envelope::CompressedFrame;
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::io::Cursor;

const EXTRA_INFO_BYTES: usize = 80;
const DEVICE_TIMESTAMP_OFFSET: usize = 20;

#[derive(Debug, Eq, PartialEq)]
pub struct DecodedDepth {
    pub flags: u8,
    pub compressed_len: u32,
    pub decompressed_len: u32,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DepthEncoding {
    Z16LittleEndian,
}

#[derive(Debug, PartialEq)]
pub struct DepthPlane {
    pub width: u32,
    pub height: u32,
    pub stride_bytes: usize,
    pub encoding: DepthEncoding,
    pub millimeters_per_unit: f32,
    pub bytes: Vec<u8>,
}

#[derive(Debug, PartialEq)]
pub struct Z16Y8Y8Frame {
    pub device_timestamp_ms: u32,
    pub depth: DepthPlane,
    pub left: Vec<u8>,
    pub right: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DepthStatistics {
    pub samples: usize,
    pub nonzero_samples: usize,
    pub minimum_nonzero_raw: Option<u16>,
    pub maximum_raw: u16,
    pub mean_nonzero_mm: Option<f64>,
}

impl DepthPlane {
    pub fn statistics(&self) -> DepthStatistics {
        let mut nonzero_samples = 0_usize;
        let mut minimum_nonzero_raw = None;
        let mut maximum_raw = 0_u16;
        let mut sum_nonzero_raw = 0_u64;
        let (samples, remainder) = self.bytes.as_chunks::<2>();
        debug_assert!(remainder.is_empty(), "validated Z16 buffer has even length");
        for bytes in samples {
            let value = u16::from_le_bytes([bytes[0], bytes[1]]);
            maximum_raw = maximum_raw.max(value);
            if value != 0 {
                nonzero_samples += 1;
                sum_nonzero_raw += u64::from(value);
                minimum_nonzero_raw =
                    Some(minimum_nonzero_raw.map_or(value, |minimum: u16| minimum.min(value)));
            }
        }
        let mean_nonzero_mm = (nonzero_samples != 0).then(|| {
            (sum_nonzero_raw as f64 / nonzero_samples as f64) * f64::from(self.millimeters_per_unit)
        });
        DepthStatistics {
            samples: self.bytes.len() / 2,
            nonzero_samples,
            minimum_nonzero_raw,
            maximum_raw,
            mean_nonzero_mm,
        }
    }
}

impl DecodedDepth {
    pub fn into_z16y8y8(
        self,
        width: u32,
        height: u32,
        millimeters_per_unit: f32,
    ) -> Result<Z16Y8Y8Frame, DepthDecodeError> {
        let pixels = usize::try_from(width)
            .ok()
            .and_then(|width| {
                usize::try_from(height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .ok_or_else(|| fail("Z16Y8Y8 frame size overflows the platform"))?;
        let depth_bytes = pixels
            .checked_mul(2)
            .ok_or_else(|| fail("Z16Y8Y8 depth size overflows the platform"))?;
        let expected_bytes = pixels
            .checked_mul(4)
            .ok_or_else(|| fail("Z16Y8Y8 frame size overflows the platform"))?;
        if self.bytes.len() != expected_bytes || self.decompressed_len as usize != expected_bytes {
            return Err(fail("decoded buffer length disagrees with Z16Y8Y8 layout"));
        }

        let mut bytes = self.bytes;
        let device_timestamp_ms = extract_depth_extra_info(&mut bytes)?;
        let right = bytes.split_off(depth_bytes + pixels);
        let left = bytes.split_off(depth_bytes);
        let depth = DecodedDepth {
            flags: self.flags,
            compressed_len: self.compressed_len,
            decompressed_len: u32::try_from(depth_bytes)
                .map_err(|_| fail("Z16Y8Y8 depth size exceeds its wire field"))?,
            bytes,
        }
        .into_z16_plane(width, height, millimeters_per_unit)?;

        Ok(Z16Y8Y8Frame {
            device_timestamp_ms,
            depth,
            left,
            right,
        })
    }

    pub fn into_z16_plane(
        self,
        width: u32,
        height: u32,
        millimeters_per_unit: f32,
    ) -> Result<DepthPlane, DepthDecodeError> {
        if width == 0
            || height == 0
            || !millimeters_per_unit.is_finite()
            || millimeters_per_unit <= 0.0
        {
            return Err(fail("invalid Z16 plane metadata"));
        }
        let stride_bytes = usize::try_from(width)
            .ok()
            .and_then(|width| width.checked_mul(2))
            .ok_or_else(|| fail("Z16 stride overflows the platform"))?;
        let expected_bytes = usize::try_from(height)
            .ok()
            .and_then(|height| stride_bytes.checked_mul(height))
            .ok_or_else(|| fail("Z16 frame size overflows the platform"))?;
        if self.bytes.len() != expected_bytes || self.decompressed_len as usize != expected_bytes {
            return Err(fail("decoded buffer length disagrees with Z16 layout"));
        }

        Ok(DepthPlane {
            width,
            height,
            stride_bytes,
            encoding: DepthEncoding::Z16LittleEndian,
            millimeters_per_unit,
            bytes: self.bytes,
        })
    }
}

pub fn extract_depth_extra_info(bytes: &mut [u8]) -> Result<u32, DepthDecodeError> {
    let extra_info = bytes
        .get_mut(..EXTRA_INFO_BYTES)
        .ok_or_else(|| fail("decoded depth frame is shorter than its extra-info prefix"))?;
    let timestamp = u32::from_le_bytes(
        extra_info[DEVICE_TIMESTAMP_OFFSET..DEVICE_TIMESTAMP_OFFSET + 4]
            .try_into()
            .expect("fixed depth timestamp field"),
    );
    extra_info.fill(0);
    Ok(timestamp)
}

#[derive(Debug)]
pub struct DepthDecodeError(String);

impl Display for DepthDecodeError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for DepthDecodeError {}

pub fn decode_quicklz(
    frame: &CompressedFrame,
    maximum_decompressed_bytes: usize,
) -> Result<DecodedDepth, DepthDecodeError> {
    if frame.payload.len() != frame.declared_payload_len as usize {
        return Err(fail("frame envelope and payload length disagree"));
    }
    if frame.payload.len() < 9 {
        return Err(fail("QuickLZ long header is truncated"));
    }
    let flags = frame.payload[0];
    if flags & 2 == 0 {
        return Err(fail("depth frame does not use a QuickLZ long header"));
    }
    let compressed_len = read_u32(&frame.payload[1..5]);
    let decompressed_len = read_u32(&frame.payload[5..9]);
    if compressed_len as usize != frame.payload.len() {
        return Err(fail(
            "QuickLZ and frame-envelope compressed lengths disagree",
        ));
    }
    if decompressed_len == 0 || decompressed_len as usize > maximum_decompressed_bytes {
        return Err(fail("QuickLZ decompressed length exceeds configured limit"));
    }

    let mut input = Cursor::new(frame.payload.as_slice());
    let maximum = u32::try_from(maximum_decompressed_bytes).unwrap_or(u32::MAX);
    let bytes = quicklz::decompress(&mut input, maximum)
        .map_err(|error| fail(format!("QuickLZ decompression failed: {error}")))?;
    if bytes.len() != decompressed_len as usize {
        return Err(fail("QuickLZ output length disagrees with its header"));
    }

    Ok(DecodedDepth {
        flags,
        compressed_len,
        decompressed_len,
        bytes,
    })
}

fn read_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes.try_into().expect("fixed four-byte field"))
}

fn fail(message: impl Into<String>) -> DepthDecodeError {
    DepthDecodeError(message.into())
}
