//! Autoawswit daemon - thin wrapper that daemonizes and delegates to the runner.

fn main() {
    // Daemonize (detach from terminal) BEFORE starting tokio runtime.
    // Forking after tokio starts is undefined behavior in multithreaded programs.
    #[cfg(unix)]
    {
        use std::process;
        match unsafe { libc::fork() } {
            -1 => {
                eprintln!("Failed to fork");
                process::exit(1);
            }
            0 => {
                // Child process continues
                if unsafe { libc::setsid() } == -1 {
                    eprintln!("Failed to create new session");
                    process::exit(1);
                }
            }
            _ => {
                // Parent exits
                process::exit(0);
            }
        }
    }

    // Setup logging after fork
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .init();

    // Acquire daemon lock before writing PID file to prevent race conditions.
    // The lock is held for the duration of PID file write; the spawning parent
    // waits for the PID file to appear before releasing its own lock.
    let lock_path = dirs::home_dir()
        .expect("Could not determine home directory")
        .join(".awswit")
        .join("autoawswit.lock");
    let _daemon_lock = {
        use fs2::FileExt;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let lock_file = opts
            .open(&lock_path)
            .expect("Failed to open daemon lock file");
        lock_file
            .lock_exclusive()
            .expect("Failed to acquire daemon lock");
        lock_file
    };

    // Write PID file after successful init (not from parent)
    if let Err(e) = awswit::autorefresh::runner::write_own_pid_file() {
        tracing::error!("Failed to write PID file: {}", e);
        std::process::exit(1);
    }

    // Release lock explicitly by dropping
    drop(_daemon_lock);

    tracing::info!("Autoawswit daemon started (pid={})", std::process::id());

    // Start tokio runtime after fork, run daemon loop with SIGTERM handling
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    rt.block_on(awswit::autorefresh::runner::run_daemon_loop());
}
