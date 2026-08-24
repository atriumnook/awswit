use std::fmt::Write as _;
use std::io::{self, Write};

use serde::Serialize;

use crate::error::AppError;
use crate::text_safety::{is_terminal_control, sanitized};
use unicode_width::UnicodeWidthChar;

pub(crate) enum WriteResult {
    Complete,
    BrokenPipe,
}

pub(crate) fn stdout(bytes: &[u8]) -> Result<WriteResult, AppError> {
    write_stream(io::stdout().lock(), bytes)
}

/// Serialize JSON while keeping directional controls visible in raw output.
///
/// JSON decoding still produces the original scalar values; only their wire
/// representation is changed so an untrusted value cannot reorder a terminal
/// or log viewer's presentation of adjacent text.
pub(crate) fn pretty_json<T: Serialize>(value: &T) -> Result<Vec<u8>, AppError> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| AppError::Output {
        source: io::Error::other(error),
    })?;
    let json = String::from_utf8(bytes).map_err(|error| AppError::Output {
        source: io::Error::new(io::ErrorKind::InvalidData, error),
    })?;
    if !json.chars().any(needs_json_terminal_escape) {
        return Ok(json.into_bytes());
    }
    let mut escaped = String::with_capacity(json.len());
    for character in json.chars() {
        // serde_json already escapes C0 characters inside strings. Raw C0
        // newlines and spaces outside strings are JSON formatting and must be
        // retained. DEL, C1, and directional controls can only occur as data,
        // so make their wire representation visibly inert.
        if needs_json_terminal_escape(character) {
            write_json_scalar_escape(&mut escaped, character);
        } else {
            escaped.push(character);
        }
    }
    Ok(escaped.into_bytes())
}

fn needs_json_terminal_escape(character: char) -> bool {
    (is_terminal_control(character) && !matches!(character, '\u{0000}'..='\u{001f}'))
        || character.width() == Some(0)
}

fn write_json_scalar_escape(output: &mut String, character: char) {
    let scalar = u32::from(character);
    if scalar <= 0xffff {
        let _ = write!(output, "\\u{scalar:04X}");
        return;
    }

    let supplementary = scalar - 0x1_0000;
    let high = 0xd800 + (supplementary >> 10);
    let low = 0xdc00 + (supplementary & 0x3ff);
    let _ = write!(output, "\\u{high:04X}\\u{low:04X}");
}

fn write_stream(mut writer: impl Write, bytes: &[u8]) -> Result<WriteResult, AppError> {
    match writer.write_all(bytes).and_then(|()| writer.flush()) {
        Ok(()) => Ok(WriteResult::Complete),
        Err(source) if source.kind() == io::ErrorKind::BrokenPipe => Ok(WriteResult::BrokenPipe),
        Err(source) => Err(AppError::Output { source }),
    }
}

pub(crate) fn diagnostic(error: &AppError) {
    let mut stderr = io::stderr().lock();
    let _ = writeln!(stderr, "awswit[{}]: {}", error.code(), error);
    if let Some(hint) = error.hint() {
        let _ = writeln!(stderr, "hint: {hint}");
    }
}

pub(crate) fn warning(code: &str, message: &str) {
    let _ = writeln!(
        io::stderr().lock(),
        "awswit[{code}]: {}",
        sanitized(message)
    );
}

/// Clap owns the grammar, but its default parse errors may echo raw argv.
/// Keep syntax failures useful without allowing control characters from an
/// untrusted argument to forge terminal output.
pub(crate) fn cli_syntax_error() {
    let mut stderr = io::stderr().lock();
    let _ = writeln!(stderr, "awswit[CLI_INVALID]: invalid command line");
    let _ = writeln!(stderr, "hint: run `awswit --help` for supported syntax");
}

#[cfg(test)]
mod tests {
    use super::*;

    struct BrokenWriter;

    impl Write for BrokenWriter {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn broken_pipe_is_a_normal_stream_outcome() {
        assert!(matches!(
            write_stream(BrokenWriter, b"output"),
            Ok(WriteResult::BrokenPipe)
        ));
    }

    #[test]
    fn json_wire_escapes_zero_cell_scalars_without_changing_decoded_values() {
        let original = serde_json::json!({
            "profile": "pro\u{200b}d",
            "variation": "plane\u{e0100}",
        });
        let bytes = pretty_json(&original).expect("serialize terminal-safe JSON");
        let wire = String::from_utf8(bytes.clone()).expect("JSON is UTF-8");

        assert!(wire.contains("pro\\u200Bd"));
        assert!(wire.contains("plane\\uDB40\\uDD00"));
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&bytes).expect("decode escaped JSON"),
            original
        );
    }
}
