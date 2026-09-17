//! Regression tests for the documented shell-wrapper shape:
//! stdout captured (`$()` / `$(awswit pick)`), stdin + stderr still a TTY.
//!
//! Unix-only: Windows CI has no POSIX PTY, and the bug is the Unix wrapper's
//! command substitution. TestBackend snapshots cannot catch this — they never
//! exercise `is_terminal()` or the real Crossterm backend.

#![cfg(unix)]

use std::fs;
use std::path::Path;
use std::process::Command;

use tempfile::TempDir;

fn setup_aws_home() -> TempDir {
    let temp_dir = tempfile::tempdir().unwrap();
    fs::write(
        temp_dir.path().join("config"),
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
        temp_dir.path().join("credentials"),
        r#"[default]
aws_access_key_id = AKIATEST
aws_secret_access_key = secret
"#,
    )
    .unwrap();
    temp_dir
}

fn helper_path() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/pty_picker.py"))
}

struct PtyResult {
    tui: bool,
    alive: bool,
    exit: i32,
    stdout: String,
    pty: String,
    helper_status: i32,
}

fn python_bin() -> &'static str {
    for bin in ["python3", "python"] {
        let ok = Command::new(bin)
            .args(["-c", "import pty"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if ok {
            return bin;
        }
    }
    panic!("python3 with the pty module is required for Unix PTY tests");
}

fn run_pty(mode: &str, extra: &[&str], home: &TempDir) -> PtyResult {
    let output = Command::new(python_bin())
        .arg(helper_path())
        .arg(mode)
        .arg(env!("CARGO_BIN_EXE_awswit"))
        .args(extra)
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("AWS_CONFIG_FILE", home.path().join("config"))
        .env(
            "AWS_SHARED_CREDENTIALS_FILE",
            home.path().join("credentials"),
        )
        .env("AWSWIT_SHELL", "bash")
        .env("TERM", "xterm-256color")
        .env_remove("AWS_PROFILE")
        .env_remove("AWS_DEFAULT_PROFILE")
        .env_remove("AWS_VAULT")
        .env_remove("AWSWIT_USE_FZF")
        .output()
        .expect("failed to spawn pty_picker.py");
    parse_pty_result(
        &String::from_utf8_lossy(&output.stdout),
        output.status.code().unwrap_or(1),
    )
}

fn parse_pty_result(combined: &str, helper_status: i32) -> PtyResult {
    let stdout_split = combined.split_once("---STDOUT---\n");
    let (header, rest) = stdout_split.unwrap_or((combined, ""));
    let (stdout, pty) = rest.split_once("\n---PTY---\n").unwrap_or((rest, ""));

    let mut tui = false;
    let mut alive = false;
    let mut exit = -1;
    for line in header.lines() {
        if let Some(v) = line.strip_prefix("tui=") {
            tui = v.trim() == "1";
        } else if let Some(v) = line.strip_prefix("alive=") {
            alive = v.trim() == "1";
        } else if let Some(v) = line.strip_prefix("exit=") {
            exit = v.trim().parse().unwrap_or(-1);
        }
    }
    PtyResult {
        tui,
        alive,
        exit,
        stdout: stdout.to_string(),
        pty: pty.to_string(),
        helper_status,
    }
}

/// The exact bug: with stdout piped (like `$()`), the picker used to skip
/// itself because `stdout().is_terminal()` was false, then fall through to
/// `default` and print an export immediately. After the fix the process
/// stays in the TUI, stdout stays empty, and the UI is on the PTY (stderr).
#[test]
fn picker_stays_open_when_stdout_is_piped() {
    let home = setup_aws_home();
    let result = run_pty("wait", &["--shell-export"], &home);
    assert_eq!(
        result.helper_status, 0,
        "helper failed: tui={} alive={} exit={}\nstdout={:?}\npty={:?}",
        result.tui, result.alive, result.exit, result.stdout, result.pty
    );
    assert!(
        result.tui,
        "expected TUI on stderr/PTY when stdout is a pipe; pty={:?}",
        result.pty
    );
    assert!(
        result.alive,
        "picker exited immediately (fell through to default?). stdout={:?}",
        result.stdout
    );
    assert!(
        result.stdout.trim().is_empty(),
        "stdout must stay empty while the picker is open (wrapper captures it): {:?}",
        result.stdout
    );
    assert!(
        !result.stdout.contains('\u{1b}'),
        "TUI escape codes leaked onto stdout: {:?}",
        result.stdout
    );
}

/// Enter in the wrapper-shaped process must emit eval-able export lines on
/// stdout only — no CSI — so `eval "$(awswit)"` receives a clean payload.
#[test]
fn picker_enter_emits_export_on_stdout_only() {
    let home = setup_aws_home();
    let result = run_pty("enter", &["--shell-export"], &home);
    assert!(
        result.tui,
        "picker never appeared on PTY; helper={} exit={}\npty={:?}",
        result.helper_status, result.exit, result.pty
    );
    assert_eq!(
        result.exit, 0,
        "expected success after Enter, got {}\nstdout={:?}\npty={:?}",
        result.exit, result.stdout, result.pty
    );
    assert!(
        result.stdout.contains("export AWS_PROFILE="),
        "expected export on stdout, got {:?}",
        result.stdout
    );
    assert!(
        result.stdout.contains("unset AWS_ACCESS_KEY_ID"),
        "credential unset missing from wrapper payload: {:?}",
        result.stdout
    );
    for line in result.stdout.lines() {
        if line.is_empty() {
            continue;
        }
        assert!(
            line.starts_with("export ") || line.starts_with("unset "),
            "non-shell line on stdout (TUI leak?): {:?}",
            line
        );
    }
    assert!(
        !result.stdout.contains('\u{1b}'),
        "TUI escape codes leaked onto stdout: {:?}",
        result.stdout
    );
}

/// `$(awswit pick)` captures stdout. The TUI must live on stderr/PTY and
/// stdout must be a single profile name.
#[test]
fn pick_subcommand_keeps_stdout_clean_under_capture() {
    let home = setup_aws_home();
    let result = run_pty("pick", &["pick"], &home);
    assert!(
        result.tui,
        "pick TUI never appeared on PTY; helper={} exit={}\npty={:?}",
        result.helper_status, result.exit, result.pty
    );
    assert_eq!(
        result.exit, 0,
        "expected success after Enter, got {}\nstdout={:?}\npty={:?}",
        result.exit, result.stdout, result.pty
    );
    let name = result.stdout.trim();
    assert!(
        name == "default" || name == "dev",
        "pick stdout should be a profile name, got {:?}",
        result.stdout
    );
    assert_eq!(
        result.stdout.lines().filter(|l| !l.is_empty()).count(),
        1,
        "pick must print exactly one line: {:?}",
        result.stdout
    );
    assert!(
        !result.stdout.contains('\u{1b}'),
        "TUI escape codes leaked onto pick stdout: {:?}",
        result.stdout
    );
}

#[test]
fn picker_esc_cancels_without_export() {
    let home = setup_aws_home();
    let result = run_pty("esc", &["--shell-export"], &home);
    assert!(
        result.tui,
        "picker never appeared; cannot test cancel. pty={:?}",
        result.pty
    );
    assert_eq!(
        result.exit, 130,
        "cancel should exit 130, got {}\nstdout={:?}",
        result.exit, result.stdout
    );
    assert!(
        result.stdout.trim().is_empty(),
        "cancel must not emit export payload: {:?}",
        result.stdout
    );
}
