use crate::calibration::ScaledDepthIntrinsics;
use crate::depth_decode::{DepthEncoding, DepthPlane};
use std::error::Error;
use std::fmt::{self, Display, Formatter};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RosTime {
    pub sec: i32,
    pub nanosec: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RosHeader {
    pub stamp: RosTime,
    pub frame_id: String,
}

#[derive(Debug, PartialEq)]
pub struct RosDepthImage {
    pub header: RosHeader,
    pub height: u32,
    pub width: u32,
    pub encoding: &'static str,
    pub is_bigendian: u8,
    pub step: u32,
    pub data: Vec<u8>,
}

#[derive(Debug, PartialEq)]
pub struct RosCameraInfo {
    pub header: RosHeader,
    pub height: u32,
    pub width: u32,
    pub distortion_model: &'static str,
    pub d: [f64; 5],
    pub k: [f64; 9],
    pub r: [f64; 9],
    pub p: [f64; 12],
}

#[derive(Debug, PartialEq)]
pub struct RosDepthCameraFrame {
    pub image: RosDepthImage,
    pub camera_info: RosCameraInfo,
}

#[derive(Debug, Eq, PartialEq)]
pub struct RosCameraError;

impl Display for RosCameraError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("depth plane cannot be represented as a ROS camera frame")
    }
}

impl Error for RosCameraError {}

pub fn map_depth_camera(
    plane: DepthPlane,
    intrinsics: ScaledDepthIntrinsics,
    stamp: RosTime,
    frame_id: &str,
) -> Result<RosDepthCameraFrame, RosCameraError> {
    let input_step = usize::try_from(plane.width)
        .ok()
        .and_then(|width| width.checked_mul(2))
        .ok_or(RosCameraError)?;
    let input_bytes = usize::try_from(plane.height)
        .ok()
        .and_then(|height| input_step.checked_mul(height))
        .ok_or(RosCameraError)?;
    if plane.width == 0
        || plane.height == 0
        || plane.encoding != DepthEncoding::Z16LittleEndian
        || plane.stride_bytes != input_step
        || plane.bytes.len() != input_bytes
        || !plane.millimeters_per_unit.is_finite()
        || plane.millimeters_per_unit <= 0.0
        || intrinsics.width != plane.width
        || intrinsics.height != plane.height
        || !valid_intrinsics(intrinsics)
        || stamp.nanosec >= 1_000_000_000
        || frame_id.is_empty()
        || frame_id.chars().any(char::is_whitespace)
    {
        return Err(RosCameraError);
    }

    let output_step = plane.width.checked_mul(4).ok_or(RosCameraError)?;
    let output_bytes = usize::try_from(plane.height)
        .ok()
        .and_then(|height| usize::try_from(output_step).ok()?.checked_mul(height))
        .ok_or(RosCameraError)?;
    let mut data = Vec::with_capacity(output_bytes);
    for sample in plane.bytes.as_chunks::<2>().0 {
        let raw = u16::from_le_bytes(*sample);
        let meters = f32::from(raw) * plane.millimeters_per_unit / 1000.0;
        data.extend_from_slice(&meters.to_le_bytes());
    }
    debug_assert_eq!(data.len(), output_bytes);

    let header = RosHeader {
        stamp,
        frame_id: frame_id.to_owned(),
    };
    let fx = f64::from(intrinsics.fx);
    let fy = f64::from(intrinsics.fy);
    let cx = f64::from(intrinsics.cx);
    let cy = f64::from(intrinsics.cy);
    Ok(RosDepthCameraFrame {
        image: RosDepthImage {
            header: header.clone(),
            height: plane.height,
            width: plane.width,
            encoding: "32FC1",
            is_bigendian: 0,
            step: output_step,
            data,
        },
        camera_info: RosCameraInfo {
            header,
            height: plane.height,
            width: plane.width,
            distortion_model: "plumb_bob",
            d: [0.0; 5],
            k: [fx, 0.0, cx, 0.0, fy, cy, 0.0, 0.0, 1.0],
            r: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            p: [fx, 0.0, cx, 0.0, 0.0, fy, cy, 0.0, 0.0, 0.0, 1.0, 0.0],
        },
    })
}

fn valid_intrinsics(intrinsics: ScaledDepthIntrinsics) -> bool {
    intrinsics.fx.is_finite()
        && intrinsics.fx > 0.0
        && intrinsics.fy.is_finite()
        && intrinsics.fy > 0.0
        && intrinsics.cx.is_finite()
        && intrinsics.cy.is_finite()
}
