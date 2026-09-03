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
    assert!(!stdout.contains("--measure-plane"));
    assert!(stdout.contains("--ros2-depth"));
    assert!(stdout.contains("experimental"));
}
