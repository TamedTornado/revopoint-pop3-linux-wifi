use crate::ros_camera::{RosCameraInfo, RosDepthCameraFrame, RosDepthImage, RosHeader, RosTime};
use rclrs::{
    ArrayValueMut, DynamicMessage, DynamicMessageViewMut, PublisherOptions, SequenceValueMut,
    SimpleValueMut, ValueMut, QOS_PROFILE_SENSOR_DATA,
};
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::sync::Arc;

#[derive(Debug)]
pub struct Ros2AdapterError(String);

impl Display for Ros2AdapterError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for Ros2AdapterError {}

fn fail(message: impl Into<String>) -> Ros2AdapterError {
    Ros2AdapterError(message.into())
}

pub fn image_message(image: RosDepthImage) -> Result<DynamicMessage, Ros2AdapterError> {
    let mut message = DynamicMessage::new(
        "sensor_msgs/msg/Image"
            .try_into()
            .map_err(|error| fail(format!("invalid Image type name: {error}")))?,
    )
    .map_err(|error| fail(format!("load Image type support: {error}")))?;
    set_header(&mut message, &image.header)?;
    set_u32(message.get_mut("height"), image.height, "Image.height")?;
    set_u32(message.get_mut("width"), image.width, "Image.width")?;
    set_string(
        message.get_mut("encoding"),
        image.encoding,
        "Image.encoding",
    )?;
    set_u8(
        message.get_mut("is_bigendian"),
        image.is_bigendian,
        "Image.is_bigendian",
    )?;
    set_u32(message.get_mut("step"), image.step, "Image.step")?;
    match message.get_mut("data") {
        Some(ValueMut::Sequence(SequenceValueMut::Uint8Sequence(value))) => {
            *value = image.data.into()
        }
        _ => return Err(fail("Image.data has an unexpected ROS type")),
    }
    Ok(message)
}

pub fn camera_info_message(info: RosCameraInfo) -> Result<DynamicMessage, Ros2AdapterError> {
    let mut message = DynamicMessage::new(
        "sensor_msgs/msg/CameraInfo"
            .try_into()
            .map_err(|error| fail(format!("invalid CameraInfo type name: {error}")))?,
    )
    .map_err(|error| fail(format!("load CameraInfo type support: {error}")))?;
    set_header(&mut message, &info.header)?;
    set_u32(message.get_mut("height"), info.height, "CameraInfo.height")?;
    set_u32(message.get_mut("width"), info.width, "CameraInfo.width")?;
    set_string(
        message.get_mut("distortion_model"),
        info.distortion_model,
        "CameraInfo.distortion_model",
    )?;
    match message.get_mut("d") {
        Some(ValueMut::Sequence(SequenceValueMut::DoubleSequence(value))) => {
            *value = (&info.d[..]).into()
        }
        _ => return Err(fail("CameraInfo.d has an unexpected ROS type")),
    }
    set_f64_array(message.get_mut("k"), &info.k, "CameraInfo.k")?;
    set_f64_array(message.get_mut("r"), &info.r, "CameraInfo.r")?;
    set_f64_array(message.get_mut("p"), &info.p, "CameraInfo.p")?;
    Ok(message)
}

fn set_header(message: &mut DynamicMessage, header: &RosHeader) -> Result<(), Ros2AdapterError> {
    let mut header_view = match message.get_mut("header") {
        Some(ValueMut::Simple(SimpleValueMut::Message(value))) => value,
        _ => return Err(fail("message header has an unexpected ROS type")),
    };
    set_stamp(&mut header_view, header.stamp)?;
    set_string(
        header_view.get_mut("frame_id"),
        &header.frame_id,
        "Header.frame_id",
    )
}

fn set_stamp(view: &mut DynamicMessageViewMut<'_>, stamp: RosTime) -> Result<(), Ros2AdapterError> {
    let mut stamp_view = match view.get_mut("stamp") {
        Some(ValueMut::Simple(SimpleValueMut::Message(value))) => value,
        _ => return Err(fail("Header.stamp has an unexpected ROS type")),
    };
    set_i32(stamp_view.get_mut("sec"), stamp.sec, "Time.sec")?;
    set_u32(stamp_view.get_mut("nanosec"), stamp.nanosec, "Time.nanosec")
}

fn set_i32(value: Option<ValueMut<'_>>, next: i32, field: &str) -> Result<(), Ros2AdapterError> {
    match value {
        Some(ValueMut::Simple(SimpleValueMut::Int32(value))) => {
            *value = next;
            Ok(())
        }
        _ => Err(fail(format!("{field} has an unexpected ROS type"))),
    }
}

fn set_u32(value: Option<ValueMut<'_>>, next: u32, field: &str) -> Result<(), Ros2AdapterError> {
    match value {
        Some(ValueMut::Simple(SimpleValueMut::Uint32(value))) => {
            *value = next;
            Ok(())
        }
        _ => Err(fail(format!("{field} has an unexpected ROS type"))),
    }
}

fn set_u8(value: Option<ValueMut<'_>>, next: u8, field: &str) -> Result<(), Ros2AdapterError> {
    match value {
        Some(ValueMut::Simple(SimpleValueMut::Uint8(value))) => {
            *value = next;
            Ok(())
        }
        _ => Err(fail(format!("{field} has an unexpected ROS type"))),
    }
}

fn set_string(
    value: Option<ValueMut<'_>>,
    next: &str,
    field: &str,
) -> Result<(), Ros2AdapterError> {
    match value {
        Some(ValueMut::Simple(SimpleValueMut::String(value))) => {
            *value = next.into();
            Ok(())
        }
        _ => Err(fail(format!("{field} has an unexpected ROS type"))),
    }
}

fn set_f64_array(
    value: Option<ValueMut<'_>>,
    next: &[f64],
    field: &str,
) -> Result<(), Ros2AdapterError> {
    match value {
        Some(ValueMut::Array(ArrayValueMut::DoubleArray(value))) if value.len() == next.len() => {
            value.copy_from_slice(next);
            Ok(())
        }
        _ => Err(fail(format!("{field} has an unexpected ROS type"))),
    }
}

pub struct Ros2CameraPublisher {
    image: rclrs::DynamicPublisher,
    camera_info: rclrs::DynamicPublisher,
}

impl Ros2CameraPublisher {
    pub fn new(node: &Arc<rclrs::Node>) -> Result<Self, Ros2AdapterError> {
        let mut image_options = PublisherOptions::new("depth/image_rect");
        image_options.qos = QOS_PROFILE_SENSOR_DATA;
        let image = node
            .create_dynamic_publisher(
                "sensor_msgs/msg/Image"
                    .try_into()
                    .map_err(|error| fail(format!("invalid Image type name: {error}")))?,
                image_options,
            )
            .map_err(|error| fail(format!("create depth image publisher: {error}")))?;
        let mut info_options = PublisherOptions::new("depth/camera_info");
        info_options.qos = QOS_PROFILE_SENSOR_DATA;
        let camera_info = node
            .create_dynamic_publisher(
                "sensor_msgs/msg/CameraInfo"
                    .try_into()
                    .map_err(|error| fail(format!("invalid CameraInfo type name: {error}")))?,
                info_options,
            )
            .map_err(|error| fail(format!("create camera info publisher: {error}")))?;
        Ok(Self { image, camera_info })
    }

    pub fn publish(&self, frame: RosDepthCameraFrame) -> Result<(), Ros2AdapterError> {
        let image = image_message(frame.image)?;
        let camera_info = camera_info_message(frame.camera_info)?;
        self.camera_info
            .publish(camera_info)
            .map_err(|error| fail(format!("publish camera info: {error}")))?;
        self.image
            .publish(image)
            .map_err(|error| fail(format!("publish depth image: {error}")))
    }
}
