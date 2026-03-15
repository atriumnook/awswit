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
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};

use super::preview::compact_preview_line;
use super::theme::Theme;
use crate::history::ProfileHistory;
use crate::profile::Profile;

/// Result of profile picker
pub enum PickerResult {
    Selected(String),
    Cancelled,
}

/// Profile entry with match score
#[derive(Clone)]
struct ProfileEntry {
    name: String,
    profile: Profile,
    is_favorite: bool,
    last_used: Option<chrono::DateTime<chrono::Utc>>,
}

/// Interactive profile picker with fuzzy search
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

    /// Run the interactive picker
    pub fn run(self) -> io::Result<PickerResult> {
        enable_raw_mode()?;
        // From here on, disable_raw_mode() must run even if run_inner fails.
        let result = self.run_inner();
        let _ = disable_raw_mode();
        result
    }

    fn run_inner(self) -> io::Result<PickerResult> {
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        // Create app state
        let mut app = PickerApp::new(self.profiles, self.history, self.theme);

        // Run event loop, ensuring terminal cleanup even on panic
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| app.run(&mut terminal)));

        // Restore terminal — always runs regardless of panic or error
        let _ = execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        );
        let _ = terminal.show_cursor();

        match result {
            Ok(inner) => inner,
            Err(panic_payload) => std::panic::resume_unwind(panic_payload),
        }
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
        let mut entries: Vec<ProfileEntry> = profiles
            .iter()
            .map(|(name, profile)| {
                let history_entry = history.get(name);
                ProfileEntry {
                    name: name.clone(),
                    profile: profile.clone(),
                    is_favorite: history.is_favorite(name),
                    last_used: history_entry.map(|h| h.last_used),
                }
            })
            .collect();

        // Sort: favorites first, then by frecency descending, then alphabetically
        let now = chrono::Utc::now();
        entries.sort_by(|a, b| history.compare_by_frecency(&a.name, &b.name, now));

        let filtered: Vec<usize> = (0..entries.len()).collect();

        let mut list_state = ListState::default();
        if !filtered.is_empty() {
            list_state.select(Some(0));
        }

        Self {
            entries,
            filtered,
            query: String::new(),
            list_state,
            theme,
            history,
            cursor_pos: 0,
            show_preview: true,
        }
    }

    fn run(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> io::Result<PickerResult> {
        loop {
            terminal.draw(|f| self.render(f))?;

            // Poll for events with timeout
            if event::poll(Duration::from_millis(100))?
                && let Event::Key(key) = event::read()?
            {
                match key.code {
                    // Navigation
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

                    // Selection
                    KeyCode::Enter => {
                        if let Some(selected) = self.get_selected_profile() {
                            return Ok(PickerResult::Selected(selected.name.clone()));
                        }
                    }

                    // Toggle favorite
                    KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        self.toggle_favorite();
                    }
                    KeyCode::Char('*') => {
                        self.toggle_favorite();
                    }

                    // Toggle preview
                    KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        self.show_preview = !self.show_preview;
                    }

                    // Cancel
                    KeyCode::Esc => {
                        return Ok(PickerResult::Cancelled);
                    }
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(PickerResult::Cancelled);
                    }

                    // Clear query
                    KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        self.query.clear();
                        self.cursor_pos = 0;
                        self.update_filter();
                    }

                    // Query editing
                    KeyCode::Char(c) => {
                        self.query.insert(self.cursor_pos, c);
                        self.cursor_pos += c.len_utf8();
                        self.update_filter();
                    }
                    KeyCode::Backspace => {
                        if self.cursor_pos > 0 {
                            let prev = self.query[..self.cursor_pos]
                                .char_indices()
                                .next_back()
                                .map(|(idx, _)| idx)
                                .unwrap_or(0);
                            self.query.remove(prev);
                            self.cursor_pos = prev;
                            self.update_filter();
                        }
                    }
                    KeyCode::Delete => {
                        if self.cursor_pos < self.query.len() {
                            self.query.remove(self.cursor_pos);
                            self.update_filter();
                        }
                    }
                    KeyCode::Left => {
                        if self.cursor_pos > 0 {
                            self.cursor_pos = self.query[..self.cursor_pos]
                                .char_indices()
                                .next_back()
                                .map(|(idx, _)| idx)
                                .unwrap_or(0);
                        }
                    }
                    KeyCode::Right => {
                        if self.cursor_pos < self.query.len() {
                            self.cursor_pos = self.query[self.cursor_pos..]
                                .char_indices()
                                .nth(1)
                                .map(|(idx, _)| self.cursor_pos + idx)
                                .unwrap_or(self.query.len());
                        }
                    }

                    _ => {}
                }
            }
        }
    }

    fn render(&mut self, frame: &mut Frame) {
        let size = frame.size();

        // Main layout
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

        // Search icon and query
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

        let search_widget = Paragraph::new(search_line);
        frame.render_widget(search_widget, inner);

        // Show match count
        let count_text = format!(" {}/{} ", self.filtered.len(), self.entries.len());
        let count_x = area.right().saturating_sub(count_text.len() as u16 + 2);
        let count_area = Rect::new(count_x, area.y, count_text.len() as u16 + 2, 1);

        let count_widget = Paragraph::new(Span::styled(count_text, self.theme.muted_style()));
        frame.render_widget(count_widget, count_area);
    }

    fn render_main_area(&mut self, frame: &mut Frame, area: Rect) {
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
        if let Some(entry) = self.get_selected_profile() {
            let preview = super::preview::ProfilePreview::new(&entry.profile, &self.theme);
            preview.render(frame, area);
        } else {
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

    fn render_help_bar(&self, frame: &mut Frame, area: Rect) {
        let help_items = vec![
            ("↑↓", "navigate"),
            ("⏎", "select"),
            ("★", "favorite"),
            ("^P", "preview"),
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

        let help_line = Line::from(help_spans);
        let help_widget = Paragraph::new(help_line);
        frame.render_widget(help_widget, area);
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

            // Sort: fuzzy score desc, then favorite, then history, then name asc
            scored.sort_by(|a, b| Self::compare_scored_entries(&self.entries, a, b));

            self.filtered = scored.iter().map(|(idx, _)| *idx).collect();
        }

        // Reset selection to first item
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
                (Some(a_time), Some(b_time)) => b_time.cmp(a_time),
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

    fn toggle_favorite(&mut self) {
        if let Some(selected) = self.list_state.selected()
            && let Some(&entry_idx) = self.filtered.get(selected)
        {
            let entry = &mut self.entries[entry_idx];
            entry.is_favorite = !entry.is_favorite;

            // Update history
            self.history.set_favorite(&entry.name, entry.is_favorite);
            if let Err(e) = crate::history::save_history(&self.history) {
                tracing::warn!("Failed to save history: {}", e);
                eprintln!("Warning: Failed to save history: {}", e);
            }
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

        // Already at 0, moving up should stay at 0
        app.move_selection(-1);
        assert_eq!(app.list_state.selected(), Some(0));

        // Move past the end
        app.move_selection(100);
        assert_eq!(app.list_state.selected(), Some(app.filtered.len() - 1));
    }

    #[test]
    fn move_selection_empty_list() {
        let profiles = HashMap::new();
        let mut app = make_app(&profiles);
        // Should not panic
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
    fn toggle_favorite_via_history() {
        let profiles = test_profiles();
        let mut app = make_app(&profiles);

        let name = app.get_selected_profile().unwrap().name.clone();
        assert!(!app.history.is_favorite(&name));

        let entry_idx = app.filtered[0];
        app.entries[entry_idx].is_favorite = true;
        app.history.set_favorite(&app.entries[entry_idx].name, true);
        assert!(app.history.is_favorite(&name));

        app.entries[entry_idx].is_favorite = false;
        app.history
            .set_favorite(&app.entries[entry_idx].name, false);
        assert!(!app.history.is_favorite(&name));
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
    fn get_selected_profile_none_when_empty() {
        let profiles = HashMap::new();
        let app = make_app(&profiles);
        assert!(app.get_selected_profile().is_none());
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
}
