use rusb::{DeviceHandle, GlobalContext};
use std::env;
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::io::{self, Write};
use std::time::Duration;

const VID: u16 = 0x2207;
const PID: u16 = 0x110c;
const INTERFACE: u8 = 0;
const TIMEOUT: Duration = Duration::from_secs(1);
const START_FINISH: u16 = 0x0100;
const DATA: u16 = 0x0200;
const EXECUTE: u16 = 0x0700;
const EXTENSION_UNIT_AND_INTERFACE: u16 = 0x0400;
const REPORT_SIZE: usize = 60;
const CONFIG_PATH: &str = "/data/wpa_supplicant.config";

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
        let mut report = [0_u8; REPORT_SIZE];
        report[..5].copy_from_slice(b"sync\0");
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
        "#pop2_enable=enable\n\\
         ctrl_interface=/var/run/wpa_supplicant\n\\
         ap_scan=1\n\\
         update_config=1\n\n\\
         network={{\n\\
         ssid=\"{}\"\n\\
         psk=\"{}\"\n\\
         key_mgmt=WPA-PSK\n\\
         pairwise=CCMP TKIP\n\\
         group=CCMP TKIP\n\\
         proto=WPA2\n\\
         }}\n",
        escape_wpa(ssid),
        escape_wpa(password)
    )
}

fn prompt(prompt: &str) -> Result<String, Box<dyn Error>> {
    print!("{prompt}");
    io::stdout().flush()?;
    let mut value = String::new();
    io::stdin().read_line(&mut value)?;
    Ok(value.trim_end_matches(['\r', '\n']).to_owned())
}

fn run(write: bool) -> Result<(), Box<dyn Error>> {
    let scanner = Scanner::open()?;
    let current_bytes = scanner.read_file(CONFIG_PATH)?;
    let current = std::str::from_utf8(&current_bytes)?;
    let current_ssid = config_value(current, "ssid").unwrap_or("");
    let password_is_set = config_value(current, "psk").is_some_and(|value| !value.is_empty());
    println!("Current client SSID: \"{current_ssid}\"");
    println!(
        "Current password configured: {}",
        if password_is_set { "yes" } else { "no" }
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
    scanner.write_file(CONFIG_PATH, next.as_bytes())?;
    let verified = scanner.read_file(CONFIG_PATH)?;
    if verified != next.as_bytes() {
        return Err(failure(
            "read-back verification failed; do not power-cycle the scanner",
        ));
    }
    scanner.sync()?;
    println!(
        "Provisioning verified and synced. Disconnect USB data and power-cycle the scanner \
         from a power adapter or power bank to enter Wi-Fi mode."
    );
    Ok(())
}

fn main() {
    let mut arguments = env::args();
    let program = arguments
        .next()
        .unwrap_or_else(|| "revopoint-pop3-wifi".to_owned());
    let write = match (arguments.next().as_deref(), arguments.next()) {
        (None, None) => false,
        (Some("--write"), None) => true,
        _ => {
            eprintln!("usage: {program} [--write]");
            std::process::exit(2);
        }
    };

    if let Err(error) = run(write) {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
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
        assert!(config.contains("ssid=\"example\""));
        assert!(config.contains("psk=\"password123\""));
        assert!(config.contains("proto=WPA2"));
    }
}
