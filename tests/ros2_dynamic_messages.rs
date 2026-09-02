#![cfg(feature = "ros2")]

use rclrs::{ArrayValue, SequenceValue, SimpleValue, Value};
use revopoint_pop3_wifi::calibration::ScaledDepthIntrinsics;
use revopoint_pop3_wifi::depth_decode::{DepthEncoding, DepthPlane};
use revopoint_pop3_wifi::ros2_adapter::{camera_info_message, image_message};
use revopoint_pop3_wifi::ros_camera::{map_depth_camera, RosTime};

fn mapped_frame() -> revopoint_pop3_wifi::ros_camera::RosDepthCameraFrame {
    map_depth_camera(
        DepthPlane {
            width: 2,
            height: 1,
            stride_bytes: 4,
            encoding: DepthEncoding::Z16LittleEndian,
            millimeters_per_unit: 0.1,
            bytes: [0_u16, 10_000]
                .into_iter()
                .flat_map(u16::to_le_bytes)
                .collect(),
        },
        ScaledDepthIntrinsics {
            width: 2,
            height: 1,
            fx: 8.0,
            fy: 9.0,
            cx: 0.75,
            cy: 0.25,
        },
        RosTime {
            sec: 123,
            nanosec: 456,
        },
        "pop3_depth_optical_frame",
    )
    .expect("mapped frame")
}

#[test]
fn builds_a_runtime_typed_sensor_msgs_image() {
    let message = image_message(mapped_frame().image).expect("dynamic Image");

    assert_eq!(u32_field(&message, "height"), 1);
    assert_eq!(u32_field(&message, "width"), 2);
    assert_eq!(string_field(&message, "encoding"), "32FC1");
    assert_eq!(u8_field(&message, "is_bigendian"), 0);
    assert_eq!(u32_field(&message, "step"), 8);
    let Value::Sequence(SequenceValue::Uint8Sequence(data)) = message.get("data").expect("data")
    else {
        panic!("unexpected Image.data type")
    };
    assert_eq!(data.len(), 8);
    assert_eq!(f32::from_le_bytes(data[0..4].try_into().unwrap()), 0.0);
    assert_eq!(f32::from_le_bytes(data[4..8].try_into().unwrap()), 1.0);
    assert_header(&message);
}

#[test]
fn builds_a_runtime_typed_sensor_msgs_camera_info() {
    let message = camera_info_message(mapped_frame().camera_info).expect("dynamic CameraInfo");

    assert_eq!(u32_field(&message, "height"), 1);
    assert_eq!(u32_field(&message, "width"), 2);
    assert_eq!(string_field(&message, "distortion_model"), "plumb_bob");
    let Value::Sequence(SequenceValue::DoubleSequence(d)) = message.get("d").expect("d") else {
        panic!("unexpected CameraInfo.d type")
    };
    assert_eq!(&d[..], &[0.0; 5]);
    let Value::Array(ArrayValue::DoubleArray(k)) = message.get("k").expect("k") else {
        panic!("unexpected CameraInfo.k type")
    };
    assert_eq!(k, [8.0, 0.0, 0.75, 0.0, 9.0, 0.25, 0.0, 0.0, 1.0]);
    assert_header(&message);
}

fn assert_header(message: &rclrs::DynamicMessage) {
    let Value::Simple(SimpleValue::Message(header)) = message.get("header").expect("header") else {
        panic!("unexpected header type")
    };
    assert_eq!(
        string_view_field(&header, "frame_id"),
        "pop3_depth_optical_frame"
    );
    let Value::Simple(SimpleValue::Message(stamp)) = header.get("stamp").expect("stamp") else {
        panic!("unexpected stamp type")
    };
    let Value::Simple(SimpleValue::Int32(sec)) = stamp.get("sec").expect("sec") else {
        panic!("unexpected sec type")
    };
    let Value::Simple(SimpleValue::Uint32(nanosec)) = stamp.get("nanosec").expect("nanosec") else {
        panic!("unexpected nanosec type")
    };
    assert_eq!(*sec, 123);
    assert_eq!(*nanosec, 456);
}

fn u32_field(message: &rclrs::DynamicMessage, name: &str) -> u32 {
    let Value::Simple(SimpleValue::Uint32(value)) = message.get(name).expect(name) else {
        panic!("unexpected {name} type")
    };
    *value
}

fn u8_field(message: &rclrs::DynamicMessage, name: &str) -> u8 {
    let Value::Simple(SimpleValue::Uint8(value)) = message.get(name).expect(name) else {
        panic!("unexpected {name} type")
    };
    *value
}

fn string_field(message: &rclrs::DynamicMessage, name: &str) -> String {
    let Value::Simple(SimpleValue::String(value)) = message.get(name).expect(name) else {
        panic!("unexpected {name} type")
    };
    value.to_string()
}

fn string_view_field(view: &rclrs::DynamicMessageView<'_>, name: &str) -> String {
    let Value::Simple(SimpleValue::String(value)) = view.get(name).expect(name) else {
        panic!("unexpected {name} type")
    };
    value.to_string()
}
