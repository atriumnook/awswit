//! Interactive AWS profile selection.
//!
//! This module deliberately accepts display-only values instead of the
//! application's profile or history types.  The caller owns parsing,
//! credential-safety policy, and persistence; the picker owns terminal
//! lifecycle, filtering, navigation, rendering, and the interaction result.

mod render;
mod state;
mod terminal;

use std::collections::HashSet;
use std::io::{self, IsTerminal};
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::text::Line;

use self::state::{Action, PickerState, ReduceOutcome};
use self::terminal::TerminalSession;
use crate::text_safety::{is_terminal_control, visible_terminal_text};

const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(50);
/// Border (2), selection marker (2), current marker (8), and favorite marker (2).
const PROFILE_ROW_FIXED_WIDTH: usize = 14;

/// Display-only classification derived from AWS shared configuration.
///
/// These variants describe configured metadata. They do not claim that a
/// credential is currently valid or identify the principal AWS will use.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum ViewProvider {
    /// No provider-specific metadata was identified.
    #[default]
    Unspecified,
    /// The profile has static credential keys configured. Values are never
    /// accepted by this module.
    StaticCredentials,
    /// The profile is configured to assume a role.
    Role,
    /// The profile references a modern `[sso-session]` section.
    SsoModern,
    /// The profile contains legacy inline SSO settings.
    SsoLegacy,
    /// The profile configures `credential_process`.
    CredentialProcess,
    /// The profile configures the AWS Login credentials provider.
    Login,
    /// The profile configures `credential_source`.
    CredentialSource,
    /// The profile configures web identity.
    WebIdentity,
}

impl ViewProvider {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Unspecified => "Unspecified",
            Self::StaticCredentials => "Static keys configured",
            Self::Role => "Role",
            Self::SsoModern => "SSO (session)",
            Self::SsoLegacy => "SSO (legacy)",
            Self::CredentialProcess => "Credential process",
            Self::Login => "AWS Login",
            Self::CredentialSource => "Credential source",
            Self::WebIdentity => "Web identity",
        }
    }
}

/// Non-secret data rendered by the picker.
///
/// `source`, `role_arn`, and `sso_session` must contain configuration metadata
/// only. In particular, callers must not pass credential values, tokens, or a
/// `credential_process` command line. Profile names must be non-empty and must
/// not contain terminal-control characters.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ViewProfile {
    pub name: String,
    pub region: Option<String>,
    pub provider: ViewProvider,
    pub source: Option<String>,
    pub account: Option<String>,
    pub role_arn: Option<String>,
    pub role_name: Option<String>,
    pub sso_session: Option<String>,
    pub current: bool,
    pub favorite: bool,
    /// Monotonic preference-history sequence. Larger values are more recent.
    pub last_used_sequence: Option<u64>,
}

/// A favorite update made during this picker invocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FavoriteChange {
    pub profile_name: String,
    pub favorite: bool,
}

/// A termination signal observed while the terminal was active.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PickerSignal {
    Interrupt,
    #[cfg(unix)]
    Terminate,
    #[cfg(unix)]
    Hangup,
}

/// Why the picker stopped.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PickerTermination {
    Selected,
    Cancelled,
    Signal(PickerSignal),
}

/// Complete interaction result.
///
/// `selection` is `Some` exactly when `termination` is `Selected`. Favorite
/// changes are deterministic deltas from the input and are returned even when
/// the user cancels. The caller decides whether and how to persist them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PickerResult {
    pub selection: Option<String>,
    pub favorite_changes: Vec<FavoriteChange>,
    pub termination: PickerTermination,
}

/// Open the interactive picker on stdin/stderr.
///
/// Stdout is never read or written. Both stdin and stderr must be terminals;
/// this permits a shell hook to capture stdout while the TUI remains attached
/// to the user's terminal.
pub(crate) fn run_picker(profiles: Vec<ViewProfile>) -> io::Result<PickerResult> {
    validate_profiles(&profiles)?;

    if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        return Err(io::Error::new(
            io::ErrorKind::NotConnected,
            "interactive profile selection requires terminal stdin and stderr",
        ));
    }

    let mut state = PickerState::new(profiles);
    let mut terminal = TerminalSession::start()?;
    let loop_result = run_event_loop(&mut terminal, &mut state);
    let restore_result = terminal.restore();
    let late_signal = terminal.take_signal();

    if let Some(signal) = late_signal {
        return Ok(state.result(PickerTermination::Signal(signal)));
    }

    let termination = loop_result?;
    restore_result?;
    Ok(state.result(termination))
}

fn run_event_loop(
    terminal: &mut TerminalSession,
    state: &mut PickerState,
) -> io::Result<PickerTermination> {
    let mut redraw = true;

    loop {
        if let Some(signal) = terminal.take_signal() {
            return Ok(PickerTermination::Signal(signal));
        }

        if redraw {
            terminal
                .terminal_mut()?
                .draw(|frame| render::render(frame, state))?;
            redraw = false;
        }

        let has_event = match event::poll(EVENT_POLL_INTERVAL) {
            Ok(has_event) => has_event,
            Err(error) => {
                if let Some(signal) = terminal.take_signal() {
                    return Ok(PickerTermination::Signal(signal));
                }
                return Err(error);
            }
        };

        if !has_event {
            continue;
        }

        let event = match event::read() {
            Ok(event) => event,
            Err(error) => {
                if let Some(signal) = terminal.take_signal() {
                    return Ok(PickerTermination::Signal(signal));
                }
                return Err(error);
            }
        };

        match event {
            Event::Key(key) if is_actionable_key(key) => {
                if let Some(action) = action_from_key(key) {
                    if matches!(action, Action::Confirm | Action::ToggleFavorite) {
                        let area = terminal.terminal_mut()?.size()?;
                        if !action_is_allowed(action, state, area.width, area.height) {
                            redraw = true;
                            continue;
                        }
                    }
                    match state.reduce(action) {
                        ReduceOutcome::Continue => redraw = true,
                        ReduceOutcome::Finish(termination) => return Ok(termination),
                    }
                }
            }
            Event::Resize(_, _) => redraw = true,
            _ => {}
        }
    }
}

fn action_is_allowed(action: Action, state: &PickerState, width: u16, height: u16) -> bool {
    !matches!(action, Action::Confirm | Action::ToggleFavorite)
        || selection_name_fits(state, width, height)
}

fn selection_name_fits(state: &PickerState, width: u16, height: u16) -> bool {
    if width < render::COMPACT_MIN_WIDTH || height < render::COMPACT_MIN_HEIGHT {
        return false;
    }
    let search_height = 3u16.min(height);
    let help_height = if height >= 8 { 2 } else { 1 };
    let main_height = height
        .saturating_sub(search_height)
        .saturating_sub(help_height);
    // The bordered profile list needs two border rows plus one visible item.
    if main_height < 3 {
        return false;
    }
    let Some(entry) = state.selected_entry() else {
        return false;
    };
    let list_width = if state.show_preview
        && width >= render::PREVIEW_MIN_WIDTH
        && height.saturating_sub(5) >= render::PREVIEW_MIN_HEIGHT
    {
        width.saturating_mul(58) / 100
    } else {
        width
    };
    let available_name_width = usize::from(list_width).saturating_sub(PROFILE_ROW_FIXED_WIDTH);
    Line::from(visible_terminal_text(&entry.profile.name)).width() <= available_name_width
}

fn is_actionable_key(key: KeyEvent) -> bool {
    matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat)
}

fn action_from_key(key: KeyEvent) -> Option<Action> {
    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);

    match key.code {
        KeyCode::Char('c') if control => Some(Action::Signal(PickerSignal::Interrupt)),
        KeyCode::Char('f') if control => Some(Action::ToggleFavorite),
        KeyCode::Char('p') if control => Some(Action::TogglePreview),
        KeyCode::Char('u') if control => Some(Action::ClearQuery),
        KeyCode::Char('j') if control => Some(Action::Move(1)),
        KeyCode::Char('k') if control => Some(Action::Move(-1)),
        KeyCode::Esc => Some(Action::Cancel),
        KeyCode::Enter => Some(Action::Confirm),
        KeyCode::Up => Some(Action::Move(-1)),
        KeyCode::Down => Some(Action::Move(1)),
        KeyCode::PageUp => Some(Action::Move(-10)),
        KeyCode::PageDown => Some(Action::Move(10)),
        KeyCode::Home => Some(Action::MoveToStart),
        KeyCode::End => Some(Action::MoveToEnd),
        KeyCode::Left => Some(Action::CursorLeft),
        KeyCode::Right => Some(Action::CursorRight),
        KeyCode::Backspace => Some(Action::Backspace),
        KeyCode::Delete => Some(Action::Delete),
        KeyCode::Char('*') if !control && !alt => Some(Action::ToggleFavorite),
        KeyCode::Char(character) if !control && !alt && !is_terminal_control(character) => {
            Some(Action::Insert(character))
        }
        _ => None,
    }
}

fn validate_profiles(profiles: &[ViewProfile]) -> io::Result<()> {
    let mut names = HashSet::with_capacity(profiles.len());
    let mut current_count = 0usize;

    for profile in profiles {
        if profile.name.is_empty()
            || u32::try_from(profile.name.chars().count()).is_err()
            || profile.name.chars().any(is_terminal_control)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "profile names must be non-empty and contain no terminal-control characters",
            ));
        }
        if !names.insert(profile.name.as_str()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "profile names must be unique",
            ));
        }
        if profile.current {
            current_count = current_count.saturating_add(1);
        }
    }

    if current_count > 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "at most one profile may be marked current",
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(name: &str) -> ViewProfile {
        ViewProfile {
            name: name.to_string(),
            region: None,
            provider: ViewProvider::Unspecified,
            source: None,
            account: None,
            role_arn: None,
            role_name: None,
            sso_session: None,
            current: false,
            favorite: false,
            last_used_sequence: None,
        }
    }

    #[test]
    fn validation_rejects_duplicate_names() {
        let error = validate_profiles(&[profile("dev"), profile("dev")]);
        assert!(matches!(error, Err(error) if error.kind() == io::ErrorKind::InvalidInput));
    }

    #[test]
    fn validation_rejects_terminal_controls() {
        let error = validate_profiles(&[profile("dev\u{1b}[2J")]);
        assert!(matches!(error, Err(error) if error.kind() == io::ErrorKind::InvalidInput));
    }

    #[test]
    fn validation_accepts_unicode_names() {
        assert!(validate_profiles(&[profile("開発-管理者")]).is_ok());
    }

    #[test]
    fn confirmation_requires_the_complete_selected_name_to_fit() {
        let mut selected = profile("production-administrator");
        selected.current = true;
        let state = PickerState::new(vec![selected]);

        assert!(!selection_name_fits(&state, 1, 1));
        assert!(!selection_name_fits(&state, 24, 20));
        assert!(!selection_name_fits(&state, 80, 5));
        assert!(!selection_name_fits(&state, 80, 6));
        assert!(selection_name_fits(&state, 80, 20));
    }

    #[test]
    fn confirmation_accounts_for_borders_highlight_and_row_prefix() {
        let mut exact_fit = profile("1234567890");
        exact_fit.current = true;
        let mut one_cell_too_wide = profile("12345678901");
        one_cell_too_wide.current = true;

        assert!(selection_name_fits(
            &PickerState::new(vec![exact_fit]),
            24,
            20
        ));
        assert!(!selection_name_fits(
            &PickerState::new(vec![one_cell_too_wide]),
            24,
            20
        ));
    }

    #[test]
    fn confirmation_uses_terminal_cell_width_for_wide_profile_names() {
        let mut two_wide = profile("開発");
        two_wide.current = true;
        let mut three_wide = profile("開発者");
        three_wide.current = true;
        let two_wide_characters = PickerState::new(vec![two_wide]);
        let three_wide_characters = PickerState::new(vec![three_wide]);

        assert!(selection_name_fits(&two_wide_characters, 18, 20));
        assert!(!selection_name_fits(&three_wide_characters, 18, 20));
    }

    #[test]
    fn confirmation_measures_the_visible_escape_for_zero_cell_profile_text() {
        let mut selected = profile("a\u{200b}b");
        selected.current = true;

        // The raw name is only two terminal cells, but its safe visible form
        // (`a\\u{200b}b`) needs ten. Confirmation must follow what was drawn.
        assert!(!selection_name_fits(
            &PickerState::new(vec![selected]),
            18,
            20
        ));
    }

    #[test]
    fn hidden_rows_are_not_eligible_for_selection_or_favorite_mutation() {
        let mut selected = profile("current-profile");
        selected.current = true;
        let state = PickerState::new(vec![selected]);

        for (width, height) in [(1, 1), (80, 5), (80, 6), (18, 20)] {
            assert!(!action_is_allowed(Action::Confirm, &state, width, height));
            assert!(!action_is_allowed(
                Action::ToggleFavorite,
                &state,
                width,
                height
            ));
            assert!(action_is_allowed(Action::Cancel, &state, width, height));
        }
    }
}
