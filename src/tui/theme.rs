use ratatui::style::{Color, Modifier, Style};

/// Color theme for the TUI
pub struct Theme {
    pub header: Style,
    pub selected: Style,
    pub normal: Style,
    pub favorite: Style,
    pub search_input: Style,
    pub search_prompt: Style,
    pub preview_key: Style,
    pub preview_value: Style,
    pub status_bar: Style,
    pub border: Style,
    pub border_focused: Style,
    pub hint: Style,
    pub profile_type_role: Style,
    pub profile_type_user: Style,
    pub profile_type_sso: Style,
}

impl Theme {
    pub fn default_theme(colors_enabled: bool) -> Self {
        if colors_enabled {
            Self {
                header: Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
                selected: Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
                normal: Style::default().fg(Color::White),
                favorite: Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
                search_input: Style::default().fg(Color::White),
                search_prompt: Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
                preview_key: Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
                preview_value: Style::default().fg(Color::White),
                status_bar: Style::default()
                    .fg(Color::White)
                    .bg(Color::DarkGray),
                border: Style::default().fg(Color::DarkGray),
                border_focused: Style::default().fg(Color::Cyan),
                hint: Style::default().fg(Color::DarkGray),
                profile_type_role: Style::default().fg(Color::Blue),
                profile_type_user: Style::default().fg(Color::Green),
                profile_type_sso: Style::default().fg(Color::Magenta),
            }
        } else {
            Self {
                header: Style::default().add_modifier(Modifier::BOLD),
                selected: Style::default()
                    .add_modifier(Modifier::REVERSED)
                    .add_modifier(Modifier::BOLD),
                normal: Style::default(),
                favorite: Style::default().add_modifier(Modifier::BOLD),
                search_input: Style::default(),
                search_prompt: Style::default().add_modifier(Modifier::BOLD),
                preview_key: Style::default().add_modifier(Modifier::BOLD),
                preview_value: Style::default(),
                status_bar: Style::default().add_modifier(Modifier::REVERSED),
                border: Style::default(),
                border_focused: Style::default().add_modifier(Modifier::BOLD),
                hint: Style::default(),
                profile_type_role: Style::default(),
                profile_type_user: Style::default(),
                profile_type_sso: Style::default(),
            }
        }
    }
}
