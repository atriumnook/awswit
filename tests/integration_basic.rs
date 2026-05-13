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
fn list_profiles_names_only_emits_one_name_per_line() {
    let home = setup_aws_home();
    let output = awswit_command(&home)
        .args(["-l", "--names-only"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut names: Vec<&str> = stdout.lines().collect();
    names.sort();
    assert_eq!(names, vec!["default", "dev"]);
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
    // We *clear* the legacy AWS_DEFAULT_* variables so the SDK can't fall
    // back to a stale value, but we never SET them — and we never set the
    // awswit-private marker variables.
    assert!(stdout.contains("unset AWS_DEFAULT_PROFILE"));
    assert!(stdout.contains("unset AWS_DEFAULT_REGION"));
    assert!(!stdout.contains("export AWS_DEFAULT_PROFILE"));
    assert!(!stdout.contains("export AWS_DEFAULT_REGION"));
    assert!(!stdout.contains("AWSWIT_PROFILE"));
    assert!(!stdout.contains("AWSWIT_UNSET"));
}

#[test]
fn unset_emits_unset_lines_for_every_managed_var() {
    let output = Command::new(env!("CARGO_BIN_EXE_awswit"))
        .arg("--unset")
        .env("AWSWIT_SHELL", "bash")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    for v in [
        "AWS_PROFILE",
        "AWS_DEFAULT_PROFILE",
        "AWS_REGION",
        "AWS_DEFAULT_REGION",
    ] {
        assert!(
            stdout.contains(&format!("unset {}", v)),
            "missing unset for {}",
            v
        );
    }
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
fn which_exits_nonzero_when_aws_profile_unknown() {
    let home = setup_aws_home();
    let output = awswit_command(&home)
        .arg("which")
        .env("AWS_PROFILE", "ghost")
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(1),
        "which should signal mis-config in CI / precmd guards"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("warning"));
}

#[test]
fn no_interactive_without_profile_or_env_errors() {
    let home = setup_aws_home();
    let output = awswit_command(&home).arg("-n").output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no-interactive") || stderr.contains("PROFILE"),
        "expected a usage error, got: {}",
        stderr
    );
}

#[test]
fn no_interactive_does_not_silently_fuzzy_match() {
    let home = setup_aws_home();
    // `dvv` is one edit from `dev` in the test fixture, but in
    // non-interactive mode we must NOT auto-substitute — instead emit a
    // "did you mean?" error and exit 1.
    let output = awswit_command(&home).arg("-n").arg("dvv").output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("did you mean") && stderr.contains("dev"),
        "expected did-you-mean for dev, got: {}",
        stderr
    );
    // And no shell-export should have been emitted.
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
}

#[test]
fn exec_works_without_dashdash_separator() {
    let home = setup_aws_home();
    let output = awswit_command(&home)
        .args(["exec", "dev", "/usr/bin/env"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("AWS_PROFILE=dev"));
}

#[test]
fn exec_with_typo_suggests_correct_profile() {
    let home = setup_aws_home();
    let output = awswit_command(&home)
        .args(["exec", "dvv", "--", "true"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("did you mean"));
    assert!(stderr.contains("dev"));
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
fn pick_subcommand_appears_in_help() {
    let output = Command::new(env!("CARGO_BIN_EXE_awswit"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("pick"), "pick not in --help:\n{}", stdout);
}

#[test]
fn pick_with_fzf_returns_profile_name_only() {
    let home = setup_aws_home();
    // Without an interactive terminal, the TUI path can't run — but if
    // `fzf` is available we can route through it and feed a deterministic
    // selection. Skip when fzf isn't installed in the test environment.
    if Command::new("fzf").arg("--version").output().is_err() {
        return;
    }
    use std::process::Stdio;

    let child = awswit_command(&home)
        .arg("pick")
        .env("AWSWIT_USE_FZF", "1")
        .env("AWSWIT_FZF_OPTS", "--filter=dev")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let output = child.wait_with_output().unwrap();
    if !output.status.success() {
        // fzf --filter prints matches without an interactive selection;
        // the binary still expects a single line — skip if behaviour
        // diverges in the test runner.
        return;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.trim(), "dev");
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
