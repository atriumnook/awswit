//! Daemon runner: main loop for the autoawswit background process.
//!
//! Extracted from `src/bin/autoawswit.rs` so the binary is a thin wrapper
//! that only handles daemonization (fork/setsid) and delegates here.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::time::sleep;

use super::daemon::AutoRefreshProfile;

/// Maximum age of expired credentials before we skip refresh entirely.
const MAX_EXPIRED_HOURS: i64 = 1;

/// Check interval between refresh cycles
const CHECK_INTERVAL_SECS: u64 = 300;
/// Refresh when credentials expire within this window
const REFRESH_WINDOW_MINS: i64 = 5;
/// Exit after this many consecutive failures
const MAX_CONSECUTIVE_FAILURES: u32 = 5;
/// Initial backoff sleep after failure
const INITIAL_BACKOFF_SECS: u64 = 60;
/// Maximum backoff sleep
const MAX_BACKOFF_SECS: u64 = 600;

/// Run the daemon main loop with graceful SIGTERM shutdown.
pub async fn run_daemon_loop() {
    #[cfg(unix)]
    run_daemon_loop_unix().await;

    #[cfg(not(unix))]
    run_daemon_loop_fallback().await;
}

#[cfg(unix)]
async fn run_daemon_loop_unix() {
    let mut consecutive_failures: u32 = 0;
    let mut backoff_secs = INITIAL_BACKOFF_SECS;

    let mut sigterm = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to install SIGTERM handler, daemon will exit: {}", e);
            return;
        }
    };

    loop {
        let refresh_result = tokio::select! {
            result = refresh_all_profiles() => Some(result),
            _ = sigterm.recv() => {
                tracing::info!("Received SIGTERM, shutting down gracefully");
                None
            }
        };

        let refresh_result = match refresh_result {
            Some(r) => r,
            None => break,
        };

        match refresh_result {
            Ok(has_profiles) => {
                if !has_profiles {
                    tracing::info!("No auto-refresh profiles, exiting");
                    break;
                }
                consecutive_failures = 0;
                backoff_secs = INITIAL_BACKOFF_SECS;
            }
            Err(e) => {
                consecutive_failures += 1;
                tracing::error!(
                    "Error refreshing profiles ({}/{}): {}",
                    consecutive_failures,
                    MAX_CONSECUTIVE_FAILURES,
                    e
                );

                if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                    tracing::error!(
                        "Exiting after {} consecutive failures",
                        MAX_CONSECUTIVE_FAILURES
                    );
                    break;
                }

                tokio::select! {
                    _ = sleep(Duration::from_secs(backoff_secs)) => {}
                    _ = sigterm.recv() => {
                        tracing::info!("Received SIGTERM during backoff, shutting down");
                        break;
                    }
                }

                backoff_secs = (backoff_secs.saturating_mul(2)).min(MAX_BACKOFF_SECS);
                continue;
            }
        }

        tokio::select! {
            _ = sleep(Duration::from_secs(CHECK_INTERVAL_SECS)) => {}
            _ = sigterm.recv() => {
                tracing::info!("Received SIGTERM during sleep, shutting down");
                break;
            }
        }
    }

    if let Ok(pid_path) = get_pid_file_path() {
        let _ = fs::remove_file(pid_path);
    }
    tracing::info!("Daemon shutdown complete");
}

#[cfg(not(unix))]
async fn run_daemon_loop_fallback() {
    let mut consecutive_failures: u32 = 0;
    let mut backoff_secs = INITIAL_BACKOFF_SECS;

    loop {
        match refresh_all_profiles().await {
            Ok(has_profiles) => {
                if !has_profiles {
                    tracing::info!("No auto-refresh profiles, exiting");
                    break;
                }
                consecutive_failures = 0;
                backoff_secs = INITIAL_BACKOFF_SECS;
            }
            Err(e) => {
                consecutive_failures += 1;
                tracing::error!(
                    "Error refreshing profiles ({}/{}): {}",
                    consecutive_failures,
                    MAX_CONSECUTIVE_FAILURES,
                    e
                );

                if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                    tracing::error!(
                        "Exiting after {} consecutive failures",
                        MAX_CONSECUTIVE_FAILURES
                    );
                    break;
                }

                sleep(Duration::from_secs(backoff_secs)).await;
                backoff_secs = (backoff_secs.saturating_mul(2)).min(MAX_BACKOFF_SECS);
                continue;
            }
        }

        sleep(Duration::from_secs(CHECK_INTERVAL_SECS)).await;
    }

    if let Ok(pid_path) = get_pid_file_path() {
        let _ = fs::remove_file(pid_path);
    }
    tracing::info!("Daemon shutdown complete");
}

/// Write our PID file after successful init (called from child process only)
pub fn write_own_pid_file() -> Result<(), Box<dyn std::error::Error>> {
    let pid_path = get_pid_file_path()?;

    if let Some(parent) = pid_path.parent() {
        fs::create_dir_all(parent)?;
    }

    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&pid_path)?;
        file.write_all(std::process::id().to_string().as_bytes())?;
    }

    #[cfg(not(unix))]
    {
        fs::write(&pid_path, std::process::id().to_string())?;
    }

    Ok(())
}

/// Return the user's home directory, or an error if it cannot be determined.
fn home_dir() -> Result<PathBuf, Box<dyn std::error::Error>> {
    dirs::home_dir().ok_or_else(|| "Could not determine home directory".into())
}

fn get_pid_file_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    Ok(home_dir()?.join(".awswit").join("autoawswit.pid"))
}

fn get_auto_refresh_dir() -> Result<PathBuf, Box<dyn std::error::Error>> {
    Ok(home_dir()?.join(".awswit").join("autorefresh"))
}

async fn refresh_all_profiles() -> Result<bool, Box<dyn std::error::Error>> {
    let profiles = load_auto_refresh_profiles()?;

    if profiles.is_empty() {
        return Ok(false);
    }

    tracing::info!("Checking {} auto-refresh profiles", profiles.len());

    let mut attempted = 0u32;
    let mut failures = 0u32;
    let mut last_error: Option<String> = None;

    for (name, profile) in &profiles {
        if should_refresh(profile) {
            attempted += 1;
            tracing::info!("Refreshing profile: {}", name);
            if let Err(e) = refresh_profile(profile).await {
                failures += 1;
                tracing::error!("Failed to refresh {}: {}", name, e);
                last_error = Some(format!("{}: {}", name, e));
            }
        }
    }

    // If we attempted refreshes and ALL of them failed, return an error
    if attempted > 0 && failures == attempted {
        return Err(format!(
            "All {} refresh attempts failed. Last error: {}",
            failures,
            last_error.unwrap_or_default()
        )
        .into());
    }

    Ok(true)
}

fn load_auto_refresh_profiles(
) -> Result<HashMap<String, AutoRefreshProfile>, Box<dyn std::error::Error>> {
    let dir = get_auto_refresh_dir()?;
    if !dir.exists() {
        return Ok(HashMap::new());
    }

    let mut profiles = HashMap::new();

    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.extension().map(|e| e == "json").unwrap_or(false) {
            let content = match fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("Failed to read {}: {}", path.display(), e);
                    continue;
                }
            };
            if let Ok(profile) = serde_json::from_str::<AutoRefreshProfile>(&content) {
                profiles.insert(profile.profile_name.clone(), profile);
            }
        }
    }

    Ok(profiles)
}

fn should_refresh(profile: &AutoRefreshProfile) -> bool {
    let expiration = match &profile.awswit_role_expiration {
        Some(exp) => exp,
        None => return false,
    };

    let exp_time = match DateTime::parse_from_rfc3339(expiration) {
        Ok(dt) => dt.with_timezone(&Utc),
        Err(e) => {
            tracing::warn!(
                "Failed to parse expiration '{}' for profile '{}': {}",
                expiration,
                profile.profile_name,
                e
            );
            return false;
        }
    };

    let now = Utc::now();
    let time_until_exp = exp_time - now;

    // Skip refresh if credentials expired more than MAX_EXPIRED_HOURS ago
    if time_until_exp < chrono::Duration::hours(-MAX_EXPIRED_HOURS) {
        tracing::warn!(
            "Credentials for '{}' expired more than {} hour(s) ago, skipping refresh",
            profile.profile_name,
            MAX_EXPIRED_HOURS
        );
        return false;
    }

    time_until_exp < chrono::Duration::minutes(REFRESH_WINDOW_MINS)
}

async fn refresh_profile(profile: &AutoRefreshProfile) -> Result<(), Box<dyn std::error::Error>> {
    if profile.awswit_command.is_empty() {
        return Err("Empty command".into());
    }

    // Validate that the command is a known sibling binary to prevent arbitrary execution.
    // The daemon binary is "autoawswit" but stored commands reference "awswit",
    // so we allow both names explicitly.
    const ALLOWED_NAMES: &[&str] = &["awswit", "autoawswit"];
    let command_path = std::path::Path::new(&profile.awswit_command[0]);

    // Resolve bare command names to an absolute path using the current exe's directory
    // instead of allowing PATH resolution (which is vulnerable to binary hijacking).
    let resolved_command: PathBuf;
    if command_path.components().count() == 1 {
        let cmd_name = command_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        if !ALLOWED_NAMES.contains(&cmd_name) {
            return Err(format!(
                "Refusing to execute '{}': not an allowed command name ({:?})",
                profile.awswit_command[0], ALLOWED_NAMES
            )
            .into());
        }
        // Resolve to sibling of current executable to prevent PATH hijacking
        let current_exe = std::env::current_exe()?;
        resolved_command = current_exe
            .parent()
            .map(|p| p.join(cmd_name))
            .ok_or_else(|| "Cannot determine parent directory of current exe".to_string())?;
        if !resolved_command.exists() {
            return Err(format!(
                "Resolved command '{}' not found",
                resolved_command.display()
            )
            .into());
        }
    } else {
        // For paths with directory components, canonicalize and verify
        let current_exe = std::env::current_exe()?;
        let canonical_exe = current_exe.canonicalize()?;
        let canonical_cmd = command_path.canonicalize().map_err(|e| {
            format!(
                "Cannot resolve command path '{}': {}",
                profile.awswit_command[0], e
            )
        })?;
        let cmd_name = canonical_cmd
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        let same_dir = canonical_cmd.parent() == canonical_exe.parent();
        if !same_dir || !ALLOWED_NAMES.contains(&cmd_name) {
            return Err(format!(
                "Refusing to execute '{}': must be a sibling of '{}'",
                profile.awswit_command[0],
                canonical_exe.display()
            )
            .into());
        }
        resolved_command = canonical_cmd;
    }

    // Verify autorefresh directory has restrictive permissions (0o700 on unix)
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let dir = get_auto_refresh_dir()?;
        if dir.exists() {
            let mode = fs::metadata(&dir)?.mode() & 0o777;
            if mode & 0o077 != 0 {
                return Err(format!(
                    "Autorefresh directory {} has insecure permissions {:o}, expected 0700",
                    dir.display(),
                    mode
                )
                .into());
            }
        }
    }

    let output = tokio::process::Command::new(&resolved_command)
        .args(&profile.awswit_command[1..])
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Command failed: {}", stderr).into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    update_credentials_file(&profile.profile_name, &stdout)?;

    tracing::info!("Successfully refreshed {}", profile.profile_name);

    Ok(())
}

/// Validate that a profile name is safe for use in INI section headers.
fn validate_profile_name(name: &str) -> Result<(), Box<dyn std::error::Error>> {
    if name.is_empty() {
        return Err("Profile name cannot be empty".into());
    }
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        return Err(format!(
            "Profile name '{}' contains invalid characters (allowed: alphanumeric, _, -, .)",
            name
        )
        .into());
    }
    Ok(())
}

fn update_credentials_file(
    profile_name: &str,
    output: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    validate_profile_name(profile_name)?;

    let mut creds: HashMap<String, String> = HashMap::new();

    for line in output.lines() {
        if let Some((key, value)) = line.split_once('=') {
            // Reject values that could inject INI section headers
            if value.starts_with('[') || value.chars().any(|c| c.is_control()) {
                return Err(
                    format!("Credential value for '{}' contains invalid characters", key).into(),
                );
            }
            creds.insert(key.to_string(), value.to_string());
        }
    }

    let access_key = creds.get("AWS_ACCESS_KEY_ID").ok_or("Missing access key")?;
    let secret_key = creds
        .get("AWS_SECRET_ACCESS_KEY")
        .ok_or("Missing secret key")?;
    let session_token = creds.get("AWS_SESSION_TOKEN");
    let expiration = creds.get("AWSWIT_EXPIRATION");

    let creds_path = home_dir()?.join(".aws").join("credentials");

    let lock_path = creds_path.with_extension("lock");
    let lock_file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)?;
    use fs2::FileExt;
    lock_file.lock_exclusive()?;

    let content = fs::read_to_string(&creds_path).unwrap_or_default();

    let mut new_section = format!(
        "[{}]\n\
        aws_access_key_id = {}\n\
        aws_secret_access_key = {}\n",
        profile_name, access_key, secret_key,
    );
    if let Some(token) = session_token {
        new_section.push_str(&format!("aws_session_token = {}\n", token));
    }
    new_section.push_str("autoawswit = true\n");
    if let Some(exp) = expiration {
        new_section.push_str(&format!("awswit_expiration = {}\n", exp));
    }

    let lines: Vec<&str> = content.lines().collect();
    let mut new_lines = Vec::new();
    let mut skip = false;

    for line in &lines {
        if line.starts_with('[') {
            skip = line.trim() == format!("[{}]", profile_name);
        }
        if !skip {
            new_lines.push(*line);
        }
    }

    let mut new_content = new_lines.join("\n");
    if !new_content.ends_with('\n') && !new_content.is_empty() {
        new_content.push('\n');
    }
    new_content.push_str(&new_section);

    // Atomic write: write to temp file then rename to prevent corruption on crash
    let tmp_path = creds_path.with_extension("tmp");
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp_path)?;
        file.write_all(new_content.as_bytes())?;
        file.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        fs::write(&tmp_path, &new_content)?;
    }
    if let Err(e) = fs::rename(&tmp_path, &creds_path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(e.into());
    }

    // lock_file released on drop

    // Update the profile metadata
    let profile_path = get_auto_refresh_dir()?.join(format!("{}.json", profile_name));
    if profile_path.exists() {
        let meta_content = fs::read_to_string(&profile_path)?;
        if let Ok(mut profile) = serde_json::from_str::<AutoRefreshProfile>(&meta_content) {
            profile.aws_access_key_id = access_key.clone();
            profile.aws_secret_access_key = secret_key.clone();
            profile.aws_session_token = session_token.cloned();
            profile.awswit_role_expiration = expiration.cloned();

            let updated = serde_json::to_string_pretty(&profile)?;
            #[cfg(unix)]
            {
                use std::io::Write;
                use std::os::unix::fs::OpenOptionsExt;
                let mut file = fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .mode(0o600)
                    .open(&profile_path)?;
                file.write_all(updated.as_bytes())?;
            }
            #[cfg(not(unix))]
            {
                fs::write(&profile_path, updated)?;
            }
        }
    }

    Ok(())
}
