//! Autoawswit daemon - thin wrapper that daemonizes and delegates to the runner.

fn main() {
    // Read the notification pipe fd BEFORE fork so both parent and child know it.
    // The spawning process (daemon.rs) sets this env var to signal readiness.
    #[cfg(unix)]
    let notify_fd: Option<i32> = std::env::var("AWSWIT_NOTIFY_FD")
        .ok()
        .and_then(|s| s.parse().ok());

    // Daemonize (detach from terminal) BEFORE starting tokio runtime.
    // Forking after tokio starts is undefined behavior in multithreaded programs.
    #[cfg(unix)]
    {
        use std::process;
        match unsafe { libc::fork() } {
            -1 => {
                eprintln!("Failed to fork");
                // Notify parent of failure before exiting
                if let Some(fd) = notify_fd {
                    notify_pipe(fd, false);
                }
                process::exit(1);
            }
            0 => {
                // Child process continues
                if unsafe { libc::setsid() } == -1 {
                    eprintln!("Failed to create new session");
                    if let Some(fd) = notify_fd {
                        notify_pipe(fd, false);
                    }
                    process::exit(1);
                }
            }
            _ => {
                // Parent exits immediately — the pipe write fd is inherited by
                // the child, so the parent's copy must be closed to avoid keeping
                // the pipe open after the child crashes.
                if let Some(fd) = notify_fd {
                    unsafe { libc::close(fd) };
                }
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
    let lock_path = dirs::home_dir()
        .expect("Could not determine home directory")
        .join(".awswit")
        .join("autoawswit.lock");
    let _daemon_lock = match awswit::utils::fs::lock_file_with_permissions(
        &lock_path,
        std::time::Duration::from_secs(30),
    ) {
        Ok(lock) => lock,
        Err(e) => {
            tracing::error!("Failed to acquire daemon lock: {}", e);
            #[cfg(unix)]
            if let Some(fd) = notify_fd {
                notify_pipe(fd, false);
            }
            std::process::exit(1);
        }
    };

    // Write PID file after successful init (not from parent)
    if let Err(e) = awswit::autorefresh::runner::write_own_pid_file() {
        tracing::error!("Failed to write PID file: {}", e);
        #[cfg(unix)]
        if let Some(fd) = notify_fd {
            notify_pipe(fd, false);
        }
        std::process::exit(1);
    }

    // Release lock explicitly by dropping
    drop(_daemon_lock);

    // Signal the spawning parent that initialization succeeded
    #[cfg(unix)]
    if let Some(fd) = notify_fd {
        notify_pipe(fd, true);
    }

    tracing::info!("Autoawswit daemon started (pid={})", std::process::id());

    // Start tokio runtime after fork, run daemon loop with SIGTERM handling
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    rt.block_on(awswit::autorefresh::runner::run_daemon_loop());
}

/// Write a success/failure byte to the notification pipe and close it.
#[cfg(unix)]
fn notify_pipe(fd: i32, success: bool) {
    let byte: [u8; 1] = if success { [1] } else { [0] };
    unsafe {
        libc::write(fd, byte.as_ptr() as *const libc::c_void, 1);
        libc::close(fd);
    }
}
