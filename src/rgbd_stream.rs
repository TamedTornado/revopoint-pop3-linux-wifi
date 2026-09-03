use crate::camera_control::{set_depth_control, DepthControl, DepthControlError};
use crate::http_stream::{get_bounded_body, get_chunked_until, StreamError, StreamLimits};
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Barrier;
use std::thread;
use std::time::Duration;

const CLOSE_STREAMS: &str = "/cgi-bin/zx_cmd.cgi?close_stream_all";
const SET_DEPTH_PROFILE: &str = "/cgi-bin/zx_cmd.cgi?cam_type=mipi&set_display_reso=1&&set_display_width=640&&set_display_height=400&&set_display_type=4";
const SET_DEPTH_FORMAT: &str = "/cgi-bin/zx_cmd.cgi?cam_type=mipi&set_depth_output_fmt=3";
const SET_RGB_PROFILE: &str =
    "/cgi-bin/zx_cmd.cgi?cam_type=usb&set_resolution=1&width=1280&height=800";
const SET_FREE_RUNNING_TRIGGER: &str = "/cgi-bin/zx_cmd.cgi?cam_type=mipi&set_trigger_mode=0";
const DEPTH_MEDIA: &str = "/cgi-bin/zx_media.cgi?camera_id=21";
const RGB_MEDIA: &str = "/cgi-bin/zx_media.cgi?camera_id=50&type_id=20";
const LED_MASTER_ON: &str =
    "/cgi-bin/zx_cmd.cgi?system_cmd=echo%20s%200xb00%201%20%3E%20/dev/rk_preisp";
const LED_IR_ON: &str =
    "/cgi-bin/zx_cmd.cgi?system_cmd=echo%20s%200xb01%201%20%3E%20/dev/rk_preisp";
const RGB_ENABLE: &str =
    "/cgi-bin/zx_cmd.cgi?system_cmd=echo%20s%200xb02%201%20%3E%20/dev/rk_preisp";
const RGB_DISABLE: &str =
    "/cgi-bin/zx_cmd.cgi?system_cmd=echo%20s%200xb02%200%20%3E%20/dev/rk_preisp";
const LED_IR_OFF: &str =
    "/cgi-bin/zx_cmd.cgi?system_cmd=echo%20s%200xb01%200%20%3E%20/dev/rk_preisp";
const LED_MASTER_OFF: &str =
    "/cgi-bin/zx_cmd.cgi?system_cmd=echo%20s%200xb00%200%20%3E%20/dev/rk_preisp";
const PROFILE_SETTLE_TIME: Duration = Duration::from_millis(300);
const EMITTER_SETTLE_TIME: Duration = Duration::from_millis(300);
const SELECTOR_ATTEMPTS: usize = 3;

#[derive(Debug)]
pub enum RgbdStreamError {
    Http {
        stage: &'static str,
        source: StreamError,
    },
    Rejected(&'static str),
    WorkerPanicked,
    Control(DepthControlError),
}

impl Display for RgbdStreamError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http { stage, source } => write!(formatter, "{stage}: {source}"),
            Self::Rejected(stage) => write!(formatter, "scanner rejected {stage}"),
            Self::WorkerPanicked => formatter.write_str("RGB-D capture worker panicked"),
            Self::Control(error) => Display::fmt(error, formatter),
        }
    }
}

impl Error for RgbdStreamError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Http { source, .. } => Some(source),
            Self::Control(error) => Some(error),
            Self::Rejected(_) | Self::WorkerPanicked => None,
        }
    }
}

pub fn capture_rgbd_until<DepthReceive, RgbReceive>(
    address: SocketAddr,
    limits: StreamLimits,
    depth_receive: DepthReceive,
    rgb_receive: RgbReceive,
) -> Result<(usize, usize), RgbdStreamError>
where
    DepthReceive: FnMut(&[u8]) -> bool + Send,
    RgbReceive: FnMut(&[u8]) -> bool + Send,
{
    capture_rgbd_until_inner(address, limits, None, depth_receive, rgb_receive)
}

pub fn capture_rgbd_until_with_control<DepthReceive, RgbReceive>(
    address: SocketAddr,
    limits: StreamLimits,
    control: DepthControl,
    depth_receive: DepthReceive,
    rgb_receive: RgbReceive,
) -> Result<(usize, usize), RgbdStreamError>
where
    DepthReceive: FnMut(&[u8]) -> bool + Send,
    RgbReceive: FnMut(&[u8]) -> bool + Send,
{
    capture_rgbd_until_inner(address, limits, Some(control), depth_receive, rgb_receive)
}

fn capture_rgbd_until_inner<DepthReceive, RgbReceive>(
    address: SocketAddr,
    limits: StreamLimits,
    control: Option<DepthControl>,
    depth_receive: DepthReceive,
    rgb_receive: RgbReceive,
) -> Result<(usize, usize), RgbdStreamError>
where
    DepthReceive: FnMut(&[u8]) -> bool + Send,
    RgbReceive: FnMut(&[u8]) -> bool + Send,
{
    if let Err(error) = configure(address, limits, control) {
        let _ = shutdown(address, limits);
        return Err(error);
    }

    let start = Barrier::new(3);
    let depth_complete = AtomicBool::new(false);
    let rgb_complete = AtomicBool::new(false);
    let captures = thread::scope(|scope| {
        let depth_start = &start;
        let depth_complete_for_depth = &depth_complete;
        let rgb_complete_for_depth = &rgb_complete;
        let mut depth_receive = depth_receive;
        let depth = scope.spawn(move || {
            depth_start.wait();
            get_chunked_until(address, DEPTH_MEDIA, limits, |chunk| {
                if depth_receive(chunk) {
                    depth_complete_for_depth.store(true, Ordering::Release);
                }
                both_complete(
                    depth_complete_for_depth.load(Ordering::Acquire),
                    rgb_complete_for_depth.load(Ordering::Acquire),
                )
            })
        });
        let rgb_start = &start;
        let depth_complete_for_rgb = &depth_complete;
        let rgb_complete_for_rgb = &rgb_complete;
        let mut rgb_receive = rgb_receive;
        let rgb = scope.spawn(move || {
            rgb_start.wait();
            get_chunked_until(address, RGB_MEDIA, limits, |chunk| {
                if rgb_receive(chunk) {
                    rgb_complete_for_rgb.store(true, Ordering::Release);
                }
                both_complete(
                    rgb_complete_for_rgb.load(Ordering::Acquire),
                    depth_complete_for_rgb.load(Ordering::Acquire),
                )
            })
        });
        start.wait();
        (depth.join(), rgb.join())
    });
    let cleanup = shutdown(address, limits);

    match captures {
        (Ok(Ok(depth)), Ok(Ok(rgb))) => cleanup.map(|()| (depth, rgb)),
        (Ok(Err(source)), _) => Err(RgbdStreamError::Http {
            stage: "capture depth media",
            source,
        }),
        (_, Ok(Err(source))) => Err(RgbdStreamError::Http {
            stage: "capture RGB media",
            source,
        }),
        (Err(_), _) | (_, Err(_)) => Err(RgbdStreamError::WorkerPanicked),
    }
}

fn both_complete(own: bool, peer: bool) -> bool {
    own && peer
}

fn configure(
    address: SocketAddr,
    limits: StreamLimits,
    control: Option<DepthControl>,
) -> Result<(), RgbdStreamError> {
    result_control(address, limits, CLOSE_STREAMS, "close existing streams")?;
    result_control(
        address,
        limits,
        SET_DEPTH_PROFILE,
        "configure depth profile",
    )?;
    thread::sleep(PROFILE_SETTLE_TIME);
    let mut selector_error = None;
    for _ in 0..SELECTOR_ATTEMPTS {
        match result_control(address, limits, SET_DEPTH_FORMAT, "configure depth output") {
            Ok(()) => {
                selector_error = None;
                break;
            }
            Err(error) => selector_error = Some(error),
        }
    }
    if let Some(error) = selector_error {
        return Err(error);
    }
    result_control(address, limits, SET_RGB_PROFILE, "configure RGB profile")?;
    result_control(
        address,
        limits,
        SET_FREE_RUNNING_TRIGGER,
        "configure free-running trigger",
    )?;
    if let Some(control) = control {
        set_depth_control(address, limits, control).map_err(RgbdStreamError::Control)?;
    }
    ok_control(address, limits, LED_MASTER_ON, "enable LED master")?;
    ok_control(address, limits, LED_IR_ON, "enable infrared projector")?;
    ok_control(address, limits, RGB_ENABLE, "enable RGB sensor")?;
    thread::sleep(EMITTER_SETTLE_TIME);
    Ok(())
}

fn shutdown(address: SocketAddr, limits: StreamLimits) -> Result<(), RgbdStreamError> {
    let results = [
        result_control(address, limits, CLOSE_STREAMS, "close RGB-D streams"),
        ok_control(address, limits, RGB_DISABLE, "disable RGB sensor"),
        ok_control(address, limits, LED_IR_OFF, "disable infrared projector"),
        ok_control(address, limits, LED_MASTER_OFF, "disable LED master"),
    ];
    results
        .into_iter()
        .find_map(Result::err)
        .map_or(Ok(()), Err)
}

fn result_control(
    address: SocketAddr,
    limits: StreamLimits,
    path: &'static str,
    stage: &'static str,
) -> Result<(), RgbdStreamError> {
    let response = get_bounded_body(address, path, limits)
        .map_err(|source| RgbdStreamError::Http { stage, source })?;
    if !matches!(response.trim_ascii(), br#"{"result":0}"# | b"{result:0}") {
        return Err(RgbdStreamError::Rejected(stage));
    }
    Ok(())
}

fn ok_control(
    address: SocketAddr,
    limits: StreamLimits,
    path: &'static str,
    stage: &'static str,
) -> Result<(), RgbdStreamError> {
    let response = get_bounded_body(address, path, limits)
        .map_err(|source| RgbdStreamError::Http { stage, source })?;
    if !response.trim_ascii().ends_with(b"[ok]") {
        return Err(RgbdStreamError::Rejected(stage));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::both_complete;

    #[test]
    fn waits_for_both_streams_to_reach_an_application_frame_boundary() {
        assert!(!both_complete(false, false));
        assert!(!both_complete(true, false));
        assert!(!both_complete(false, true));
        assert!(both_complete(true, true));
    }
}
