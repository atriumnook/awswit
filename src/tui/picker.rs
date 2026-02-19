use std::collections::HashMap;
use std::io;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;
use ratatui::Terminal;

use crate::error::{AwswitError, Result};
use crate::history::HistoryStorage;
use crate::profile::types::Profile;
use crate::tui::preview::render_preview;
use crate::tui::theme::Theme;
use crate::utils::fuzzy::filter_profiles;

/// Interactive profile picker state
pub struct Picker {
    profiles: HashMap<String, Profile>,
    sorted_names: Vec<String>,
    filtered_names: Vec<String>,
    search_query: String,
    list_state: ListState,
    history: HistoryStorage,
    theme: Theme,
    fuzzy_enabled: bool,
}

impl Picker {
    pub fn new(
        profiles: HashMap<String, Profile>,
        history: HistoryStorage,
        colors_enabled: bool,
        fuzzy_enabled: bool,
    ) -> Self {
        let all_names: Vec<String> = profiles.keys().cloned().collect();
        let sorted_names = history.sorted_profile_names(&all_names);
        let filtered_names = sorted_names.clone();

        let mut list_state = ListState::default();
        if !filtered_names.is_empty() {
            list_state.select(Some(0));
        }

        Self {
            profiles,
            sorted_names,
            filtered_names,
            search_query: String::new(),
            list_state,
            history,
            theme: Theme::default_theme(colors_enabled),
            fuzzy_enabled,
        }
    }

    /// Run the interactive picker and return the selected profile name
    pub fn run(&mut self) -> Result<String> {
        enable_raw_mode()?;
        // Use a guard to ensure terminal state is always restored,
        // even on errors or panics
        let _cleanup = TerminalCleanupGuard;

        let mut stderr = io::stderr();
        stderr.execute(EnterAlternateScreen)?;

        let backend = CrosstermBackend::new(stderr);
        let mut terminal = Terminal::new(backend)?;

        let result = self.event_loop(&mut terminal);

        // Explicit cleanup (guard also handles it on drop)
        let _ = terminal.backend_mut().execute(LeaveAlternateScreen);
        let _ = disable_raw_mode();

        result
    }

    fn event_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stderr>>,
    ) -> Result<String> {
        loop {
            terminal.draw(|f| self.render(f))?;

            if let Event::Key(key) = event::read()? {
                match self.handle_key(key) {
                    PickerAction::Continue => {}
                    PickerAction::Select(name) => return Ok(name),
                    PickerAction::Quit => return Err(AwswitError::UserCancelled),
                }
            }
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> PickerAction {
        match key.code {
            KeyCode::Esc => PickerAction::Quit,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                PickerAction::Quit
            }
            KeyCode::Enter => {
                if let Some(idx) = self.list_state.selected() {
                    if idx < self.filtered_names.len() {
                        return PickerAction::Select(self.filtered_names[idx].clone());
                    }
                }
                PickerAction::Continue
            }
            KeyCode::Up => {
                self.move_selection(-1);
                PickerAction::Continue
            }
            KeyCode::Down => {
                self.move_selection(1);
                PickerAction::Continue
            }
            KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_selection(-1);
                PickerAction::Continue
            }
            KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_selection(1);
                PickerAction::Continue
            }
            KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                // Toggle favorite
                if let Some(idx) = self.list_state.selected() {
                    if idx < self.filtered_names.len() {
                        let name = self.filtered_names[idx].clone();
                        let _ = self.history.toggle_favorite(&name);
                        // Re-sort
                        self.sorted_names = self
                            .history
                            .sorted_profile_names(&self.profiles.keys().cloned().collect::<Vec<_>>());
                        self.update_filter();
                    }
                }
                PickerAction::Continue
            }
            KeyCode::Backspace => {
                self.search_query.pop();
                self.update_filter();
                PickerAction::Continue
            }
            KeyCode::Char(c) => {
                self.search_query.push(c);
                self.update_filter();
                PickerAction::Continue
            }
            _ => PickerAction::Continue,
        }
    }

    fn move_selection(&mut self, delta: i32) {
        if self.filtered_names.is_empty() {
            return;
        }
        let current = self.list_state.selected().unwrap_or(0) as i32;
        let new_idx = (current + delta).rem_euclid(self.filtered_names.len() as i32) as usize;
        self.list_state.select(Some(new_idx));
    }

    fn update_filter(&mut self) {
        self.filtered_names =
            filter_profiles(&self.search_query, &self.sorted_names, self.fuzzy_enabled);

        if self.filtered_names.is_empty() {
            self.list_state.select(None);
        } else {
            self.list_state.select(Some(0));
        }
    }

    fn render(&mut self, frame: &mut Frame) {
        let size = frame.size();

        // Main layout: search bar | content | status bar
        let main_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Search bar
                Constraint::Min(5),   // Content
                Constraint::Length(1), // Status bar
            ])
            .split(size);

        self.render_search_bar(frame, main_chunks[0]);

        // Content area: profile list | preview panel
        let content_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(50), // Profile list
                Constraint::Percentage(50), // Preview
            ])
            .split(main_chunks[1]);

        self.render_profile_list(frame, content_chunks[0]);

        let selected_profile = self
            .list_state
            .selected()
            .and_then(|idx| self.filtered_names.get(idx))
            .and_then(|name| self.profiles.get(name));

        render_preview(
            frame,
            content_chunks[1],
            selected_profile,
            &self.history,
            &self.theme,
        );

        self.render_status_bar(frame, main_chunks[2]);
    }

    fn render_search_bar(&self, frame: &mut Frame, area: Rect) {
        let search_text = Line::from(vec![
            Span::styled(" > ", self.theme.search_prompt),
            Span::styled(&self.search_query, self.theme.search_input),
            Span::raw("_"),
        ]);

        let block = Block::default()
            .title(" Search (type to filter) ")
            .borders(Borders::ALL)
            .border_style(self.theme.border_focused);

        let paragraph = Paragraph::new(search_text).block(block);
        frame.render_widget(paragraph, area);
    }

    fn render_profile_list(&mut self, frame: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = self
            .filtered_names
            .iter()
            .map(|name| {
                let is_fav = self.history.is_favorite(name);
                let prefix = if is_fav { "★ " } else { "  " };
                let profile_type = self
                    .profiles
                    .get(name)
                    .map(|p| p.profile_type())
                    .unwrap_or(crate::profile::types::ProfileType::User);

                let type_style = match profile_type {
                    crate::profile::types::ProfileType::Role => self.theme.profile_type_role,
                    crate::profile::types::ProfileType::Sso => self.theme.profile_type_sso,
                    _ => self.theme.profile_type_user,
                };

                let style = if is_fav {
                    self.theme.favorite
                } else {
                    self.theme.normal
                };

                ListItem::new(Line::from(vec![
                    Span::styled(prefix, style),
                    Span::styled(name.clone(), style),
                    Span::raw(" "),
                    Span::styled(format!("[{}]", profile_type), type_style),
                ]))
            })
            .collect();

        let block = Block::default()
            .title(format!(
                " Profiles ({}/{}) ",
                self.filtered_names.len(),
                self.sorted_names.len()
            ))
            .borders(Borders::ALL)
            .border_style(self.theme.border);

        let list = List::new(items)
            .block(block)
            .highlight_style(self.theme.selected)
            .highlight_symbol("▶ ");

        frame.render_stateful_widget(list, area, &mut self.list_state);
    }

    fn render_status_bar(&self, frame: &mut Frame, area: Rect) {
        let hints = Line::from(vec![
            Span::styled(" Enter", self.theme.header),
            Span::styled(":Select ", self.theme.hint),
            Span::styled("Esc", self.theme.header),
            Span::styled(":Quit ", self.theme.hint),
            Span::styled("↑↓", self.theme.header),
            Span::styled(":Navigate ", self.theme.hint),
            Span::styled("Ctrl+F", self.theme.header),
            Span::styled(":Favorite ", self.theme.hint),
        ]);

        let paragraph = Paragraph::new(hints).style(self.theme.status_bar);
        frame.render_widget(paragraph, area);
    }
}

enum PickerAction {
    Continue,
    Select(String),
    Quit,
}

/// RAII guard that ensures terminal state is restored on drop.
/// This handles cleanup even if the picker panics or returns early on error.
struct TerminalCleanupGuard;

impl Drop for TerminalCleanupGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = io::stderr().execute(LeaveAlternateScreen);
    }
}
