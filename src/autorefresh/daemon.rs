use std::fs;
use std::path::PathBuf;
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::aws::Credentials;
use crate::cli::Args;
use crate::error::AwswitError;

/// Auto-refresh profile metadata
#[derive(Clone, Serialize, Deserialize)]
pub struct AutoRefreshProfile {
    pub profile_name: String,
    pub awswit_command: Vec<String>,
    pub aws_access_key_id: String,
    pub aws_secret_access_key: String,
    pub aws_session_token: Option<String>,
    pub awswit_role_expiration: Option<String>,
    pub awswit_cache_name: Option<String>,
    pub aws_role_arn: Option<String>,
    pub region: Option<String>,
}

impl std::fmt::Debug for AutoRefreshProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AutoRefreshProfile")
            .field("profile_name", &self.profile_name)
            .field("awswit_command", &self.awswit_command)
            .field("aws_access_key_id", &"[REDACTED]")
            .field("aws_secret_access_key", &"[REDACTED]")
            .field(
                "aws_session_token",
                &self.aws_session_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("awswit_role_expiration", &self.awswit_role_expiration)
            .field("awswit_cache_name", &self.awswit_cache_name)
            .field("aws_role_arn", &self.aws_role_arn)
            .field("region", &self.region)
            .finish()
    }
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

    // Refuse auto-refresh for MFA-protected profiles (daemon has no terminal for MFA prompt).
    // Limitation: this only checks if an MFA token was explicitly provided on the CLI.
    // We cannot check `mfa_serial` in the profile config here because this function only
    // receives Args and Credentials, not the full parsed profile map. Adding a `profiles`
    // parameter would require threading the profile map through `start_auto_refresh` and
    // all its callers. The caller (main.rs) should guard against auto-refresh for
    // MFA-protected profiles before reaching this point.
    if args.mfa_token.is_some() {
        return Err(AwswitError::AutoRefreshError {
            message: "Auto-refresh is not supported for MFA-protected profiles. \
                     The daemon cannot prompt for MFA tokens."
                .to_string(),
        });
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
    if let Some(ref role_arn) = args.role_arn {
        command_parts.push("--role-arn".to_string());
        command_parts.push(role_arn.clone());
    }
    if let Some(ref source_profile) = args.source_profile {
        command_parts.push("--source-profile".to_string());
        command_parts.push(source_profile.clone());
    }
    if let Some(role_duration) = args.role_duration {
        command_parts.push("--role-duration".to_string());
        command_parts.push(role_duration.to_string());
    }

    let auto_profile = AutoRefreshProfile {
        profile_name: auto_profile_name.clone(),
        awswit_command: command_parts,
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
        kill_autoawswit_daemon()?;
    }

    Ok(())
}

/// Stop all auto-refresh processes
pub async fn stop_all_auto_refresh() -> Result<(), AwswitError> {
    tracing::info!("Stopping all auto-refresh processes");

    let profiles = list_auto_refresh_profiles()?;

    for profile in profiles {
        remove_auto_refresh_profile(&profile)?;
        remove_auto_refresh_credentials(&profile)?;
    }

    kill_autoawswit_daemon()?;

    Ok(())
}

fn get_auto_refresh_dir() -> Result<PathBuf, AwswitError> {
    let home = dirs::home_dir().ok_or_else(|| AwswitError::AutoRefreshError {
        message: "Could not determine home directory".to_string(),
    })?;
    Ok(home.join(".awswit").join("autorefresh"))
}

fn save_auto_refresh_profile(profile: &AutoRefreshProfile) -> Result<(), AwswitError> {
    let dir = get_auto_refresh_dir()?;
    fs::create_dir_all(&dir)?;

    // Ensure restrictive permissions on the autorefresh directory (C3)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }

    let path = dir.join(format!("{}.json", profile.profile_name));
    let content = serde_json::to_string_pretty(profile)?;

    crate::utils::fs::atomic_write_restricted(&path, content.as_bytes())?;

    Ok(())
}

fn remove_auto_refresh_profile(profile_name: &str) -> Result<(), AwswitError> {
    let path = get_auto_refresh_dir()?.join(format!("{}.json", profile_name));
    if path.exists() {
        fs::remove_file(&path)?;
    }
    Ok(())
}

fn list_auto_refresh_profiles() -> Result<Vec<String>, AwswitError> {
    let dir = get_auto_refresh_dir()?;
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut profiles = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        if entry
            .path()
            .extension()
            .map(|e| e == "json")
            .unwrap_or(false)
        {
            if let Some(name) = entry.path().file_stem() {
                profiles.push(name.to_string_lossy().to_string());
            }
        }
    }

    Ok(profiles)
}

fn write_auto_refresh_credentials(
    profile_name: &str,
    creds: &Credentials,
) -> Result<(), AwswitError> {
    use fs2::FileExt;

    let home = dirs::home_dir().ok_or_else(|| AwswitError::AutoRefreshError {
        message: "Could not determine home directory".to_string(),
    })?;
    let creds_path = home.join(".aws").join("credentials");

    // Lock the credentials file to prevent concurrent corruption
    let lock_file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(creds_path.with_extension("lock"))?;
    lock_file.lock_exclusive()?;
    // Lock is released on Drop — no manual unlock needed, which also ensures
    // cleanup on early-return error paths.

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

    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&creds_path)?;
        file.write_all(new_content.as_bytes())?;
    }
    #[cfg(not(unix))]
    {
        fs::write(&creds_path, new_content)?;
    }

    Ok(())
}

fn remove_auto_refresh_credentials(profile_name: &str) -> Result<(), AwswitError> {
    let home = dirs::home_dir().ok_or_else(|| AwswitError::AutoRefreshError {
        message: "Could not determine home directory".to_string(),
    })?;
    let creds_path = home.join(".aws").join("credentials");

    if !creds_path.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(&creds_path)?;

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

fn get_pid_file_path() -> Result<PathBuf, AwswitError> {
    let dir = get_auto_refresh_dir()?;
    let parent = dir.parent().ok_or_else(|| AwswitError::AutoRefreshError {
        message: "Autorefresh directory has no parent".to_string(),
    })?;
    Ok(parent.join("autoawswit.pid"))
}

fn get_daemon_lock_path() -> Result<PathBuf, AwswitError> {
    let dir = get_auto_refresh_dir()?;
    let parent = dir.parent().ok_or_else(|| AwswitError::AutoRefreshError {
        message: "Autorefresh directory has no parent".to_string(),
    })?;
    Ok(parent.join("autoawswit.lock"))
}

fn spawn_autoawswit_daemon() -> Result<(), AwswitError> {
    use fs2::FileExt;

    // Advisory lock protects the entire is_running → spawn sequence
    let lock_path = get_daemon_lock_path()?;
    let lock_file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|e| AwswitError::AutoRefreshError {
            message: format!("Failed to open lock: {}", e),
        })?;
    lock_file
        .lock_exclusive()
        .map_err(|e| AwswitError::AutoRefreshError {
            message: format!("Failed to acquire lock: {}", e),
        })?;

    // Lock is released on Drop — no manual unlock needed, which also ensures
    // cleanup on early-return error paths.

    // Check if already running (under lock)
    if is_autoawswit_running()? {
        tracing::debug!("Autoawswit daemon already running");
        return Ok(());
    }

    // Spawn the daemon process
    let exe = std::env::current_exe().map_err(|e| AwswitError::AutoRefreshError {
        message: e.to_string(),
    })?;

    // Look for autoawswit binary in same directory
    let autoawswit_path = exe
        .parent()
        .map(|p| p.join("autoawswit"))
        .filter(|p| p.exists());

    match autoawswit_path {
        Some(path) => {
            let _child = Command::new(path)
                .spawn()
                .map_err(|e| AwswitError::AutoRefreshError {
                    message: format!("Failed to spawn daemon: {}", e),
                })?;

            // Daemon writes its own PID after successful init — do NOT write from parent.
            // The daemon may crash immediately; writing PID here would leave a stale file.
            tracing::info!("Spawned autoawswit daemon process");
            Ok(())
        }
        None => Err(AwswitError::AutoRefreshError {
            message: "autoawswit binary not found in the same directory as the current executable"
                .to_string(),
        }),
    }
}

fn kill_autoawswit_daemon() -> Result<(), AwswitError> {
    use fs2::FileExt;

    let lock_path = get_daemon_lock_path()?;
    let lock_file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|e| AwswitError::AutoRefreshError {
            message: format!("Failed to open lock: {}", e),
        })?;
    lock_file
        .lock_exclusive()
        .map_err(|e| AwswitError::AutoRefreshError {
            message: format!("Failed to acquire lock: {}", e),
        })?;
    // Lock is released on Drop — no manual unlock needed.

    let pid_path = get_pid_file_path()?;

    if let Some(pid) = read_pid(&pid_path) {
        #[cfg(unix)]
        {
            if let Ok(pid_i32) = i32::try_from(pid) {
                // Verify the process is actually autoawswit before sending signal
                let mut verified = false;
                let mut process_gone = false;

                // Check if process exists at all
                if unsafe { libc::kill(pid_i32, 0) } != 0 {
                    process_gone = true;
                }

                #[cfg(target_os = "linux")]
                if !process_gone {
                    let comm_path = format!("/proc/{}/comm", pid);
                    match fs::read_to_string(&comm_path) {
                        Ok(comm) if comm.trim() == "autoawswit" => {
                            verified = true;
                        }
                        Ok(comm) => {
                            tracing::warn!(
                                "PID {} is '{}', not autoawswit — refusing to kill",
                                pid,
                                comm.trim()
                            );
                        }
                        Err(_) => {
                            tracing::warn!("Cannot read /proc/{}/comm — process may be gone", pid);
                            process_gone = true;
                        }
                    }
                }

                // On non-Linux unix, fall back to kill without /proc verification
                #[cfg(not(target_os = "linux"))]
                if !process_gone {
                    verified = true;
                }

                if verified {
                    unsafe { libc::kill(pid_i32, libc::SIGTERM) };
                    tracing::info!("Killed autoawswit daemon (pid={})", pid);
                    let _ = fs::remove_file(&pid_path);
                } else if process_gone {
                    // Process is confirmed gone, safe to clean up PID file
                    let _ = fs::remove_file(&pid_path);
                }
                // If not verified and not gone, leave PID file intact
            } else {
                tracing::warn!("PID {} exceeds i32::MAX, cannot signal", pid);
            }
        }

        #[cfg(not(unix))]
        {
            let _ = pid;
            let _ = fs::remove_file(&pid_path);
        }
    }

    Ok(())
}

/// Read PID from a PID file, returning None if missing or unparseable
fn read_pid(pid_path: &std::path::Path) -> Option<u32> {
    let pid_str = fs::read_to_string(pid_path).ok()?;
    pid_str.trim().parse::<u32>().ok()
}

/// Check if the autoawswit daemon is running, verifying process identity
fn is_autoawswit_running() -> Result<bool, AwswitError> {
    let pid_path = get_pid_file_path()?;

    let pid = match read_pid(&pid_path) {
        Some(p) => p,
        None => return Ok(false),
    };

    let pid_i32 = match i32::try_from(pid) {
        Ok(p) => p,
        Err(_) => {
            tracing::warn!("PID {} exceeds i32::MAX, removing stale PID file", pid);
            let _ = fs::remove_file(&pid_path);
            return Ok(false);
        }
    };

    #[cfg(unix)]
    {
        // Check if process exists
        if unsafe { libc::kill(pid_i32, 0) } != 0 {
            // Process is gone, clean up stale PID file
            let _ = fs::remove_file(&pid_path);
            return Ok(false);
        }

        // On Linux, verify the process is actually autoawswit (not a recycled PID)
        // via /proc/{pid}/comm. On other unix platforms, fall back to the kill(0)
        // check above — no /proc filesystem is available.
        #[cfg(target_os = "linux")]
        {
            let comm_path = format!("/proc/{}/comm", pid);
            match fs::read_to_string(&comm_path) {
                Ok(comm) => {
                    if comm.trim() != "autoawswit" {
                        tracing::warn!(
                            "PID {} is '{}', not autoawswit — stale PID file",
                            pid,
                            comm.trim()
                        );
                        let _ = fs::remove_file(&pid_path);
                        return Ok(false);
                    }
                }
                Err(_) => {
                    // /proc not available or process gone between kill(0) and read
                    let _ = fs::remove_file(&pid_path);
                    return Ok(false);
                }
            }
        }

        Ok(true)
    }

    #[cfg(not(unix))]
    {
        let _ = pid_i32;
        Ok(false)
    }
}
