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

use super::credentials_file;
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
    {
        let mut sigterm =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!(
                        "Failed to install SIGTERM handler, daemon will exit: {}",
                        e
                    );
                    return;
                }
            };
        run_daemon_loop_inner(Some(&mut sigterm)).await;
    }

    #[cfg(not(unix))]
    run_daemon_loop_inner().await;
}

/// Interruptible sleep that returns `false` if a SIGTERM was received.
async fn interruptible_sleep(
    duration: Duration,
    #[cfg(unix)] sigterm: Option<&mut tokio::signal::unix::Signal>,
) -> bool {
    #[cfg(unix)]
    if let Some(sig) = sigterm {
        tokio::select! {
            _ = sleep(duration) => return true,
            _ = sig.recv() => {
                tracing::info!("Received SIGTERM during sleep, shutting down");
                return false;
            }
        }
    }

    sleep(duration).await;
    true
}

/// Core daemon loop shared between unix and non-unix platforms.
/// On unix, `sigterm` enables graceful SIGTERM shutdown.
/// On non-unix, the daemon exits when all profiles are removed or after MAX_CONSECUTIVE_FAILURES.
async fn run_daemon_loop_inner(
    #[cfg(unix)] mut sigterm: Option<&mut tokio::signal::unix::Signal>,
) {
    let mut consecutive_failures: u32 = 0;
    let mut backoff_secs = INITIAL_BACKOFF_SECS;

    loop {
        // Refresh all profiles, with optional SIGTERM interruption
        #[cfg(unix)]
        let refresh_result = if let Some(ref mut sig) = sigterm {
            tokio::select! {
                result = refresh_all_profiles() => Some(result),
                _ = sig.recv() => {
                    tracing::info!("Received SIGTERM, shutting down gracefully");
                    None
                }
            }
        } else {
            Some(refresh_all_profiles().await)
        };

        #[cfg(not(unix))]
        let refresh_result = Some(refresh_all_profiles().await);

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

                let keep_running = interruptible_sleep(
                    Duration::from_secs(backoff_secs),
                    #[cfg(unix)]
                    sigterm.as_deref_mut(),
                )
                .await;
                if !keep_running {
                    break;
                }

                backoff_secs = (backoff_secs.saturating_mul(2)).min(MAX_BACKOFF_SECS);
                continue;
            }
        }

        let keep_running = interruptible_sleep(
            Duration::from_secs(CHECK_INTERVAL_SECS),
            #[cfg(unix)]
            sigterm.as_deref_mut(),
        )
        .await;
        if !keep_running {
            break;
        }
    }

    if let Ok(pid_path) = get_pid_file_path() {
        if let Err(e) = fs::remove_file(&pid_path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!("Failed to remove PID file {}: {}", pid_path.display(), e);
            }
        }
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

fn get_pid_file_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    Ok(crate::utils::paths::awswit_home_dir()?.join("autoawswit.pid"))
}

fn get_auto_refresh_dir() -> Result<PathBuf, Box<dyn std::error::Error>> {
    Ok(crate::utils::paths::awswit_home_dir()?.join("autorefresh"))
}

async fn refresh_all_profiles() -> Result<bool, Box<dyn std::error::Error>> {
    let profiles = load_auto_refresh_profiles()?;

    if profiles.is_empty() {
        return Ok(false);
    }

    tracing::info!("Checking {} auto-refresh profiles", profiles.len());

    // Clean up profiles that have been expired for too long
    let mut expired_profiles = Vec::new();
    for (name, profile) in &profiles {
        if let Some(ref exp_str) = profile.awswit_role_expiration {
            if let Ok(exp_time) = DateTime::parse_from_rfc3339(exp_str) {
                let exp_utc = exp_time.with_timezone(&Utc);
                if exp_utc + chrono::Duration::hours(MAX_EXPIRED_HOURS) < Utc::now() {
                    tracing::info!(
                        "Cleaning up long-expired profile '{}' (expired at {})",
                        name,
                        exp_str
                    );
                    expired_profiles.push(name.clone());
                }
            }
        }
    }

    if !expired_profiles.is_empty() {
        // Remove expired profile JSON files
        let dir = get_auto_refresh_dir()?;
        for name in &expired_profiles {
            let json_path = dir.join(format!("{}.json", name));
            if let Err(e) = fs::remove_file(&json_path) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!("Failed to remove expired profile {}: {}", name, e);
                }
            }
        }
        // Remove expired credential sections in batch
        let creds_path = crate::utils::paths::aws_credentials_path()?;
        if let Err(e) = credentials_file::remove_credentials_batch(&creds_path, &expired_profiles)
        {
            tracing::warn!("Failed to remove expired credential sections: {}", e);
        }
    }

    // Re-check remaining profiles (exclude expired ones)
    let profiles: HashMap<_, _> = profiles
        .into_iter()
        .filter(|(name, _)| !expired_profiles.contains(name))
        .collect();

    if profiles.is_empty() {
        return Ok(false);
    }

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
            match serde_json::from_str::<AutoRefreshProfile>(&content) {
                Ok(profile) => {
                    profiles.insert(profile.profile_name.clone(), profile);
                }
                Err(e) => {
                    tracing::warn!("Failed to parse {}: {}", path.display(), e);
                }
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

    let output = tokio::time::timeout(
        Duration::from_secs(60),
        tokio::process::Command::new(&resolved_command)
            .args(&profile.awswit_command[1..])
            .output(),
    )
    .await
    .map_err(|_| "refresh_profile timed out after 60 seconds".to_string())??;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Command failed: {}", stderr).into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    update_credentials_file(&profile.profile_name, &stdout)?;

    tracing::info!("Successfully refreshed {}", profile.profile_name);

    Ok(())
}

fn update_credentials_file(
    profile_name: &str,
    output: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut creds: HashMap<String, String> = HashMap::new();

    for line in output.lines() {
        if let Some((key, value)) = line.split_once('=') {
            creds.insert(key.to_string(), value.to_string());
        }
    }

    let access_key = creds.get("AWS_ACCESS_KEY_ID").ok_or("Missing access key")?;
    let secret_key = creds
        .get("AWS_SECRET_ACCESS_KEY")
        .ok_or("Missing secret key")?;
    let session_token = creds.get("AWS_SESSION_TOKEN");
    let expiration = creds.get("AWSWIT_EXPIRATION");

    // Write credentials first — this is the critical path. If we crash after
    // writing credentials but before updating metadata, the metadata will be
    // stale but self-correcting (next refresh cycle will fix it). The reverse
    // (metadata updated but credentials stale) would leave users with expired creds.
    let creds_path = crate::utils::paths::aws_credentials_path()?;
    credentials_file::write_credentials_from_output(
        &creds_path,
        profile_name,
        access_key,
        secret_key,
        session_token.map(|s| s.as_str()),
        expiration.map(|s| s.as_str()),
    )
    .map_err(|e| -> Box<dyn std::error::Error> { format!("{}", e).into() })?;

    // Update profile metadata after credentials are written
    let profile_path = get_auto_refresh_dir()?.join(format!("{}.json", profile_name));
    if profile_path.exists() {
        let meta_content = fs::read_to_string(&profile_path)?;
        match serde_json::from_str::<AutoRefreshProfile>(&meta_content) {
            Ok(mut profile) => {
                profile.aws_access_key_id = access_key.clone();
                profile.aws_secret_access_key = secret_key.clone();
                profile.aws_session_token = session_token.cloned();
                profile.awswit_role_expiration = expiration.cloned();

                let updated = serde_json::to_string_pretty(&profile)?;
                crate::utils::fs::atomic_write_restricted(&profile_path, updated.as_bytes())
                    .map_err(|e| format!("Failed to write profile metadata: {}", e))?;
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to parse profile metadata {}: {}, skipping metadata update",
                    profile_path.display(),
                    e
                );
            }
        }
    }

    Ok(())
}
