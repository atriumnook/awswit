use std::ffi::OsString;
use std::path::PathBuf;

use clap::{Args as ClapArgs, Parser, Subcommand, ValueEnum};

/// A safe, fast AWS profile selector.
#[derive(Parser, Debug)]
#[command(name = "awswit", author, version)]
#[command(
    about = "Select an AWS profile without owning its credentials",
    long_about = None,
    after_help = "Current-shell activation requires the generated hook because a child process cannot modify its parent shell.\n  Bash:       eval \"$(awswit init bash)\"\n  Zsh:        eval \"$(awswit init zsh)\"\n  Fish:       awswit init fish | source\n  PowerShell: Invoke-Expression ((awswit init powershell) -join [Environment]::NewLine)\n\nUse `awswit exec PROFILE -- COMMAND` to change only the child command environment."
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub(crate) enum Command {
    /// Select a profile for the current shell
    Activate(ActivateArgs),

    /// Run one command with an exact profile, without changing the parent shell
    Exec(ExecArgs),

    /// List selectable profiles
    List(ListArgs),

    /// Diagnose local configuration without contacting AWS
    Doctor(DoctorArgs),

    /// Clear managed profile and region variables through the loaded shell hook
    Unset,

    /// Print a shell integration script
    Init {
        #[arg(value_enum)]
        shell: Shell,
    },

    /// Print a static completion script
    Completions {
        #[arg(value_enum)]
        shell: Shell,
    },
}

#[derive(ClapArgs, Debug, Default)]
pub(crate) struct SourceArgs {
    /// AWS shared config path (takes precedence over AWS_CONFIG_FILE)
    #[arg(long, value_name = "PATH")]
    pub(crate) config_file: Option<PathBuf>,

    /// AWS shared credentials path (takes precedence over AWS_SHARED_CREDENTIALS_FILE)
    #[arg(long, value_name = "PATH")]
    pub(crate) credentials_file: Option<PathBuf>,
}

#[derive(ClapArgs, Debug, Default)]
pub(crate) struct ActivateArgs {
    /// Exact profile name; omit to use the interactive picker
    #[arg(value_name = "PROFILE", conflicts_with = "named_profile")]
    pub(crate) profile: Option<String>,

    /// Exact profile name escape hatch for names beginning with `-`
    #[arg(long = "profile", value_name = "PROFILE", allow_hyphen_values = true)]
    pub(crate) named_profile: Option<String>,

    /// Override the selected profile's configured region
    #[arg(long, value_name = "REGION")]
    pub(crate) region: Option<String>,

    /// Explicitly remove detected credential overrides from the activated shell
    #[arg(long)]
    pub(crate) clear_credential_overrides: bool,

    #[command(flatten)]
    pub(crate) sources: SourceArgs,
}

#[derive(ClapArgs, Debug)]
pub(crate) struct ExecArgs {
    /// Exact profile name
    #[arg(value_name = "PROFILE", required_unless_present = "named_profile")]
    pub(crate) profile: Option<String>,

    /// Exact profile name escape hatch for names beginning with `-`
    #[arg(
        long = "profile",
        value_name = "PROFILE",
        allow_hyphen_values = true,
        conflicts_with = "profile"
    )]
    pub(crate) named_profile: Option<String>,

    /// Override the selected profile's configured region
    #[arg(long, value_name = "REGION")]
    pub(crate) region: Option<String>,

    /// Explicitly remove detected credential overrides from the child process
    #[arg(long)]
    pub(crate) clear_credential_overrides: bool,

    #[command(flatten)]
    pub(crate) sources: SourceArgs,

    /// Executable and arguments (use `--` before the executable)
    #[arg(
        required = true,
        num_args = 1..,
        last = true,
        allow_hyphen_values = true,
        value_name = "COMMAND"
    )]
    pub(crate) command: Vec<OsString>,
}

#[derive(ClapArgs, Debug)]
pub(crate) struct ListArgs {
    /// Output format
    #[arg(long, value_enum, default_value_t = ListFormat::Human)]
    pub(crate) format: ListFormat,

    #[command(flatten)]
    pub(crate) sources: SourceArgs,
}

#[derive(ClapArgs, Debug)]
pub(crate) struct DoctorArgs {
    /// Output format
    #[arg(long, value_enum, default_value_t = DoctorFormat::Human)]
    pub(crate) format: DoctorFormat,

    #[command(flatten)]
    pub(crate) sources: SourceArgs,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum ListFormat {
    Human,
    Names,
    Json,
    /// Internal shell-completion feed. It omits names whose insertion cannot
    /// be distinguished visually by completion APIs without display labels.
    #[value(hide = true)]
    Completion,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum DoctorFormat {
    Human,
    Json,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum Shell {
    Bash,
    Zsh,
    Fish,
    #[value(alias = "pwsh")]
    Powershell,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_invocation_means_interactive_activation() {
        let cli = Cli::try_parse_from(["awswit"]).unwrap();
        assert!(cli.command.is_none());
    }

    #[test]
    fn exact_activation_is_explicit() {
        let cli = Cli::try_parse_from(["awswit", "activate", "prod"]).unwrap();
        let Some(Command::Activate(args)) = cli.command else {
            panic!("expected activation");
        };
        assert_eq!(args.profile.as_deref(), Some("prod"));
    }

    #[test]
    fn legacy_positional_is_not_accepted_by_the_binary() {
        let error = Cli::try_parse_from(["awswit", "prod"]).unwrap_err();
        assert_eq!(error.kind(), clap::error::ErrorKind::InvalidSubcommand);
    }

    #[test]
    fn exec_keeps_arguments_as_an_argv() {
        let cli = Cli::try_parse_from([
            "awswit",
            "exec",
            "prod",
            "--",
            "printf",
            "%s",
            "$HOME; rm -rf nope",
        ])
        .unwrap();
        let Some(Command::Exec(args)) = cli.command else {
            panic!("expected exec");
        };
        assert_eq!(args.profile.as_deref(), Some("prod"));
        assert_eq!(args.command[0], "printf");
        assert_eq!(args.command[2], "$HOME; rm -rf nope");
    }

    #[test]
    fn exec_requires_the_command_separator() {
        let error = Cli::try_parse_from(["awswit", "exec", "prod", "printf"])
            .expect_err("command without -- must be rejected");
        assert_eq!(error.kind(), clap::error::ErrorKind::UnknownArgument);
    }

    #[test]
    fn exec_can_address_a_leading_hyphen_profile() {
        let cli = Cli::try_parse_from(["awswit", "exec", "--profile=-h", "--", "true"])
            .expect("hyphenated profile must be representable");
        let Some(Command::Exec(args)) = cli.command else {
            panic!("expected exec");
        };
        assert_eq!(args.profile, None);
        assert_eq!(args.named_profile.as_deref(), Some("-h"));
        assert_eq!(args.command, [OsString::from("true")]);
    }

    #[test]
    fn activate_can_address_a_leading_hyphen_profile() {
        let cli = Cli::try_parse_from(["awswit", "activate", "--profile=-h"])
            .expect("hyphenated profile must be representable");
        let Some(Command::Activate(args)) = cli.command else {
            panic!("expected activate");
        };
        assert_eq!(args.profile, None);
        assert_eq!(args.named_profile.as_deref(), Some("-h"));
    }

    #[test]
    fn source_options_are_command_local() {
        let cli = Cli::try_parse_from([
            "awswit",
            "list",
            "--format",
            "names",
            "--config-file",
            "/tmp/aws config",
        ])
        .unwrap();
        let Some(Command::List(args)) = cli.command else {
            panic!("expected list");
        };
        assert_eq!(args.format, ListFormat::Names);
        assert_eq!(
            args.sources.config_file,
            Some(PathBuf::from("/tmp/aws config"))
        );
    }

    #[test]
    fn root_help_explains_the_process_boundary_and_all_supported_hooks() {
        use clap::CommandFactory;

        let help = Cli::command().render_long_help().to_string();

        assert!(help.contains("a child process cannot modify its parent shell"));
        for shell in ["Bash", "Zsh", "Fish", "PowerShell"] {
            assert!(help.contains(shell), "missing {shell} setup from help");
        }
        assert!(help.contains("-join [Environment]::NewLine"));
        assert!(help.contains("exec PROFILE -- COMMAND"));
    }
}
