//! Shared terminal-safety rules for Rust trust boundaries.

use unicode_width::UnicodeWidthChar;

pub(crate) fn is_terminal_control(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{061c}'
                | '\u{200e}'
                | '\u{200f}'
                | '\u{2028}'..='\u{202e}'
                | '\u{2066}'..='\u{2069}'
        )
}

pub(crate) fn is_safe_terminal_text(value: &str) -> bool {
    !value.chars().any(is_terminal_control)
}

/// Return whether every scalar has an unambiguous visible terminal footprint.
/// Shell completion APIs that cannot separate a display label from an exact
/// insertion value use this stricter predicate.
pub(crate) fn is_unambiguous_terminal_text(value: &str) -> bool {
    !value
        .chars()
        .any(|character| is_terminal_control(character) || character.width() == Some(0))
}

/// Preserve ordinary Unicode while making terminal controls and zero-cell
/// scalars visible. The returned text is for human display only; machine
/// interfaces continue to carry the exact original value.
pub(crate) fn visible_terminal_text(value: &str) -> String {
    let mut visible = String::with_capacity(value.len());
    for character in value.chars() {
        if is_terminal_control(character) || character.width() == Some(0) {
            visible.extend(character.escape_default());
        } else {
            visible.push(character);
        }
    }
    visible
}

pub(crate) fn sanitized(value: &str) -> String {
    value.chars().flat_map(char::escape_default).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_text_exposes_zero_cell_scalars_without_rejecting_unicode() {
        assert_eq!(
            visible_terminal_text("pro\u{200b}d e\u{301}"),
            "pro\\u{200b}d e\\u{301}"
        );
        assert_eq!(visible_terminal_text("東京-開発 🦀"), "東京-開発 🦀");
    }

    #[test]
    fn completion_safety_requires_a_visible_footprint_for_every_scalar() {
        assert!(is_unambiguous_terminal_text("東京-開発 🦀"));
        assert!(!is_unambiguous_terminal_text("pro\u{200b}d"));
        assert!(!is_unambiguous_terminal_text("e\u{301}"));
    }

    #[test]
    fn unicode_line_and_paragraph_separators_are_terminal_controls() {
        for separator in ['\u{2028}', '\u{2029}'] {
            assert!(is_terminal_control(separator));
            assert!(!is_safe_terminal_text(&format!("before{separator}after")));
            assert!(!is_unambiguous_terminal_text(&format!(
                "before{separator}after"
            )));
        }
        assert_eq!(
            visible_terminal_text("line\u{2028}paragraph\u{2029}end"),
            "line\\u{2028}paragraph\\u{2029}end"
        );
    }
}
