//! Autoawswit daemon - refreshes credentials automatically

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::time::sleep;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AutoRefreshProfile {
    profile_name: String,
    awswit_command: Vec<String>,
    aws_access_key_id: String,
    aws_secret_access_key: String,
    aws_session_token: Option<String>,
    awswit_role_expiration: Option<String>,
    awswit_cache_name: Option<String>,
    aws_role_arn: Option<String>,
    region: Option<String>,
}

fn main() {
    // Daemonize (detach from terminal) BEFORE starting tokio runtime
    // Forking after tokio starts is undefined behavior in multithreaded programs
    #[cfg(unix)]
    {
        use std::process;
        match unsafe { libc::fork() } {
            -1 => {
                eprintln!("Failed to fork");
                process::exit(1);
            }
            0 => {
                // Child process continues
                if unsafe { libc::setsid() } == -1 {
                    eprintln!("Failed to create new session");
                    process::exit(1);
                }
            }
            _ => {
                // Parent exits
                process::exit(0);
            }
        }
    }

    // Setup logging after fork
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .init();

    // Write our own PID file after successful init (not from parent)
    if let Err(e) = write_own_pid_file() {
        tracing::error!("Failed to write PID file: {}", e);
        std::process::exit(1);
    }

    tracing::info!("Autoawswit daemon started (pid={})", std::process::id());

    // Start tokio runtime after fork
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    rt.block_on(async_main());

    // Clean up PID file on normal exit.
    // NOTE: On SIGTERM this line is unreachable. Stale PID files are detected
    // and cleaned up by `is_autoawswit_running()` in the daemon spawner, which
    // verifies the PID is alive and belongs to an autoawswit process before
    // treating it as valid.
    let _ = fs::remove_file(get_pid_file_path());
}

fn write_own_pid_file() -> Result<(), Box<dyn std::error::Error>> {
    let pid_path = get_pid_file_path();

    // Ensure parent directory exists
    if let Some(parent) = pid_path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Write PID with restrictive permissions (0o600)
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

fn get_pid_file_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".awswit")
        .join("autoawswit.pid")
}

async fn async_main() {
    let mut consecutive_failures: u32 = 0;
    let mut backoff_secs = INITIAL_BACKOFF_SECS;

    loop {
        match refresh_all_profiles().await {
            Ok(has_profiles) => {
                if !has_profiles {
                    tracing::info!("No auto-refresh profiles, exiting");
                    break;
                }
                // Reset failure tracking on success
                consecutive_failures = 0;
                backoff_secs = INITIAL_BACKOFF_SECS;
            }
            Err(e) => {
                consecutive_failures += 1;
                tracing::error!(
                    "Error refreshing profiles ({}/{}): {}",
                    consecutive_failures, MAX_CONSECUTIVE_FAILURES, e
                );

                if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                    tracing::error!(
                        "Exiting after {} consecutive failures",
                        MAX_CONSECUTIVE_FAILURES
                    );
                    break;
                }

                // Exponential backoff on failure
                sleep(Duration::from_secs(backoff_secs)).await;
                backoff_secs = (backoff_secs.saturating_mul(2)).min(MAX_BACKOFF_SECS);
                continue;
            }
        }

        sleep(Duration::from_secs(CHECK_INTERVAL_SECS)).await;
    }
}

async fn refresh_all_profiles() -> Result<bool, Box<dyn std::error::Error>> {
    let profiles = load_auto_refresh_profiles()?;

    if profiles.is_empty() {
        return Ok(false);
    }

    tracing::info!("Checking {} auto-refresh profiles", profiles.len());

    for (name, profile) in &profiles {
        if should_refresh(profile) {
            tracing::info!("Refreshing profile: {}", name);
            if let Err(e) = refresh_profile(profile).await {
                tracing::error!("Failed to refresh {}: {}", name, e);
            }
        }
    }

    Ok(true)
}

fn get_auto_refresh_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".awswit")
        .join("autorefresh")
}

fn load_auto_refresh_profiles() -> Result<HashMap<String, AutoRefreshProfile>, Box<dyn std::error::Error>> {
    let dir = get_auto_refresh_dir();
    if !dir.exists() {
        return Ok(HashMap::new());
    }

    let mut profiles = HashMap::new();

    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.extension().map(|e| e == "json").unwrap_or(false) {
            let content = fs::read_to_string(&path)?;
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
        Err(_) => return false,
    };

    let now = Utc::now();
    let time_until_exp = exp_time - now;

    time_until_exp < chrono::Duration::minutes(REFRESH_WINDOW_MINS)
}

async fn refresh_profile(profile: &AutoRefreshProfile) -> Result<(), Box<dyn std::error::Error>> {
    // Re-run the awswit command to get new credentials
    if profile.awswit_command.is_empty() {
        return Err("Empty command".into());
    }

    let output = tokio::process::Command::new(&profile.awswit_command[0])
        .args(&profile.awswit_command[1..])
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Command failed: {}", stderr).into());
    }

    // Parse the output and update credentials
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Update the credentials file
    update_credentials_file(&profile.profile_name, &stdout)?;

    tracing::info!("Successfully refreshed {}", profile.profile_name);

    Ok(())
}

fn update_credentials_file(profile_name: &str, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    // Parse the output (key=value format)
    let mut creds: HashMap<String, String> = HashMap::new();

    for line in output.lines() {
        if let Some((key, value)) = line.split_once('=') {
            creds.insert(key.to_string(), value.to_string());
        }
    }

    let access_key = creds.get("AWS_ACCESS_KEY_ID").ok_or("Missing access key")?;
    let secret_key = creds.get("AWS_SECRET_ACCESS_KEY").ok_or("Missing secret key")?;
    let session_token = creds.get("AWS_SESSION_TOKEN");
    let expiration = creds.get("AWSWIT_EXPIRATION");

    // Read existing credentials file
    let creds_path = dirs::home_dir()
        .unwrap_or_default()
        .join(".aws")
        .join("credentials");

    // Acquire exclusive lock before reading/writing credentials file
    let lock_path = creds_path.with_extension("credentials.lock");
    let lock_file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .open(&lock_path)?;
    use fs2::FileExt;
    lock_file.lock_exclusive()?;

    let content = fs::read_to_string(&creds_path).unwrap_or_default();

    // Update the profile section
    let new_section = format!(
        "[{}]\n\
        aws_access_key_id = {}\n\
        aws_secret_access_key = {}\n\
        aws_session_token = {}\n\
        autoawswit = true\n\
        awswit_expiration = {}\n",
        profile_name,
        access_key,
        secret_key,
        session_token.map(|s| s.as_str()).unwrap_or(""),
        expiration.map(|s| s.as_str()).unwrap_or("")
    );

    // Simple approach: remove old section and add new
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
    if !new_content.ends_with('\n') {
        new_content.push('\n');
    }
    new_content.push_str(&new_section);

    fs::write(&creds_path, new_content)?;

    lock_file.unlock()?;

    // Update the profile metadata
    let profile_path = get_auto_refresh_dir().join(format!("{}.json", profile_name));
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
