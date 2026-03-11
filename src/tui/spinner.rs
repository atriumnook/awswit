use console::{style, Emoji};
use indicatif::{ProgressBar, ProgressStyle};
use std::time::Duration;

static LOCK: Emoji<'_, '_> = Emoji("🔐 ", "");
static KEY: Emoji<'_, '_> = Emoji("🔑 ", "");
static ROCKET: Emoji<'_, '_> = Emoji("🚀 ", "");
static CHECK: Emoji<'_, '_> = Emoji("✅ ", "[OK] ");
static CROSS: Emoji<'_, '_> = Emoji("❌ ", "[ERR] ");

/// Spinner for AWS operations
pub struct AwswitSpinner {
    progress: ProgressBar,
    _start_message: String,
}

impl AwswitSpinner {
    /// Create a new spinner with a message
    pub fn new(message: &str) -> Self {
        let progress = ProgressBar::new_spinner();
        progress.set_style(
            ProgressStyle::default_spinner()
                .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ ")
                .template("{spinner:.cyan} {msg}")
                .expect("Invalid spinner template"),
        );
        progress.set_message(message.to_string());
        progress.enable_steady_tick(Duration::from_millis(80));

        Self {
            progress,
            _start_message: message.to_string(),
        }
    }

    /// Create spinner for MFA prompt
    pub fn mfa() -> Self {
        Self::new(&format!("{}Waiting for MFA token...", LOCK))
    }

    /// Create spinner for assuming role
    pub fn assuming_role(profile: &str) -> Self {
        Self::new(&format!("{}Assuming role: {}", KEY, style(profile).cyan()))
    }

    /// Create spinner for getting session token
    pub fn session_token() -> Self {
        Self::new(&format!("{}Getting session token...", KEY))
    }

    /// Create spinner for refreshing credentials
    pub fn refreshing() -> Self {
        Self::new(&format!("{}Refreshing credentials...", ROCKET))
    }

    /// Update the spinner message
    pub fn set_message(&self, message: &str) {
        self.progress.set_message(message.to_string());
    }

    /// Finish with success
    pub fn finish_success(&self, message: &str) {
        self.progress
            .finish_with_message(format!("{}{}", CHECK, style(message).green()));
    }

    /// Finish with error
    pub fn finish_error(&self, message: &str) {
        self.progress
            .finish_with_message(format!("{}{}", CROSS, style(message).red()));
    }

    /// Finish and clear
    pub fn finish_clear(&self) {
        self.progress.finish_and_clear();
    }
}

impl Drop for AwswitSpinner {
    fn drop(&mut self) {
        if !self.progress.is_finished() {
            self.progress.finish_and_clear();
        }
    }
}

/// Multi-step progress indicator
#[allow(dead_code)]
pub struct MultiStepProgress {
    steps: Vec<String>,
    current: usize,
    progress: ProgressBar,
}

#[allow(dead_code)]
impl MultiStepProgress {
    pub fn new(steps: Vec<&str>) -> Self {
        let total = steps.len() as u64;
        let progress = ProgressBar::new(total);
        progress.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.cyan} [{bar:30.cyan/dim}] {pos}/{len} {msg}")
                .expect("Invalid progress template")
                .progress_chars("━━╸"),
        );
        progress.enable_steady_tick(Duration::from_millis(100));

        Self {
            steps: steps.into_iter().map(String::from).collect(),
            current: 0,
            progress,
        }
    }

    pub fn next_step(&mut self) {
        if self.current < self.steps.len() {
            self.progress.set_message(self.steps[self.current].clone());
            self.progress.inc(1);
            self.current += 1;
        }
    }

    pub fn finish(&self) {
        self.progress.finish_with_message(format!("{}Done!", CHECK));
    }
}

/// Simple status line that updates in place
pub struct StatusLine;

impl StatusLine {
    pub fn info(message: &str) {
        eprintln!("{} {}", style("ℹ").cyan(), message);
    }

    pub fn success(message: &str) {
        eprintln!("{} {}", CHECK, style(message).green());
    }

    pub fn warning(message: &str) {
        eprintln!("{} {}", style("⚠").yellow(), style(message).yellow());
    }

    pub fn error(message: &str) {
        eprintln!("{} {}", CROSS, style(message).red());
    }

    pub fn profile_assumed(profile: &str, expiration: Option<&str>) {
        eprintln!();
        eprintln!("{} Profile: {}", CHECK, style(profile).cyan().bold());
        if let Some(exp) = expiration {
            eprintln!("   {} Expires: {}", style("⏱").dim(), style(exp).dim());
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
