use revopoint_pop3_wifi::scanner_input::{ScannerInput, ScannerInputError};
use std::str::FromStr;

#[test]
fn parses_existing_ip_arguments_as_wifi_input() {
    assert_eq!(
        ScannerInput::from_str("192.168.8.245").expect("IPv4 input"),
        ScannerInput::Wifi("192.168.8.245:80".parse().expect("socket address"))
    );
    assert_eq!(
        ScannerInput::from_str("2001:db8::1").expect("IPv6 input"),
        ScannerInput::Wifi("[2001:db8::1]:80".parse().expect("socket address"))
    );
}

#[test]
fn represents_usb_without_falling_back_to_networking() {
    let input = ScannerInput::from_str("usb").expect("USB input mode");
    assert_eq!(input, ScannerInput::Usb);
    let error = input
        .network_address()
        .expect_err("USB must not silently use a network address");
    assert_eq!(error, ScannerInputError::UsbBackendNotImplemented);
    assert_eq!(
        error.to_string(),
        "USB media acquisition is not implemented yet; use a scanner IP for Wi-Fi input"
    );
}

#[test]
fn rejects_hostnames_ports_and_unknown_modes() {
    for invalid in ["", "scanner.local", "192.168.8.245:80", "bluetooth"] {
        assert!(
            ScannerInput::from_str(invalid).is_err(),
            "accepted {invalid:?}"
        );
    }
}
