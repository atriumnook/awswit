use clap::{Parser, Subcommand};

/// AWSume-rs: A Rust implementation of AWSume - AWS Assume Made Awesome
///
/// A convenient way to manage session tokens and assume role credentials.
#[derive(Parser, Debug, Clone)]
#[command(name = "awswit")]
#[command(author, version, about, long_about = None)]
#[command(after_help = "Thank you for using AWSume-rs! https://github.com/yourusername/awswit")]
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
    #[arg(short = 'k', long = "kill")]
    pub kill_refresher: bool,

    /// A profile to output credentials to
    #[arg(short = 'o', long = "output-profile", value_name = "output_profile")]
    pub output_profile: Option<String>,

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

    /// Session policy JSON document
    #[arg(long = "session-policy", value_name = "session_policy")]
    pub session_policy: Option<String>,

    /// Session policy ARNs
    #[arg(long = "session-policy-arns", value_name = "session_policy_arns", num_args = 1..)]
    pub session_policy_arns: Option<Vec<String>>,

    /// Role duration in seconds
    #[arg(long = "role-duration", value_name = "role_duration")]
    pub role_duration: Option<i32>,

    /// Use SAML assertion for authentication
    #[arg(long = "with-saml", conflicts_with = "with_web_identity")]
    pub with_saml: bool,

    /// Use web identity for authentication
    #[arg(long = "with-web-identity", conflicts_with = "with_saml")]
    pub with_web_identity: bool,

    /// Path to credentials file
    #[arg(long = "credentials-file", value_name = "credentials_file")]
    pub credentials_file: Option<String>,

    /// Path to config file
    #[arg(long = "config-file", value_name = "config_file")]
    pub config_file: Option<String>,

    /// Manage awswit configuration (set/get/reset/list)
    #[arg(long = "config", value_name = "option", num_args = 0..)]
    pub config: Option<Vec<String>>,

    /// Display INFO level logs
    #[arg(long = "info")]
    pub info: bool,

    /// Display DEBUG level logs
    #[arg(long = "debug")]
    pub debug: bool,

    /// Clean up expired output profiles
    #[arg(long = "clean")]
    pub clean: bool,

    /// Principal ARN for SAML
    #[arg(long = "principal-arn", value_name = "principal_arn")]
    pub principal_arn: Option<String>,

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

#[cfg(test)]
mod tests {
    use super::*;

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
}

impl Default for Args {
    fn default() -> Self {
        Self {
            command: None,
            profile_name: None,
            version: false,
            force_refresh: false,
            show_commands: false,
            unset: false,
            auto_refresh: false,
            kill_refresher: false,
            output_profile: None,
            list_profiles: None,
            refresh_autocomplete: false,
            role_arn: None,
            source_profile: None,
            external_id: None,
            mfa_token: None,
            region: None,
            session_name: None,
            session_policy: None,
            session_policy_arns: None,
            role_duration: None,
            with_saml: false,
            with_web_identity: false,
            credentials_file: None,
            config_file: None,
            config: None,
            info: false,
            debug: false,
            clean: false,
            principal_arn: None,
            no_interactive: false,
        }
    }
}
