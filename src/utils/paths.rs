use std::path::{Path, PathBuf};

/// XDG-compliant directory for awswit state (history.json lives here).
///
/// Resolution order:
///   1. `$XDG_DATA_HOME/awswit/`
///   2. `~/.local/share/awswit/`
///
/// If a pre-0.1.0 file exists at `~/.awswit/`, callers may migrate it on
/// first access via [`migrate_legacy_into`].
pub fn data_dir() -> Result<PathBuf, std::io::Error> {
    if let Some(p) = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(p).join("awswit"));
    }
    let home = home_dir()?;
    Ok(home.join(".local").join("share").join("awswit"))
}

/// Legacy `~/.awswit/` directory used by awswit before 0.1.0. We only read
/// from it for one-time migration.
pub fn legacy_dir() -> Result<PathBuf, std::io::Error> {
    Ok(home_dir()?.join(".awswit"))
}

fn home_dir() -> Result<PathBuf, std::io::Error> {
    dirs::home_dir().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Could not determine home directory",
        )
    })
}

/// If `legacy` exists and `target` does not, move the legacy file into place
/// so existing users keep their history. Returns the path actually used.
pub fn migrate_legacy_into(legacy: &Path, target: &Path) -> std::io::Result<()> {
    if target.exists() || !legacy.exists() {
        return Ok(());
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Best-effort rename; fall back to copy + delete across filesystems.
    if std::fs::rename(legacy, target).is_err() {
        std::fs::copy(legacy, target)?;
        let _ = std::fs::remove_file(legacy);
    }
    Ok(())
}
