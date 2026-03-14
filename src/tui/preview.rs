use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use super::theme::{ProfileType, Theme};
use crate::history::ProfileHistory;
use crate::profile::Profile;

/// Profile preview panel
pub struct ProfilePreview<'a> {
    profile: &'a Profile,
    history: Option<&'a ProfileHistory>,
    theme: &'a Theme,
}

impl<'a> ProfilePreview<'a> {
    pub fn new(profile: &'a Profile, theme: &'a Theme) -> Self {
        Self {
            profile,
            history: None,
            theme,
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.theme.muted))
            .title(Span::styled(
                format!(" {} ", self.profile.name),
                self.theme.title_style(),
            ));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        // Layout for preview content
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Type & Region
                Constraint::Length(3), // Role ARN
                Constraint::Length(2), // Source profile chain
                Constraint::Length(2), // MFA info
                Constraint::Min(0),    // Last used / extra info
            ])
            .split(inner);

        self.render_type_region(frame, chunks[0]);
        self.render_role_arn(frame, chunks[1]);
        self.render_source_chain(frame, chunks[2]);
        self.render_mfa_info(frame, chunks[3]);
        self.render_history(frame, chunks[4]);
    }

    fn render_type_region(&self, frame: &mut Frame, area: Rect) {
        let profile_type = if self.profile.is_role_profile() {
            ProfileType::Role
        } else if self.profile.is_sso_profile() {
            ProfileType::Sso
        } else {
            ProfileType::User
        };

        let type_span = Span::styled(
            format!(
                "{}{}",
                profile_type.icon(&self.theme.icons),
                profile_type.label()
            ),
            profile_type.style(self.theme).add_modifier(Modifier::BOLD),
        );

        let region = self.profile.region.as_deref().unwrap_or("(default)");
        let region_span = Span::styled(
            format!("  {}Region: {}", self.theme.icons.region, region),
            self.theme.muted_style(),
        );

        let account = self
            .profile
            .get_account_id()
            .unwrap_or_else(|| "Unknown".to_string());
        let account_span = Span::styled(
            format!("  {}Account: {}", self.theme.icons.account, account),
            self.theme.muted_style(),
        );

        let lines = vec![
            Line::from(vec![type_span]),
            Line::from(vec![region_span, account_span]),
        ];

        let paragraph = Paragraph::new(lines);
        frame.render_widget(paragraph, area);
    }

    fn render_role_arn(&self, frame: &mut Frame, area: Rect) {
        if let Some(ref role_arn) = self.profile.role_arn {
            let label = Span::styled(
                format!("{}Role: ", self.theme.icons.role),
                self.theme.muted_style(),
            );
            let value = Span::styled(role_arn.clone(), Style::default().fg(self.theme.secondary));

            let line = Line::from(vec![label, value]);
            let paragraph = Paragraph::new(vec![line]).wrap(Wrap { trim: true });
            frame.render_widget(paragraph, area);
        }
    }

    fn render_source_chain(&self, frame: &mut Frame, area: Rect) {
        let mut chain_parts = vec![];

        if let Some(ref source) = self.profile.source_profile {
            chain_parts.push(Span::styled(
                format!("{}Chain: ", self.theme.icons.chain),
                self.theme.muted_style(),
            ));
            chain_parts.push(Span::styled(
                source.clone(),
                Style::default().fg(self.theme.primary),
            ));
            chain_parts.push(Span::styled(
                format!(" {} ", self.theme.icons.arrow_right),
                self.theme.muted_style(),
            ));
            chain_parts.push(Span::styled(
                self.profile.name.clone(),
                Style::default().fg(self.theme.accent),
            ));
        } else if let Some(ref cred_source) = self.profile.credential_source {
            chain_parts.push(Span::styled(
                format!("{}Source: ", self.theme.icons.key),
                self.theme.muted_style(),
            ));
            chain_parts.push(Span::styled(
                cred_source.clone(),
                Style::default().fg(self.theme.warning),
            ));
        }

        if !chain_parts.is_empty() {
            let line = Line::from(chain_parts);
            let paragraph = Paragraph::new(vec![line]);
            frame.render_widget(paragraph, area);
        }
    }

    fn render_mfa_info(&self, frame: &mut Frame, area: Rect) {
        let mfa_status = if self.profile.mfa_serial.is_some() {
            Span::styled(
                format!("{}MFA Required", self.theme.icons.mfa),
                self.theme.warning_style(),
            )
        } else {
            Span::styled(
                format!("{}No MFA", self.theme.icons.check),
                self.theme.success_style(),
            )
        };

        let line = Line::from(vec![mfa_status]);
        let paragraph = Paragraph::new(vec![line]);
        frame.render_widget(paragraph, area);
    }

    fn render_history(&self, frame: &mut Frame, area: Rect) {
        if let Some(history) = self.history
            && let Some(entry) = history.get(&self.profile.name)
        {
            let duration = chrono::Utc::now() - entry.last_used;
            let time_ago = format_duration(duration);

            let line = Line::from(vec![
                Span::styled(
                    format!("{}Last used: ", self.theme.icons.clock),
                    self.theme.muted_style(),
                ),
                Span::styled(time_ago, Style::default().fg(self.theme.muted)),
            ]);

            let use_count = Line::from(vec![Span::styled(
                format!("  Used {} times", entry.use_count),
                self.theme.muted_style(),
            )]);

            let paragraph = Paragraph::new(vec![line, use_count]);
            frame.render_widget(paragraph, area);
        }
    }
}

/// Format a duration as human-readable time ago string
fn format_duration(duration: chrono::Duration) -> String {
    let secs = duration.num_seconds();
    if secs < 60 {
        "just now".to_string()
    } else if secs < 3600 {
        format!("{} minutes ago", secs / 60)
    } else if secs < 86400 {
        format!("{} hours ago", secs / 3600)
    } else if secs < 604800 {
        format!("{} days ago", secs / 86400)
    } else {
        format!("{} weeks ago", secs / 604800)
    }
}

/// Compact preview line for list view
pub fn compact_preview_line(profile: &Profile, theme: &Theme) -> Line<'static> {
    let profile_type = if profile.is_role_profile() {
        ProfileType::Role
    } else if profile.is_sso_profile() {
        ProfileType::Sso
    } else {
        ProfileType::User
    };

    let type_span = Span::styled(
        format!("{:<5}", profile_type.label()),
        profile_type.style(theme),
    );

    let region = profile.region.as_deref().unwrap_or("-");
    let region_span = Span::styled(format!("{:<12}", region), theme.muted_style());

    let account = profile.get_account_id().unwrap_or_else(|| "-".to_string());
    let account_span = Span::styled(account, theme.muted_style());

    let mfa_span = if profile.mfa_serial.is_some() {
        Span::styled(theme.icons.mfa.to_string(), theme.warning_style())
    } else {
        Span::raw("  ")
    };

    Line::from(vec![
        type_span,
        Span::raw(" "),
        region_span,
        Span::raw(" "),
        account_span,
        Span::raw(" "),
        mfa_span,
    ])
}
