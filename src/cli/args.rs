use clap::{Parser, Subcommand};

/// awswit: A fast, modern AWS profile switcher with interactive TUI
///
/// A convenient way to manage session tokens and assume role credentials.
#[derive(Parser, Clone, Default)]
#[command(name = "awswit")]
#[command(author, about, long_about = None, disable_version_flag = true)]
#[command(after_help = "Thank you for using awswit!")]
#[command(args_conflicts_with_subcommands = true)]
pub struct Args {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// The target profile name
    #[arg(value_name = "profile_name")]
    pub profile_name: Option<String>,

    /// Display the current version of awswit
    #[arg(short = 'v', long = "version")]
    pub version: bool,

    /// Force refresh credentials
    #[arg(short = 'r', long = "refresh")]
    pub force_refresh: bool,

    /// Show the commands to set the credentials
    #[arg(short = 's', long = "show-commands")]
    pub show_commands: bool,

    /// Unset your AWS environment variables
    #[arg(short = 'u', long = "unset")]
    pub unset: bool,

    /// Auto-refresh credentials in the background
    #[arg(short = 'a', long = "auto-refresh")]
    pub auto_refresh: bool,

    /// Kill the auto-refresher for a profile (or all if no profile specified)
    #[arg(short = 'k', long = "kill-refresher", alias = "kill")]
    pub kill_refresher: bool,

    /// List available profiles. Pass 'more' for additional details
    #[arg(short = 'l', long = "list-profiles", value_name = "detail_level", num_args = 0..=1, default_missing_value = "")]
    pub list_profiles: Option<String>,

    /// Refresh the autocomplete profile cache
    #[arg(long = "refresh-autocomplete")]
    pub refresh_autocomplete: bool,

    /// Role ARN to assume (can use shorthand: account_id:role_name)
    #[arg(long = "role-arn", value_name = "role_arn")]
    pub role_arn: Option<String>,

    /// Source profile to use for assuming the role
    #[arg(long = "source-profile", value_name = "source_profile")]
    pub source_profile: Option<String>,

    /// External ID for assuming the role
    #[arg(long = "external-id", value_name = "external_id")]
    pub external_id: Option<String>,

    /// MFA token code
    #[arg(long = "mfa-token", value_name = "mfa_token")]
    pub mfa_token: Option<String>,

    /// AWS region to use
    #[arg(long = "region", value_name = "region")]
    pub region: Option<String>,

    /// Session name for the assumed role
    #[arg(long = "session-name", value_name = "session_name")]
    pub session_name: Option<String>,

    /// Role duration in seconds
    #[arg(long = "role-duration", value_name = "role_duration")]
    pub role_duration: Option<i32>,

    /// Path to credentials file
    #[arg(long = "credentials-file", value_name = "credentials_file")]
    pub credentials_file: Option<String>,

    /// Path to config file
    #[arg(long = "config-file", value_name = "config_file")]
    pub config_file: Option<String>,

    /// Display INFO level logs
    #[arg(long = "info")]
    pub info: bool,

    /// Display DEBUG level logs
    #[arg(long = "debug")]
    pub debug: bool,

    /// Disable interactive mode (use when piping or scripting)
    #[arg(long = "no-interactive", short = 'n')]
    pub no_interactive: bool,
}

#[derive(Subcommand, Debug, Clone)]
pub enum Command {
    /// Output shell integration script
    ///
    /// Usage:
    ///   eval "$(awswit init bash)"
    ///   eval "$(awswit init zsh)"
    ///   awswit init fish | source
    ///   awswit init powershell | Invoke-Expression
    Init {
        /// Shell type (bash, zsh, fish, powershell)
        shell: String,
    },
}

impl Args {
    /// Parse role ARN from shorthand format (account_id:role_name) if needed
    pub fn resolve_role_arn(&self) -> Option<String> {
        self.role_arn.as_ref().map(|arn| {
            if arn.starts_with("arn:") {
                arn.clone()
            } else if arn.contains(':') {
                // Shorthand format: account_id:role_name
                // Note: shorthand always uses arn:aws partition. For GovCloud/China, use full ARN.
                let parts: Vec<&str> = arn.splitn(2, ':').collect();
                if parts.len() == 2 {
                    format!("arn:aws:iam::{}:role/{}", parts[0], parts[1])
                } else {
                    arn.clone()
                }
            } else {
                arn.clone()
            }
        })
    }

    /// Get session name with fallback
    pub fn get_session_name(&self, profile_name: &str) -> String {
        self.session_name.clone().unwrap_or_else(|| {
            if profile_name.len() < 2 {
                format!("_{}_", profile_name)
            } else {
                profile_name.to_string()
            }
        })
    }

    /// Check if interactive mode is disabled
    pub fn interactive_disabled(&self) -> bool {
        self.no_interactive
    }
}

impl std::fmt::Debug for Args {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Args")
            .field("command", &self.command)
            .field("profile_name", &self.profile_name)
            .field("version", &self.version)
            .field("force_refresh", &self.force_refresh)
            .field("show_commands", &self.show_commands)
            .field("unset", &self.unset)
            .field("auto_refresh", &self.auto_refresh)
            .field("kill_refresher", &self.kill_refresher)
            .field("list_profiles", &self.list_profiles)
            .field("refresh_autocomplete", &self.refresh_autocomplete)
            .field("role_arn", &self.role_arn)
            .field("source_profile", &self.source_profile)
            .field("external_id", &self.external_id)
            .field("mfa_token", &self.mfa_token.as_ref().map(|_| "[REDACTED]"))
            .field("region", &self.region)
            .field("session_name", &self.session_name)
            .field("role_duration", &self.role_duration)
            .field("credentials_file", &self.credentials_file)
            .field("config_file", &self.config_file)
            .field("info", &self.info)
            .field("debug", &self.debug)
            .field("no_interactive", &self.no_interactive)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_debug_redacts_mfa_token() {
        let args = Args {
            mfa_token: Some("123456".to_string()),
            ..Default::default()
        };
        let debug_output = format!("{:?}", args);
        assert!(!debug_output.contains("123456"));
        assert!(debug_output.contains("[REDACTED]"));
    }

    #[test]
    fn test_resolve_role_arn_full() {
        let args = Args {
            role_arn: Some("arn:aws:iam::123456789012:role/MyRole".to_string()),
            ..Default::default()
        };
        assert_eq!(
            args.resolve_role_arn(),
            Some("arn:aws:iam::123456789012:role/MyRole".to_string())
        );
    }

    #[test]
    fn test_resolve_role_arn_shorthand() {
        let args = Args {
            role_arn: Some("123456789012:MyRole".to_string()),
            ..Default::default()
        };
        assert_eq!(
            args.resolve_role_arn(),
            Some("arn:aws:iam::123456789012:role/MyRole".to_string())
        );
    }

    #[test]
    fn test_get_session_name_default() {
        let args = Args::default();
        assert_eq!(args.get_session_name("my-profile"), "my-profile");
    }

    #[test]
    fn test_get_session_name_short_profile() {
        let args = Args::default();
        assert_eq!(args.get_session_name("x"), "_x_");
    }

    #[test]
    fn test_kill_refresher_flag_both_forms() {
        use clap::Parser;
        let args = Args::try_parse_from(["awswit", "--kill-refresher"]).unwrap();
        assert!(args.kill_refresher);
        let args = Args::try_parse_from(["awswit", "--kill"]).unwrap();
        assert!(args.kill_refresher);
        let args = Args::try_parse_from(["awswit", "-k"]).unwrap();
        assert!(args.kill_refresher);
    }
}
