#![forbid(unsafe_code)]
//! Application entry point for the `awswit` executable.
//!
//! The crate intentionally exposes one process-level Interface.  AWS parsing,
//! safety policy, shell activation, preferences, terminal handling, and child
//! execution remain implementation details.

mod activation;
mod application;
mod catalog;
mod cli;
mod error;
mod history;
mod output;
mod process;
mod safety;
mod text_safety;
mod tui;

use std::ffi::OsString;
use std::process::ExitCode;

use clap::Parser;

/// Parse one invocation, run it, report safe diagnostics, and return its
/// process disposition.  Raw arguments are never logged.
pub fn entry(arguments: impl IntoIterator<Item = OsString>) -> ExitCode {
    let cli = match cli::Cli::try_parse_from(arguments) {
        Ok(cli) => cli,
        Err(error) => {
            if error.use_stderr() {
                output::cli_syntax_error();
                return ExitCode::from(2);
            }
            let _ = error.print();
            return ExitCode::SUCCESS;
        }
    };

    match application::run(cli) {
        Ok(application::Completion::Exit(code)) => ExitCode::from(code),
        Ok(application::Completion::Cancelled) => ExitCode::from(130),
        Ok(application::Completion::Terminated(signal)) => terminate_for_signal(signal),
        Err(error) => {
            output::diagnostic(&error);
            ExitCode::from(error.exit_code())
        }
    }
}

#[cfg(unix)]
fn terminate_for_signal(signal: i32) -> ExitCode {
    if signal_hook::low_level::emulate_default_handler(signal).is_err() {
        return ExitCode::from(1);
    }
    // A catchable default signal does not return.  This is only a defensive
    // fallback for an invalid or ignored signal.
    ExitCode::from(1)
}

#[cfg(not(unix))]
fn terminate_for_signal(signal: i32) -> ExitCode {
    if signal == 2 {
        ExitCode::from(130)
    } else {
        ExitCode::from(1)
    }
}

#[cfg(all(test, not(unix)))]
mod tests {
    use super::*;

    #[test]
    fn windows_keyboard_interrupt_uses_conventional_exit_code() {
        assert_eq!(terminate_for_signal(2), ExitCode::from(130));
    }
}
