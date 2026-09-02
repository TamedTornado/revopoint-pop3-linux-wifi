use crate::frame_envelope::CompressedFrame;
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::io::Cursor;

#[derive(Debug, Eq, PartialEq)]
pub struct DecodedDepth {
    pub flags: u8,
    pub compressed_len: u32,
    pub decompressed_len: u32,
    pub bytes: Vec<u8>,
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
