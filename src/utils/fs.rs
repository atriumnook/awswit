use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static WRITE_COUNTER: AtomicU64 = AtomicU64::new(0);

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
    let counter = WRITE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp_path = parent.join(format!(
        ".{}.{}.{}.{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("file"),
        std::process::id(),
        nanos,
        counter
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
/// sync and async contexts. In async context (autorefresh daemon), this blocks
/// the executor thread but the max sleep is 1s with exponential backoff, and
/// lock contention is rare. If this becomes problematic, wrap call sites in
/// `tokio::task::spawn_blocking`.
pub fn lock_exclusive_with_timeout(file: &std::fs::File, timeout: Duration) -> io::Result<()> {
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
