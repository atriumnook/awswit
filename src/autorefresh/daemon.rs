use std::fs;
use std::path::PathBuf;
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::aws::Credentials;
use crate::cli::Args;
use crate::error::AwswitError;

/// Auto-refresh profile metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoRefreshProfile {
    pub profile_name: String,
    pub awswit_command: String,
    pub aws_access_key_id: String,
    pub aws_secret_access_key: String,
    pub aws_session_token: Option<String>,
    pub awswit_role_expiration: Option<String>,
    pub awswit_cache_name: Option<String>,
    pub aws_role_arn: Option<String>,
    pub region: Option<String>,
}

/// Start auto-refresh for a profile
pub async fn start_auto_refresh(
    profile_name: &str,
    args: &Args,
    credentials: &Credentials,
) -> Result<(), AwswitError> {
    tracing::info!("Starting auto-refresh for profile: {}", profile_name);

    // Check role duration limit
    if let Some(duration) = args.role_duration {
        if duration > 3600 {
            return Err(AwswitError::AutoRefreshDurationLimit);
        }
    }

    // Refuse auto-refresh for MFA-protected profiles (daemon has no terminal for MFA prompt)
    if args.mfa_token.is_some() {
        return Err(AwswitError::AutoRefreshError(
            "Auto-refresh is not supported for MFA-protected profiles. \
             The daemon cannot prompt for MFA tokens.".to_string()
        ));
    }

    // Create auto-refresh profile
    let auto_profile_name = format!("autoawswit-{}", profile_name);
    
    // Build the awswit command to replay
    let mut command_parts = vec!["awswit".to_string()];
    command_parts.push(profile_name.to_string());
    
    if let Some(ref region) = args.region {
        command_parts.push("--region".to_string());
        command_parts.push(region.clone());
    }
    if let Some(ref session_name) = args.session_name {
        command_parts.push("--session-name".to_string());
        command_parts.push(session_name.clone());
    }
    if let Some(ref external_id) = args.external_id {
        command_parts.push("--external-id".to_string());
        command_parts.push(external_id.clone());
    }

    let auto_profile = AutoRefreshProfile {
        profile_name: auto_profile_name.clone(),
        awswit_command: command_parts.join(" "),
        aws_access_key_id: credentials.access_key_id.clone(),
        aws_secret_access_key: credentials.secret_access_key.clone(),
        aws_session_token: credentials.session_token.clone(),
        awswit_role_expiration: credentials.expiration.map(|e| e.to_rfc3339()),
        awswit_cache_name: Some(format!("session-{}", credentials.access_key_id)),
        aws_role_arn: args.role_arn.clone(),
        region: credentials.region.clone(),
    };

    // Save auto-refresh profile metadata
    save_auto_refresh_profile(&auto_profile)?;

    // Write credentials to credentials file with auto-refresh prefix
    write_auto_refresh_credentials(&auto_profile_name, credentials)?;

    // Spawn the autoawswit daemon if not already running
    spawn_autoawswit_daemon()?;

    eprintln!(
        "Started auto-refresh for '{}'. Credentials will be refreshed automatically.",
        profile_name
    );

    Ok(())
}

/// Stop auto-refresh for a specific profile
pub async fn stop_auto_refresh(profile_name: &str) -> Result<(), AwswitError> {
    tracing::info!("Stopping auto-refresh for profile: {}", profile_name);

    let auto_profile_name = format!("autoawswit-{}", profile_name);

    // Remove the auto-refresh profile metadata
    remove_auto_refresh_profile(&auto_profile_name)?;

    // Remove from credentials file
    remove_auto_refresh_credentials(&auto_profile_name)?;

    // Check if any auto-refresh profiles remain
    let remaining = list_auto_refresh_profiles()?;
    if remaining.is_empty() {
        // Kill the daemon
        kill_autoawswit_daemon()?;
    }

    Ok(())
}

/// Stop all auto-refresh processes
pub async fn stop_all_auto_refresh() -> Result<(), AwswitError> {
    tracing::info!("Stopping all auto-refresh processes");

    // Get all auto-refresh profiles
    let profiles = list_auto_refresh_profiles()?;

    for profile in profiles {
        remove_auto_refresh_profile(&profile)?;
        remove_auto_refresh_credentials(&profile)?;
    }

    // Kill the daemon
    kill_autoawswit_daemon()?;

    Ok(())
}

fn get_auto_refresh_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".awswit")
        .join("autorefresh")
}

fn save_auto_refresh_profile(profile: &AutoRefreshProfile) -> Result<(), AwswitError> {
    let dir = get_auto_refresh_dir();
    fs::create_dir_all(&dir)?;

    let path = dir.join(format!("{}.json", profile.profile_name));
    let content = serde_json::to_string_pretty(profile)?;

    // Write with restrictive permissions since file contains secrets
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)?;
        file.write_all(content.as_bytes())?;
    }

    #[cfg(not(unix))]
    {
        fs::write(&path, content)?;
    }

    Ok(())
}

fn remove_auto_refresh_profile(profile_name: &str) -> Result<(), AwswitError> {
    let path = get_auto_refresh_dir().join(format!("{}.json", profile_name));
    if path.exists() {
        fs::remove_file(&path)?;
    }
    Ok(())
}

fn list_auto_refresh_profiles() -> Result<Vec<String>, AwswitError> {
    let dir = get_auto_refresh_dir();
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut profiles = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        if entry.path().extension().map(|e| e == "json").unwrap_or(false) {
            if let Some(name) = entry.path().file_stem() {
                profiles.push(name.to_string_lossy().to_string());
            }
        }
    }

    Ok(profiles)
}

fn write_auto_refresh_credentials(profile_name: &str, creds: &Credentials) -> Result<(), AwswitError> {
    use fs2::FileExt;
    use std::io::Write;

    let creds_path = dirs::home_dir()
        .unwrap_or_default()
        .join(".aws")
        .join("credentials");

    // Lock the credentials file to prevent concurrent corruption
    let lock_file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .open(creds_path.with_extension("lock"))?;
    lock_file.lock_exclusive()?;

    let content = fs::read_to_string(&creds_path).unwrap_or_default();

    // Remove existing section to avoid duplicates
    let section_header = format!("[{}]", profile_name);
    let lines: Vec<&str> = content.lines().collect();
    let mut new_lines = Vec::new();
    let mut skip = false;

    for line in &lines {
        if line.starts_with('[') {
            skip = line.trim() == section_header;
        }
        if !skip {
            new_lines.push(*line);
        }
    }

    let mut new_content = new_lines.join("\n");
    if !new_content.ends_with('\n') && !new_content.is_empty() {
        new_content.push('\n');
    }

    let new_section = format!(
        "[{}]\n\
        aws_access_key_id = {}\n\
        aws_secret_access_key = {}\n\
        aws_session_token = {}\n\
        autoawswit = true\n\
        awswit_expiration = {}\n",
        profile_name,
        creds.access_key_id,
        creds.secret_access_key,
        creds.session_token.as_deref().unwrap_or(""),
        creds.expiration.map(|e| e.to_rfc3339()).unwrap_or_default()
    );

    new_content.push_str(&new_section);
    fs::write(&creds_path, new_content)?;

    lock_file.unlock()?;

    Ok(())
}

fn remove_auto_refresh_credentials(profile_name: &str) -> Result<(), AwswitError> {
    let creds_path = dirs::home_dir()
        .unwrap_or_default()
        .join(".aws")
        .join("credentials");

    if !creds_path.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(&creds_path)?;

    // Removal: find exact [profile_name] section and remove until next section
    let section_header = format!("[{}]", profile_name);
    let lines: Vec<&str> = content.lines().collect();
    let mut new_lines = Vec::new();
    let mut skip = false;

    for line in lines {
        if line.starts_with('[') {
            skip = line.trim() == section_header;
        }
        if !skip {
            new_lines.push(line);
        }
    }

    fs::write(&creds_path, new_lines.join("\n"))?;

    Ok(())
}

fn get_pid_file_path() -> PathBuf {
    get_auto_refresh_dir().parent()
        .unwrap_or_else(|| std::path::Path::new("/tmp"))
        .join("autoawswit.pid")
}

fn spawn_autoawswit_daemon() -> Result<(), AwswitError> {
    // Check if already running via PID file
    if is_autoawswit_running() {
        tracing::debug!("Autoawswit daemon already running");
        return Ok(());
    }

    // Spawn the daemon process
    let exe = std::env::current_exe()
        .map_err(|e| AwswitError::AutoRefreshError(e.to_string()))?;

    // Look for autoawswit binary in same directory
    let autoawswit_path = exe.parent()
        .map(|p| p.join("autoawswit"))
        .filter(|p| p.exists());

    if let Some(path) = autoawswit_path {
        let child = Command::new(path)
            .spawn()
            .map_err(|e| AwswitError::AutoRefreshError(format!("Failed to spawn daemon: {}", e)))?;

        // Write PID file
        let pid_path = get_pid_file_path();
        let _ = fs::write(&pid_path, child.id().to_string());

        tracing::info!("Started autoawswit daemon (pid={})", child.id());
    } else {
        tracing::warn!("autoawswit binary not found");
    }

    Ok(())
}

fn kill_autoawswit_daemon() -> Result<(), AwswitError> {
    let pid_path = get_pid_file_path();

    if let Ok(pid_str) = fs::read_to_string(&pid_path) {
        if let Ok(pid) = pid_str.trim().parse::<u32>() {
            #[cfg(unix)]
            {
                unsafe { libc::kill(pid as i32, libc::SIGTERM) };
            }
            tracing::info!("Killed autoawswit daemon (pid={})", pid);
        }
        let _ = fs::remove_file(&pid_path);
    }

    Ok(())
}

fn is_autoawswit_running() -> bool {
    let pid_path = get_pid_file_path();

    if let Ok(pid_str) = fs::read_to_string(&pid_path) {
        if let Ok(pid) = pid_str.trim().parse::<i32>() {
            #[cfg(unix)]
            {
                // kill(pid, 0) checks if process exists without sending a signal
                if unsafe { libc::kill(pid, 0) } == 0 {
                    return true;
                }
            }
            // Process is gone, clean up stale PID file
            let _ = fs::remove_file(&pid_path);
        }
    }

    false
}
