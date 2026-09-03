use std::process::Command;

#[test]
fn help_is_available_without_scanner_hardware() {
    let output = Command::new(env!("CARGO_BIN_EXE_revopoint-pop3-wifi"))
        .arg("--help")
        .output()
        .expect("run provisioning CLI");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("help is UTF-8");
    assert!(stdout.contains("Usage:"));
    assert!(stdout.contains("--write"));
    assert!(stdout.contains("--diagnose"));
    assert!(stdout.contains("--smoke-pair"));
    assert!(stdout.contains("--smoke-depth"));
    assert!(stdout.contains("--smoke-rgb"));
    assert!(stdout.contains("--smoke-rgbd"));
    assert!(stdout.contains("--replay-archive"));
    assert!(stdout.contains("--capture-turntable"));
    assert!(stdout.contains("--inspect-rgb-calibration"));
    assert!(stdout.contains("--depth-controls"));
    assert!(stdout.contains("--set-depth-exposure"));
    assert!(stdout.contains("--set-depth-auto-exposure"));
    assert!(stdout.contains("--depth-auto-exposure"));
    assert!(stdout.contains("--merge-turntable"));
    assert!(stdout.contains("--write-turntable-schedule"));
    assert!(stdout.contains("usb is reserved"));
    assert!(!stdout.contains("--measure-plane"));
    assert!(stdout.contains("--ros2-depth"));
    assert!(stdout.contains("experimental"));
}

#[test]
fn usb_is_a_recognized_but_explicitly_unimplemented_capture_input() {
    let output = Command::new(env!("CARGO_BIN_EXE_revopoint-pop3-wifi"))
        .args(["--smoke-depth", "usb", "/tmp/pop3-unused"])
        .output()
        .expect("run capture CLI");

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8(output.stderr)
        .expect("stderr is UTF-8")
        .contains("USB media acquisition is not implemented yet"));
}

#[test]
fn turntable_merge_command_rejects_an_unsafe_session_before_reading_archives() {
    let output = Command::new(env!("CARGO_BIN_EXE_revopoint-pop3-wifi"))
        .args([
            "--merge-turntable",
            "/tmp/pop3-unused",
            "../escape",
            "/tmp/pop3-unused.ply",
        ])
        .output()
        .expect("run turntable merge CLI");

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8(output.stderr)
        .expect("stderr is UTF-8")
        .contains("turntable session ID is invalid"));
}
