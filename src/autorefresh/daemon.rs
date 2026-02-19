use std::path::PathBuf;
use std::process::Command;

use sysinfo::{System, SystemExt, ProcessExt, PidExt};

use crate::config::AwswitConfig;
use crate::error::{AwswitError, Result};

const DAEMON_PID_FILE: &str = "autoawswit.pid";

/// Get the path to the PID file for the auto-refresh daemon
fn pid_file_path() -> PathBuf {
    AwswitConfig::config_dir().join(DAEMON_PID_FILE)
}

/// Start the auto-refresh daemon for a profile
pub fn start_daemon(profile_name: &str) -> Result<()> {
    // Check if daemon is already running
    if is_daemon_running() {
        kill_daemon()?;
    }

    let exe = std::env::current_exe().map_err(|e| {
        AwswitError::Other(format!("Failed to get current executable path: {}", e))
    })?;

    // The daemon binary name
    let daemon_exe = exe
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("autoawswit");

    if !daemon_exe.exists() {
        // Fall back to using the main binary with special env var
        let child = Command::new(&exe)
            .env("AWSWIT_DAEMON_MODE", "1")
            .env("AWSWIT_DAEMON_PROFILE", profile_name)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| AwswitError::Other(format!("Failed to start daemon: {}", e)))?;

        save_pid(child.id())?;
    } else {
        let child = Command::new(&daemon_exe)
            .arg(profile_name)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| AwswitError::Other(format!("Failed to start daemon: {}", e)))?;

        save_pid(child.id())?;
    }

    Ok(())
}

/// Kill the running auto-refresh daemon
pub fn kill_daemon() -> Result<()> {
    let pid_path = pid_file_path();
    if !pid_path.exists() {
        return Ok(());
    }

    let pid_str = std::fs::read_to_string(&pid_path)
        .map_err(|e| AwswitError::Other(format!("Failed to read PID file: {}", e)))?;

    let pid: u32 = pid_str.trim().parse().map_err(|e| {
        AwswitError::Other(format!("Invalid PID in file: {}", e))
    })?;

    // Try to kill the process
    #[cfg(unix)]
    {
        unsafe {
            libc::kill(pid as i32, libc::SIGTERM);
        }
    }

    #[cfg(not(unix))]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .output();
    }

    let _ = std::fs::remove_file(&pid_path);

    Ok(())
}

/// Check if the daemon is currently running
pub fn is_daemon_running() -> bool {
    let pid_path = pid_file_path();
    if !pid_path.exists() {
        return false;
    }

    let pid_str = match std::fs::read_to_string(&pid_path) {
        Ok(s) => s,
        Err(_) => return false,
    };

    let pid: u32 = match pid_str.trim().parse() {
        Ok(p) => p,
        Err(_) => {
            let _ = std::fs::remove_file(&pid_path);
            return false;
        }
    };

    // Check if process exists
    let sys = System::new_all();
    let sysinfo_pid = sysinfo::Pid::from_u32(pid);
    if let Some(process) = sys.process(sysinfo_pid) {
        let name = process.name().to_lowercase();
        name.contains("awswit") || name.contains("autoawswit")
    } else {
        let _ = std::fs::remove_file(&pid_path);
        false
    }
}

/// Save the daemon PID to file
fn save_pid(pid: u32) -> Result<()> {
    let pid_path = pid_file_path();
    let dir = pid_path.parent().unwrap();
    if !dir.exists() {
        AwswitConfig::ensure_config_dir()?;
    }

    std::fs::write(&pid_path, pid.to_string())
        .map_err(|e| AwswitError::Other(format!("Failed to write PID file: {}", e)))?;

    Ok(())
}

/// Run the auto-refresh daemon loop
pub async fn run_daemon_loop(profile_name: &str) -> Result<()> {
    use crate::cache::CacheManager;
    use crate::config::{self, AwswitConfig};
    use crate::profile::ProfileResolver;

    let config = AwswitConfig::load()?;
    let profiles = config::load_profiles(None, None)?;
    let cache_manager = CacheManager::new()?;

    let resolver = ProfileResolver::new(profiles, cache_manager, config.clone(), true);

    loop {
        tracing::info!("Auto-refreshing credentials for profile: {}", profile_name);

        match resolver.resolve(profile_name, None).await {
            Ok(creds) => {
                tracing::info!(
                    "Credentials refreshed, expires: {:?}",
                    creds.expiration
                );

                // Calculate sleep duration (refresh at 75% of remaining time)
                let sleep_secs = if let Some(exp) = creds.expiration {
                    let remaining = (exp - chrono::Utc::now()).num_seconds();
                    if remaining <= 0 {
                        60
                    } else {
                        (remaining as u64 * 3) / 4
                    }
                } else {
                    3600 // Default: refresh every hour
                };

                tracing::info!("Sleeping for {} seconds before next refresh", sleep_secs);
                tokio::time::sleep(tokio::time::Duration::from_secs(sleep_secs)).await;
            }
            Err(e) => {
                tracing::error!("Failed to refresh credentials: {}", e);
                // Retry after 60 seconds on error
                tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
            }
        }
    }
}
