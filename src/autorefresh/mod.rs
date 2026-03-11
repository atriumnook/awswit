pub mod credentials_file;
mod daemon;
pub mod runner;

pub use daemon::{start_auto_refresh, stop_all_auto_refresh, stop_auto_refresh};

use std::path::PathBuf;

/// Get the autorefresh directory path (~/.awswit/autorefresh/)
pub(crate) fn get_auto_refresh_dir() -> Result<PathBuf, crate::error::AwswitError> {
    crate::utils::paths::awswit_home_dir()
        .map(|p| p.join("autorefresh"))
        .map_err(|e| crate::error::AwswitError::AutoRefreshError {
            message: e.to_string(),
        })
}

/// Get the PID file path (~/.awswit/autoawswit.pid)
pub(crate) fn get_pid_file_path() -> Result<PathBuf, crate::error::AwswitError> {
    crate::utils::paths::awswit_home_dir()
        .map(|p| p.join("autoawswit.pid"))
        .map_err(|e| crate::error::AwswitError::AutoRefreshError {
            message: e.to_string(),
        })
}
