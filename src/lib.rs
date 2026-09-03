use rusb::{DeviceHandle, GlobalContext};
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::io::{self, Write};
use std::net::{IpAddr, SocketAddr};
use std::thread;
use std::time::Duration;

pub mod calibration;
pub mod depth_decode;
pub mod depth_stream;
pub mod frame_envelope;
pub mod http_stream;
pub mod pair_decode;
pub mod rgb_calibration;
pub mod rgb_decode;
pub mod rgb_registration;
pub mod rgb_stream;
pub mod rgbd_pair;
pub mod rgbd_stream;
#[cfg(feature = "ros2")]
pub mod ros2_adapter;
pub mod ros_camera;
pub mod stereo_calibration;
pub mod stereo_depth;
pub mod stereo_match;

const VID: u16 = 0x2207;
const PID: u16 = 0x110c;
const INTERFACE: u8 = 0;
const TIMEOUT: Duration = Duration::from_secs(5);
const START_FINISH: u16 = 0x0100;
const DATA: u16 = 0x0200;
const EXECUTE: u16 = 0x0700;
const EXTENSION_UNIT_AND_INTERFACE: u16 = 0x0400;
const REPORT_SIZE: usize = 60;
const CONFIG_PATH: &str = "/data/wpa_supplicant.config";
const AP_CONFIG_PATH: &str = "/data/hostapd.conf";

#[derive(Debug)]
struct MessageError(String);

impl Display for MessageError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for MessageError {}

fn failure(message: impl Into<String>) -> Box<dyn Error> {
    Box::new(MessageError(message.into()))
}

struct Scanner {
    handle: DeviceHandle<GlobalContext>,
    detached_kernel_driver: bool,
    claimed_interface: bool,
}

impl Scanner {
    fn open() -> Result<Self, Box<dyn Error>> {
        let handle = rusb::open_device_with_vid_pid(VID, PID)
            .ok_or_else(|| failure("POP 3 not found or inaccessible"))?;

        let detached_kernel_driver = if handle.kernel_driver_active(INTERFACE)? {
            handle.detach_kernel_driver(INTERFACE)?;
            true
        } else {
            false
        };

        if let Err(error) = handle.claim_interface(INTERFACE) {
            if detached_kernel_driver {
                let _ = handle.attach_kernel_driver(INTERFACE);
            }
            return Err(Box::new(error));
        }

        Ok(Self {
            handle,
            detached_kernel_driver,
            claimed_interface: true,
        })
    }

    fn read_file(&self, path: &str) -> Result<Vec<u8>, Box<dyn Error>> {
        let mut report = path_report(1, path)?;
        self.write_report(START_FINISH, &report)?;

        let result = (|| {
            let mut contents = Vec::with_capacity(1024);
            for _ in 0..128 {
                report.fill(0);
                let received = self.handle.read_control(
                    0xa1,
                    0x81,
                    DATA,
                    EXTENSION_UNIT_AND_INTERFACE,
                    &mut report,
                    TIMEOUT,
                )?;
                if received < 4 {
                    return Err(failure("device returned a short read block"));
                }

                let count = u32::from_le_bytes(report[..4].try_into().unwrap()) as usize;
                if count == 0 {
                    return Ok(contents);
                }
                if count > 56 || count + 4 > received {
                    return Err(failure("device returned an invalid block length"));
                }
                contents.extend_from_slice(&report[4..4 + count]);
            }
            Err(failure("device file exceeded the read limit"))
        })();

        report.fill(0);
        let finish_result = self.write_report(START_FINISH, &report);
        match (result, finish_result) {
            (Ok(contents), Ok(())) => Ok(contents),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
        }
    }

    fn write_file(&self, path: &str, contents: &[u8]) -> Result<(), Box<dyn Error>> {
        let mut report = path_report(2, path)?;
        self.write_report(START_FINISH, &report)?;

        let write_result = (|| {
            for chunk in contents.chunks(56) {
                report.fill(0);
                report[..4].copy_from_slice(&(chunk.len() as u32).to_le_bytes());
                report[4..4 + chunk.len()].copy_from_slice(chunk);
                self.write_report(DATA, &report)?;
            }
            Ok(())
        })();

        report.fill(0);
        let finish_result = self.write_report(START_FINISH, &report);
        match (write_result, finish_result) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error),
        }
    }

    fn sync(&self) -> Result<(), Box<dyn Error>> {
        self.execute("sync")
    }

    fn execute(&self, command: &str) -> Result<(), Box<dyn Error>> {
        if command.len() + 1 > REPORT_SIZE {
            return Err(failure("scanner command is too long"));
        }
        let mut report = [0_u8; REPORT_SIZE];
        report[..command.len()].copy_from_slice(command.as_bytes());
        self.write_report(EXECUTE, &report)
    }

    fn write_report(&self, value: u16, report: &[u8; REPORT_SIZE]) -> Result<(), Box<dyn Error>> {
        let written = self.handle.write_control(
            0x21,
            0x01,
            value,
            EXTENSION_UNIT_AND_INTERFACE,
            report,
            TIMEOUT,
        )?;
        if written != report.len() {
            return Err(failure("USB control write was truncated"));
        }
        Ok(())
    }
}

impl Drop for Scanner {
    fn drop(&mut self) {
        if self.claimed_interface {
            let _ = self.handle.release_interface(INTERFACE);
        }
        if self.detached_kernel_driver {
            let _ = self.handle.attach_kernel_driver(INTERFACE);
        }
    }
}

fn path_report(operation: u8, path: &str) -> Result<[u8; REPORT_SIZE], Box<dyn Error>> {
    let path = path.as_bytes();
    if path.len() + 2 > REPORT_SIZE {
        return Err(failure("device path is too long"));
    }
    let mut report = [0_u8; REPORT_SIZE];
    report[0] = operation;
    report[1..1 + path.len()].copy_from_slice(path);
    Ok(report)
}

fn config_value<'a>(config: &'a str, key: &str) -> Option<&'a str> {
    config.lines().find_map(|line| {
        let (found_key, value) = line.split_once('=')?;
        if found_key.trim() != key {
            return None;
        }
        Some(value.trim().trim_matches('"'))
    })
}

fn validate_credential(name: &str, value: &str) -> Result<(), Box<dyn Error>> {
    if value.chars().any(char::is_control) {
        return Err(failure(format!("{name} contains a control character")));
    }
    Ok(())
}

fn escape_wpa(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn make_config(ssid: &str, password: &str) -> String {
    format!(
        concat!(
            "#pop2_enable=enable\n",
            "ctrl_interface=/var/run/wpa_supplicant\n",
            "ap_scan=1\n",
            "update_config=1\n\n",
            "network={{\n",
            "ssid=\"{}\"\n",
            "psk=\"{}\"\n",
            "key_mgmt=WPA-PSK\n",
            "pairwise=CCMP TKIP\n",
            "group=CCMP TKIP\n",
            "proto=WPA2\n",
            "}}\n",
        ),
        escape_wpa(ssid),
        escape_wpa(password)
    )
}

fn disable_access_point(config: &str) -> Result<String, Box<dyn Error>> {
    const ENABLED: &str = "#pop2_enable=enable";
    const DISABLED: &str = "#pop2_enable=disable";

    if config.lines().any(|line| line == DISABLED) {
        return Ok(config.to_owned());
    }
    if !config.lines().any(|line| line == ENABLED) {
        return Err(failure(
            "access-point configuration has no recognized mode marker",
        ));
    }
    Ok(config.replacen(ENABLED, DISABLED, 1))
}

fn prompt(prompt: &str) -> Result<String, Box<dyn Error>> {
    print!("{prompt}");
    io::stdout().flush()?;
    let mut value = String::new();
    io::stdin().read_line(&mut value)?;
    Ok(value.trim_end_matches(['\r', '\n']).to_owned())
}

fn run(write: bool, diagnose: bool) -> Result<(), Box<dyn Error>> {
    let scanner = Scanner::open()?;
    if diagnose {
        scanner.execute("killall wpa_supplicant 2>/dev/null; true")?;
        scanner.execute("rm -f /var/run/wpa_supplicant/wlan0")?;
        scanner.execute("ln -sf /data/wpa_supplicant.config /tmp/w")?;
        scanner.execute("wpa_supplicant -B -iwlan0 -c/tmp/w >/data/ws 2>&1")?;
        thread::sleep(Duration::from_secs(8));
        scanner.execute("udhcpc -i wlan0 -n -q >/data/dhcp 2>&1 &")?;
        thread::sleep(Duration::from_secs(5));
        scanner.execute("wpa_cli status >/data/wifi-status.txt 2>&1")?;
        scanner.execute("ip addr show wlan0 >/data/wifi-ip.txt 2>&1")?;
        scanner.execute("ps | grep '[w]pa' >/data/wifi-process.txt 2>&1")?;
        for (label, path) in [
            ("wpa_supplicant status", "/data/wifi-status.txt"),
            ("wpa_supplicant startup", "/data/ws"),
            ("DHCP client", "/data/dhcp"),
            ("wlan0 address", "/data/wifi-ip.txt"),
            ("Wi-Fi processes", "/data/wifi-process.txt"),
        ] {
            let output = scanner.read_file(path)?;
            println!("--- {label} ---");
            print!("{}", String::from_utf8_lossy(&output));
            if !output.ends_with(b"\n") {
                println!();
            }
        }
        return Ok(());
    }
    let current_bytes = scanner.read_file(CONFIG_PATH)?;
    let current = std::str::from_utf8(&current_bytes)?;
    let access_point_bytes = scanner.read_file(AP_CONFIG_PATH)?;
    let access_point = std::str::from_utf8(&access_point_bytes)?;
    let current_ssid = config_value(current, "ssid").unwrap_or("");
    let password_is_set = config_value(current, "psk").is_some_and(|value| !value.is_empty());
    let access_point_is_enabled = access_point
        .lines()
        .any(|line| line == "#pop2_enable=enable");
    println!("Current client SSID: \"{current_ssid}\"");
    println!(
        "Current password configured: {}",
        if password_is_set { "yes" } else { "no" }
    );
    println!(
        "Scanner access point enabled: {}",
        if access_point_is_enabled { "yes" } else { "no" }
    );

    if !write {
        return Ok(());
    }

    let ssid = prompt("New Wi-Fi SSID: ")?;
    let password = rpassword::prompt_password("New Wi-Fi password (input hidden): ")?;
    validate_credential("SSID", &ssid)?;
    validate_credential("password", &password)?;
    if ssid.is_empty() || ssid.len() > 32 {
        return Err(failure("SSID must be 1-32 bytes"));
    }
    if !(8..=63).contains(&password.len()) {
        return Err(failure("WPA2 password must be 8-63 bytes"));
    }

    let confirmation = prompt(&format!(
        "Will provision SSID \"{ssid}\". Type WRITE to continue: "
    ))?;
    if confirmation != "WRITE" {
        println!("Cancelled; scanner unchanged.");
        return Ok(());
    }

    let next = make_config(&ssid, &password);
    let next_access_point = disable_access_point(access_point)?;
    scanner.write_file(CONFIG_PATH, next.as_bytes())?;
    let verified = scanner.read_file(CONFIG_PATH)?;
    if verified != next.as_bytes() {
        return Err(failure(
            "read-back verification failed; do not power-cycle the scanner",
        ));
    }
    scanner.write_file(AP_CONFIG_PATH, next_access_point.as_bytes())?;
    let verified_access_point = scanner.read_file(AP_CONFIG_PATH)?;
    if verified_access_point != next_access_point.as_bytes() {
        return Err(failure(
            "access-point read-back verification failed; do not power-cycle the scanner",
        ));
    }
    scanner.sync()?;
    println!(
        "Client credentials and AP-disable configuration verified and synced. Disconnect USB \
         data, power-cycle from a power adapter or power bank, then verify that the scanner \
         joins the LAN."
    );
    Ok(())
}

const PAIR_PREFIX_BYTES: usize = 1024 * 1024;
const DEPTH_PREFIX_BYTES: usize = 2 * 1024 * 1024;
const RGB_PREFIX_BYTES: usize = 1024 * 1024;
const MAXIMUM_DISPARITY: u16 = 160;
const MINIMUM_MATCH_MARGIN_PERCENT: u16 = 1;

fn network_limits() -> http_stream::StreamLimits {
    http_stream::StreamLimits {
        connect_timeout: Duration::from_secs(3),
        idle_timeout: Duration::from_secs(3),
        total_timeout: Duration::from_secs(15),
        max_header_bytes: 16 * 1024,
        max_body_bytes: 2 * 1024 * 1024,
    }
}

fn capture_pair_frame(
    address: SocketAddr,
    limits: http_stream::StreamLimits,
) -> Result<(usize, pair_decode::Y8Pair), Box<dyn Error>> {
    let mut parser = frame_envelope::FrameEnvelopeParser::new(2 * 1024 * 1024, 4);
    let mut first_pair = None;
    let mut frame_error: Option<Box<dyn Error>> = None;
    let received =
        depth_stream::capture_pair_prefix(address, limits, PAIR_PREFIX_BYTES, |chunk| {
            if first_pair.is_some() || frame_error.is_some() {
                return;
            }
            match parser.push(chunk) {
                Ok(frames) => {
                    if let Some(frame) = frames.into_iter().next() {
                        match depth_decode::decode_quicklz(&frame, 2 * 1024 * 1024)
                            .map_err(|error| Box::new(error) as Box<dyn Error>)
                            .and_then(|decoded| {
                                pair_decode::decode_wire_y8_pair(decoded.bytes, 640, 400)
                                    .map_err(|error| Box::new(error) as Box<dyn Error>)
                            }) {
                            Ok(pair) => first_pair = Some(pair),
                            Err(error) => frame_error = Some(error),
                        }
                    }
                }
                Err(error) => frame_error = Some(Box::new(error)),
            }
        })?;
    if let Some(error) = frame_error {
        return Err(error);
    }
    let pair =
        first_pair.ok_or_else(|| failure("bounded capture contained no complete PAIR frame"))?;
    Ok((received, pair))
}

fn capture_depth_frame(
    address: SocketAddr,
    limits: http_stream::StreamLimits,
) -> Result<(usize, depth_decode::Z16Y8Y8Frame), Box<dyn Error>> {
    let scale = depth_stream::get_depth_scale_mm(address, limits)?;
    let mut parser = frame_envelope::FrameEnvelopeParser::new(2 * 1024 * 1024, 4);
    let mut first_frame = None;
    let mut frame_error: Option<Box<dyn Error>> = None;
    let received =
        depth_stream::capture_depth_prefix(address, limits, DEPTH_PREFIX_BYTES, |chunk| {
            if first_frame.is_some() || frame_error.is_some() {
                return;
            }
            match parser.push(chunk) {
                Ok(frames) => {
                    if let Some(frame) = frames.into_iter().next() {
                        match depth_decode::decode_quicklz(&frame, 640 * 400 * 4)
                            .map_err(|error| Box::new(error) as Box<dyn Error>)
                            .and_then(|decoded| {
                                decoded
                                    .into_z16y8y8(640, 400, scale)
                                    .map_err(|error| Box::new(error) as Box<dyn Error>)
                            }) {
                            Ok(frame) => first_frame = Some(frame),
                            Err(error) => frame_error = Some(error),
                        }
                    }
                }
                Err(error) => frame_error = Some(Box::new(error)),
            }
        })?;
    if let Some(error) = frame_error {
        return Err(error);
    }
    let frame = first_frame
        .ok_or_else(|| failure("bounded capture contained no complete Z16Y8Y8 frame"))?;
    Ok((received, frame))
}

fn capture_rgb_frame(
    address: SocketAddr,
    limits: http_stream::StreamLimits,
) -> Result<(usize, Vec<u8>, rgb_decode::JpegInformation), Box<dyn Error>> {
    let mut parser = frame_envelope::FrameEnvelopeParser::new(2 * 1024 * 1024, 4);
    let mut first_frame = None;
    let mut frame_error: Option<Box<dyn Error>> = None;
    let received = rgb_stream::capture_rgb_prefix(address, limits, RGB_PREFIX_BYTES, |chunk| {
        if first_frame.is_some() || frame_error.is_some() {
            return;
        }
        match parser.push(chunk) {
            Ok(frames) => {
                if let Some(frame) = frames.into_iter().next() {
                    match rgb_decode::inspect_jpeg(&frame.payload) {
                        Ok(information) => first_frame = Some((frame.payload, information)),
                        Err(error) => frame_error = Some(Box::new(error)),
                    }
                }
            }
            Err(error) => frame_error = Some(Box::new(error)),
        }
    })?;
    if let Some(error) = frame_error {
        return Err(error);
    }
    let (jpeg, information) = first_frame
        .ok_or_else(|| failure("bounded capture contained no complete RGB JPEG frame"))?;
    Ok((received, jpeg, information))
}

struct ReconstructedPair {
    left_rectified: Vec<u8>,
    right_rectified: Vec<u8>,
    disparity: stereo_match::DisparityMap,
    depth: depth_decode::DepthPlane,
    global_disparity: u16,
    global_depth_mm: f32,
}

fn reconstruct_pair(
    pair: &pair_decode::Y8Pair,
    maps: stereo_calibration::StereoMapParameters,
    reprojection: stereo_calibration::ReprojectionMatrix,
) -> Result<ReconstructedPair, Box<dyn Error>> {
    let left_rectified =
        stereo_calibration::rectify_y8(&pair.left, pair.width, pair.height, maps.left)?;
    let right_rectified =
        stereo_calibration::rectify_y8(&pair.right, pair.width, pair.height, maps.right)?;
    let global_disparity = stereo_match::global_sad_disparity_y8(
        &left_rectified,
        &right_rectified,
        pair.width,
        pair.height,
        stereo_match::GlobalMatchParameters {
            disparities: 0..=MAXIMUM_DISPARITY,
            border: 15,
        },
    )?;
    let disparity_scale = maps.left.calibration_width as f32 / pair.width as f32;
    let global_depth_mm = reprojection
        .depth_mm(f32::from(global_disparity), disparity_scale)
        .ok_or_else(|| failure("global disparity cannot be reprojected to positive depth"))?;
    let disparity = stereo_match::block_match_y8_consistent(
        &left_rectified,
        &right_rectified,
        pair.width,
        pair.height,
        stereo_match::ConsistentMatchParameters {
            disparities: 0..=MAXIMUM_DISPARITY,
            radius: 7,
            minimum_margin_percent: MINIMUM_MATCH_MARGIN_PERCENT,
            consistency_tolerance: 1,
        },
    )?;
    let disparity = stereo_match::filter_disparity_consensus(&disparity, 2, 8, 3)?;
    let depth = stereo_depth::reproject_z16(
        &disparity,
        reprojection,
        maps.left.calibration_width,
        maps.left.calibration_height,
    )?;
    Ok(ReconstructedPair {
        left_rectified,
        right_rectified,
        disparity,
        depth,
        global_disparity,
        global_depth_mm,
    })
}

fn smoke_pair(ip: &str, output_prefix: &str) -> Result<(), Box<dyn Error>> {
    let address = SocketAddr::new(ip.parse::<IpAddr>()?, 80);
    let limits = network_limits();
    let (received, pair) = capture_pair_frame(address, limits)?;
    let maps = stereo_calibration::get_stereo_map_parameters(address, limits)?;
    let reprojection = stereo_calibration::get_reprojection_matrix(address, limits)?;
    let reconstructed = reconstruct_pair(&pair, maps, reprojection)?;
    let disparity = &reconstructed.disparity;
    let valid_pixels = disparity.valid_count();
    let valid_percent = valid_pixels as f64 * 100.0 / disparity.values.len() as f64;
    let mut valid_disparities = disparity
        .values
        .iter()
        .copied()
        .filter(|value| *value != u16::MAX)
        .collect::<Vec<_>>();
    valid_disparities.sort_unstable();
    let median_disparity = valid_disparities
        .get(valid_disparities.len() / 2)
        .copied()
        .ok_or_else(|| failure("block matcher produced no valid disparities"))?;
    let depth_statistics = stereo_depth::depth_z_statistics(&reconstructed.depth)?;
    let left_path = format!("{output_prefix}-left.pgm");
    let right_path = format!("{output_prefix}-right.pgm");
    let left_rectified_path = format!("{output_prefix}-left-rectified.pgm");
    let right_rectified_path = format!("{output_prefix}-right-rectified.pgm");
    let disparity_path = format!("{output_prefix}-disparity.pgm");
    let depth_path = format!("{output_prefix}-depth-mm.pgm");
    std::fs::write(
        &left_path,
        pair_decode::encode_y8_pgm(pair.width, pair.height, &pair.left)?,
    )?;
    std::fs::write(
        &right_path,
        pair_decode::encode_y8_pgm(pair.width, pair.height, &pair.right)?,
    )?;
    std::fs::write(
        &left_rectified_path,
        pair_decode::encode_y8_pgm(pair.width, pair.height, &reconstructed.left_rectified)?,
    )?;
    std::fs::write(
        &right_rectified_path,
        pair_decode::encode_y8_pgm(pair.width, pair.height, &reconstructed.right_rectified)?,
    )?;
    std::fs::write(
        &disparity_path,
        stereo_match::encode_disparity_pgm(disparity, MAXIMUM_DISPARITY)?,
    )?;
    std::fs::write(
        &depth_path,
        stereo_depth::encode_z16_pgm(&reconstructed.depth)?,
    )?;
    println!(
        "PAIR stream smoke passed: bytes={received}, resolution={}x{}, left={left_path}, right={right_path}, left_rectified={left_rectified_path}, right_rectified={right_rectified_path}, disparity={disparity_path}, depth_mm={depth_path}, valid_pixels={valid_pixels} ({valid_percent:.1}%), global_disparity_px={}, global_depth_mm={:.1}, median_disparity_px={median_disparity}, experimental_median_depth_mm={:.1}, depth_mad_mm={:.1}, depth_p10_p90_mm={:.1}..{:.1}",
        pair.width,
        pair.height,
        reconstructed.global_disparity,
        reconstructed.global_depth_mm,
        depth_statistics.median_mm,
        depth_statistics.median_absolute_deviation_mm,
        depth_statistics.p10_mm,
        depth_statistics.p90_mm
    );
    Ok(())
}

fn smoke_depth(ip: &str, output_prefix: &str) -> Result<(), Box<dyn Error>> {
    let address = SocketAddr::new(ip.parse::<IpAddr>()?, 80);
    let (received, frame) = capture_depth_frame(address, network_limits())?;
    let statistics = frame.depth.statistics();
    let robust_statistics = stereo_depth::depth_z_statistics(&frame.depth)?;
    let valid_percent = statistics.nonzero_samples as f64 * 100.0 / statistics.samples as f64;
    let depth_path = format!("{output_prefix}-depth-mm.pgm");
    let left_path = format!("{output_prefix}-left.pgm");
    let right_path = format!("{output_prefix}-right.pgm");
    std::fs::write(&depth_path, stereo_depth::encode_z16_pgm(&frame.depth)?)?;
    std::fs::write(
        &left_path,
        pair_decode::encode_y8_pgm(frame.depth.width, frame.depth.height, &frame.left)?,
    )?;
    std::fs::write(
        &right_path,
        pair_decode::encode_y8_pgm(frame.depth.width, frame.depth.height, &frame.right)?,
    )?;
    println!(
        "Z16Y8Y8 stream smoke passed: bytes={received}, resolution={}x{}, timestamp_ms={}, scale_mm={}, depth={depth_path}, left={left_path}, right={right_path}, valid_pixels={} ({valid_percent:.1}%), min_raw={:?}, max_raw={}, mean_depth_mm={:?}, median_depth_mm={:.1}, depth_mad_mm={:.1}, depth_p10_p90_mm={:.1}..{:.1}",
        frame.depth.width,
        frame.depth.height,
        frame.device_timestamp_ms,
        frame.depth.millimeters_per_unit,
        statistics.nonzero_samples,
        statistics.minimum_nonzero_raw,
        statistics.maximum_raw,
        statistics.mean_nonzero_mm,
        robust_statistics.median_mm,
        robust_statistics.median_absolute_deviation_mm,
        robust_statistics.p10_mm,
        robust_statistics.p90_mm,
    );
    Ok(())
}

fn smoke_rgb(ip: &str, output_prefix: &str) -> Result<(), Box<dyn Error>> {
    let address = SocketAddr::new(ip.parse::<IpAddr>()?, 80);
    let (received, jpeg, information) = capture_rgb_frame(address, network_limits())?;
    if information.width != 1280 || information.height != 800 {
        return Err(failure(
            "RGB JPEG dimensions disagree with the selected profile",
        ));
    }
    let path = format!("{output_prefix}-rgb.jpg");
    std::fs::write(&path, &jpeg[..information.encoded_len])?;
    println!(
        "RGB stream smoke passed: bytes={received}, resolution={}x{}, image={path}",
        information.width, information.height
    );
    Ok(())
}

fn smoke_rgbd(ip: &str, output_prefix: &str) -> Result<(), Box<dyn Error>> {
    const CANDIDATE_FRAMES: usize = 8;

    let address = SocketAddr::new(ip.parse::<IpAddr>()?, 80);
    let mut limits = network_limits();
    limits.max_body_bytes = 8 * 1024 * 1024;
    let mut depth_parser = frame_envelope::FrameEnvelopeParser::new(2 * 1024 * 1024, 4);
    let mut rgb_parser = frame_envelope::FrameEnvelopeParser::new(2 * 1024 * 1024, 4);
    let mut depth_envelopes = Vec::with_capacity(CANDIDATE_FRAMES);
    let mut rgb_frames = Vec::with_capacity(CANDIDATE_FRAMES);
    let mut depth_error = None;
    let mut rgb_error = None;
    let (depth_received, rgb_received) = rgbd_stream::capture_rgbd_until(
        address,
        limits,
        |chunk| {
            if depth_envelopes.len() < CANDIDATE_FRAMES && depth_error.is_none() {
                match depth_parser.push(chunk) {
                    Ok(frames) => depth_envelopes.extend(
                        frames
                            .into_iter()
                            .take(CANDIDATE_FRAMES - depth_envelopes.len()),
                    ),
                    Err(error) => depth_error = Some(error.to_string()),
                }
            }
            depth_envelopes.len() == CANDIDATE_FRAMES || depth_error.is_some()
        },
        |chunk| {
            if rgb_frames.len() < CANDIDATE_FRAMES && rgb_error.is_none() {
                match rgb_parser.push(chunk) {
                    Ok(frames) => {
                        for frame in frames.into_iter().take(CANDIDATE_FRAMES - rgb_frames.len()) {
                            match rgb_decode::inspect_jpeg(&frame.payload) {
                                Ok(information) => rgb_frames.push((frame.payload, information)),
                                Err(error) => {
                                    rgb_error = Some(error.to_string());
                                    break;
                                }
                            }
                        }
                    }
                    Err(error) => rgb_error = Some(error.to_string()),
                }
            }
            rgb_frames.len() == CANDIDATE_FRAMES || rgb_error.is_some()
        },
    )?;
    if let Some(error) = depth_error {
        return Err(failure(format!("depth frame envelope: {error}")));
    }
    if let Some(error) = rgb_error {
        return Err(failure(format!("RGB frame: {error}")));
    }
    let scale = depth_stream::get_depth_scale_mm(address, limits)?;
    let depth_intrinsics =
        calibration::get_depth_intrinsics(address, limits)?.for_resolution(640, 400)?;
    let calibration = rgb_calibration::get_rgb_calibration(address, limits)?;
    let mut depth_frames = depth_envelopes
        .iter()
        .map(|envelope| {
            depth_decode::decode_quicklz(envelope, 640 * 400 * 4)?.into_z16y8y8(640, 400, scale)
        })
        .collect::<Result<Vec<_>, _>>()?;
    if depth_frames.is_empty() || rgb_frames.is_empty() {
        return Err(failure(
            "bounded concurrent capture did not contain both depth and RGB candidates",
        ));
    }
    if rgb_frames
        .iter()
        .any(|(_, rgb)| rgb.width != 1280 || rgb.height != 800 || rgb.device_timestamp_ms.is_none())
    {
        return Err(failure(
            "concurrent RGB frame disagrees with the selected timestamped profile",
        ));
    }
    let depth_timestamps = depth_frames
        .iter()
        .map(|frame| frame.device_timestamp_ms)
        .collect::<Vec<_>>();
    let rgb_timestamps = rgb_frames
        .iter()
        .map(|(_, frame)| frame.device_timestamp_ms.expect("validated timestamp"))
        .collect::<Vec<_>>();
    let selected = rgbd_pair::select_closest_pair(
        &depth_timestamps,
        &rgb_timestamps,
        rgbd_pair::PairingPolicy::default(),
    )
    .ok_or_else(|| failure("captured RGB-D candidates contain no bounded timestamp pair"))?;
    let depth = depth_frames.swap_remove(selected.depth_index);
    let (rgb_payload, rgb) = rgb_frames.swap_remove(selected.rgb_index);
    let rgb_timestamp_ms = rgb.device_timestamp_ms.expect("validated timestamp");

    let depth_path = format!("{output_prefix}-depth-mm.pgm");
    let rgb_path = format!("{output_prefix}-rgb.jpg");
    let colored_path = format!("{output_prefix}-colored.ply");
    let rgb_image = rgb_registration::decode_jpeg_rgb(&rgb_payload[..rgb.encoded_len])?;
    let colored_points =
        rgb_registration::colorize_depth(&depth.depth, depth_intrinsics, &rgb_image, calibration)?;
    if colored_points.is_empty() {
        return Err(failure(
            "paired RGB and depth frames have no spatially overlapping valid points",
        ));
    }
    std::fs::write(&depth_path, stereo_depth::encode_z16_pgm(&depth.depth)?)?;
    std::fs::write(&rgb_path, &rgb_payload[..rgb.encoded_len])?;
    std::fs::write(
        &colored_path,
        rgb_registration::encode_binary_ply(&colored_points),
    )?;
    println!(
        "Concurrent RGB-D smoke passed: depth_bytes={depth_received}, rgb_bytes={rgb_received}, depth_resolution={}x{}, rgb_resolution={}x{}, depth_timestamp_ms={}, rgb_timestamp_ms={}, timestamp_delta_ms={}, rgb_calibration={}x{}, colored_points={}, depth={depth_path}, rgb={rgb_path}, colored={colored_path}",
        depth.depth.width,
        depth.depth.height,
        rgb.width,
        rgb.height,
        depth.device_timestamp_ms,
        rgb_timestamp_ms,
        selected.timestamps.absolute_delta_ms,
        calibration.intrinsics.calibration_width,
        calibration.intrinsics.calibration_height,
        colored_points.len(),
    );
    Ok(())
}

fn inspect_rgb_calibration(ip: &str) -> Result<(), Box<dyn Error>> {
    let address = SocketAddr::new(ip.parse::<IpAddr>()?, 80);
    let calibration = rgb_calibration::get_rgb_calibration(address, network_limits())?;
    let intrinsics = calibration.intrinsics;
    println!(
        "RGB intrinsics: calibration={}x{}, fx={}, fy={}, cx={}, cy={}",
        intrinsics.calibration_width,
        intrinsics.calibration_height,
        intrinsics.fx,
        intrinsics.fy,
        intrinsics.cx,
        intrinsics.cy
    );
    println!("RGB distortion: {:?}", calibration.distortion.coefficients);
    println!(
        "Left-to-RGB rotation (column-major): {:?}",
        calibration.left_to_rgb.rotation
    );
    println!(
        "Left-to-RGB translation (mm): {:?}",
        calibration.left_to_rgb.translation_mm
    );
    Ok(())
}

#[cfg(feature = "ros2")]
fn ros2_depth(ip: &str) -> Result<(), Box<dyn Error>> {
    use rclrs::CreateBasicExecutor;

    const BATCHES: usize = 20;
    let address = SocketAddr::new(ip.parse::<IpAddr>()?, 80);
    let limits = network_limits();
    let intrinsics =
        calibration::get_depth_intrinsics(address, limits)?.for_resolution(640, 400)?;
    let context = rclrs::Context::default();
    let executor = context.create_basic_executor();
    let node = executor.create_node("revopoint_pop3_depth")?;
    let publisher = ros2_adapter::Ros2CameraPublisher::new(&node)?;
    println!(
        "Publishing {BATCHES} scanner-computed depth frames on depth/image_rect and depth/camera_info"
    );
    thread::sleep(Duration::from_secs(1));

    for batch in 0..BATCHES {
        let (_, direct) = capture_depth_frame(address, limits)?;
        let frame = ros_camera::map_depth_camera(
            direct.depth,
            intrinsics,
            current_ros_time()?,
            "pop3_depth_optical_frame",
        )?;
        publisher.publish(frame)?;
        println!(
            "published scanner-computed depth frame {}/{BATCHES}",
            batch + 1
        );
    }
    Ok(())
}

#[cfg(feature = "ros2")]
fn current_ros_time() -> Result<ros_camera::RosTime, Box<dyn Error>> {
    let elapsed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?;
    Ok(ros_camera::RosTime {
        sec: i32::try_from(elapsed.as_secs())
            .map_err(|_| failure("current UNIX timestamp exceeds ROS Time.sec"))?,
        nanosec: elapsed.subsec_nanos(),
    })
}

#[cfg(not(feature = "ros2"))]
fn ros2_depth(_ip: &str) -> Result<(), Box<dyn Error>> {
    Err(failure(
        "ROS 2 support is disabled; source Jazzy and rebuild with --features ros2",
    ))
}

pub fn main_entry(arguments: impl IntoIterator<Item = String>) -> i32 {
    let mut arguments = arguments.into_iter();
    let program = arguments
        .next()
        .unwrap_or_else(|| "revopoint-pop3-wifi".to_owned());
    let arguments = arguments.collect::<Vec<_>>();
    let (write, diagnose) = match arguments.as_slice() {
        [] => (false, false),
        [argument] if argument == "--write" => (true, false),
        [argument] if argument == "--diagnose" => (false, true),
        [argument, ip, output_prefix] if argument == "--smoke-pair" => {
            return match smoke_pair(ip, output_prefix) {
                Ok(()) => 0,
                Err(error) => {
                    eprintln!("Error: {error}");
                    1
                }
            }
        }
        [argument, ip, output_prefix] if argument == "--smoke-depth" => {
            return match smoke_depth(ip, output_prefix) {
                Ok(()) => 0,
                Err(error) => {
                    eprintln!("Error: {error}");
                    1
                }
            };
        }
        [argument, ip, output_prefix] if argument == "--smoke-rgb" => {
            return match smoke_rgb(ip, output_prefix) {
                Ok(()) => 0,
                Err(error) => {
                    eprintln!("Error: {error}");
                    1
                }
            };
        }
        [argument, ip, output_prefix] if argument == "--smoke-rgbd" => {
            return match smoke_rgbd(ip, output_prefix) {
                Ok(()) => 0,
                Err(error) => {
                    eprintln!("Error: {error}");
                    1
                }
            };
        }
        [argument, ip] if argument == "--inspect-rgb-calibration" => {
            return match inspect_rgb_calibration(ip) {
                Ok(()) => 0,
                Err(error) => {
                    eprintln!("Error: {error}");
                    1
                }
            };
        }
        [argument, ip] if argument == "--ros2-depth" => {
            return match ros2_depth(ip) {
                Ok(()) => 0,
                Err(error) => {
                    eprintln!("Error: {error}");
                    1
                }
            }
        }
        [argument] if argument == "--help" || argument == "-h" => {
            println!("Usage: {program} [--write | --diagnose | --smoke-depth IP OUTPUT_PREFIX | --smoke-rgb IP OUTPUT_PREFIX | --smoke-rgbd IP OUTPUT_PREFIX | --inspect-rgb-calibration IP | --smoke-pair IP OUTPUT_PREFIX | --ros2-depth IP]");
            println!();
            println!("Options:");
            println!("  --write       Provision Wi-Fi client credentials over USB");
            println!("  --diagnose    Report scanner-side Wi-Fi diagnostics over USB");
            println!("  --smoke-depth IP OUTPUT_PREFIX  Save device-computed depth and infrared PGM images");
            println!("  --smoke-rgb IP OUTPUT_PREFIX  Save one validated RGB JPEG image");
            println!(
                "  --smoke-rgbd IP OUTPUT_PREFIX  Save concurrently acquired depth and RGB images"
            );
            println!(
                "  --inspect-rgb-calibration IP  Print RGB intrinsics and left-to-RGB transform"
            );
            println!("  --smoke-pair IP OUTPUT_PREFIX  Save left/right infrared PGM images");
            println!("  --ros2-depth IP  Publish 20 experimental reconstructed ROS 2 frames");
            println!("  -h, --help    Show this help");
            return 0;
        }
        _ => {
            eprintln!("Usage: {program} [--write | --diagnose | --smoke-depth IP OUTPUT_PREFIX | --smoke-rgb IP OUTPUT_PREFIX | --smoke-rgbd IP OUTPUT_PREFIX | --inspect-rgb-calibration IP | --smoke-pair IP OUTPUT_PREFIX | --ros2-depth IP]");
            return 2;
        }
    };

    if let Err(error) = run(write, diagnose) {
        eprintln!("Error: {error}");
        return 1;
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_config_values_without_exposing_them_by_default() {
        let config = "network={\n  ssid=\"test network\"\n  psk=\"secret123\"\n}\n";
        assert_eq!(config_value(config, "ssid"), Some("test network"));
        assert_eq!(config_value(config, "psk"), Some("secret123"));
    }

    #[test]
    fn escapes_wpa_quoted_values() {
        assert_eq!(escape_wpa("a\\b\"c"), "a\\\\b\\\"c");
    }

    #[test]
    fn generated_config_contains_wpa2_credentials() {
        let config = make_config("example", "password123");
        assert_eq!(
            config,
            concat!(
                "#pop2_enable=enable\n",
                "ctrl_interface=/var/run/wpa_supplicant\n",
                "ap_scan=1\n",
                "update_config=1\n\n",
                "network={\n",
                "ssid=\"example\"\n",
                "psk=\"password123\"\n",
                "key_mgmt=WPA-PSK\n",
                "pairwise=CCMP TKIP\n",
                "group=CCMP TKIP\n",
                "proto=WPA2\n",
                "}\n",
            )
        );
    }

    #[test]
    fn disables_access_point_without_rebuilding_vendor_configuration() {
        let config = "#pop2_enable=enable\ninterface=wlan0\nssid=POP3Plus\n";
        assert_eq!(
            disable_access_point(config).unwrap(),
            "#pop2_enable=disable\ninterface=wlan0\nssid=POP3Plus\n"
        );
    }

    #[test]
    fn accepts_an_already_disabled_access_point() {
        let config = "#pop2_enable=disable\ninterface=wlan0\n";
        assert_eq!(disable_access_point(config).unwrap(), config);
    }
}
