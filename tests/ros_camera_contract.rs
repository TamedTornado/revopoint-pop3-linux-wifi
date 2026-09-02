use revopoint_pop3_wifi::calibration::ScaledDepthIntrinsics;
use revopoint_pop3_wifi::depth_decode::{DepthEncoding, DepthPlane};
use revopoint_pop3_wifi::ros_camera::{map_depth_camera, RosTime};

fn depth_plane(bytes: Vec<u8>) -> DepthPlane {
    DepthPlane {
        width: 2,
        height: 2,
        stride_bytes: 4,
        encoding: DepthEncoding::Z16LittleEndian,
        millimeters_per_unit: 0.1,
        bytes,
    }
}

fn intrinsics() -> ScaledDepthIntrinsics {
    ScaledDepthIntrinsics {
        width: 2,
        height: 2,
        fx: 8.0,
        fy: 9.0,
        cx: 0.75,
        cy: 1.25,
    }
}

#[test]
fn maps_z16_units_to_standard_32fc1_meters() {
    let raw = [0_u16, 1, 10_000, u16::MAX]
        .into_iter()
        .flat_map(u16::to_le_bytes)
        .collect();
    let frame = map_depth_camera(
        depth_plane(raw),
        intrinsics(),
        RosTime {
            sec: 123,
            nanosec: 456,
        },
        "pop3_depth_optical_frame",
    )
    .expect("ROS camera frame");

    assert_eq!(frame.image.width, 2);
    assert_eq!(frame.image.height, 2);
    assert_eq!(frame.image.encoding, "32FC1");
    assert_eq!(frame.image.is_bigendian, 0);
    assert_eq!(frame.image.step, 8);
    let values = frame
        .image
        .data
        .as_chunks::<4>()
        .0
        .iter()
        .map(|bytes| f32::from_le_bytes(*bytes))
        .collect::<Vec<_>>();
    assert_eq!(values[0], 0.0);
    assert!((values[1] - 0.0001).abs() < 1.0e-10);
    assert_eq!(values[2], 1.0);
    assert!((values[3] - 6.5535).abs() < 1.0e-6);
}

#[test]
fn maps_rectified_pinhole_camera_info_with_a_shared_header() {
    let frame = map_depth_camera(
        depth_plane(vec![0; 8]),
        intrinsics(),
        RosTime {
            sec: 123,
            nanosec: 456,
        },
        "pop3_depth_optical_frame",
    )
    .expect("ROS camera frame");

    assert_eq!(frame.image.header, frame.camera_info.header);
    assert_eq!(frame.camera_info.width, 2);
    assert_eq!(frame.camera_info.height, 2);
    assert_eq!(frame.camera_info.distortion_model, "plumb_bob");
    assert_eq!(frame.camera_info.d, [0.0; 5]);
    assert_eq!(
        frame.camera_info.k,
        [8.0, 0.0, 0.75, 0.0, 9.0, 1.25, 0.0, 0.0, 1.0]
    );
    assert_eq!(
        frame.camera_info.r,
        [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]
    );
    assert_eq!(
        frame.camera_info.p,
        [8.0, 0.0, 0.75, 0.0, 0.0, 9.0, 1.25, 0.0, 0.0, 0.0, 1.0, 0.0,]
    );
}

#[test]
fn rejects_inconsistent_metadata_before_building_messages() {
    let wrong_dimensions = ScaledDepthIntrinsics {
        width: 3,
        ..intrinsics()
    };
    assert!(map_depth_camera(
        depth_plane(vec![0; 8]),
        wrong_dimensions,
        RosTime { sec: 0, nanosec: 0 },
        "pop3_depth_optical_frame",
    )
    .is_err());
    let wrong_height = ScaledDepthIntrinsics {
        height: 3,
        ..intrinsics()
    };
    assert!(map_depth_camera(
        depth_plane(vec![0; 8]),
        wrong_height,
        RosTime { sec: 0, nanosec: 0 },
        "pop3_depth_optical_frame",
    )
    .is_err());
    assert!(map_depth_camera(
        depth_plane(vec![0; 6]),
        intrinsics(),
        RosTime { sec: 0, nanosec: 0 },
        "pop3_depth_optical_frame",
    )
    .is_err());
    assert!(map_depth_camera(
        depth_plane(vec![0; 8]),
        intrinsics(),
        RosTime {
            sec: 0,
            nanosec: 1_000_000_000
        },
        "pop3_depth_optical_frame",
    )
    .is_err());
    assert!(map_depth_camera(
        depth_plane(vec![0; 8]),
        intrinsics(),
        RosTime { sec: 0, nanosec: 0 },
        "",
    )
    .is_err());
    assert_eq!(
        map_depth_camera(
            depth_plane(vec![0; 8]),
            intrinsics(),
            RosTime { sec: 0, nanosec: 0 },
            "bad frame",
        )
        .expect_err("whitespace frame ID must fail")
        .to_string(),
        "depth plane cannot be represented as a ROS camera frame"
    );

    let invalid_planes = [
        DepthPlane {
            width: 0,
            height: 2,
            stride_bytes: 0,
            encoding: DepthEncoding::Z16LittleEndian,
            millimeters_per_unit: 0.1,
            bytes: Vec::new(),
        },
        DepthPlane {
            width: 2,
            height: 0,
            stride_bytes: 4,
            encoding: DepthEncoding::Z16LittleEndian,
            millimeters_per_unit: 0.1,
            bytes: Vec::new(),
        },
        DepthPlane {
            stride_bytes: 5,
            ..depth_plane(vec![0; 8])
        },
        DepthPlane {
            millimeters_per_unit: f32::NAN,
            ..depth_plane(vec![0; 8])
        },
        DepthPlane {
            millimeters_per_unit: 0.0,
            ..depth_plane(vec![0; 8])
        },
    ];
    for plane in invalid_planes {
        let matching_intrinsics = ScaledDepthIntrinsics {
            width: plane.width,
            height: plane.height,
            ..intrinsics()
        };
        assert!(map_depth_camera(
            plane,
            matching_intrinsics,
            RosTime { sec: 0, nanosec: 0 },
            "pop3_depth_optical_frame",
        )
        .is_err());
    }

    for invalid_intrinsics in [
        ScaledDepthIntrinsics {
            fx: f32::NAN,
            ..intrinsics()
        },
        ScaledDepthIntrinsics {
            fx: 0.0,
            ..intrinsics()
        },
        ScaledDepthIntrinsics {
            fy: f32::NAN,
            ..intrinsics()
        },
        ScaledDepthIntrinsics {
            fy: 0.0,
            ..intrinsics()
        },
        ScaledDepthIntrinsics {
            cx: f32::NAN,
            ..intrinsics()
        },
        ScaledDepthIntrinsics {
            cy: f32::NAN,
            ..intrinsics()
        },
    ] {
        assert!(map_depth_camera(
            depth_plane(vec![0; 8]),
            invalid_intrinsics,
            RosTime { sec: 0, nanosec: 0 },
            "pop3_depth_optical_frame",
        )
        .is_err());
    }
}
