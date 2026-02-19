use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::history::HistoryStorage;
use crate::profile::types::Profile;
use crate::tui::theme::Theme;

/// Render the preview panel showing profile details
pub fn render_preview(
    frame: &mut Frame,
    area: Rect,
    profile: Option<&Profile>,
    history: &HistoryStorage,
    theme: &Theme,
) {
    let block = Block::default()
        .title(" Profile Details ")
        .borders(Borders::ALL)
        .border_style(theme.border_focused);

    let Some(profile) = profile else {
        let paragraph = Paragraph::new("No profile selected").block(block);
        frame.render_widget(paragraph, area);
        return;
    };

    let mut lines: Vec<Line> = Vec::new();

    // Profile name header
    lines.push(Line::from(vec![Span::styled(
        format!("  {}", profile.name),
        theme.header,
    )]));
    lines.push(Line::from(""));

    // Profile details
    for (key, value) in profile.detail_lines() {
        lines.push(Line::from(vec![
            Span::styled(format!("  {:>18}: ", key), theme.preview_key),
            Span::styled(value, theme.preview_value),
        ]));
    }

    // History info
    if let Some(entry) = history.get(&profile.name) {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            "  --- History ---",
            theme.hint,
        )]));
        lines.push(Line::from(vec![
            Span::styled("       Use Count: ", theme.preview_key),
            Span::styled(entry.use_count.to_string(), theme.preview_value),
        ]));
        lines.push(Line::from(vec![
            Span::styled("       Last Used: ", theme.preview_key),
            Span::styled(
                entry.last_used.format("%Y-%m-%d %H:%M").to_string(),
                theme.preview_value,
            ),
        ]));
        if entry.is_favorite {
            lines.push(Line::from(vec![Span::styled(
                "       * Favorite",
                theme.favorite,
            )]));
        }
    }

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}
