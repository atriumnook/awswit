//! End-to-end integration tests that invoke the awswit binary.

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
        .env_remove("AWS_DEFAULT_PROFILE")
        .env_remove("AWS_VAULT");
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
fn list_profiles_tsv_when_piped() {
    let home = setup_aws_home();
    let output = awswit_command(&home).arg("-l").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Output goes through a pipe in this Command setup, so we should get TSV.
    assert!(stdout.contains("default\t"));
    assert!(stdout.contains("dev\tRole"));
}

#[test]
fn list_profiles_json() {
    let home = setup_aws_home();
    let output = awswit_command(&home)
        .args(["-l", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let arr = v.as_array().unwrap();
    assert!(arr.iter().any(|e| e["name"] == "default"));
    assert!(
        arr.iter()
            .any(|e| e["name"] == "dev" && e["type"] == "Role")
    );
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
        .arg("--no-interactive")
        .arg("missing-profile")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Profile not found"));
}

#[test]
fn shell_export_emits_export_lines_to_stdout_only() {
    let home = setup_aws_home();
    let output = awswit_command(&home)
        .args(["--shell-export", "--no-interactive", "default"])
        .env("AWSWIT_SHELL", "bash")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("export AWS_PROFILE='default'"));
    assert!(stdout.contains("export AWS_REGION='us-west-2'"));
    // No status message must leak onto stdout (would corrupt `eval`).
    for line in stdout.lines() {
        assert!(
            line.starts_with("export ") || line.starts_with("unset "),
            "non-shell line on stdout: {:?}",
            line
        );
    }
    // The legacy variables are gone.
    assert!(!stdout.contains("AWS_DEFAULT_PROFILE"));
    assert!(!stdout.contains("AWS_DEFAULT_REGION"));
    assert!(!stdout.contains("AWSWIT_PROFILE"));
    assert!(!stdout.contains("AWSWIT_UNSET"));
}

#[test]
fn unset_emits_unset_lines() {
    let output = Command::new(env!("CARGO_BIN_EXE_awswit"))
        .arg("--unset")
        .env("AWSWIT_SHELL", "bash")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("unset AWS_PROFILE"));
    assert!(stdout.contains("unset AWS_REGION"));
    assert!(!stdout.contains("AWSWIT_UNSET"));
}

#[test]
fn aws_vault_session_emits_warning_on_stderr() {
    let home = setup_aws_home();
    let output = awswit_command(&home)
        .args(["--shell-export", "--no-interactive", "default"])
        .env("AWS_VAULT", "myvault")
        .env("AWSWIT_SHELL", "bash")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("AWS_VAULT"),
        "expected aws-vault warning on stderr, got: {}",
        stderr
    );
}

#[test]
fn unknown_profile_suggests_near_matches() {
    let home = setup_aws_home();
    // With fuzzy disabled, a near-miss should fall through to a hard error
    // that surfaces "did you mean…?" candidates.
    let output = awswit_command(&home)
        .arg("--no-interactive")
        .arg("dvv") // close to "dev"
        .env("AWSWIT_NO_FUZZY", "1")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("did you mean"),
        "expected suggestion, got: {}",
        stderr
    );
    assert!(
        stderr.contains("dev"),
        "expected 'dev' in suggestions: {}",
        stderr
    );
}

#[test]
fn exec_runs_command_with_profile_env() {
    let home = setup_aws_home();
    // Use /usr/bin/env to print the env in a portable way.
    let output = awswit_command(&home)
        .args(["exec", "dev", "--", "/usr/bin/env"])
        .output()
        .unwrap();
    assert!(output.status.success(), "exec exited {:?}", output.status);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("AWS_PROFILE=dev"));
    // dev profile has region us-west-2 in the test fixture.
    assert!(stdout.contains("AWS_REGION=us-west-2"));
}

#[test]
fn exec_propagates_exit_code() {
    let home = setup_aws_home();
    let output = awswit_command(&home)
        .args(["exec", "default", "--", "sh", "-c", "exit 42"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(42));
}

#[test]
fn which_reports_unset_when_no_profile() {
    let home = setup_aws_home();
    let output = awswit_command(&home)
        .arg("which")
        .env_remove("AWS_PROFILE")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("(unset)"), "got: {}", stdout);
}

#[test]
fn which_reports_current_profile() {
    let home = setup_aws_home();
    let output = awswit_command(&home)
        .arg("which")
        .env("AWS_PROFILE", "dev")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("AWS_PROFILE: dev"));
    assert!(stdout.contains("type:"));
}

#[test]
fn doctor_reports_clean_config() {
    let home = setup_aws_home();
    let output = awswit_command(&home).arg("doctor").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("no issues found"));
}

#[test]
fn doctor_flags_missing_source_profile() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config_path = temp_dir.path().join("config");
    fs::write(
        &config_path,
        r#"[profile broken]
role_arn = arn:aws:iam::123456789012:role/X
source_profile = nonexistent
"#,
    )
    .unwrap();
    let creds = temp_dir.path().join("credentials");
    fs::write(&creds, "").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_awswit"))
        .arg("doctor")
        .env("HOME", temp_dir.path())
        .env("USERPROFILE", temp_dir.path())
        .env("AWS_CONFIG_FILE", &config_path)
        .env("AWS_SHARED_CREDENTIALS_FILE", &creds)
        .env_remove("AWS_PROFILE")
        .env_remove("AWS_VAULT")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("source_profile") && stdout.contains("nonexistent"),
        "expected source_profile error, got: {}",
        stdout
    );
}

#[test]
fn prompt_outputs_format_string() {
    let output = Command::new(env!("CARGO_BIN_EXE_awswit"))
        .args(["prompt", "--format", "(aws: {})"])
        .env("AWS_PROFILE", "prod")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "(aws: prod)");
}

#[test]
fn prompt_accepts_percent_s_placeholder() {
    let output = Command::new(env!("CARGO_BIN_EXE_awswit"))
        .args(["prompt", "--format", "[%s]"])
        .env("AWS_PROFILE", "stg")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "[stg]");
}

#[test]
fn prompt_outputs_default_when_unset() {
    let output = Command::new(env!("CARGO_BIN_EXE_awswit"))
        .args(["prompt", "--default", "(no aws)"])
        .env_remove("AWS_PROFILE")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "(no aws)");
}
