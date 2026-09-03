use crate::http_stream::{get_bounded_body, get_chunked_prefix, StreamError, StreamLimits};
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::net::SocketAddr;

const CLOSE_STREAMS: &str = "/cgi-bin/zx_cmd.cgi?close_stream_all";
const SET_RGB_PROFILE: &str =
    "/cgi-bin/zx_cmd.cgi?cam_type=usb&set_resolution=1&width=1280&height=800";
const SET_FREE_RUNNING_TRIGGER: &str = "/cgi-bin/zx_cmd.cgi?cam_type=mipi&set_trigger_mode=0";
const RGB_MEDIA: &str = "/cgi-bin/zx_media.cgi?camera_id=50&type_id=20";
const LED_MASTER_ON: &str =
    "/cgi-bin/zx_cmd.cgi?system_cmd=echo%20s%200xb00%201%20%3E%20/dev/rk_preisp";
const RGB_ENABLE: &str =
    "/cgi-bin/zx_cmd.cgi?system_cmd=echo%20s%200xb02%201%20%3E%20/dev/rk_preisp";
const RGB_DISABLE: &str =
    "/cgi-bin/zx_cmd.cgi?system_cmd=echo%20s%200xb02%200%20%3E%20/dev/rk_preisp";
const LED_MASTER_OFF: &str =
    "/cgi-bin/zx_cmd.cgi?system_cmd=echo%20s%200xb00%200%20%3E%20/dev/rk_preisp";

#[derive(Debug)]
pub enum RgbStreamError {
    Http {
        stage: &'static str,
        source: StreamError,
    },
    Rejected(&'static str),
}

impl Display for RgbStreamError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http { stage, source } => write!(formatter, "{stage}: {source}"),
            Self::Rejected(stage) => write!(formatter, "scanner rejected {stage}"),
        }
    }
}

impl Error for RgbStreamError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Http { source, .. } => Some(source),
            Self::Rejected(_) => None,
        }
    }
}

pub fn capture_rgb_prefix(
    address: SocketAddr,
    limits: StreamLimits,
    prefix_bytes: usize,
    receive: impl FnMut(&[u8]),
) -> Result<usize, RgbStreamError> {
    result_control(address, limits, CLOSE_STREAMS, "close existing streams")?;
    result_control(address, limits, SET_RGB_PROFILE, "configure RGB profile")?;
    ok_control(address, limits, LED_MASTER_ON, "enable LED master")?;
    if let Err(error) = ok_control(address, limits, RGB_ENABLE, "enable RGB sensor") {
        let _ = ok_control(address, limits, LED_MASTER_OFF, "disable LED master");
        return Err(error);
    }
    if let Err(error) = result_control(
        address,
        limits,
        SET_FREE_RUNNING_TRIGGER,
        "configure free-running trigger",
    ) {
        let _ = ok_control(address, limits, RGB_DISABLE, "disable RGB sensor");
        let _ = ok_control(address, limits, LED_MASTER_OFF, "disable LED master");
        return Err(error);
    }

    let capture = get_chunked_prefix(address, RGB_MEDIA, limits, prefix_bytes, receive);
    let close = result_control(address, limits, CLOSE_STREAMS, "close RGB stream");
    let rgb_off = ok_control(address, limits, RGB_DISABLE, "disable RGB sensor");
    let master_off = ok_control(address, limits, LED_MASTER_OFF, "disable LED master");
    match (capture, close, rgb_off, master_off) {
        (Ok(received), Ok(()), Ok(()), Ok(())) => Ok(received),
        (Err(source), _, _, _) => Err(RgbStreamError::Http {
            stage: "capture RGB media",
            source,
        }),
        (Ok(_), Err(error), _, _)
        | (Ok(_), Ok(()), Err(error), _)
        | (Ok(_), Ok(()), Ok(()), Err(error)) => Err(error),
    }
}

fn result_control(
    address: SocketAddr,
    limits: StreamLimits,
    path: &'static str,
    stage: &'static str,
) -> Result<(), RgbStreamError> {
    let response = get_bounded_body(address, path, limits)
        .map_err(|source| RgbStreamError::Http { stage, source })?;
    if !matches!(response.trim_ascii(), br#"{"result":0}"# | b"{result:0}") {
        return Err(RgbStreamError::Rejected(stage));
    }
    Ok(())
}

fn ok_control(
    address: SocketAddr,
    limits: StreamLimits,
    path: &'static str,
    stage: &'static str,
) -> Result<(), RgbStreamError> {
    let response = get_bounded_body(address, path, limits)
        .map_err(|source| RgbStreamError::Http { stage, source })?;
    if !response.trim_ascii().ends_with(b"[ok]") {
        return Err(RgbStreamError::Rejected(stage));
    }
    Ok(())
}
