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

#[cfg(unix)]
#[test]
fn exec_propagates_child_exit_code() {
    let home = setup_aws_home();
    let output = awswit_command(&home)
        .args(["exec", "default", "--", "sh", "-c", "exit 42"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(42));
}

#[cfg(unix)]
#[test]
fn exec_injects_env_without_leaking_parent_values() {
    let home = setup_aws_home();
    let output = awswit_command(&home)
        .env("AWS_SESSION_TOKEN", "LEAKED_TOKEN")
        .env("AWS_REGION", "leaked-region")
        .args(["exec", "default", "--", "env"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("AWS_ACCESS_KEY_ID=AKIATEST"));
    assert!(stdout.contains("AWS_SECRET_ACCESS_KEY=secret"));
    assert!(stdout.contains("AWS_REGION=us-west-2"));
    assert!(stdout.contains("AWS_DEFAULT_REGION=us-west-2"));
    assert!(stdout.contains("AWSWIT_PROFILE=default"));
    assert!(!stdout.contains("AWS_SESSION_TOKEN=LEAKED_TOKEN"));
}
