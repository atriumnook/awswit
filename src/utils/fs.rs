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
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW)
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

/// A held file lock. The lock is released when this value is dropped.
pub struct FileLockGuard {
    // Hold the fd-lock write guard which keeps the advisory lock.
    // Field order matters: _guard borrows from _lock, but since we use
    // Option-take in Drop, Rust's drop order (fields in declaration order)
    // is fine — we just need both to live together.
    _lock: Option<fd_lock::RwLock<std::fs::File>>,
}

// Safety: FileLockGuard just holds a file descriptor lock, safe to send across threads.
unsafe impl Send for FileLockGuard {}

impl FileLockGuard {
    fn new(mut rw_lock: fd_lock::RwLock<std::fs::File>) -> Self {
        // Acquire the blocking write lock and leak the guard so the lock
        // persists. The lock is released when the RwLock (and its inner File)
        // is dropped.
        let guard = rw_lock
            .write()
            .expect("fd-lock write after try_write succeeded");
        std::mem::forget(guard);
        Self {
            _lock: Some(rw_lock),
        }
    }
}

/// Open (or create) a lock file with restrictive permissions and acquire an exclusive lock.
///
/// On Unix, the file is created with mode 0o600 and `O_NOFOLLOW` to prevent
/// symlink attacks. Returns a guard; the lock is released when the guard is dropped.
///
/// Uses `fd-lock` for cross-platform advisory locking with exponential backoff.
/// Uses `thread::sleep` for backoff intentionally — this is called from both
/// sync and async contexts. In async context (autorefresh daemon), this blocks
/// the executor thread but the max sleep is 1s with exponential backoff, and
/// lock contention is rare. If this becomes problematic, wrap call sites in
/// `tokio::task::spawn_blocking`.
pub fn lock_file_with_permissions(path: &Path, timeout: Duration) -> io::Result<FileLockGuard> {
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = opts.open(path)?;
    let mut rw_lock = fd_lock::RwLock::new(file);

    let start = Instant::now();
    let mut backoff = Duration::from_millis(10);
    let max_backoff = Duration::from_secs(1);
    let mut first = true;

    loop {
        if !first {
            if start.elapsed() >= timeout {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("Lock acquisition timed out after {:?}", timeout),
                ));
            }
            std::thread::sleep(backoff);
            backoff = (backoff * 2).min(max_backoff);
        }
        first = false;

        let locked = rw_lock.try_write().is_ok();
        if locked {
            return Ok(FileLockGuard::new(rw_lock));
        }
    }
}
