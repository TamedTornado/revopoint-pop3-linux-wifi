use crate::http_stream::{get_bounded_body, get_chunked_prefix, StreamError, StreamLimits};
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::net::SocketAddr;
use std::thread;
use std::time::Duration;

// The network SDK maps STREAM_FORMAT_Z16 to firmware selector 2. Selector 1
// requests the binocular infrared pair, whose adjacent Y8 bytes can look like
// plausible but false little-endian depths.
const SET_DEPTH_FORMAT: &str = "/cgi-bin/zx_cmd.cgi?cam_type=mipi&set_depth_output_fmt=2";
const SET_DEPTH_PROFILE: &str = "/cgi-bin/zx_cmd.cgi?cam_type=mipi&set_display_reso=1&&set_display_width=640&&set_display_height=400&&set_display_type=2";
const GET_DEPTH_RESOLUTION: &str = "/cgi-bin/zx_cmd.cgi?cam_type=mipi&get_depth_reso";
const GET_DEPTH_SCALE: &str = "/cgi-bin/zx_cmd.cgi?cam_type=mipi&algo_get_cmd_buf=2328";
const DEPTH_MEDIA: &str = "/cgi-bin/zx_media.cgi?camera_id=21";
const CLOSE_STREAMS: &str = "/cgi-bin/zx_cmd.cgi?close_stream_all";
const PROFILE_SETTLE_TIME: Duration = Duration::from_millis(300);
const SELECTOR_ATTEMPTS: usize = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DepthResolution {
    pub width: u32,
    pub height: u32,
    pub bytes_per_pixel: u8,
}

impl DepthResolution {
    pub fn stride_bytes(self) -> Option<usize> {
        usize::try_from(self.width)
            .ok()?
            .checked_mul(usize::from(self.bytes_per_pixel))
    }

    pub fn frame_bytes(self) -> Option<usize> {
        self.stride_bytes()?
            .checked_mul(usize::try_from(self.height).ok()?)
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct DepthResolutionError;

impl Display for DepthResolutionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("scanner returned an invalid current depth resolution")
    }
}

impl Error for DepthResolutionError {}

#[derive(Debug, Eq, PartialEq)]
pub struct DepthScaleError;

impl Display for DepthScaleError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("scanner returned an invalid depth scale")
    }
}

impl Error for DepthScaleError {}

pub fn parse_current_resolution(response: &[u8]) -> Result<DepthResolution, DepthResolutionError> {
    const MARKER: &[u8] = b"\"curr-resolution\":\"";
    let start = response
        .windows(MARKER.len())
        .position(|window| window == MARKER)
        .map(|position| position + MARKER.len())
        .ok_or(DepthResolutionError)?;
    let end = response[start..]
        .iter()
        .position(|byte| *byte == b'"')
        .map(|position| start + position)
        .ok_or(DepthResolutionError)?;
    let value = std::str::from_utf8(&response[start..end]).map_err(|_| DepthResolutionError)?;
    let mut components = value.split('x');
    let resolution = DepthResolution {
        width: parse_nonzero(components.next())?,
        height: parse_nonzero(components.next())?,
        bytes_per_pixel: parse_nonzero(components.next())?,
    };
    if components.next().is_some() || resolution.frame_bytes().is_none() {
        return Err(DepthResolutionError);
    }
    Ok(resolution)
}

fn parse_nonzero<T>(component: Option<&str>) -> Result<T, DepthResolutionError>
where
    T: std::str::FromStr + Default + PartialEq,
{
    let value = component
        .ok_or(DepthResolutionError)?
        .parse::<T>()
        .map_err(|_| DepthResolutionError)?;
    if value == T::default() {
        return Err(DepthResolutionError);
    }
    Ok(value)
}

pub fn parse_depth_scale_mm(response: &[u8]) -> Result<f32, DepthScaleError> {
    if response.len() != 60 || response[0] != 0xff || response[59] != 4 {
        return Err(DepthScaleError);
    }
    let divisor = u32::from_le_bytes(response[1..5].try_into().expect("four-byte divisor"));
    if divisor == 0 {
        return Err(DepthScaleError);
    }
    Ok(1.0 / divisor as f32)
}

#[derive(Debug)]
pub enum DepthStreamError {
    Http {
        stage: &'static str,
        source: StreamError,
    },
    RejectedProfile,
    RejectedConfiguration,
    InvalidResolution(DepthResolutionError),
    InvalidScale(DepthScaleError),
}

impl Display for DepthStreamError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http { stage, source } => write!(formatter, "{stage}: {source}"),
            Self::RejectedProfile => formatter.write_str("scanner rejected the Z16 depth profile"),
            Self::RejectedConfiguration => {
                formatter.write_str("scanner rejected the depth output configuration")
            }
            Self::InvalidResolution(error) => Display::fmt(error, formatter),
            Self::InvalidScale(error) => Display::fmt(error, formatter),
        }
    }
}

impl Error for DepthStreamError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Http { source, .. } => Some(source),
            Self::RejectedProfile | Self::RejectedConfiguration => None,
            Self::InvalidResolution(error) => Some(error),
            Self::InvalidScale(error) => Some(error),
        }
    }
}

pub fn get_current_depth_resolution(
    address: SocketAddr,
    limits: StreamLimits,
) -> Result<DepthResolution, DepthStreamError> {
    let response = get_bounded_body(address, GET_DEPTH_RESOLUTION, limits).map_err(|source| {
        DepthStreamError::Http {
            stage: "query current depth resolution",
            source,
        }
    })?;
    parse_current_resolution(&response).map_err(DepthStreamError::InvalidResolution)
}

pub fn get_depth_scale_mm(
    address: SocketAddr,
    limits: StreamLimits,
) -> Result<f32, DepthStreamError> {
    let response = get_bounded_body(address, GET_DEPTH_SCALE, limits).map_err(|source| {
        DepthStreamError::Http {
            stage: "query depth scale",
            source,
        }
    })?;
    parse_depth_scale_mm(&response).map_err(DepthStreamError::InvalidScale)
}

pub fn capture_depth_prefix(
    address: SocketAddr,
    limits: StreamLimits,
    prefix_bytes: usize,
    receive: impl FnMut(&[u8]),
) -> Result<usize, DepthStreamError> {
    let profile = get_bounded_body(address, SET_DEPTH_PROFILE, limits).map_err(|source| {
        DepthStreamError::Http {
            stage: "configure Z16 depth profile",
            source,
        }
    })?;
    if trim_ascii_whitespace(&profile) != br#"{"result":0}"# {
        return Err(DepthStreamError::RejectedProfile);
    }

    // RevoScan's Windows SDK gives the resolution change 300 ms to settle,
    // then retries the output-selector command up to three times. It does not
    // reboot or reconnect the scanner when changing selectors.
    thread::sleep(PROFILE_SETTLE_TIME);
    for attempt in 0..SELECTOR_ATTEMPTS {
        match get_bounded_body(address, SET_DEPTH_FORMAT, limits) {
            Ok(configuration) if trim_ascii_whitespace(&configuration) == br#"{"result":0}"# => {
                break;
            }
            Ok(_) => {
                if attempt + 1 == SELECTOR_ATTEMPTS {
                    return Err(DepthStreamError::RejectedConfiguration);
                }
            }
            Err(source) => {
                if attempt + 1 == SELECTOR_ATTEMPTS {
                    return Err(DepthStreamError::Http {
                        stage: "configure depth output",
                        source,
                    });
                }
            }
        }
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
