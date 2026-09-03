use std::error::Error;
use std::fmt::{self, Display, Formatter};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JpegInformation {
    pub width: u16,
    pub height: u16,
    pub encoded_len: usize,
    pub device_timestamp_ms: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JpegError;

impl Display for JpegError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("RGB frame is not a complete bounded JPEG image")
    }
}

impl Error for JpegError {}

pub fn inspect_jpeg(bytes: &[u8]) -> Result<JpegInformation, JpegError> {
    let (encoded_len, device_timestamp_ms) = if bytes.ends_with(&[0xff, 0xd9]) {
        (bytes.len(), None)
    } else if bytes.len() >= 6 && bytes[bytes.len() - 6..bytes.len() - 4] == [0xff, 0xd9] {
        (
            bytes.len() - 4,
            Some(u32::from_le_bytes(
                bytes[bytes.len() - 4..]
                    .try_into()
                    .expect("four-byte RGB timestamp"),
            )),
        )
    } else {
        return Err(JpegError);
    };
    let jpeg = &bytes[..encoded_len];
    if !jpeg.starts_with(&[0xff, 0xd8]) {
        return Err(JpegError);
    }

    let mut offset = 2_usize;
    loop {
        if jpeg.get(offset) != Some(&0xff) {
            return Err(JpegError);
        }
        while jpeg.get(offset) == Some(&0xff) {
            offset += 1;
        }
        let marker = *jpeg.get(offset).ok_or(JpegError)?;
        offset += 1;
        if marker == 0xd9 {
            return Err(JpegError);
        }
        if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            continue;
        }

        let length_bytes = jpeg.get(offset..offset + 2).ok_or(JpegError)?;
        let length = usize::from(u16::from_be_bytes(
            length_bytes.try_into().map_err(|_| JpegError)?,
        ));
        if length < 2 {
            return Err(JpegError);
        }
        let end = offset.checked_add(length).ok_or(JpegError)?;
        let segment = jpeg.get(offset..end).ok_or(JpegError)?;
        if is_start_of_frame(marker) {
            if segment.len() < 8 {
                return Err(JpegError);
            }
            let height = u16::from_be_bytes([segment[3], segment[4]]);
            let width = u16::from_be_bytes([segment[5], segment[6]]);
            if width == 0 || height == 0 {
                return Err(JpegError);
            }
            return Ok(JpegInformation {
                width,
                height,
                encoded_len,
                device_timestamp_ms,
            });
        }
        if marker == 0xda {
            return Err(JpegError);
        }
        offset = end;
    }
}

fn is_start_of_frame(marker: u8) -> bool {
    matches!(
        marker,
        0xc0 | 0xc1 | 0xc2 | 0xc3 | 0xc5 | 0xc6 | 0xc7 | 0xc9 | 0xca | 0xcb | 0xcd | 0xce | 0xcf
    )
}
