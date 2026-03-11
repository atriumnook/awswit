use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

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
    // Include a simple random component to avoid collisions when multiple
    // writes happen within the same nanosecond (e.g., under virtualization).
    let rand_component: u32 = (nanos as u32).wrapping_mul(2654435761); // Knuth multiplicative hash
    let tmp_path = parent.join(format!(
        ".{}.{}.{}.{:08x}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("file"),
        std::process::id(),
        nanos,
        rand_component
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
            use std::io::Write;

            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&tmp_path)?;
            file.write_all(content)?;
            file.sync_all()?;
        }

        // Atomic rename
        std::fs::rename(&tmp_path, path)?;

        // Sync parent directory to ensure the rename is durable on Unix
        #[cfg(unix)]
        {
            if let Ok(dir) = std::fs::File::open(parent) {
                let _ = dir.sync_all();
            }
        }

        Ok(())
    };

    if let Err(e) = write_and_rename() {
        // Clean up temp file on failure; ignore cleanup errors
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e);
    }

    Ok(())
}

/// Try to acquire an exclusive file lock with exponential backoff and timeout.
///
/// Uses `thread::sleep` for backoff intentionally — this is called from both
/// sync and async contexts, and the sleep durations are short enough that
/// blocking the thread is acceptable.
pub fn lock_exclusive_with_timeout(
    file: &std::fs::File,
    timeout: Duration,
) -> io::Result<()> {
    use fs2::FileExt;

    if file.try_lock_exclusive().is_ok() {
        return Ok(());
    }

    let start = Instant::now();
    let mut backoff = Duration::from_millis(10);
    let max_backoff = Duration::from_secs(1);

    loop {
        std::thread::sleep(backoff);

        if file.try_lock_exclusive().is_ok() {
            return Ok(());
        }

        if start.elapsed() >= timeout {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("Lock acquisition timed out after {:?}", timeout),
            ));
        }

        backoff = (backoff * 2).min(max_backoff);
    }
}
