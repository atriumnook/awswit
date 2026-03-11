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

    // Write PID file after successful init (not from parent)
    if let Err(e) = awswit::autorefresh::runner::write_own_pid_file() {
        tracing::error!("Failed to write PID file: {}", e);
        std::process::exit(1);
    }

    tracing::info!("Autoawswit daemon started (pid={})", std::process::id());

    // Start tokio runtime after fork, run daemon loop with SIGTERM handling
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    rt.block_on(awswit::autorefresh::runner::run_daemon_loop());
}
