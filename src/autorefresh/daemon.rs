// Lock ordering (to prevent deadlock):
//   1. daemon lock   (autoawswit.lock)
//   2. credentials lock (credentials.lock)
// Always acquire daemon lock first if both are needed.
// Exception: refresh_profile() (runner.rs) acquires only credentials lock
// without daemon lock, as it runs within the daemon process which inherently
// owns the daemon lifecycle.
// All public functions (start/stop/stop_all) acquire daemon lock first.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::credentials_file;
use crate::aws::Credentials;
use crate::cli::Args;
use crate::error::AwswitError;

/// Auto-refresh profile metadata
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AutoRefreshProfile {
    pub profile_name: String,
    pub awswit_command: Vec<String>,
    pub awswit_role_expiration: Option<String>,
    pub awswit_cache_name: Option<String>,
    pub aws_role_arn: Option<String>,
    pub region: Option<String>,
    #[serde(default = "default_profile_version")]
    pub version: u32,
    #[serde(default)]
    pub awswit_binary_version: Option<String>,
    /// Path to the AWS credentials file to write to.
    /// Captured at registration time so the daemon writes to the correct file
    /// even when `AWS_SHARED_CREDENTIALS_FILE` is not set in the daemon's environment.
    #[serde(default)]
    pub credentials_file_path: Option<String>,
}

fn default_profile_version() -> u32 {
    1
}

/// Start auto-refresh for a profile
pub async fn start_auto_refresh(
    profile_name: &str,
    args: &Args,
    credentials: &Credentials,
    requires_mfa: bool,
) -> Result<(), AwswitError> {
    tracing::info!("Starting auto-refresh for profile: {}", profile_name);

    // Check role duration limit
    if let Some(duration) = args.role_duration
        && duration > 3600
    {
        return Err(AwswitError::AutoRefreshDurationLimit);
    }

    // Refuse auto-refresh for MFA-protected profiles (daemon has no terminal for MFA prompt).
    if requires_mfa || args.mfa_token.is_some() {
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

    // Capture the credentials file path so the daemon writes to the correct location,
    // even if AWS_SHARED_CREDENTIALS_FILE is not set in the daemon's environment.
    let creds_path = credentials_file::get_aws_credentials_path()?;

    let auto_profile = AutoRefreshProfile {
        profile_name: auto_profile_name.clone(),
        awswit_command: command_parts,
        awswit_role_expiration: credentials.expiration.map(|e| e.to_rfc3339()),
        awswit_cache_name: Some(format!("session-{}", profile_name)),
        aws_role_arn: args.role_arn.clone(),
        region: credentials.region.clone(),
        version: default_profile_version(),
        awswit_binary_version: Some(env!("CARGO_PKG_VERSION").to_string()),
        credentials_file_path: Some(creds_path.to_string_lossy().to_string()),
    };

    // Save auto-refresh profile metadata
    save_auto_refresh_profile(&auto_profile)?;

    // Write credentials and spawn daemon off the async runtime to avoid
    // blocking the tokio executor with file-lock backoff sleeps.
    // Both operations run under daemon lock to maintain documented lock ordering:
    // daemon lock → credentials lock.
    let creds_path = credentials_file::get_aws_credentials_path()?;
    let write_profile_name = auto_profile_name.clone();
    let write_creds = credentials.clone();
    tokio::task::spawn_blocking(move || {
        let lock_path = get_daemon_lock_path()?;
        with_daemon_lock(&lock_path, || {
            credentials_file::write_credentials(&creds_path, &write_profile_name, &write_creds)?;
            spawn_daemon_if_not_running()
        })
    })
    .await
    .map_err(|e| AwswitError::AutoRefreshError {
        message: format!("Blocking task panicked: {}", e),
    })??;

    eprintln!(
        "Started auto-refresh for '{}'. Credentials will be refreshed automatically.",
        profile_name
    );

    Ok(())
}

/// Stop auto-refresh for a specific profile
pub fn stop_auto_refresh(profile_name: &str) -> Result<(), AwswitError> {
    tracing::info!("Stopping auto-refresh for profile: {}", profile_name);

    let auto_profile_name = format!("autoawswit-{}", profile_name);

    // Hold daemon lock for the entire stop operation to prevent TOCTOU
    let lock_path = get_daemon_lock_path()?;
    with_daemon_lock(&lock_path, || {
        // Remove the auto-refresh profile metadata
        remove_auto_refresh_profile(&auto_profile_name)?;

        // Remove from credentials file
        let creds_path = credentials_file::get_aws_credentials_path()?;
        credentials_file::remove_credentials(&creds_path, &auto_profile_name)?;

        // Check if any auto-refresh profiles remain
        let remaining = list_auto_refresh_profiles()?;
        if remaining.is_empty() {
            kill_autoawswit_daemon_inner()?;
        }

        Ok(())
    })?;

    Ok(())
}

/// Stop all auto-refresh processes
pub fn stop_all_auto_refresh() -> Result<(), AwswitError> {
    tracing::info!("Stopping all auto-refresh processes");

    // Hold daemon lock for the entire stop-all operation to prevent races
    // with concurrent start_auto_refresh calls.
    let lock_path = get_daemon_lock_path()?;
    with_daemon_lock(&lock_path, || {
        let profiles = list_auto_refresh_profiles()?;
        let creds_path = credentials_file::get_aws_credentials_path()?;

        // Remove all profile metadata files
        for profile in &profiles {
            remove_auto_refresh_profile(profile)?;
        }

        // Remove all credential sections in a single lock-read-write cycle
        credentials_file::remove_credentials_batch(&creds_path, &profiles)?;

        kill_autoawswit_daemon_inner()?;

        Ok(())
    })
}

/// Sanitize a profile name to prevent path traversal attacks.
/// Rejects names containing path separators or parent directory references.
fn sanitize_profile_name(name: &str) -> Result<&str, AwswitError> {
    if name.contains('/')
        || name.contains('\\')
        || name.contains("..")
        || name.contains('\0')
        || name.is_empty()
    {
        return Err(AwswitError::ValidationError {
            message: format!(
                "Invalid profile name '{}': must not contain path separators, '..', or null bytes",
                name
            ),
        });
    }
    Ok(name)
}

fn save_auto_refresh_profile(profile: &AutoRefreshProfile) -> Result<(), AwswitError> {
    let safe_name = sanitize_profile_name(&profile.profile_name)?;
    let dir = super::get_auto_refresh_dir()?;
    fs::create_dir_all(&dir)?;

    // Ensure restrictive permissions on the autorefresh directory (C3)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }

    let path = dir.join(format!("{}.json", safe_name));
    let content = serde_json::to_string_pretty(profile)?;

    crate::utils::fs::atomic_write_restricted(&path, content.as_bytes())?;

    Ok(())
}

fn remove_auto_refresh_profile(profile_name: &str) -> Result<(), AwswitError> {
    let safe_name = sanitize_profile_name(profile_name)?;
    let path = super::get_auto_refresh_dir()?.join(format!("{}.json", safe_name));
    if let Err(e) = fs::remove_file(&path)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        return Err(e.into());
    }
    Ok(())
}

fn list_auto_refresh_profiles() -> Result<Vec<String>, AwswitError> {
    let dir = super::get_auto_refresh_dir()?;
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
            && let Some(name) = entry.path().file_stem()
            && !name.is_empty()
        {
            profiles.push(name.to_string_lossy().to_string());
        }
    }

    Ok(profiles)
}

fn get_daemon_lock_path() -> Result<PathBuf, AwswitError> {
    let dir = super::get_auto_refresh_dir()?;
    let parent = dir.parent().ok_or_else(|| AwswitError::AutoRefreshError {
        message: "Autorefresh directory has no parent".to_string(),
    })?;
    Ok(parent.join("autoawswit.lock"))
}

/// Acquire the daemon advisory lock with timeout.
fn open_daemon_lock(lock_path: &std::path::Path) -> Result<fd_lock::RwLock<fs::File>, AwswitError> {
    crate::utils::fs::lock_file_with_permissions(lock_path).map_err(|e| {
        AwswitError::AutoRefreshError {
            message: format!("Failed to open daemon lock: {}", e),
        }
    })
}

fn with_daemon_lock<T, F>(lock_path: &std::path::Path, operation: F) -> Result<T, AwswitError>
where
    F: FnOnce() -> Result<T, AwswitError>,
{
    let mut daemon_lock = open_daemon_lock(lock_path)?;
    let mut result = None;
    crate::utils::fs::lock_exclusive_with_timeout(
        &mut daemon_lock,
        Duration::from_secs(30),
        |_daemon_guard| {
            result = Some(operation());
        },
    )
    .map_err(|e| AwswitError::AutoRefreshError {
        message: format!("Failed to acquire daemon lock: {}", e),
    })?;

    match result {
        Some(result) => result,
        None => Err(AwswitError::AutoRefreshError {
            message: "Daemon lock callback did not execute".to_string(),
        }),
    }
}

/// Spawn the daemon if not already running. Caller must already hold the daemon lock.
fn spawn_daemon_if_not_running() -> Result<(), AwswitError> {
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
            // Create a pipe for the daemon child to signal successful initialization.
            // This replaces PID-file polling which is timing-dependent and unreliable.
            #[cfg(unix)]
            let (read_fd, write_fd) = {
                let mut fds = [0i32; 2];
                if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
                    return Err(AwswitError::AutoRefreshError {
                        message: format!(
                            "Failed to create notification pipe: {}",
                            std::io::Error::last_os_error()
                        ),
                    });
                }
                (fds[0], fds[1])
            };

            let spawn_result = {
                let mut cmd = Command::new(&path);
                #[cfg(unix)]
                {
                    // Pass write fd to child via env var; the child will write a
                    // success/failure byte after init completes.
                    cmd.env("AWSWIT_NOTIFY_FD", write_fd.to_string());
                    // Ensure the write fd is inherited (not close-on-exec)
                    use std::os::unix::process::CommandExt;
                    unsafe {
                        cmd.pre_exec(move || {
                            // Clear FD_CLOEXEC on the write fd so it survives exec
                            let flags = libc::fcntl(write_fd, libc::F_GETFD);
                            if flags < 0 {
                                return Err(std::io::Error::last_os_error());
                            }
                            if libc::fcntl(write_fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                                return Err(std::io::Error::last_os_error());
                            }
                            Ok(())
                        });
                    }
                }
                cmd.spawn()
            };

            let mut child = spawn_result.map_err(|e| {
                #[cfg(unix)]
                {
                    unsafe {
                        libc::close(read_fd);
                        libc::close(write_fd);
                    }
                }
                AwswitError::AutoRefreshError {
                    message: format!("Failed to spawn daemon: {}", e),
                }
            })?;

            #[cfg(unix)]
            {
                // Close write end in parent so we get EOF if child dies without writing
                unsafe { libc::close(write_fd) };

                // Read from pipe with timeout — the child writes 1 byte:
                //   0x01 = success, 0x00 = failure, EOF = crashed
                use std::os::unix::io::FromRawFd;
                let read_file = unsafe { std::fs::File::from_raw_fd(read_fd) };

                let (tx, rx) = std::sync::mpsc::channel();
                let handle = std::thread::spawn(move || {
                    use std::io::Read;
                    let mut buf = [0u8; 1];
                    let mut f = read_file;
                    let result = f.read_exact(&mut buf);
                    let _ = tx.send(result.map(|()| buf[0]));
                });

                let timeout = std::time::Duration::from_secs(10);
                match rx.recv_timeout(timeout) {
                    Ok(Ok(1)) => {
                        let _ = handle.join();
                        tracing::info!("Spawned autoawswit daemon (pipe handshake confirmed)");
                        Ok(())
                    }
                    Ok(Ok(_)) => {
                        // Child reported failure
                        let _ = handle.join();
                        if let Err(e) = child.kill() {
                            tracing::warn!("Failed to kill daemon child process: {}", e);
                        }
                        if let Err(e) = child.wait() {
                            tracing::warn!("Failed to wait for daemon child process: {}", e);
                        }
                        Err(AwswitError::AutoRefreshError {
                            message: "Daemon reported initialization failure".to_string(),
                        })
                    }
                    Ok(Err(_)) | Err(_) => {
                        // EOF (child died) or timeout
                        let _ = handle.join();
                        if let Err(e) = child.kill() {
                            tracing::warn!("Failed to kill daemon child process: {}", e);
                        }
                        if let Err(e) = child.wait() {
                            tracing::warn!("Failed to wait for daemon child process: {}", e);
                        }
                        Err(AwswitError::AutoRefreshError {
                            message: "Daemon process failed to initialize (pipe handshake failed)"
                                .to_string(),
                        })
                    }
                }
            }

            // Non-unix fallback: poll for PID file
            #[cfg(not(unix))]
            {
                let pid_path = super::get_pid_file_path()?;
                let start = std::time::Instant::now();
                let timeout = std::time::Duration::from_secs(5);
                let poll_interval = std::time::Duration::from_millis(100);

                while start.elapsed() < timeout {
                    if pid_path.exists() {
                        tracing::info!("Spawned autoawswit daemon process (PID file confirmed)");
                        return Ok(());
                    }
                    std::thread::sleep(poll_interval);
                }

                if let Err(e) = child.kill() {
                    tracing::warn!("Failed to kill daemon child process: {}", e);
                }
                if let Err(e) = child.wait() {
                    tracing::warn!("Failed to wait for daemon child process: {}", e);
                }
                Err(AwswitError::AutoRefreshError {
                    message: format!(
                        "Daemon process failed to initialize within {:?} (PID file not written)",
                        timeout
                    ),
                })
            }
        }
        None => Err(AwswitError::AutoRefreshError {
            message: "autoawswit binary not found in the same directory as the current executable"
                .to_string(),
        }),
    }
}

/// Verify that a PID belongs to the autoawswit process.
/// Returns `Some(true)` if verified, `Some(false)` if different process, `None` if cannot determine.
#[cfg(unix)]
fn verify_process_identity(pid: u32) -> Option<bool> {
    #[cfg(target_os = "linux")]
    {
        let comm_path = format!("/proc/{}/comm", pid);
        match std::fs::read_to_string(&comm_path) {
            Ok(comm) if comm.trim() == "autoawswit" => Some(true),
            Ok(_) => Some(false),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(false), // process gone
            Err(_) => None, // cannot verify (EPERM, etc.)
        }
    }
    #[cfg(target_os = "macos")]
    {
        match std::process::Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "comm="])
            .output()
        {
            Ok(output) if output.status.success() => {
                let comm = String::from_utf8_lossy(&output.stdout);
                let name = comm.trim().rsplit('/').next().unwrap_or("");
                Some(name == "autoawswit")
            }
            Ok(_) => Some(false),
            Err(_) => None,
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = pid;
        None // cannot verify on this platform
    }
}

/// Kill the daemon. Caller must already hold the daemon lock.
fn kill_autoawswit_daemon_inner() -> Result<(), AwswitError> {
    let pid_path = super::get_pid_file_path()?;

    if let Some(pid) = read_pid(&pid_path) {
        #[cfg(unix)]
        {
            if let Ok(pid_i32) = i32::try_from(pid) {
                // Check if process exists at all
                let process_gone = if unsafe { libc::kill(pid_i32, 0) } != 0 {
                    let err = std::io::Error::last_os_error();
                    if err.raw_os_error() == Some(libc::ESRCH) {
                        // No such process — it's gone
                        true
                    } else {
                        // EPERM or other error — process exists but we can't signal it;
                        // conservatively assume it's alive, leave PID file alone
                        tracing::warn!(
                            "kill(0) for PID {} returned {}: assuming process is alive",
                            pid,
                            err
                        );
                        return Ok(());
                    }
                } else {
                    false
                };

                if process_gone {
                    remove_stale_pid_file(&pid_path);
                    return Ok(());
                }

                // Verify process identity using platform-specific helper
                match verify_process_identity(pid) {
                    Some(true) => {
                        // Verified as autoawswit — send SIGTERM
                        unsafe { libc::kill(pid_i32, libc::SIGTERM) };
                        tracing::info!("Sent SIGTERM to autoawswit daemon (pid={})", pid);

                        // Wait for the process to actually exit before removing PID file
                        let start = std::time::Instant::now();
                        let wait_timeout = std::time::Duration::from_secs(5);
                        let poll_interval = std::time::Duration::from_millis(100);
                        loop {
                            if unsafe { libc::kill(pid_i32, 0) } != 0 {
                                break;
                            }
                            if start.elapsed() >= wait_timeout {
                                tracing::warn!(
                                    "Daemon (pid={}) did not exit within {:?} after SIGTERM, sending SIGKILL",
                                    pid,
                                    wait_timeout
                                );
                                unsafe { libc::kill(pid_i32, libc::SIGKILL) };
                                // Reap the process to prevent zombie and PID reuse.
                                // Use WNOHANG loop since we may not be the direct parent.
                                crate::utils::process::reap_process(pid_i32);
                                break;
                            }
                            std::thread::sleep(poll_interval);
                        }

                        remove_stale_pid_file(&pid_path);
                    }
                    Some(false) => {
                        // Different process occupies this PID — clean up stale PID file
                        tracing::warn!(
                            "PID {} is not autoawswit — refusing to kill, removing stale PID file",
                            pid
                        );
                        remove_stale_pid_file(&pid_path);
                    }
                    None => {
                        // Cannot verify (non-Linux or EPERM on /proc) — refuse to kill
                        tracing::warn!(
                            "Cannot verify PID {} identity — refusing to send signal. \
                             Manually stop the daemon or remove the PID file at: {}",
                            pid,
                            pid_path.display()
                        );
                    }
                }
            } else {
                tracing::warn!("PID {} exceeds i32::MAX, cannot signal", pid);
            }
        }

        #[cfg(not(unix))]
        {
            let _ = pid;
            remove_stale_pid_file(&pid_path);
        }
    }

    Ok(())
}

/// Read PID from a PID file, returning None if missing or unparseable.
/// Logs a warning for errors other than NotFound (e.g., PermissionDenied).
fn read_pid(pid_path: &std::path::Path) -> Option<u32> {
    match fs::read_to_string(pid_path) {
        Ok(pid_str) => pid_str.trim().parse::<u32>().ok(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            tracing::warn!("Failed to read PID file {}: {}", pid_path.display(), e);
            None
        }
    }
}

/// Remove a stale PID file, logging non-NotFound errors.
fn remove_stale_pid_file(pid_path: &std::path::Path) {
    if let Err(e) = fs::remove_file(pid_path)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(
            "Failed to remove stale PID file {}: {}",
            pid_path.display(),
            e
        );
    }
}

/// Check if the autoawswit daemon is running, verifying process identity
fn is_autoawswit_running() -> Result<bool, AwswitError> {
    let pid_path = super::get_pid_file_path()?;

    let pid = match read_pid(&pid_path) {
        Some(p) => p,
        None => return Ok(false),
    };

    let pid_i32 = match i32::try_from(pid) {
        Ok(p) => p,
        Err(_) => {
            tracing::warn!("PID {} exceeds i32::MAX, removing stale PID file", pid);
            remove_stale_pid_file(&pid_path);
            return Ok(false);
        }
    };

    #[cfg(unix)]
    {
        // Verify process identity first (on platforms that support it)
        match verify_process_identity(pid) {
            Some(true) => {
                // Confirmed as autoawswit — check if still alive
            }
            Some(false) => {
                // Different process or process gone — stale PID file
                tracing::warn!("PID {} is not autoawswit — stale PID file", pid);
                remove_stale_pid_file(&pid_path);
                return Ok(false);
            }
            None => {
                // Cannot verify identity (non-Linux or EPERM on /proc).
                // Fall through to kill(0) check below. On non-Linux unix,
                // we can only confirm a process exists at this PID but cannot
                // verify it is autoawswit (PID recycling risk).
                tracing::debug!(
                    "Cannot verify process identity for PID {} — falling back to kill(0)",
                    pid
                );
            }
        }

        // Confirm the process is still alive
        if unsafe { libc::kill(pid_i32, 0) } != 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::ESRCH) {
                // No such process — remove stale PID file
                remove_stale_pid_file(&pid_path);
                return Ok(false);
            }
            // EPERM or other error — process exists but we can't signal it;
            // conservatively assume daemon is alive
            tracing::warn!(
                "kill(0) for PID {} returned {}: conservatively assuming daemon is alive",
                pid,
                err
            );
            return Ok(true);
        }

        Ok(true)
    }

    #[cfg(not(unix))]
    {
        let _ = pid_i32;
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use tempfile::TempDir;

    fn create_test_credentials_path() -> (TempDir, PathBuf) {
        let temp_dir = TempDir::new().unwrap();
        let creds_dir = temp_dir.path().join(".aws");
        fs::create_dir_all(&creds_dir).unwrap();
        (temp_dir, creds_dir.join("credentials"))
    }

    fn test_credentials() -> Credentials {
        Credentials {
            access_key_id: "AKIATEST".to_string(),
            secret_access_key: "secret".to_string(),
            session_token: Some("token".to_string()),
            expiration: Some(Utc::now() + Duration::hours(1)),
            region: Some("us-east-1".to_string()),
        }
    }

    #[test]
    fn auto_refresh_profile_does_not_serialize_secrets() {
        let profile = AutoRefreshProfile {
            profile_name: "test".to_string(),
            awswit_command: vec!["awswit".to_string(), "test".to_string()],
            awswit_role_expiration: None,
            awswit_cache_name: None,
            aws_role_arn: None,
            region: None,
            version: default_profile_version(),
            awswit_binary_version: Some("0.1.0".to_string()),
            credentials_file_path: None,
        };
        let json = serde_json::to_string(&profile).unwrap();
        assert!(!json.contains("secret_access_key"));
        assert!(!json.contains("session_token"));
        assert!(!json.contains("access_key_id"));
    }

    #[test]
    fn write_auto_refresh_credentials_replaces_existing_section_atomically() {
        let (_temp_dir, creds_path) = create_test_credentials_path();
        fs::write(
            &creds_path,
            "[default]\naws_access_key_id = ORIGINAL\n[autoawswit-test]\naws_access_key_id = OLD\n[other]\naws_access_key_id = OTHER\n",
        )
        .unwrap();

        credentials_file::write_credentials(&creds_path, "autoawswit-test", &test_credentials())
            .unwrap();

        let content = fs::read_to_string(&creds_path).unwrap();
        assert!(content.contains("[default]\naws_access_key_id = ORIGINAL\n"));
        assert!(content.contains("[other]\naws_access_key_id = OTHER\n"));
        assert!(content.contains("[autoawswit-test]\n"));
        assert_eq!(content.matches("[autoawswit-test]").count(), 1);
        assert!(content.contains("aws_access_key_id = AKIATEST\n"));
        assert!(content.contains("aws_secret_access_key = secret\n"));
        assert!(content.contains("aws_session_token = token\n"));
        assert!(content.contains("autoawswit = true\n"));
    }

    #[test]
    fn remove_auto_refresh_credentials_updates_file_without_direct_write() {
        let (_temp_dir, creds_path) = create_test_credentials_path();
        fs::write(
            &creds_path,
            "[default]\naws_access_key_id = ORIGINAL\n[autoawswit-test]\naws_access_key_id = OLD\n[other]\naws_access_key_id = OTHER\n",
        )
        .unwrap();

        credentials_file::remove_credentials(&creds_path, "autoawswit-test").unwrap();

        let content = fs::read_to_string(&creds_path).unwrap();
        assert!(content.contains("[default]\naws_access_key_id = ORIGINAL\n"));
        assert!(content.contains("[other]\naws_access_key_id = OTHER\n"));
        assert!(!content.contains("[autoawswit-test]"));
    }
}
