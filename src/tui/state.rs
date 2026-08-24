use std::cmp::Ordering;
use std::collections::BTreeMap;

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use super::{FavoriteChange, PickerResult, PickerSignal, PickerTermination, ViewProfile};
use crate::text_safety::is_terminal_control;

const MAX_QUERY_BYTES: usize = 4 * 1024;

#[derive(Clone, Debug)]
pub(super) struct Entry {
    pub(super) profile: ViewProfile,
    pub(super) favorite: bool,
}

#[derive(Debug)]
pub(super) struct PickerState {
    pub(super) entries: Vec<Entry>,
    pub(super) filtered: Vec<usize>,
    pub(super) focus: Option<usize>,
    pub(super) query: String,
    pub(super) cursor: usize,
    pub(super) show_preview: bool,
    original_favorites: BTreeMap<String, bool>,
    matcher: Matcher,
    utf32_buffer: Vec<char>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Action {
    Move(i32),
    MoveToStart,
    MoveToEnd,
    CursorLeft,
    CursorRight,
    Insert(char),
    Backspace,
    Delete,
    ClearQuery,
    ToggleFavorite,
    TogglePreview,
    Confirm,
    Cancel,
    Signal(PickerSignal),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReduceOutcome {
    Continue,
    Finish(PickerTermination),
}

impl PickerState {
    pub(super) fn new(profiles: Vec<ViewProfile>) -> Self {
        let original_favorites = profiles
            .iter()
            .map(|profile| (profile.name.clone(), profile.favorite))
            .collect();

        let mut entries: Vec<_> = profiles
            .into_iter()
            .map(|profile| Entry {
                favorite: profile.favorite,
                profile,
            })
            .collect();
        entries.sort_by(compare_default);

        let filtered: Vec<_> = (0..entries.len()).collect();
        let focus = entries.iter().position(|entry| entry.profile.current);

        Self {
            entries,
            filtered,
            focus,
            query: String::new(),
            cursor: 0,
            show_preview: true,
            original_favorites,
            matcher: Matcher::new(Config::DEFAULT),
            utf32_buffer: Vec::new(),
        }
    }

    pub(super) fn reduce(&mut self, action: Action) -> ReduceOutcome {
        match action {
            Action::Move(delta) => self.move_focus(delta),
            Action::MoveToStart => {
                self.focus = (!self.filtered.is_empty()).then_some(0);
            }
            Action::MoveToEnd => {
                self.focus = self.filtered.len().checked_sub(1);
            }
            Action::CursorLeft => self.cursor_left(),
            Action::CursorRight => self.cursor_right(),
            Action::Insert(character) => {
                if is_terminal_control(character) {
                    return ReduceOutcome::Continue;
                }
                if self.query.len().saturating_add(character.len_utf8()) > MAX_QUERY_BYTES {
                    return ReduceOutcome::Continue;
                }
                self.query.insert(self.cursor, character);
                self.cursor = self.cursor.saturating_add(character.len_utf8());
                self.update_filter(true);
            }
            Action::Backspace => self.backspace(),
            Action::Delete => self.delete(),
            Action::ClearQuery => {
                if !self.query.is_empty() {
                    self.query.clear();
                    self.cursor = 0;
                    self.update_filter(true);
                }
            }
            Action::ToggleFavorite => self.toggle_favorite(),
            Action::TogglePreview => self.show_preview = !self.show_preview,
            Action::Confirm => {
                if self.selected_entry().is_some() {
                    return ReduceOutcome::Finish(PickerTermination::Selected);
                }
            }
            Action::Cancel => return ReduceOutcome::Finish(PickerTermination::Cancelled),
            Action::Signal(signal) => {
                return ReduceOutcome::Finish(PickerTermination::Signal(signal));
            }
        }

        ReduceOutcome::Continue
    }

    pub(super) fn result(&self, termination: PickerTermination) -> PickerResult {
        let selection = matches!(termination, PickerTermination::Selected)
            .then(|| {
                self.selected_entry()
                    .map(|entry| entry.profile.name.clone())
            })
            .flatten();

        let favorite_changes = self
            .entries
            .iter()
            .filter_map(|entry| {
                let original = self.original_favorites.get(&entry.profile.name)?;
                (*original != entry.favorite).then(|| FavoriteChange {
                    profile_name: entry.profile.name.clone(),
                    favorite: entry.favorite,
                })
            })
            .collect();

        PickerResult {
            selection,
            favorite_changes,
            termination,
        }
    }

    pub(super) fn selected_entry(&self) -> Option<&Entry> {
        let entry_index = self.focus.and_then(|focus| self.filtered.get(focus))?;
        self.entries.get(*entry_index)
    }

    fn move_focus(&mut self, delta: i32) {
        if self.filtered.is_empty() {
            self.focus = None;
            return;
        }

        let last = self.filtered.len().saturating_sub(1);
        self.focus = match self.focus {
            None if delta < 0 => Some(last),
            None => Some(0),
            Some(current) if delta < 0 => {
                let distance = usize::try_from(delta.unsigned_abs()).unwrap_or(usize::MAX);
                Some(current.saturating_sub(distance))
            }
            Some(current) => {
                let distance = usize::try_from(delta.unsigned_abs()).unwrap_or(usize::MAX);
                Some(current.saturating_add(distance).min(last))
            }
        };
    }

    fn cursor_left(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.cursor = self
            .query
            .get(..self.cursor)
            .and_then(|prefix| prefix.char_indices().next_back().map(|(index, _)| index))
            .unwrap_or(0);
    }

    fn cursor_right(&mut self) {
        if self.cursor >= self.query.len() {
            return;
        }
        self.cursor = self
            .query
            .get(self.cursor..)
            .and_then(|suffix| suffix.char_indices().nth(1).map(|(index, _)| index))
            .map_or(self.query.len(), |offset| {
                self.cursor.saturating_add(offset)
            });
    }

    fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }

        let previous = self
            .query
            .get(..self.cursor)
            .and_then(|prefix| prefix.char_indices().next_back().map(|(index, _)| index));
        if let Some(previous) = previous {
            self.query.drain(previous..self.cursor);
            self.cursor = previous;
            self.update_filter(true);
        }
    }

    fn delete(&mut self) {
        if self.cursor >= self.query.len() {
            return;
        }

        let next = self
            .query
            .get(self.cursor..)
            .and_then(|suffix| suffix.char_indices().nth(1).map(|(index, _)| index))
            .map_or(self.query.len(), |offset| {
                self.cursor.saturating_add(offset)
            });
        self.query.drain(self.cursor..next);
        self.update_filter(true);
    }

    fn toggle_favorite(&mut self) {
        let focused_name = self
            .selected_entry()
            .map(|entry| entry.profile.name.clone());
        let Some(focused_name) = focused_name else {
            return;
        };

        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.profile.name == focused_name)
        {
            entry.favorite = !entry.favorite;
        }
        self.update_filter(false);
        self.focus_name(&focused_name);
    }

    fn update_filter(&mut self, focus_first_when_missing: bool) {
        let previous_name = self
            .selected_entry()
            .map(|entry| entry.profile.name.clone());
        let pattern = Pattern::new(
            &self.query,
            CaseMatching::Ignore,
            Normalization::Smart,
            AtomKind::Fuzzy,
        );
        let mut matches = Vec::with_capacity(self.entries.len());

        for (index, entry) in self.entries.iter().enumerate() {
            // Profile names are validated as finite Rust strings before the
            // picker starts. Reuse nucleo's scratch allocations across rows.
            self.utf32_buffer.clear();
            let haystack = Utf32Str::new(&entry.profile.name, &mut self.utf32_buffer);
            if let Some(score) = pattern.score(haystack, &mut self.matcher) {
                matches.push((index, score));
            }
        }

        matches.sort_by(|left, right| {
            right
                .1
                .cmp(&left.1)
                .then_with(|| compare_entries(&self.entries[left.0], &self.entries[right.0]))
        });
        self.filtered = matches.into_iter().map(|(index, _)| index).collect();

        self.focus = previous_name
            .as_deref()
            .and_then(|name| self.filtered_position(name));
        if self.focus.is_none() && focus_first_when_missing && !self.filtered.is_empty() {
            self.focus = Some(0);
        }
    }

    fn focus_name(&mut self, name: &str) {
        self.focus = self.filtered_position(name);
    }

    fn filtered_position(&self, name: &str) -> Option<usize> {
        self.filtered.iter().position(|entry_index| {
            self.entries
                .get(*entry_index)
                .is_some_and(|entry| entry.profile.name == name)
        })
    }
}

fn compare_default(left: &Entry, right: &Entry) -> Ordering {
    compare_entries(left, right)
}

fn compare_entries(left: &Entry, right: &Entry) -> Ordering {
    right
        .favorite
        .cmp(&left.favorite)
        .then_with(|| {
            right
                .profile
                .last_used_sequence
                .cmp(&left.profile.last_used_sequence)
        })
        .then_with(|| left.profile.name.cmp(&right.profile.name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::ViewProvider;

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

    fn type_query(state: &mut PickerState, query: &str) {
        for character in query.chars() {
            assert_eq!(
                state.reduce(Action::Insert(character)),
                ReduceOutcome::Continue
            );
        }
    }

    #[test]
    fn current_profile_has_initial_focus() {
        let mut current = profile("production");
        current.current = true;
        let state = PickerState::new(vec![profile("development"), current]);
        assert_eq!(
            state
                .selected_entry()
                .map(|entry| entry.profile.name.as_str()),
            Some("production")
        );
    }

    #[test]
    fn no_current_profile_starts_without_selection() {
        let mut state = PickerState::new(vec![profile("development"), profile("production")]);
        assert!(state.selected_entry().is_none());
        assert_eq!(state.reduce(Action::Confirm), ReduceOutcome::Continue);
    }

    #[test]
    fn empty_catalog_cannot_be_confirmed() {
        let mut state = PickerState::new(Vec::new());
        state.reduce(Action::Move(1));
        assert_eq!(state.reduce(Action::Confirm), ReduceOutcome::Continue);
        assert!(
            state
                .result(PickerTermination::Cancelled)
                .selection
                .is_none()
        );
    }

    #[test]
    fn fuzzy_query_only_selects_an_exact_visible_name() {
        let mut state = PickerState::new(vec![profile("production-admin"), profile("staging")]);
        type_query(&mut state, "prdadm");
        assert_eq!(
            state
                .selected_entry()
                .map(|entry| entry.profile.name.as_str()),
            Some("production-admin")
        );
        assert_eq!(
            state.reduce(Action::Confirm),
            ReduceOutcome::Finish(PickerTermination::Selected)
        );
        assert_eq!(
            state
                .result(PickerTermination::Selected)
                .selection
                .as_deref(),
            Some("production-admin")
        );
    }

    #[test]
    fn zero_matches_cannot_be_confirmed() {
        let mut state = PickerState::new(vec![profile("development")]);
        type_query(&mut state, "zzz");
        assert!(state.filtered.is_empty());
        assert_eq!(state.reduce(Action::Confirm), ReduceOutcome::Continue);
    }

    #[test]
    fn favorite_changes_are_deltas_and_reverting_removes_delta() {
        let mut current = profile("development");
        current.current = true;
        let mut state = PickerState::new(vec![current]);
        state.reduce(Action::ToggleFavorite);
        assert_eq!(
            state.result(PickerTermination::Cancelled).favorite_changes,
            vec![FavoriteChange {
                profile_name: "development".to_string(),
                favorite: true,
            }]
        );
        state.reduce(Action::ToggleFavorite);
        assert!(
            state
                .result(PickerTermination::Cancelled)
                .favorite_changes
                .is_empty()
        );
    }

    #[test]
    fn unicode_query_editing_preserves_character_boundaries() {
        let mut state = PickerState::new(vec![profile("東京-開発")]);
        type_query(&mut state, "東京");
        state.reduce(Action::CursorLeft);
        state.reduce(Action::Delete);
        assert_eq!(state.query, "東");
        state.reduce(Action::Backspace);
        assert!(state.query.is_empty());
    }

    #[test]
    fn ordering_is_favorite_then_sequence_then_name() {
        let mut favorite = profile("z-favorite");
        favorite.favorite = true;
        let mut recent = profile("b-recent");
        recent.last_used_sequence = Some(20);
        let mut older = profile("a-older");
        older.last_used_sequence = Some(10);
        let state = PickerState::new(vec![older, recent, favorite]);
        let names: Vec<_> = state
            .entries
            .iter()
            .map(|entry| entry.profile.name.as_str())
            .collect();
        assert_eq!(names, vec!["z-favorite", "b-recent", "a-older"]);
    }

    #[test]
    fn matching_is_case_insensitive_and_deterministic() {
        let mut state = PickerState::new(vec![profile("MY-SSO-PROFILE"), profile("role-profile")]);
        type_query(&mut state, "sso");
        let names: Vec<_> = state
            .filtered
            .iter()
            .filter_map(|index| state.entries.get(*index))
            .map(|entry| entry.profile.name.as_str())
            .collect();
        assert_eq!(names, vec!["MY-SSO-PROFILE"]);
    }

    #[test]
    fn matcher_prefers_contiguous_boundary_match() {
        let mut state = PickerState::new(vec![
            profile("profile-archive-directory"),
            profile("production-admin"),
        ]);
        type_query(&mut state, "prod-adm");
        assert_eq!(
            state
                .selected_entry()
                .map(|entry| entry.profile.name.as_str()),
            Some("production-admin")
        );
    }

    #[test]
    fn matcher_supports_unicode_queries() {
        let mut state = PickerState::new(vec![profile("東京-開発"), profile("大阪-本番")]);
        type_query(&mut state, "東京開");
        assert_eq!(
            state
                .selected_entry()
                .map(|entry| entry.profile.name.as_str()),
            Some("東京-開発")
        );
    }

    #[test]
    fn query_growth_is_bounded() {
        let mut state = PickerState::new(vec![profile("development")]);
        for _ in 0..=MAX_QUERY_BYTES {
            state.reduce(Action::Insert('x'));
        }
        assert_eq!(state.query.len(), MAX_QUERY_BYTES);
    }

    #[test]
    fn terminal_controls_and_bidi_marks_never_enter_the_query() {
        let mut state = PickerState::new(vec![profile("development")]);
        for character in ['\u{1b}', '\u{061c}', '\u{200f}', '\u{202e}', '\u{2066}'] {
            assert_eq!(
                state.reduce(Action::Insert(character)),
                ReduceOutcome::Continue
            );
        }
        assert!(state.query.is_empty());
    }

    #[test]
    #[ignore = "release-mode performance contract; CI runs this explicitly"]
    fn performance_contract_fuzzy_filter_1000_profiles_p95() {
        use std::time::{Duration, Instant};

        if cfg!(debug_assertions) {
            panic!("run this contract with --release");
        }
        const PROFILE_COUNT: usize = 1_000;
        const SAMPLES: usize = 101;
        const P95_BUDGET: Duration = Duration::from_millis(16);

        let profiles = (0..PROFILE_COUNT)
            .map(|index| profile(&format!("production-profile-{index:04}")))
            .collect();
        let mut state = PickerState::new(profiles);
        let mut samples = Vec::with_capacity(SAMPLES);
        for _ in 0..SAMPLES {
            let started = Instant::now();
            assert_eq!(state.reduce(Action::Insert('p')), ReduceOutcome::Continue);
            samples.push(started.elapsed());
            assert_eq!(state.reduce(Action::ClearQuery), ReduceOutcome::Continue);
        }
        samples.sort_unstable();
        let p95 = samples[(SAMPLES * 95).div_ceil(100) - 1];
        eprintln!("fuzzy-filter-1000 p95={p95:?}, budget={P95_BUDGET:?}");
        assert!(
            p95 <= P95_BUDGET,
            "fuzzy-filter-1000 p95 {p95:?} exceeded {P95_BUDGET:?}"
        );
    }
}
