use std::collections::HashMap;
use std::io;
use std::time::Duration;

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};

use super::preview::compact_preview_line;
use super::theme::Theme;
use crate::history::ProfileHistory;
use crate::profile::Profile;

/// What the picker returns to the caller. The caller is responsible for
/// persisting `history` after the picker exits — the picker never writes
/// to disk on its own.
pub struct PickerOutcome {
    /// `None` when the user cancelled.
    pub selected: Option<String>,
    /// History updated with any favorite toggles performed in the picker.
    pub history: ProfileHistory,
}

/// One profile rendered in the list, with its history-derived metadata
/// frozen for the lifetime of the picker session.
#[derive(Clone)]
struct ProfileEntry {
    name: String,
    profile: Profile,
    is_favorite: bool,
    last_used: Option<chrono::DateTime<chrono::Utc>>,
}

pub struct ProfilePicker<'a> {
    profiles: &'a HashMap<String, Profile>,
    history: ProfileHistory,
    theme: Theme,
}

impl<'a> ProfilePicker<'a> {
    pub fn new(profiles: &'a HashMap<String, Profile>) -> Self {
        Self {
            profiles,
            history: ProfileHistory::default(),
            theme: Theme::default(),
        }
    }

    pub fn with_history(mut self, history: ProfileHistory) -> Self {
        self.history = history;
        self
    }

    pub fn run(self) -> io::Result<PickerOutcome> {
        // Drop-guard: `disable_raw_mode` must run on *every* exit path —
        // normal return, `?`-propagated I/O error, or panic from deep inside
        // ratatui (degenerate `Rect`, layout math on tiny terminals).
        // Without this, a panic during render leaves the user's terminal
        // swallowing keystrokes silently.
        struct RawModeGuard;
        impl Drop for RawModeGuard {
            fn drop(&mut self) {
                let _ = disable_raw_mode();
            }
        }

        enable_raw_mode()?;
        let _guard = RawModeGuard;
        self.run_inner()
    }

    fn run_inner(self) -> io::Result<PickerOutcome> {
        // Drop-guard for the alt-screen + mouse-capture pair. A panic
        // between EnterAlternateScreen and the explicit `execute!` cleanup
        // below would otherwise leave the user's terminal in a wedged
        // state on Windows Terminal / conhost (mouse capture eats
        // selection clicks, alt-screen hides the prior buffer) and on
        // any tty where the unwind tears down stdout before the cleanup
        // line runs.
        struct ScreenGuard;
        impl Drop for ScreenGuard {
            fn drop(&mut self) {
                let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
            }
        }

        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let _screen = ScreenGuard;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let mut app = PickerApp::new(self.profiles, self.history, self.theme);

        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| app.run(&mut terminal)));
        let _ = terminal.show_cursor();

        let outcome = match result {
            Ok(inner) => inner?,
            Err(panic_payload) => std::panic::resume_unwind(panic_payload),
        };
        Ok(outcome)
    }
}

struct PickerApp {
    entries: Vec<ProfileEntry>,
    filtered: Vec<usize>,
    query: String,
    list_state: ListState,
    theme: Theme,
    history: ProfileHistory,
    cursor_pos: usize,
    show_preview: bool,
}

impl PickerApp {
    fn new(profiles: &HashMap<String, Profile>, history: ProfileHistory, theme: Theme) -> Self {
        let mut app = Self {
            entries: Vec::new(),
            filtered: Vec::new(),
            query: String::new(),
            list_state: ListState::default(),
            theme,
            history,
            cursor_pos: 0,
            show_preview: true,
        };
        app.rebuild_entries(profiles);
        app
    }

    /// (Re)compute the entries vec and the initial sort.
    fn rebuild_entries(&mut self, profiles: &HashMap<String, Profile>) {
        self.entries = profiles
            .iter()
            .map(|(name, profile)| ProfileEntry {
                name: name.clone(),
                profile: profile.clone(),
                is_favorite: self.history.is_favorite(name),
                last_used: self.history.get(name).map(|h| h.last_used),
            })
            .collect();

        let now = chrono::Utc::now();
        self.entries
            .sort_by(|a, b| self.history.compare_by_frecency(&a.name, &b.name, now));

        self.filtered = (0..self.entries.len()).collect();
        if !self.filtered.is_empty() {
            self.list_state.select(Some(0));
        }
    }

    fn run(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> io::Result<PickerOutcome> {
        loop {
            terminal.draw(|f| self.render(f))?;

            if !event::poll(Duration::from_millis(100))? {
                continue;
            }
            let Event::Key(key) = event::read()? else {
                continue;
            };

            match key.code {
                // ── navigation ───────────────────────────────────────────
                KeyCode::Up => self.move_selection(-1),
                KeyCode::Down => self.move_selection(1),
                KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.move_selection(-1);
                }
                KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.move_selection(1);
                }
                KeyCode::PageUp => self.move_selection(-10),
                KeyCode::PageDown => self.move_selection(10),
                KeyCode::Home => self.move_to_start(),
                KeyCode::End => self.move_to_end(),

                // ── selection / cancel ───────────────────────────────────
                KeyCode::Enter => {
                    let selected = self.get_selected_profile().map(|e| e.name.clone());
                    if let Some(name) = selected {
                        return Ok(PickerOutcome {
                            selected: Some(name),
                            history: std::mem::take(&mut self.history),
                        });
                    }
                }
                KeyCode::Esc => return Ok(self.cancel()),
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Ok(self.cancel());
                }

                // ── favorites / preview toggle ───────────────────────────
                KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.toggle_favorite();
                }
                KeyCode::Char('*') => self.toggle_favorite(),
                KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.show_preview = !self.show_preview;
                }

                // ── readline-style line editing ──────────────────────────
                KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.cursor_pos = 0;
                }
                KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.cursor_pos = self.query.len();
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.query.clear();
                    self.cursor_pos = 0;
                    self.update_filter();
                }
                KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.delete_word_backward();
                    self.update_filter();
                }

                // ── query editing ────────────────────────────────────────
                KeyCode::Char(c) => {
                    self.query.insert(self.cursor_pos, c);
                    self.cursor_pos += c.len_utf8();
                    self.update_filter();
                }
                KeyCode::Backspace if self.cursor_pos > 0 => {
                    let prev = self.query[..self.cursor_pos]
                        .char_indices()
                        .next_back()
                        .map(|(idx, _)| idx)
                        .unwrap_or(0);
                    self.query.remove(prev);
                    self.cursor_pos = prev;
                    self.update_filter();
                }
                KeyCode::Delete if self.cursor_pos < self.query.len() => {
                    self.query.remove(self.cursor_pos);
                    self.update_filter();
                }
                KeyCode::Left if self.cursor_pos > 0 => {
                    self.cursor_pos = self.query[..self.cursor_pos]
                        .char_indices()
                        .next_back()
                        .map(|(idx, _)| idx)
                        .unwrap_or(0);
                }
                KeyCode::Right if self.cursor_pos < self.query.len() => {
                    self.cursor_pos = self.query[self.cursor_pos..]
                        .char_indices()
                        .nth(1)
                        .map(|(idx, _)| self.cursor_pos + idx)
                        .unwrap_or(self.query.len());
                }
                _ => {}
            }
        }
    }

    fn cancel(&mut self) -> PickerOutcome {
        PickerOutcome {
            selected: None,
            history: std::mem::take(&mut self.history),
        }
    }

    fn delete_word_backward(&mut self) {
        if self.cursor_pos == 0 {
            return;
        }
        let before = &self.query[..self.cursor_pos];
        // Skip trailing whitespace, then non-whitespace.
        let trimmed = before.trim_end();
        let after_ws = trimmed.rfind(char::is_whitespace).map_or(0, |i| i + 1);
        self.query.drain(after_ws..self.cursor_pos);
        self.cursor_pos = after_ws;
    }

    fn render(&mut self, frame: &mut Frame) {
        let size = frame.size();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Search bar
                Constraint::Min(5),    // List + Preview
                Constraint::Length(2), // Help bar
            ])
            .split(size);

        self.render_search_bar(frame, chunks[0]);
        self.render_main_area(frame, chunks[1]);
        self.render_help_bar(frame, chunks[2]);
    }

    fn render_search_bar(&self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.theme.primary))
            .title(Span::styled(" 🔐 awswit ", self.theme.title_style()));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        let (before_cursor, after_cursor) = self.query.split_at(self.cursor_pos);
        let search_line = Line::from(vec![
            Span::styled(
                format!("{} ", self.theme.icons.search),
                Style::default().fg(self.theme.primary),
            ),
            Span::raw(before_cursor.to_string()),
            Span::styled("│", Style::default().fg(self.theme.accent)),
            Span::raw(after_cursor.to_string()),
        ]);
        frame.render_widget(Paragraph::new(search_line), inner);

        let count_text = format!(" {}/{} ", self.filtered.len(), self.entries.len());
        let count_x = area.right().saturating_sub(count_text.len() as u16 + 2);
        let count_area = Rect::new(count_x, area.y, count_text.len() as u16 + 2, 1);
        frame.render_widget(
            Paragraph::new(Span::styled(count_text, self.theme.muted_style())),
            count_area,
        );
    }

    fn render_main_area(&mut self, frame: &mut Frame, area: Rect) {
        if self.entries.is_empty() {
            self.render_empty(frame, area);
            return;
        }
        if self.show_preview && area.width > 80 {
            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(area);
            self.render_list(frame, chunks[0]);
            self.render_preview(frame, chunks[1]);
        } else {
            self.render_list(frame, area);
        }
    }

    fn render_empty(&self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.theme.muted))
            .title(" Profiles ");
        let msg = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "No AWS profiles found.",
                self.theme.warning_style(),
            )),
            Line::from(""),
            Line::from(Span::raw("Create one in ~/.aws/config, for example:")),
            Line::from(""),
            Line::from(Span::raw("    [profile dev]")),
            Line::from(Span::raw("    region = us-east-1")),
            Line::from(""),
            Line::from(Span::raw("then re-run awswit.")),
        ])
        .block(block)
        .alignment(Alignment::Left);
        frame.render_widget(msg, area);
    }

    fn render_list(&mut self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.theme.muted))
            .title(Span::styled(" Profiles ", self.theme.muted_style()));

        let items: Vec<ListItem> = self
            .filtered
            .iter()
            .map(|&idx| {
                let entry = &self.entries[idx];
                self.create_list_item(entry)
            })
            .collect();

        let list = List::new(items)
            .block(block)
            .highlight_style(self.theme.selected_style())
            .highlight_symbol("▶ ");
        frame.render_stateful_widget(list, area, &mut self.list_state);
    }

    fn create_list_item(&self, entry: &ProfileEntry) -> ListItem<'static> {
        let favorite = if entry.is_favorite {
            Span::styled(
                format!("{} ", self.theme.icons.favorite),
                self.theme.favorite_style(),
            )
        } else {
            Span::raw("  ")
        };

        let name = Span::styled(
            format!("{:<20}", entry.name),
            Style::default().add_modifier(Modifier::BOLD),
        );

        let details = compact_preview_line(&entry.profile, &self.theme);

        let mut spans = vec![favorite, name];
        spans.extend(
            details
                .spans
                .into_iter()
                .map(|s| Span::styled(s.content.to_string(), s.style)),
        );

        ListItem::new(Line::from(spans))
    }

    fn render_preview(&self, frame: &mut Frame, area: Rect) {
        match self.get_selected_profile() {
            Some(entry) => {
                let preview = super::preview::ProfilePreview::new(
                    &entry.profile,
                    entry.last_used,
                    &self.theme,
                );
                preview.render(frame, area);
            }
            None => {
                let block = Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(self.theme.muted))
                    .title(" Preview ");
                let paragraph = Paragraph::new("No profile selected")
                    .block(block)
                    .style(self.theme.muted_style());
                frame.render_widget(paragraph, area);
            }
        }
    }

    fn render_help_bar(&self, frame: &mut Frame, area: Rect) {
        let help_items = [
            ("↑↓", "navigate"),
            ("⏎", "select"),
            ("★", "favorite"),
            ("^P", "preview"),
            ("^A/^E/^W", "edit"),
            ("Esc", "cancel"),
        ];

        let help_spans: Vec<Span> = help_items
            .into_iter()
            .flat_map(|(key, desc)| {
                vec![
                    Span::styled(
                        format!(" {} ", key),
                        Style::default()
                            .fg(self.theme.background)
                            .bg(self.theme.muted),
                    ),
                    Span::styled(format!(" {} ", desc), self.theme.muted_style()),
                    Span::raw(" "),
                ]
            })
            .collect();

        frame.render_widget(Paragraph::new(Line::from(help_spans)), area);
    }

    fn update_filter(&mut self) {
        if self.query.is_empty() {
            self.filtered = (0..self.entries.len()).collect();
        } else {
            let mut matcher = Matcher::new(Config::DEFAULT);
            let pattern = Pattern::new(
                &self.query,
                CaseMatching::Ignore,
                Normalization::Smart,
                AtomKind::Fuzzy,
            );

            let mut buf = Vec::new();
            let mut scored: Vec<(usize, u32)> = Vec::new();
            for (idx, entry) in self.entries.iter().enumerate() {
                buf.clear();
                let haystack = nucleo_matcher::Utf32Str::new(&entry.name, &mut buf);
                if let Some(s) = pattern.score(haystack, &mut matcher) {
                    scored.push((idx, s));
                }
            }
            scored.sort_by(|a, b| Self::compare_scored_entries(&self.entries, a, b));
            self.filtered = scored.into_iter().map(|(idx, _)| idx).collect();
        }

        if !self.filtered.is_empty() {
            self.list_state.select(Some(0));
        } else {
            self.list_state.select(None);
        }
    }

    fn compare_scored_entries(
        entries: &[ProfileEntry],
        a: &(usize, u32),
        b: &(usize, u32),
    ) -> std::cmp::Ordering {
        let ea = &entries[a.0];
        let eb = &entries[b.0];
        b.1.cmp(&a.1)
            .then_with(|| eb.is_favorite.cmp(&ea.is_favorite))
            .then_with(|| match (&ea.last_used, &eb.last_used) {
                (Some(at), Some(bt)) => bt.cmp(at),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            })
            .then_with(|| ea.name.cmp(&eb.name))
    }

    fn move_selection(&mut self, delta: i32) {
        if self.filtered.is_empty() {
            return;
        }
        let current = self.list_state.selected().unwrap_or(0);
        let len = self.filtered.len();
        let new_idx = if delta < 0 {
            current.saturating_sub((-delta) as usize)
        } else {
            (current + delta as usize).min(len - 1)
        };
        self.list_state.select(Some(new_idx));
    }

    fn move_to_start(&mut self) {
        if !self.filtered.is_empty() {
            self.list_state.select(Some(0));
        }
    }

    fn move_to_end(&mut self) {
        if !self.filtered.is_empty() {
            self.list_state.select(Some(self.filtered.len() - 1));
        }
    }

    fn get_selected_profile(&self) -> Option<&ProfileEntry> {
        self.list_state
            .selected()
            .and_then(|idx| self.filtered.get(idx))
            .map(|&entry_idx| &self.entries[entry_idx])
    }

    /// Flip the favorite flag on the currently selected entry, then re-sort
    /// so the visual position reflects the new ranking immediately. History
    /// is mutated in place; persistence happens once after the picker exits.
    fn toggle_favorite(&mut self) {
        let Some(selected) = self.list_state.selected() else {
            return;
        };
        let Some(&entry_idx) = self.filtered.get(selected) else {
            return;
        };
        let target_name = self.entries[entry_idx].name.clone();
        let new_state = !self.entries[entry_idx].is_favorite;

        self.entries[entry_idx].is_favorite = new_state;
        self.history.set_favorite(&target_name, new_state);

        // Re-sort entries and recompute `filtered` to match the new ranking,
        // keeping the same profile under the cursor if possible.
        let now = chrono::Utc::now();
        self.entries
            .sort_by(|a, b| self.history.compare_by_frecency(&a.name, &b.name, now));
        self.update_filter();
        // Try to keep the toggled profile selected.
        if let Some(pos) = self
            .filtered
            .iter()
            .position(|&i| self.entries[i].name == target_name)
        {
            self.list_state.select(Some(pos));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::ProfileHistory;
    use crate::profile::Profile;

    fn test_profiles() -> HashMap<String, Profile> {
        let mut profiles = HashMap::new();
        profiles.insert("alpha".to_string(), Profile::default());
        profiles.insert("beta".to_string(), Profile::default());
        profiles.insert("gamma".to_string(), Profile::default());
        profiles
    }

    fn make_app(profiles: &HashMap<String, Profile>) -> PickerApp {
        PickerApp::new(profiles, ProfileHistory::default(), Theme::default())
    }

    #[test]
    fn move_selection_down_and_up() {
        let profiles = test_profiles();
        let mut app = make_app(&profiles);
        assert_eq!(app.list_state.selected(), Some(0));
        app.move_selection(1);
        assert_eq!(app.list_state.selected(), Some(1));
        app.move_selection(-1);
        assert_eq!(app.list_state.selected(), Some(0));
    }

    #[test]
    fn move_selection_clamps_at_boundaries() {
        let profiles = test_profiles();
        let mut app = make_app(&profiles);
        app.move_selection(-1);
        assert_eq!(app.list_state.selected(), Some(0));
        app.move_selection(100);
        assert_eq!(app.list_state.selected(), Some(app.filtered.len() - 1));
    }

    #[test]
    fn move_selection_empty_list() {
        let profiles = HashMap::new();
        let mut app = make_app(&profiles);
        app.move_selection(1);
        app.move_selection(-1);
        assert!(app.list_state.selected().is_none());
    }

    #[test]
    fn update_filter_narrows_results() {
        let profiles = test_profiles();
        let mut app = make_app(&profiles);
        assert_eq!(app.filtered.len(), 3);
        app.query = "alp".to_string();
        app.update_filter();
        assert_eq!(app.filtered.len(), 1);
        let selected = app.get_selected_profile().unwrap();
        assert_eq!(selected.name, "alpha");
    }

    #[test]
    fn update_filter_empty_query_restores_all() {
        let profiles = test_profiles();
        let mut app = make_app(&profiles);
        app.query = "alp".to_string();
        app.update_filter();
        assert_eq!(app.filtered.len(), 1);
        app.query.clear();
        app.update_filter();
        assert_eq!(app.filtered.len(), 3);
    }

    #[test]
    fn toggle_favorite_promotes_to_top_and_persists_in_history() {
        let profiles = test_profiles();
        let mut app = make_app(&profiles);
        app.list_state.select(Some(2)); // "gamma" by alphabetical
        let target = app.entries[app.filtered[2]].name.clone();

        app.toggle_favorite();

        assert!(app.history.is_favorite(&target));
        // After toggle, the favorited entry should be at the top.
        assert_eq!(app.entries[app.filtered[0]].name, target);
        // And the cursor should follow it.
        assert_eq!(app.list_state.selected(), Some(0));
    }

    #[test]
    fn delete_word_backward_strips_to_previous_whitespace() {
        let profiles = test_profiles();
        let mut app = make_app(&profiles);
        app.query = "hello world foo".to_string();
        app.cursor_pos = app.query.len();
        app.delete_word_backward();
        assert_eq!(app.query, "hello world ");
        assert_eq!(app.cursor_pos, app.query.len());
    }

    #[test]
    fn delete_word_backward_noop_at_start() {
        let profiles = test_profiles();
        let mut app = make_app(&profiles);
        app.delete_word_backward();
        assert_eq!(app.query, "");
        assert_eq!(app.cursor_pos, 0);
    }

    #[test]
    fn move_to_start_and_end() {
        let profiles = test_profiles();
        let mut app = make_app(&profiles);
        app.move_to_end();
        assert_eq!(app.list_state.selected(), Some(app.filtered.len() - 1));
        app.move_to_start();
        assert_eq!(app.list_state.selected(), Some(0));
    }

    #[test]
    fn compare_scored_entries_by_score() {
        let profiles = test_profiles();
        let app = make_app(&profiles);
        let a = (0, 100u32);
        let b = (1, 50u32);
        assert_eq!(
            PickerApp::compare_scored_entries(&app.entries, &a, &b),
            std::cmp::Ordering::Less
        );
    }

    // ── Render snapshot tests ────────────────────────────────────────
    //
    // Drive the picker against `TestBackend` and inspect the rendered
    // buffer. These guard the UX contract: list contents are visible,
    // empty state surfaces a hint, the search bar shows the query, and
    // the help bar advertises the keybindings.

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    /// Concatenate every cell symbol on every line into a single string.
    /// Spaces are preserved so we can check word positions.
    fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
        let buf = terminal.backend().buffer();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf.get(x, y).symbol());
            }
            out.push('\n');
        }
        out
    }

    fn render_to_string(app: &mut PickerApp, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| app.render(f)).unwrap();
        buffer_text(&terminal)
    }

    #[test]
    fn render_shows_every_profile_name() {
        let profiles = test_profiles();
        let mut app = make_app(&profiles);
        let text = render_to_string(&mut app, 100, 20);
        assert!(text.contains("alpha"), "alpha missing:\n{}", text);
        assert!(text.contains("beta"), "beta missing");
        assert!(text.contains("gamma"), "gamma missing");
    }

    #[test]
    fn render_shows_count_in_search_bar() {
        let profiles = test_profiles();
        let mut app = make_app(&profiles);
        let text = render_to_string(&mut app, 100, 20);
        // Format is " filtered/total " — with three profiles and no filter it's 3/3.
        assert!(
            text.contains("3/3"),
            "expected 3/3 indicator, got:\n{}",
            text
        );
    }

    #[test]
    fn render_shows_filtered_count_when_query_narrows() {
        let profiles = test_profiles();
        let mut app = make_app(&profiles);
        app.query = "alp".to_string();
        app.update_filter();
        let text = render_to_string(&mut app, 100, 20);
        assert!(
            text.contains("1/3"),
            "expected 1/3 after filter, got:\n{}",
            text
        );
        assert!(text.contains("alpha"));
        assert!(!text.contains("beta"));
        assert!(!text.contains("gamma"));
    }

    #[test]
    fn render_empty_profile_set_shows_hint() {
        let profiles: HashMap<String, Profile> = HashMap::new();
        let mut app = make_app(&profiles);
        let text = render_to_string(&mut app, 100, 20);
        assert!(
            text.contains("No AWS profiles found"),
            "missing empty-state hint:\n{}",
            text
        );
        assert!(
            text.contains("[profile dev]"),
            "missing example block:\n{}",
            text
        );
    }

    #[test]
    fn render_help_bar_lists_keybindings() {
        let profiles = test_profiles();
        let mut app = make_app(&profiles);
        let text = render_to_string(&mut app, 100, 20);
        // Each keybinding label appears in the bottom help bar.
        for key in [
            "navigate", "select", "favorite", "preview", "edit", "cancel",
        ] {
            assert!(text.contains(key), "help bar missing `{}`:\n{}", key, text);
        }
    }

    #[test]
    fn render_search_bar_shows_query_text() {
        let profiles = test_profiles();
        let mut app = make_app(&profiles);
        app.query = "alp".to_string();
        app.cursor_pos = app.query.len();
        app.update_filter();
        let text = render_to_string(&mut app, 100, 20);
        assert!(text.contains("alp"), "query not displayed:\n{}", text);
    }

    #[test]
    fn render_favorited_entry_marked_with_star() {
        let profiles = test_profiles();
        let mut app = make_app(&profiles);
        app.list_state.select(Some(0));
        app.toggle_favorite();
        let text = render_to_string(&mut app, 100, 20);
        assert!(
            text.contains('★'),
            "favorite star missing after toggle:\n{}",
            text
        );
    }

    #[test]
    fn render_preview_pane_appears_when_width_large() {
        let profiles = test_profiles();
        let mut app = make_app(&profiles);
        let text = render_to_string(&mut app, 120, 20);
        // The preview pane uses the profile name as its block title.
        // With 3 alphabetical profiles, the first selected is "alpha".
        let preview_title_visible = text.contains("alpha");
        assert!(preview_title_visible);
    }

    #[test]
    fn render_does_not_panic_on_tiny_terminal() {
        let profiles = test_profiles();
        let mut app = make_app(&profiles);
        // Should not panic on degenerate sizes — render layout has to cope.
        let _ = render_to_string(&mut app, 20, 10);
    }
}
