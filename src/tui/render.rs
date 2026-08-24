use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};

use super::state::{Entry, PickerState};
use crate::text_safety::visible_terminal_text;

pub(super) const COMPACT_MIN_WIDTH: u16 = 12;
// Search (3) + one bordered list row (3) + compact help (1).
pub(super) const COMPACT_MIN_HEIGHT: u16 = 7;
pub(super) const PREVIEW_MIN_WIDTH: u16 = 78;
pub(super) const PREVIEW_MIN_HEIGHT: u16 = 10;

#[derive(Clone, Copy, Debug)]
struct Theme {
    color: bool,
}

impl Theme {
    fn from_environment() -> Self {
        let no_color = std::env::var_os("NO_COLOR").is_some();
        let dumb_terminal = std::env::var("TERM").is_ok_and(|term| term == "dumb");
        Self {
            color: !no_color && !dumb_terminal,
        }
    }

    fn title(self) -> Style {
        self.with_color(Color::Cyan).add_modifier(Modifier::BOLD)
    }

    fn selected(self) -> Style {
        if self.color {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        }
    }

    fn current(self) -> Style {
        self.with_color(Color::Green).add_modifier(Modifier::BOLD)
    }

    fn favorite(self) -> Style {
        self.with_color(Color::Yellow).add_modifier(Modifier::BOLD)
    }

    fn muted(self) -> Style {
        self.with_color(Color::DarkGray)
    }

    fn label(self) -> Style {
        self.with_color(Color::Blue).add_modifier(Modifier::BOLD)
    }

    fn with_color(self, color: Color) -> Style {
        if self.color {
            Style::default().fg(color)
        } else {
            Style::default()
        }
    }
}

pub(super) fn render(frame: &mut Frame, state: &PickerState) {
    let area = frame.area();
    let theme = Theme::from_environment();

    if area.width < COMPACT_MIN_WIDTH || area.height < COMPACT_MIN_HEIGHT {
        render_compact(frame, state, area, theme);
        return;
    }

    let search_height = 3u16.min(area.height);
    let help_height = if area.height >= 8 { 2 } else { 1 };
    let main_height = area
        .height
        .saturating_sub(search_height)
        .saturating_sub(help_height);
    let search_area = Rect::new(area.x, area.y, area.width, search_height);
    let main_area = Rect::new(
        area.x,
        area.y.saturating_add(search_height),
        area.width,
        main_height,
    );
    let help_area = Rect::new(
        area.x,
        area.y
            .saturating_add(search_height)
            .saturating_add(main_height),
        area.width,
        help_height,
    );

    render_search(frame, state, search_area, theme);
    render_main(frame, state, main_area, theme);
    render_help(frame, help_area, theme);
}

fn render_compact(frame: &mut Frame, state: &PickerState, area: Rect, theme: Theme) {
    let message = if state.filtered.is_empty() {
        "awswit: no match"
    } else {
        "awswit: resize to select"
    };
    frame.render_widget(
        Paragraph::new(message)
            .alignment(Alignment::Center)
            .style(theme.title()),
        area,
    );
}

fn render_search(frame: &mut Frame, state: &PickerState, area: Rect, theme: Theme) {
    let title = format!(
        " awswit - AWS profiles ({}/{}) ",
        state.filtered.len(),
        state.entries.len()
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(title, theme.title()));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let before = state.query.get(..state.cursor).unwrap_or_default();
    let after = state.query.get(state.cursor..).unwrap_or_default();
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Filter: ", theme.label()),
            Span::raw(visible_terminal_text(before)),
            Span::styled("|", theme.current()),
            Span::raw(visible_terminal_text(after)),
        ])),
        inner,
    );
}

fn render_main(frame: &mut Frame, state: &PickerState, area: Rect, theme: Theme) {
    if state.show_preview && area.width >= PREVIEW_MIN_WIDTH && area.height >= PREVIEW_MIN_HEIGHT {
        let list_width = area.width.saturating_mul(58) / 100;
        let preview_width = area.width.saturating_sub(list_width);
        let list_area = Rect::new(area.x, area.y, list_width, area.height);
        let preview_area = Rect::new(
            area.x.saturating_add(list_width),
            area.y,
            preview_width,
            area.height,
        );
        render_list(frame, state, list_area, theme);
        render_preview(frame, state.selected_entry(), preview_area, theme);
    } else {
        render_list(frame, state, area, theme);
    }
}

fn render_list(frame: &mut Frame, state: &PickerState, area: Rect, theme: Theme) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(" Profiles ", theme.title()));

    if state.filtered.is_empty() {
        frame.render_widget(
            Paragraph::new("No profiles match the filter")
                .block(block)
                .alignment(Alignment::Center)
                .style(theme.muted()),
            area,
        );
        return;
    }

    let items: Vec<_> = state
        .filtered
        .iter()
        .filter_map(|index| state.entries.get(*index))
        .map(|entry| profile_item(entry, theme))
        .collect();
    let mut list_state = ListState::default();
    list_state.select(state.focus);
    let list = List::new(items)
        .block(block)
        .highlight_symbol("> ")
        .highlight_style(theme.selected());
    frame.render_stateful_widget(list, area, &mut list_state);
}

fn profile_item(entry: &Entry, theme: Theme) -> ListItem<'static> {
    let current = if entry.profile.current {
        Span::styled("CURRENT ", theme.current())
    } else {
        Span::raw("        ")
    };
    let favorite = if entry.favorite {
        Span::styled("* ", theme.favorite())
    } else {
        Span::raw("  ")
    };
    let name = Span::styled(visible_terminal_text(&entry.profile.name), Modifier::BOLD);
    let provider = Span::styled(
        format!("  [{}]", entry.profile.provider.label()),
        theme.muted(),
    );
    let region = entry.profile.region.as_deref().map(|region| {
        Span::styled(
            format!("  {}", visible_terminal_text(region)),
            theme.muted(),
        )
    });
    let recent = entry
        .profile
        .last_used_sequence
        .map(|sequence| Span::styled(format!("  USED#{sequence}"), theme.muted()));

    let mut spans = vec![current, favorite, name, provider];
    if let Some(recent) = recent {
        spans.push(recent);
    }
    if let Some(region) = region {
        spans.push(region);
    }
    ListItem::new(Line::from(spans))
}

fn render_preview(frame: &mut Frame, entry: Option<&Entry>, area: Rect, theme: Theme) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(" Configured metadata ", theme.title()));
    let Some(entry) = entry else {
        frame.render_widget(
            Paragraph::new("Move to a profile to inspect its configuration")
                .block(block)
                .wrap(Wrap { trim: true })
                .style(theme.muted()),
            area,
        );
        return;
    };

    let profile = &entry.profile;
    let mut lines = vec![metadata_line("Name", &profile.name, theme)];
    lines.push(metadata_line("Provider", profile.provider.label(), theme));
    push_metadata(&mut lines, "Region", profile.region.as_deref(), theme);
    push_metadata(&mut lines, "Account", profile.account.as_deref(), theme);
    push_metadata(&mut lines, "Source", profile.source.as_deref(), theme);
    push_metadata(&mut lines, "Role ARN", profile.role_arn.as_deref(), theme);
    push_metadata(&mut lines, "SSO role", profile.role_name.as_deref(), theme);
    push_metadata(
        &mut lines,
        "SSO session",
        profile.sso_session.as_deref(),
        theme,
    );
    if let Some(sequence) = profile.last_used_sequence {
        lines.push(metadata_line(
            "Usage order",
            &format!("#{sequence} (larger is more recent)"),
            theme,
        ));
    }
    lines.push(Line::default());
    lines.push(Line::from(Span::styled(
        "Authentication is resolved by the AWS CLI or SDK.",
        theme.muted(),
    )));

    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn push_metadata(
    lines: &mut Vec<Line<'static>>,
    label: &'static str,
    value: Option<&str>,
    theme: Theme,
) {
    if let Some(value) = value.filter(|value| !value.is_empty()) {
        lines.push(metadata_line(label, value, theme));
    }
}

fn metadata_line(label: &'static str, value: &str, theme: Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label}: "), theme.label()),
        Span::raw(visible_terminal_text(value)),
    ])
}

fn render_help(frame: &mut Frame, area: Rect, theme: Theme) {
    let text = if area.width >= 72 {
        "Up/Down navigate  Enter select  * favorite  Ctrl+P preview  Esc cancel"
    } else if area.width >= 36 {
        "Up/Down move  Enter select  Esc cancel"
    } else {
        "Enter select  Esc cancel"
    };
    frame.render_widget(
        Paragraph::new(text)
            .alignment(Alignment::Center)
            .style(theme.muted()),
        area,
    );
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::tui::{ViewProfile, ViewProvider};

    fn profile(name: &str) -> ViewProfile {
        ViewProfile {
            name: name.to_string(),
            region: Some("ap-northeast-1".to_string()),
            provider: ViewProvider::SsoModern,
            source: Some("shared-sso".to_string()),
            account: Some("111122223333".to_string()),
            role_arn: None,
            role_name: Some("ReadOnly".to_string()),
            sso_session: Some("shared-sso".to_string()),
            current: true,
            favorite: false,
            last_used_sequence: Some(1),
        }
    }

    #[test]
    fn renders_extremely_small_terminals_without_panicking() {
        for (width, height) in [(1, 1), (5, 2), (11, 4), (12, 5)] {
            let backend = TestBackend::new(width, height);
            let mut terminal = Terminal::new(backend).unwrap();
            let state = PickerState::new(vec![profile("開発-管理者")]);
            terminal.draw(|frame| render(frame, &state)).unwrap();
        }
    }

    #[test]
    fn terminal_too_short_for_a_visible_row_requests_resize() {
        for height in [5, 6] {
            let backend = TestBackend::new(40, height);
            let mut terminal = Terminal::new(backend).unwrap();
            let state = PickerState::new(vec![profile("production")]);
            terminal.draw(|frame| render(frame, &state)).unwrap();

            assert!(terminal.backend().to_string().contains("resize to select"));
        }
    }

    #[test]
    fn renders_after_resize() {
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = PickerState::new(vec![profile("production")]);
        terminal.draw(|frame| render(frame, &state)).unwrap();
        terminal.resize(Rect::new(0, 0, 20, 5)).unwrap();
        terminal.draw(|frame| render(frame, &state)).unwrap();
    }

    #[test]
    fn logical_recency_is_visible_without_exposing_timestamps() {
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = PickerState::new(vec![profile("production")]);
        terminal.draw(|frame| render(frame, &state)).unwrap();

        let rendered = terminal.backend().to_string();
        assert!(rendered.contains("USED#1"));
        assert!(rendered.contains("Usage order: #1 (larger is more recent)"));
    }

    #[test]
    fn zero_cell_profile_and_metadata_scalars_are_visible() {
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut view = profile("pro\u{200b}d");
        view.region = Some("us\u{200d}-east-1".to_owned());
        let state = PickerState::new(vec![view]);
        terminal.draw(|frame| render(frame, &state)).unwrap();

        let rendered = terminal.backend().to_string();
        assert!(rendered.contains("pro\\u{200b}d"));
        assert!(rendered.contains("us\\u{200d}-east-1"));
    }

    #[test]
    fn metadata_control_characters_are_neutralized() {
        assert_eq!(visible_terminal_text("safe\u{1b}[2J"), "safe\\u{1b}[2J");
        assert_eq!(visible_terminal_text("a\nb"), "a\\nb");
        assert_eq!(visible_terminal_text("pro\u{200b}d"), "pro\\u{200b}d");
    }

    #[test]
    fn plain_theme_uses_non_color_selection_cue() {
        let style = Theme { color: false }.selected();
        assert!(style.add_modifier.contains(Modifier::REVERSED));
        assert!(style.add_modifier.contains(Modifier::BOLD));
        assert!(style.fg.is_none());
        assert!(style.bg.is_none());
    }
}
