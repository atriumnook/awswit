use crossterm::style::Stylize;
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const SPINNER_CHARS: &[char] = &[
    '\u{2801}', '\u{2802}', '\u{2804}', '\u{2840}', '\u{2880}', '\u{2820}', '\u{2810}', '\u{2808}',
];

const LOCK_EMOJI: &str = if cfg!(windows) { "" } else { "\u{1f510} " };
const KEY_EMOJI: &str = if cfg!(windows) { "" } else { "\u{1f511} " };
const ROCKET_EMOJI: &str = if cfg!(windows) { "" } else { "\u{1f680} " };
const CHECK: &str = if cfg!(windows) { "[OK] " } else { "\u{2705} " };
const CROSS: &str = if cfg!(windows) { "[ERR] " } else { "\u{274c} " };

/// Spinner for AWS operations
pub struct AwswitSpinner {
    finished: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl AwswitSpinner {
    /// Create a new spinner with a message
    pub fn new(message: &str) -> Self {
        let finished = Arc::new(AtomicBool::new(false));
        let finished_clone = finished.clone();
        let msg = message.to_string();

        let handle = std::thread::spawn(move || {
            let mut stderr = std::io::stderr();
            let mut idx = 0usize;
            while !finished_clone.load(Ordering::Relaxed) {
                let ch = SPINNER_CHARS[idx % SPINNER_CHARS.len()];
                let _ = write!(stderr, "\r{} {}", ch.to_string().cyan(), msg);
                let _ = stderr.flush();
                idx += 1;
                std::thread::sleep(Duration::from_millis(80));
            }
        });

        Self {
            finished,
            handle: Some(handle),
        }
    }

    /// Create spinner for MFA prompt
    pub fn mfa() -> Self {
        Self::new(&format!("{}Waiting for MFA token...", LOCK_EMOJI))
    }

    /// Create spinner for assuming role
    pub fn assuming_role(profile: &str) -> Self {
        Self::new(&format!("{}Assuming role: {}", KEY_EMOJI, profile.cyan()))
    }

    /// Create spinner for getting session token
    pub fn session_token() -> Self {
        Self::new(&format!("{}Getting session token...", KEY_EMOJI))
    }

    /// Create spinner for refreshing credentials
    pub fn refreshing() -> Self {
        Self::new(&format!("{}Refreshing credentials...", ROCKET_EMOJI))
    }

    /// Update the spinner message (no-op in this minimal implementation;
    /// kept for API compatibility)
    pub fn set_message(&self, _message: &str) {
        // The background thread runs with the original message.
    }

    fn stop_thread(&mut self) {
        self.finished.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        // Clear the spinner line
        let mut stderr = std::io::stderr();
        let _ = write!(stderr, "\r\x1b[2K");
        let _ = stderr.flush();
    }

    /// Finish with success
    pub fn finish_success(&self, message: &str) {
        // We need interior mutability here but the public API takes &self.
        // Use a small trick: set finished flag so the thread stops, then print.
        self.finished.store(true, Ordering::Relaxed);
        // Give the thread a moment to exit its write loop
        std::thread::sleep(Duration::from_millis(10));
        let mut stderr = std::io::stderr();
        let _ = write!(stderr, "\r\x1b[2K");
        let _ = writeln!(stderr, "{}{}", CHECK, message.green());
        let _ = stderr.flush();
    }

    /// Finish with error
    pub fn finish_error(&self, message: &str) {
        self.finished.store(true, Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(10));
        let mut stderr = std::io::stderr();
        let _ = write!(stderr, "\r\x1b[2K");
        let _ = writeln!(stderr, "{}{}", CROSS, message.red());
        let _ = stderr.flush();
    }

    /// Finish and clear
    pub fn finish_clear(&self) {
        self.finished.store(true, Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(10));
        let mut stderr = std::io::stderr();
        let _ = write!(stderr, "\r\x1b[2K");
        let _ = stderr.flush();
    }
}

impl Drop for AwswitSpinner {
    fn drop(&mut self) {
        if !self.finished.load(Ordering::Relaxed) {
            self.stop_thread();
        } else if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Simple status line that updates in place
pub struct StatusLine;

impl StatusLine {
    pub fn info(message: &str) {
        eprintln!("{} {}", "\u{2139}".cyan(), message);
    }

    pub fn success(message: &str) {
        eprintln!("{}{}", CHECK, message.green());
    }

    pub fn warning(message: &str) {
        eprintln!("{} {}", "\u{26a0}".yellow(), message.yellow());
    }

    pub fn error(message: &str) {
        eprintln!("{}{}", CROSS, message.red());
    }

    pub fn profile_assumed(profile: &str, expiration: Option<&str>) {
        eprintln!();
        eprintln!("{} Profile: {}", CHECK, profile.cyan().bold());
        if let Some(exp) = expiration {
            eprintln!("   {} Expires: {}", "\u{23f1}".dark_grey(), exp.dark_grey());
        }
        eprintln!();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spinner_creation() {
        let spinner = AwswitSpinner::new("Test message");
        spinner.finish_clear();
    }

    #[test]
    fn test_status_line() {
        // These just print, no assertions needed
        StatusLine::info("Info message");
        StatusLine::success("Success message");
    }
}
