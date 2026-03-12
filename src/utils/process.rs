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
