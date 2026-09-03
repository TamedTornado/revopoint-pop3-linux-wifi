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
#[cfg(feature = "ros2")]
pub mod ros2_adapter;
pub mod ros_camera;
pub mod stereo_calibration;
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

fn smoke_pair(ip: &str, output_prefix: &str) -> Result<(), Box<dyn Error>> {
    const PREFIX_BYTES: usize = 1024 * 1024;
    const MAXIMUM_DISPARITY: u16 = 160;
    const MINIMUM_MATCH_MARGIN_PERCENT: u16 = 1;
    let address = SocketAddr::new(ip.parse::<IpAddr>()?, 80);
    let limits = http_stream::StreamLimits {
        connect_timeout: Duration::from_secs(3),
        idle_timeout: Duration::from_secs(3),
        total_timeout: Duration::from_secs(15),
        max_header_bytes: 16 * 1024,
        max_body_bytes: 2 * 1024 * 1024,
    };
    let mut parser = frame_envelope::FrameEnvelopeParser::new(2 * 1024 * 1024, 4);
    let mut first_pair = None;
    let mut frame_error: Option<Box<dyn Error>> = None;
    let received = depth_stream::capture_pair_prefix(address, limits, PREFIX_BYTES, |chunk| {
        if first_pair.is_some() || frame_error.is_some() {
            return;
        }
        match parser.push(chunk) {
            Ok(frames) => {
                if let Some(frame) = frames.into_iter().next() {
                    match depth_decode::decode_quicklz(&frame, 2 * 1024 * 1024)
                        .map_err(|error| Box::new(error) as Box<dyn Error>)
                        .and_then(|decoded| {
                            pair_decode::decode_y8_pair(decoded.bytes, 640, 400)
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
    let maps = stereo_calibration::get_stereo_map_parameters(address, limits)?;
    let reprojection = stereo_calibration::get_reprojection_matrix(address, limits)?;
    let left_rectified =
        stereo_calibration::rectify_y8(&pair.left, pair.width, pair.height, maps.left)?;
    let right_rectified =
        stereo_calibration::rectify_y8(&pair.right, pair.width, pair.height, maps.right)?;
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
    let disparity_scale = maps.left.calibration_width as f32 / pair.width as f32;
    let experimental_median_depth_mm = reprojection
        .depth_mm(f32::from(median_disparity), disparity_scale)
        .ok_or_else(|| {
            failure("median disparity cannot be reprojected to positive metric depth")
        })?;
    let left_path = format!("{output_prefix}-left.pgm");
    let right_path = format!("{output_prefix}-right.pgm");
    let left_rectified_path = format!("{output_prefix}-left-rectified.pgm");
    let right_rectified_path = format!("{output_prefix}-right-rectified.pgm");
    let disparity_path = format!("{output_prefix}-disparity.pgm");
    let valid_pixels = disparity.valid_count();
    let valid_percent = valid_pixels as f64 * 100.0 / disparity.values.len() as f64;
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
        pair_decode::encode_y8_pgm(pair.width, pair.height, &left_rectified)?,
    )?;
    std::fs::write(
        &right_rectified_path,
        pair_decode::encode_y8_pgm(pair.width, pair.height, &right_rectified)?,
    )?;
    std::fs::write(
        &disparity_path,
        stereo_match::encode_disparity_pgm(&disparity, MAXIMUM_DISPARITY)?,
    )?;
    println!(
        "PAIR stream smoke passed: bytes={received}, resolution={}x{}, left={left_path}, right={right_path}, left_rectified={left_rectified_path}, right_rectified={right_rectified_path}, disparity={disparity_path}, valid_pixels={valid_pixels} ({valid_percent:.1}%), median_disparity_px={median_disparity}, experimental_median_depth_mm={experimental_median_depth_mm:.1}",
        pair.width, pair.height
    );
    Ok(())
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
        [argument] if argument == "--help" || argument == "-h" => {
            println!("Usage: {program} [--write | --diagnose | --smoke-pair IP OUTPUT_PREFIX]");
            println!();
            println!("Options:");
            println!("  --write       Provision Wi-Fi client credentials over USB");
            println!("  --diagnose    Report scanner-side Wi-Fi diagnostics over USB");
            println!("  --smoke-pair IP OUTPUT_PREFIX  Save left/right infrared PGM images");
            println!("  -h, --help    Show this help");
            return 0;
        }
        _ => {
            eprintln!("Usage: {program} [--write | --diagnose | --smoke-pair IP OUTPUT_PREFIX]");
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
