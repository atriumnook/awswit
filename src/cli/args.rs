use clap::{Parser, Subcommand};

/// Interactive AWS profile switcher with fuzzy search and frecency sorting.
#[derive(Parser, Clone, Default, Debug)]
#[command(name = "awswit", version, about)]
#[command(disable_version_flag = true)]
#[command(args_conflicts_with_subcommands = true)]
pub struct Args {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Profile to switch to. Omit for the interactive picker.
    #[arg(value_name = "PROFILE")]
    pub profile_name: Option<String>,

    /// Print the awswit version and exit.
    #[arg(short = 'v', long = "version")]
    pub version: bool,

    /// Emit shell `export` / `unset` lines on stdout for `eval`.
    ///
    /// This is what the shell wrapper (`awswit init <shell>`) uses internally.
    /// You can also call it directly: `eval "$(awswit --shell-export prod)"`.
    #[arg(long = "shell-export", short = 's')]
    pub shell_export: bool,

    /// Unset every AWS_* variable awswit manages.
    #[arg(short = 'u', long = "unset")]
    pub unset: bool,

    /// List profiles. Default: human-readable when stdout is a terminal, TSV otherwise.
    #[arg(short = 'l', long = "list")]
    pub list: bool,

    /// With --list, emit JSON instead of TSV / pretty output.
    #[arg(long = "json")]
    pub json: bool,

    /// AWS config file path (defaults to $AWS_CONFIG_FILE or ~/.aws/config).
    #[arg(long = "config-file", value_name = "PATH")]
    pub config_file: Option<String>,

    /// Override the region exported with the selected profile.
    #[arg(long = "region", value_name = "REGION")]
    pub region: Option<String>,

    /// Skip the interactive picker; resolve the profile by name or $AWS_PROFILE.
    #[arg(long = "no-interactive", short = 'n')]
    pub no_interactive: bool,

    /// Use external `fzf` for selection instead of the built-in TUI.
    #[arg(long = "fzf")]
    pub use_fzf: bool,

    /// Verbose logging (INFO).
    #[arg(long = "verbose")]
    pub verbose: bool,

    /// Debug logging.
    #[arg(long = "debug")]
    pub debug: bool,
}

#[derive(Subcommand, Debug, Clone)]
pub enum Command {
    /// Print the shell integration snippet — pipe into your rc file.
    ///
    /// Examples:
    ///   eval "$(awswit init bash)"
    ///   awswit init fish | source
    Init {
        /// Shell type: bash | zsh | fish | powershell
        shell: String,
    },

    /// Generate static shell completion scripts.
    Completions {
        /// Shell type
        shell: clap_complete::Shell,
    },

    /// Run a command with a specific profile, without modifying the parent shell.
    ///
    /// Example:
    ///   awswit exec prod -- aws s3 ls
    ///
    /// The `--` separator is required when passing flags to the inner command,
    /// otherwise they'll be parsed as awswit's own flags.
    Exec {
        /// Profile to run the command under.
        profile: String,

        /// Override the region for this invocation only.
        #[arg(long = "region", value_name = "REGION")]
        region: Option<String>,

        /// Command and arguments to execute.
        #[arg(last = true, required = true, value_name = "CMD")]
        cmd: Vec<String>,
    },

    /// Show the currently active AWS profile and its details.
    Which,

    /// Audit ~/.aws/config and SSO token cache for common breakage.
    ///
    /// Exits non-zero when at least one error-level issue is found.
    Doctor,

    /// Print just the current profile name (for shell prompt integration).
    ///
    /// Example bash PS1:
    ///   PS1='[\u@\h $(awswit prompt)] \w \$ '
    Prompt {
        /// Wrap output in this format string with `{}` as placeholder.
        #[arg(long = "format", default_value = "{}")]
        format: String,

        /// Output when no profile is set.
        #[arg(long = "default", default_value = "")]
        default: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parses_defaults() {
        let args = Args::try_parse_from(["awswit"]).unwrap();
        assert!(!args.version);
        assert!(!args.unset);
        assert!(args.profile_name.is_none());
    }

    #[test]
    fn parses_profile_name_positional() {
        let args = Args::try_parse_from(["awswit", "prod"]).unwrap();
        assert_eq!(args.profile_name.as_deref(), Some("prod"));
    }

    #[test]
    fn parses_shell_export_short() {
        let args = Args::try_parse_from(["awswit", "-s", "prod"]).unwrap();
        assert!(args.shell_export);
    }

    #[test]
    fn parses_list_json() {
        let args = Args::try_parse_from(["awswit", "-l", "--json"]).unwrap();
        assert!(args.list);
        assert!(args.json);
    }

    #[test]
    fn parses_exec_subcommand() {
        let args =
            Args::try_parse_from(["awswit", "exec", "prod", "--", "aws", "s3", "ls"]).unwrap();
        match args.command {
            Some(Command::Exec { profile, cmd, .. }) => {
                assert_eq!(profile, "prod");
                assert_eq!(cmd, vec!["aws".to_string(), "s3".into(), "ls".into()]);
            }
            _ => panic!("expected Exec"),
        }
    }

    #[test]
    fn exec_requires_command() {
        let res = Args::try_parse_from(["awswit", "exec", "prod"]);
        assert!(res.is_err(), "exec without CMD should fail");
    }

    #[test]
    fn parses_which_and_doctor() {
        assert!(matches!(
            Args::try_parse_from(["awswit", "which"]).unwrap().command,
            Some(Command::Which)
        ));
        assert!(matches!(
            Args::try_parse_from(["awswit", "doctor"]).unwrap().command,
            Some(Command::Doctor)
        ));
    }
}
