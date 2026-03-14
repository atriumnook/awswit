use crossterm::style::Stylize;

const CHECK: &str = if cfg!(windows) { "[OK] " } else { "\u{2705} " };

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

    pub fn profile_switched(profile: &str) {
        eprintln!();
        eprintln!("{} Profile: {}", CHECK, profile.cyan().bold());
        eprintln!();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_line() {
        StatusLine::info("Info message");
        StatusLine::success("Success message");
    }
}
