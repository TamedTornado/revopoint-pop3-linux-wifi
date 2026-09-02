use std::error::Error;
use std::fmt::{self, Display, Formatter};

const MAGIC: [u8; 4] = 0x1122_3344_u32.to_le_bytes();
const HEADER_BYTES: usize = 8;

#[derive(Debug, Eq, PartialEq)]
pub struct CompressedFrame {
    pub declared_payload_len: u32,
    pub payload: Vec<u8>,
}

#[derive(Debug)]
pub struct FrameEnvelopeError(String);

impl Display for FrameEnvelopeError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for FrameEnvelopeError {}

pub struct FrameEnvelopeParser {
    maximum_payload_bytes: usize,
    maximum_preamble_bytes: usize,
    synchronized: bool,
    declared_payload_len: Option<usize>,
    buffer: Vec<u8>,
}

impl FrameEnvelopeParser {
    pub fn new(maximum_payload_bytes: usize, maximum_preamble_bytes: usize) -> Self {
        Self {
            maximum_payload_bytes,
            maximum_preamble_bytes,
            synchronized: false,
            declared_payload_len: None,
            buffer: Vec::with_capacity(HEADER_BYTES),
        }
    }

    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<CompressedFrame>, FrameEnvelopeError> {
        let mut frames = Vec::new();
        for byte in bytes {
            self.buffer.push(*byte);
            if !self.synchronized {
                if self.buffer.ends_with(&MAGIC) {
                    let preamble_bytes = self.buffer.len() - MAGIC.len();
                    if preamble_bytes > self.maximum_preamble_bytes {
                        return Err(fail("frame preamble exceeds configured limit"));
                    }
                    self.buffer.drain(..preamble_bytes);
                    self.synchronized = true;
                } else if self.buffer.len() > self.maximum_preamble_bytes + MAGIC.len() - 1 {
                    return Err(fail("frame magic was not found within preamble limit"));
                }
                continue;
            }

            if self.buffer.len() == MAGIC.len() && self.buffer.as_slice() != MAGIC {
                return Err(fail("invalid frame magic"));
            }
            if self.buffer.len() == HEADER_BYTES {
                let declared = u32::from_le_bytes(
                    self.buffer[4..8]
                        .try_into()
                        .expect("fixed frame length field"),
                ) as usize;
                if declared == 0 || declared > self.maximum_payload_bytes {
                    return Err(fail("invalid frame payload length"));
                }
                self.declared_payload_len = Some(declared);
            }
            let Some(declared) = self.declared_payload_len else {
                continue;
            };
            if self.buffer.len() == HEADER_BYTES + declared {
                let payload = self.buffer.split_off(HEADER_BYTES);
                self.buffer.clear();
                self.declared_payload_len = None;
                frames.push(CompressedFrame {
                    declared_payload_len: declared as u32,
                    payload,
                });
            }
        }
        Ok(frames)
    }

    pub fn finish(&self) -> Result<(), FrameEnvelopeError> {
        if self.buffer.is_empty() {
            Ok(())
        } else {
            Err(fail("frame stream ended with a truncated envelope"))
        }
    }
}

fn fail(message: impl Into<String>) -> FrameEnvelopeError {
    FrameEnvelopeError(message.into())
}
