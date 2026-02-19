use clap::Parser;

#[derive(Parser, Debug, Clone)]
#[command(
    name = "awswit",
    version = env!("CARGO_PKG_VERSION"),
    about = "Fast AWS profile switcher with interactive TUI",
    long_about = "awswit - A fast, Rust-based AWS profile switcher with fuzzy search and interactive TUI"
)]
pub struct Args {
    /// Profile name to switch to
    #[arg(value_name = "PROFILE_NAME")]
    pub profile_name: Option<String>,

    /// Force credential refresh (ignore cache)
    #[arg(short = 'r', long = "refresh")]
    pub refresh: bool,

    /// Show export commands without executing
    #[arg(short = 's', long = "show-commands")]
    pub show_commands: bool,

    /// Unset AWS environment variables
    #[arg(short = 'u', long = "unset")]
    pub unset: bool,

    /// Enable auto-refresh for credentials
    #[arg(short = 'a', long = "auto-refresh")]
    pub auto_refresh: bool,

    /// Kill the auto-refresh daemon
    #[arg(short = 'k', long = "kill-refresher")]
    pub kill_refresher: bool,

    /// List profiles (use 'more' for detailed view)
    #[arg(short = 'l', long = "list-profiles", num_args = 0..=1, default_missing_value = "simple")]
    pub list_profiles: Option<String>,

    /// Disable interactive mode
    #[arg(short = 'n', long = "no-interactive")]
    pub no_interactive: bool,

    /// Directly specify a role ARN
    #[arg(long = "role-arn")]
    pub role_arn: Option<String>,

    /// Specify source profile for role assumption
    #[arg(long = "source-profile")]
    pub source_profile: Option<String>,

    /// External ID for role assumption
    #[arg(long = "external-id")]
    pub external_id: Option<String>,

    /// MFA token code
    #[arg(long = "mfa-token")]
    pub mfa_token: Option<String>,

    /// AWS region
    #[arg(long = "region")]
    pub region: Option<String>,

    /// Session name for role assumption
    #[arg(long = "session-name")]
    pub session_name: Option<String>,

    /// Role session duration in seconds
    #[arg(long = "role-duration")]
    pub role_duration: Option<i32>,

    /// Path to AWS config file
    #[arg(long = "config-file")]
    pub config_file: Option<String>,

    /// Path to AWS credentials file
    #[arg(long = "credentials-file")]
    pub credentials_file: Option<String>,

    /// Enable debug logging
    #[arg(long = "debug")]
    pub debug: bool,

    /// Enable info logging
    #[arg(long = "info")]
    pub info: bool,

    /// Toggle favorite for a profile
    #[arg(long = "favorite")]
    pub favorite: Option<String>,

    /// Generate shell completion script
    #[arg(long = "completion")]
    pub completion: Option<String>,

    /// Output format for credential_process
    #[arg(long = "credential-process")]
    pub credential_process: bool,
}

impl Args {
    pub fn parse_args() -> Self {
        Args::parse()
    }
}
