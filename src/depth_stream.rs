use crate::http_stream::{get_bounded_body, get_chunked_prefix, StreamError, StreamLimits};
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::net::SocketAddr;

const SET_DEPTH_FORMAT: &str = "/cgi-bin/zx_cmd.cgi?cam_type=mipi&set_depth_output_fmt=1";
const DEPTH_MEDIA: &str = "/cgi-bin/zx_media.cgi?camera_id=21";
const CLOSE_STREAMS: &str = "/cgi-bin/zx_cmd.cgi?close_stream_all";

#[derive(Debug)]
pub enum DepthStreamError {
    Http {
        stage: &'static str,
        source: StreamError,
    },
    RejectedConfiguration,
}

impl Display for DepthStreamError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http { stage, source } => write!(formatter, "{stage}: {source}"),
            Self::RejectedConfiguration => {
                formatter.write_str("scanner rejected the depth output configuration")
            }
        }
    }
}

impl Error for DepthStreamError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Http { source, .. } => Some(source),
            Self::RejectedConfiguration => None,
        }
    }
}

pub fn capture_depth_prefix(
    address: SocketAddr,
    limits: StreamLimits,
    prefix_bytes: usize,
    receive: impl FnMut(&[u8]),
) -> Result<usize, DepthStreamError> {
    let configuration = get_bounded_body(address, SET_DEPTH_FORMAT, limits).map_err(|source| {
        DepthStreamError::Http {
            stage: "configure depth output",
            source,
        }
    })?;
    if trim_ascii_whitespace(&configuration) != br#"{"result":0}"# {
        return Err(DepthStreamError::RejectedConfiguration);
    }

    let capture = get_chunked_prefix(address, DEPTH_MEDIA, limits, prefix_bytes, receive);
    let close = get_bounded_body(address, CLOSE_STREAMS, limits);
    match (capture, close) {
        (Ok(received), Ok(_)) => Ok(received),
        (Err(source), _) => Err(DepthStreamError::Http {
            stage: "capture depth media",
            source,
        }),
        (Ok(_), Err(source)) => Err(DepthStreamError::Http {
            stage: "close streams",
            source,
        }),
    }
}

fn trim_ascii_whitespace(bytes: &[u8]) -> &[u8] {
    bytes.trim_ascii()
}
