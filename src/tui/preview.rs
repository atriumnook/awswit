use chrono::{DateTime, Utc};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use super::theme::{ProfileType, Theme};
use crate::profile::Profile;

/// Right-hand preview pane for the selected profile.
///
/// We keep one paragraph per logical row and let ratatui wrap, so long
/// role ARNs and SSO start URLs don't get clipped on narrow terminals.
pub struct ProfilePreview<'a> {
    profile: &'a Profile,
    last_used: Option<DateTime<Utc>>,
    theme: &'a Theme,
}

impl<'a> ProfilePreview<'a> {
    pub fn new(profile: &'a Profile, last_used: Option<DateTime<Utc>>, theme: &'a Theme) -> Self {
        Self {
            profile,
            last_used,
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

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2), // Type / Region / Account
                Constraint::Min(2),    // Role ARN (wraps)
                Constraint::Length(2), // Source chain
                Constraint::Length(2), // MFA / last-used
            ])
            .split(inner);

        self.render_type_region(frame, chunks[0]);
        self.render_role_arn(frame, chunks[1]);
        self.render_source_chain(frame, chunks[2]);
        self.render_footer(frame, chunks[3]);
    }

    fn render_type_region(&self, frame: &mut Frame, area: Rect) {
        let profile_type = profile_type(self.profile);

        let type_span = Span::styled(
            format!(
                "{}{}",
                profile_type.icon(&self.theme.icons),
                profile_type.label()
            ),
            profile_type.style(self.theme).add_modifier(Modifier::BOLD),
        );
        let region = self.profile.region.as_deref().unwrap_or("(default)");
        let account = self
            .profile
            .get_account_id()
            .unwrap_or_else(|| "Unknown".to_string());

        let lines = vec![
            Line::from(vec![type_span]),
            Line::from(vec![
                Span::styled(
                    format!("{}Region: {}", self.theme.icons.region, region),
                    self.theme.muted_style(),
                ),
                Span::raw("  "),
                Span::styled(
                    format!("{}Account: {}", self.theme.icons.account, account),
                    self.theme.muted_style(),
                ),
            ]),
        ];
        frame.render_widget(Paragraph::new(lines), area);
    }

    fn render_role_arn(&self, frame: &mut Frame, area: Rect) {
        if let Some(role_arn) = &self.profile.role_arn {
            let line = Line::from(vec![
                Span::styled(
                    format!("{}Role: ", self.theme.icons.role),
                    self.theme.muted_style(),
                ),
                Span::styled(role_arn.clone(), Style::default().fg(self.theme.secondary)),
            ]);
            frame.render_widget(Paragraph::new(line).wrap(Wrap { trim: true }), area);
        } else if let Some(start) = &self.profile.sso_start_url {
            let line = Line::from(vec![
                Span::styled(
                    format!("{}SSO start: ", self.theme.icons.key),
                    self.theme.muted_style(),
                ),
                Span::styled(start.clone(), Style::default().fg(self.theme.secondary)),
            ]);
            frame.render_widget(Paragraph::new(line).wrap(Wrap { trim: true }), area);
        }
    }

    fn render_source_chain(&self, frame: &mut Frame, area: Rect) {
        let mut spans: Vec<Span> = vec![];
        if let Some(source) = &self.profile.source_profile {
            spans.push(Span::styled(
                format!("{}Chain: ", self.theme.icons.chain),
                self.theme.muted_style(),
            ));
            spans.push(Span::styled(
                source.clone(),
                Style::default().fg(self.theme.primary),
            ));
            spans.push(Span::styled(
                format!(" {} ", self.theme.icons.arrow_right),
                self.theme.muted_style(),
            ));
            spans.push(Span::styled(
                self.profile.name.clone(),
                Style::default().fg(self.theme.accent),
            ));
        } else if let Some(cred) = &self.profile.credential_source {
            spans.push(Span::styled(
                format!("{}Source: ", self.theme.icons.key),
                self.theme.muted_style(),
            ));
            spans.push(Span::styled(
                cred.clone(),
                Style::default().fg(self.theme.warning),
            ));
        }
        if !spans.is_empty() {
            frame.render_widget(Paragraph::new(Line::from(spans)), area);
        }
    }

    fn render_footer(&self, frame: &mut Frame, area: Rect) {
        let mfa = if self.profile.mfa_serial.is_some() {
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

        let mut spans = vec![mfa, Span::raw("  ")];
        if let Some(ts) = self.last_used {
            spans.push(Span::styled(
                format!("Last used {}", human_age(ts)),
                self.theme.muted_style(),
            ));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }
}

fn profile_type(p: &Profile) -> ProfileType {
    if p.is_sso_profile() {
        ProfileType::Sso
    } else if p.is_role_profile() {
        ProfileType::Role
    } else {
        ProfileType::User
    }
}

/// `"just now"`, `"5m ago"`, `"3h ago"`, `"2d ago"`, `"5w ago"`.
fn human_age(ts: DateTime<Utc>) -> String {
    let delta = Utc::now() - ts;
    let secs = delta.num_seconds();
    if secs < 60 {
        "just now".into()
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else if secs < 86_400 * 14 {
        format!("{}d ago", secs / 86_400)
    } else {
        format!("{}w ago", secs / (86_400 * 7))
    }
}

/// Compact preview line used in the list view.
pub fn compact_preview_line(profile: &Profile, theme: &Theme) -> Line<'static> {
    let profile_type = profile_type(profile);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_age_buckets() {
        let now = Utc::now();
        assert_eq!(human_age(now), "just now");
        assert_eq!(human_age(now - chrono::Duration::minutes(5)), "5m ago");
        assert_eq!(human_age(now - chrono::Duration::hours(3)), "3h ago");
        assert_eq!(human_age(now - chrono::Duration::days(2)), "2d ago");
        assert_eq!(human_age(now - chrono::Duration::days(30)), "4w ago");
    }
}
