use std::ffi::OsStr;

use ratatui::style::{Color, Modifier, Style};

/// Theme configuration for the TUI
#[derive(Debug, Clone)]
pub struct Theme {
    // Colors
    pub primary: Color,
    pub secondary: Color,
    pub accent: Color,
    pub success: Color,
    pub warning: Color,
    pub error: Color,
    pub muted: Color,
    pub background: Color,
    pub surface: Color,
    /// No colors at all; emphasis comes from modifiers only.
    pub monochrome: bool,

    // Icons (Nerd Font compatible)
    pub icons: Icons,
}

#[derive(Debug, Clone)]
pub struct Icons {
    pub role: &'static str,
    pub user: &'static str,
    pub favorite: &'static str,
    pub favorite_empty: &'static str,
    pub mfa: &'static str,
    pub region: &'static str,
    pub account: &'static str,
    pub arrow_right: &'static str,
    pub arrow_down: &'static str,
    pub check: &'static str,
    pub cross: &'static str,
    pub clock: &'static str,
    pub lock: &'static str,
    pub key: &'static str,
    pub chain: &'static str,
    pub search: &'static str,
    pub spinner: &'static [&'static str],
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            // Tokyo Night inspired colors
            primary: Color::Rgb(122, 162, 247),   // Blue
            secondary: Color::Rgb(187, 154, 247), // Purple
            accent: Color::Rgb(125, 207, 255),    // Cyan
            success: Color::Rgb(158, 206, 106),   // Green
            warning: Color::Rgb(224, 175, 104),   // Yellow/Orange
            error: Color::Rgb(247, 118, 142),     // Red
            muted: Color::Rgb(86, 95, 137),       // Gray
            background: Color::Rgb(26, 27, 38),   // Dark
            surface: Color::Rgb(36, 40, 59),      // Slightly lighter
            monochrome: false,

            icons: Icons::default(),
        }
    }
}

impl Default for Icons {
    fn default() -> Self {
        Self {
            role: "󰁥 ",          // Role icon
            user: "󰀄 ",          // User icon
            favorite: "★",       // Filled star
            favorite_empty: "☆", // Empty star
            mfa: "󰌋 ",           // Shield/lock
            region: "󰍎 ",        // Globe
            account: "󰋊 ",       // Building
            arrow_right: "→",
            arrow_down: "↓",
            check: "✓",
            cross: "✗",
            clock: "󰥔 ", // Clock
            lock: "󰌾 ",  // Lock
            key: "󰌆 ",   // Key
            chain: "󰌷 ", // Chain link
            search: " ", // Nerd Font search icon (nf-cod-search)
            spinner: &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
        }
    }
}

impl Theme {
    /// The default theme, or a colorless one when `NO_COLOR` is set to a
    /// non-empty value (https://no-color.org).
    pub fn from_env() -> Self {
        Self::for_no_color(std::env::var_os("NO_COLOR").as_deref())
    }

    fn for_no_color(no_color: Option<&OsStr>) -> Self {
        if no_color.is_some_and(|v| !v.is_empty()) {
            Self::monochrome()
        } else {
            Self::default()
        }
    }

    fn monochrome() -> Self {
        Self {
            primary: Color::Reset,
            secondary: Color::Reset,
            accent: Color::Reset,
            success: Color::Reset,
            warning: Color::Reset,
            error: Color::Reset,
            muted: Color::Reset,
            background: Color::Reset,
            surface: Color::Reset,
            monochrome: true,
            icons: Icons::default(),
        }
    }

    // Style helpers
    pub fn title_style(&self) -> Style {
        Style::default()
            .fg(self.primary)
            .add_modifier(Modifier::BOLD)
    }

    pub fn selected_style(&self) -> Style {
        // Without colors the selection would be invisible; reverse it instead.
        if self.monochrome {
            return Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD);
        }
        Style::default()
            .fg(self.background)
            .bg(self.primary)
            .add_modifier(Modifier::BOLD)
    }

    pub fn highlight_style(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    pub fn muted_style(&self) -> Style {
        Style::default().fg(self.muted)
    }

    pub fn success_style(&self) -> Style {
        Style::default().fg(self.success)
    }

    pub fn warning_style(&self) -> Style {
        Style::default().fg(self.warning)
    }

    pub fn error_style(&self) -> Style {
        Style::default().fg(self.error)
    }

    pub fn favorite_style(&self) -> Style {
        Style::default().fg(self.warning)
    }
}

impl Icons {}

/// Profile type indicator with color
pub enum ProfileType {
    Role,
    User,
    Sso,
}

impl ProfileType {
    pub fn style(&self, theme: &Theme) -> Style {
        match self {
            ProfileType::Role => Style::default().fg(theme.secondary),
            ProfileType::User => Style::default().fg(theme.primary),
            ProfileType::Sso => Style::default().fg(theme.accent),
        }
    }

    pub fn icon(&self, icons: &Icons) -> &'static str {
        match self {
            ProfileType::Role => icons.role,
            ProfileType::User => icons.user,
            ProfileType::Sso => icons.key,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            ProfileType::Role => "Role",
            ProfileType::User => "User",
            ProfileType::Sso => "SSO",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_color_set_and_non_empty_disables_color() {
        let t = Theme::for_no_color(Some(OsStr::new("1")));
        assert!(t.monochrome);
        assert_eq!(t.primary, Color::Reset);
        assert!(t.selected_style().add_modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn no_color_unset_or_empty_keeps_color() {
        assert!(!Theme::for_no_color(None).monochrome);
        assert!(!Theme::for_no_color(Some(OsStr::new(""))).monochrome);
    }
}
