use crate::calibration::ScaledDepthIntrinsics;
use crate::depth_decode::DepthPlane;
use crate::rgb_calibration::RgbCalibration;
use jpeg_decoder::{Decoder, PixelFormat};
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::io::Cursor;

const RGB_CHANNELS: usize = 3;
const MAXIMUM_DECODED_RGB_BYTES: usize = 33_554_432;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RgbImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColoredPoint {
    pub position_mm: [f32; 3],
    pub rgb: [u8; 3],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RegistrationError;

impl Display for RegistrationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("RGB-D data cannot be registered")
    }
}

impl Error for RegistrationError {}

pub fn decode_jpeg_rgb(bytes: &[u8]) -> Result<RgbImage, RegistrationError> {
    let mut decoder = Decoder::new(Cursor::new(bytes));
    decoder.set_max_decoding_buffer_size(MAXIMUM_DECODED_RGB_BYTES);
    let pixels = decoder.decode().map_err(|_| RegistrationError)?;
    let information = decoder.info().ok_or(RegistrationError)?;
    if information.pixel_format != PixelFormat::RGB24 {
        return Err(RegistrationError);
    }
    let image = RgbImage {
        width: u32::from(information.width),
        height: u32::from(information.height),
        pixels,
    };
    valid_rgb_image(&image)
        .then_some(image)
        .ok_or(RegistrationError)
}

pub fn colorize_depth(
    depth: &DepthPlane,
    depth_intrinsics: ScaledDepthIntrinsics,
    rgb: &RgbImage,
    calibration: RgbCalibration,
) -> Result<Vec<ColoredPoint>, RegistrationError> {
    let pixels = image_pixels(depth.width, depth.height)?;
    if depth.width == 0
        || depth.height == 0
        || depth.stride_bytes != pixels_per_row_bytes(depth.width, 2)?
        || depth.bytes.len() != pixels.checked_mul(2).ok_or(RegistrationError)?
        || !depth.millimeters_per_unit.is_finite()
        || depth.millimeters_per_unit <= 0.0
        || depth_intrinsics.width != depth.width
        || depth_intrinsics.height != depth.height
        || !valid_depth_intrinsics(depth_intrinsics)
        || !valid_rgb_image(rgb)
    {
        return Err(RegistrationError);
    }

    let rgb_scale_x = rgb.width as f32 / f32::from(calibration.intrinsics.calibration_width);
    let rgb_scale_y = rgb.height as f32 / f32::from(calibration.intrinsics.calibration_height);
    let rgb_fx = calibration.intrinsics.fx * rgb_scale_x;
    let rgb_fy = calibration.intrinsics.fy * rgb_scale_y;
    let rgb_cx = calibration.intrinsics.cx * rgb_scale_x;
    let rgb_cy = calibration.intrinsics.cy * rgb_scale_y;
    if ![rgb_fx, rgb_fy, rgb_cx, rgb_cy]
        .into_iter()
        .all(f32::is_finite)
        || rgb_fx <= 0.0
        || rgb_fy <= 0.0
    {
        return Err(RegistrationError);
    }

    let mut points = Vec::with_capacity(pixels);
    for (index, sample) in depth.bytes.as_chunks::<2>().0.iter().enumerate() {
        let raw = u16::from_le_bytes(*sample);
        if raw == 0 {
            continue;
        }
        let z = f32::from(raw) * depth.millimeters_per_unit;
        let column = (index % usize::try_from(depth.width).map_err(|_| RegistrationError)?) as f32;
        let row = (index / usize::try_from(depth.width).map_err(|_| RegistrationError)?) as f32;
        let x = (column - depth_intrinsics.cx) * z / depth_intrinsics.fx;
        let y = (row - depth_intrinsics.cy) * z / depth_intrinsics.fy;
        let Some([rgb_x, rgb_y]) = project_depth_point(
            [x, y, z],
            rgb.width,
            rgb.height,
            [rgb_fx, rgb_fy, rgb_cx, rgb_cy],
            calibration,
        ) else {
            continue;
        };
        let rgb_index = (rgb_y as usize * rgb.width as usize + rgb_x as usize) * RGB_CHANNELS;
        points.push(ColoredPoint {
            position_mm: [x, y, z],
            rgb: rgb.pixels[rgb_index..rgb_index + RGB_CHANNELS]
                .try_into()
                .expect("validated RGB pixel"),
        });
    }
    Ok(points)
}

pub fn project_depth_point(
    position_mm: [f32; 3],
    rgb_width: u32,
    rgb_height: u32,
    rgb_intrinsics: [f32; 4],
    calibration: RgbCalibration,
) -> Option<[u32; 2]> {
    let [rgb_fx, rgb_fy, rgb_cx, rgb_cy] = rgb_intrinsics;
    let transformed = transform_depth_to_rgb(
        position_mm,
        calibration.left_to_rgb.rotation,
        calibration.left_to_rgb.translation_mm,
    );
    if transformed[2] <= 0.0 || !transformed.into_iter().all(f32::is_finite) {
        return None;
    }
    let rgb_x = rgb_fx * transformed[0] / transformed[2] + rgb_cx;
    let rgb_y = rgb_fy * transformed[1] / transformed[2] + rgb_cy;
    if !rgb_x.is_finite()
        || !rgb_y.is_finite()
        || rgb_x < 0.0
        || rgb_y < 0.0
        || rgb_x >= rgb_width as f32
        || rgb_y >= rgb_height as f32
    {
        return None;
    }
    Some([rgb_x as u32, rgb_y as u32])
}

pub fn encode_binary_ply(points: &[ColoredPoint]) -> Vec<u8> {
    let header = format!(
        concat!(
            "ply\n",
            "format binary_little_endian 1.0\n",
            "comment positions are millimeters in the left depth camera frame\n",
            "element vertex {}\n",
            "property float x\nproperty float y\nproperty float z\n",
            "property uchar red\nproperty uchar green\nproperty uchar blue\n",
            "end_header\n"
        ),
        points.len()
    );
    let mut bytes =
        Vec::with_capacity(header.len().saturating_add(points.len().saturating_mul(15)));
    bytes.extend_from_slice(header.as_bytes());
    for point in points {
        for coordinate in point.position_mm {
            bytes.extend_from_slice(&coordinate.to_le_bytes());
        }
        bytes.extend_from_slice(&point.rgb);
    }
    bytes
}

fn image_pixels(width: u32, height: u32) -> Result<usize, RegistrationError> {
    usize::try_from(width)
        .ok()
        .and_then(|width| usize::try_from(height).ok()?.checked_mul(width))
        .ok_or(RegistrationError)
}

fn pixels_per_row_bytes(width: u32, channels: usize) -> Result<usize, RegistrationError> {
    usize::try_from(width)
        .ok()
        .and_then(|width| width.checked_mul(channels))
        .ok_or(RegistrationError)
}

fn valid_rgb_image(image: &RgbImage) -> bool {
    image.width != 0
        && image
            .width
            .checked_mul(image.height)
            .and_then(|pixels| pixels.checked_mul(RGB_CHANNELS as u32))
            .is_some_and(|bytes| usize::try_from(bytes).ok() == Some(image.pixels.len()))
}

fn valid_depth_intrinsics(intrinsics: ScaledDepthIntrinsics) -> bool {
    intrinsics.fx.is_finite()
        && intrinsics.fx > 0.0
        && intrinsics.fy.is_finite()
        && intrinsics.fy > 0.0
        && intrinsics.cx.is_finite()
        && intrinsics.cy.is_finite()
}

fn transform_depth_to_rgb(point: [f32; 3], rotation: [f32; 9], translation: [f32; 3]) -> [f32; 3] {
    let translated = [
        point[0] + translation[0],
        point[1] + translation[1],
        point[2] + translation[2],
    ];
    [
        rotation[0] * translated[0] + rotation[1] * translated[1] + rotation[2] * translated[2],
        rotation[3] * translated[0] + rotation[4] * translated[1] + rotation[5] * translated[2],
        rotation[6] * translated[0] + rotation[7] * translated[1] + rotation[8] * translated[2],
    ]
}
