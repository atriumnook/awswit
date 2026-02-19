use std::io::{self, Write};

use indicatif::{ProgressBar, ProgressStyle};

/// Create a spinner for showing progress during credential operations
pub fn create_spinner(message: &str) -> ProgressBar {
    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.cyan} {msg}")
            .unwrap_or_else(|_| ProgressStyle::default_spinner())
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
    );
    spinner.set_message(message.to_string());
    spinner.enable_steady_tick(std::time::Duration::from_millis(80));
    spinner
}

/// Print a success message to stderr
pub fn print_success(message: &str) {
    let _ = writeln!(io::stderr(), "\x1b[32m✓\x1b[0m {}", message);
}

/// Print an error message to stderr
pub fn print_error(message: &str) {
    let _ = writeln!(io::stderr(), "\x1b[31m✗\x1b[0m {}", message);
}

/// Print an info message to stderr
pub fn print_info(message: &str) {
    let _ = writeln!(io::stderr(), "\x1b[36mℹ\x1b[0m {}", message);
}

/// Print a warning message to stderr
pub fn print_warning(message: &str) {
    let _ = writeln!(io::stderr(), "\x1b[33m⚠\x1b[0m {}", message);
}
