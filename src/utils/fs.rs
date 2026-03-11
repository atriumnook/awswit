use std::io;
use std::path::Path;

/// Atomically write content to a file with restrictive permissions (0o600 on Unix).
/// Uses write-to-temp + rename to avoid partial writes.
pub fn atomic_write_restricted(path: &Path, content: &[u8]) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "path has no parent directory")
    })?;

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp_path = parent.join(format!(
        ".{}.{}.{}.tmp",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file"),
        std::process::id(),
        nanos
    ));

    // Write to temp file with restrictive permissions
    let write_and_rename = || -> io::Result<()> {
        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&tmp_path)?;
            file.write_all(content)?;
            file.sync_all()?;
        }

        #[cfg(not(unix))]
        {
            std::fs::write(&tmp_path, content)?;
        }

        // Atomic rename
        std::fs::rename(&tmp_path, path)?;
        Ok(())
    };

    if let Err(e) = write_and_rename() {
        // Clean up temp file on failure; ignore cleanup errors
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e);
    }

    Ok(())
}
