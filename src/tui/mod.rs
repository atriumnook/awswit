use std::io::IsTerminal;

pub mod fzf;
pub mod picker;
mod preview;
mod theme;

pub use picker::ProfilePicker;
pub use theme::Theme;

/// Whether an interactive picker should run for a bare `awswit` invocation.
///
/// The documented shell wrapper captures stdout via `$()` / command
/// substitution, so `stdout().is_terminal()` is **false** even at an
/// interactive prompt. Detect interactivity from stdin and stderr instead;
/// the TUI renders to stderr (or `/dev/tty`), leaving stdout for the
/// eval-able export payload. Named `awswit prod` never needs this gate.
pub fn should_open_picker(profile_given: bool, no_interactive: bool) -> bool {
    should_open_picker_with(
        profile_given,
        no_interactive,
        std::io::stdin().is_terminal(),
        std::io::stderr().is_terminal(),
    )
}

/// Testable form of [`should_open_picker`]. stdout TTY is intentionally not
/// a parameter — gating on it is the bug the wrapper regression tests exist
/// to prevent.
pub fn should_open_picker_with(
    profile_given: bool,
    no_interactive: bool,
    stdin_is_tty: bool,
    stderr_is_tty: bool,
) -> bool {
    !profile_given && !no_interactive && stdin_is_tty && stderr_is_tty
}

#[cfg(test)]
mod want_picker_tests {
    use super::*;

    #[test]
    fn wrapper_stdout_capture_still_opens_picker() {
        // Historical bug: `stdout().is_terminal()` was false under `$()`, so
        // bare `awswit` through the wrapper never launched the picker.
        assert!(should_open_picker_with(false, false, true, true));
    }

    #[test]
    fn named_profile_skips_picker() {
        assert!(!should_open_picker_with(true, false, true, true));
    }

    #[test]
    fn no_interactive_skips_picker() {
        assert!(!should_open_picker_with(false, true, true, true));
    }

    #[test]
    fn piped_stdin_skips_picker() {
        assert!(!should_open_picker_with(false, false, false, true));
    }

    #[test]
    fn piped_stderr_skips_picker() {
        assert!(!should_open_picker_with(false, false, true, false));
    }
}
