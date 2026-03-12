/// Gracefully terminate a process: SIGTERM → wait → SIGKILL → waitpid.
///
/// Sends SIGTERM, waits up to `grace_period` for the process to exit,
/// then sends SIGKILL if still running and reaps the zombie.
#[cfg(unix)]
pub fn graceful_kill(pid: i32, grace_period: std::time::Duration) {
    // Send SIGTERM
    unsafe { libc::kill(pid, libc::SIGTERM) };

    let start = std::time::Instant::now();
    let poll_interval = std::time::Duration::from_millis(50);

    loop {
        if unsafe { libc::kill(pid, 0) } != 0 {
            return; // Process exited
        }
        if start.elapsed() >= grace_period {
            // Force kill
            unsafe { libc::kill(pid, libc::SIGKILL) };
            // Reap the process to prevent zombie
            reap_process(pid);
            return;
        }
        std::thread::sleep(poll_interval);
    }
}

/// Attempt to reap a process after SIGKILL to prevent zombies and PID reuse.
#[cfg(unix)]
pub fn reap_process(pid: i32) {
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_millis(500);
    loop {
        let ret = unsafe { libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) };
        if ret == pid || ret == -1 {
            break;
        }
        if start.elapsed() >= timeout {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}
