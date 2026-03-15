use std::fs;
use std::process::Command;

use tempfile::TempDir;

fn setup_aws_home() -> TempDir {
    let temp_dir = tempfile::tempdir().unwrap();
    let config_path = temp_dir.path().join("config");
    let credentials_path = temp_dir.path().join("credentials");

    fs::write(
        &config_path,
        r#"[default]
region = us-west-2

[profile dev]
role_arn = arn:aws:iam::123456789012:role/DevRole
source_profile = default
region = us-west-2
"#,
    )
    .unwrap();

    fs::write(
        &credentials_path,
        r#"[default]
aws_access_key_id = AKIATEST
aws_secret_access_key = secret
"#,
    )
    .unwrap();

    temp_dir
}

fn awswit_command(home: &TempDir) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_awswit"));
    cmd.env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("AWS_CONFIG_FILE", home.path().join("config"))
        .env(
            "AWS_SHARED_CREDENTIALS_FILE",
            home.path().join("credentials"),
        )
        .env_remove("AWS_PROFILE")
        .env_remove("AWS_DEFAULT_PROFILE");
    cmd
}

#[test]
fn help_outputs_usage() {
    let output = Command::new(env!("CARGO_BIN_EXE_awswit"))
        .arg("--help")
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Usage:"));
    assert!(stdout.contains("completions"));
}

#[test]
fn version_outputs_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_awswit"))
        .arg("--version")
        .output()
        .unwrap();

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        format!("awswit {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn list_profiles_succeeds() {
    let home = setup_aws_home();
    let output = awswit_command(&home).arg("-l").output().unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("PROFILE"));
    assert!(stdout.contains("default"));
    assert!(stdout.contains("dev"));
}

#[test]
fn completions_bash_outputs_script() {
    let output = Command::new(env!("CARGO_BIN_EXE_awswit"))
        .args(["completions", "bash"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("_awswit()"));
}

#[test]
fn missing_profile_returns_error() {
    let home = setup_aws_home();
    let output = awswit_command(&home)
        .arg("missing-profile")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("[E001] Profile not found: missing-profile"));
}

#[test]
fn profile_selection_outputs_aws_profile() {
    let home = setup_aws_home();
    let output = awswit_command(&home)
        .args(["--no-interactive", "default"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("AWS_PROFILE=default"));
    assert!(stdout.contains("AWSWIT_PROFILE=default"));
    assert!(stdout.contains("AWS_REGION=us-west-2"));
}

#[test]
fn unset_outputs_unset_marker() {
    let output = Command::new(env!("CARGO_BIN_EXE_awswit"))
        .arg("--unset")
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("AWSWIT_UNSET=1"));
}
