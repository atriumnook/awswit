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

    /// With --list, emit only profile names, one per line — a fast path for
    /// shell tab-completion. Skips history and SSO-cache I/O.
    #[arg(long = "names-only")]
    pub names_only: bool,

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
    /// Examples:{n}
    ///   eval "$(awswit init bash)"{n}
    ///   eval "$(awswit init zsh)"{n}
    ///   awswit init fish | source{n}
    ///   awswit init powershell | Invoke-Expression
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
    /// Examples:{n}
    ///   awswit exec prod -- aws s3 ls{n}
    ///   awswit exec prod aws s3 ls
    ///
    /// `--` is optional when CMD has no leading flags. Use it (and please
    /// do) when CMD itself starts with `-`, or when you want to be
    /// explicit. exec sets `AWS_PROFILE` and (if defined) the profile's
    /// region for the child only — the parent shell is untouched.
    Exec {
        /// Profile to run the command under.
        profile: String,

        /// Override the region for this invocation only.
        #[arg(long = "region", value_name = "REGION")]
        region: Option<String>,

        /// Command and arguments to execute.
        #[arg(
            trailing_var_arg = true,
            allow_hyphen_values = true,
            required = true,
            value_name = "CMD"
        )]
        cmd: Vec<String>,
    },

    /// Open the picker and print only the selected profile name to stdout.
    ///
    /// Designed for pipeline composition — no shell mutation, no status
    /// chatter on stdout. Exits 130 if the user cancelled.
    ///
    /// Examples:{n}
    ///   awswit exec "$(awswit pick)" -- aws sts get-caller-identity{n}
    ///   aws --profile "$(awswit pick)" s3 ls{n}
    ///   aws sso login --profile "$(awswit pick)"
    Pick,

    /// Show the currently active AWS profile and its details.
    Which,

    /// Audit ~/.aws/config and SSO token cache for common breakage.
    ///
    /// Exits non-zero when at least one error-level issue is found.
    Doctor {
        /// Emit findings as a JSON array on stdout (for CI integration).
        #[arg(long = "json")]
        json: bool,
    },

    /// Print just the current profile name (for shell prompt integration).
    ///
    /// Both `{}` and `%s` work as the placeholder in --format.
    ///
    /// `prompt` writes its output with no trailing newline — your shell's
    /// PS1 / RPROMPT is expected to render it inline. To inspect the value
    /// interactively, run `awswit prompt; echo`.
    ///
    /// Example bash PS1:
    ///   PS1='[\u@\h $(awswit prompt --format "{} " --default "")] \w \$ '
    Prompt {
        /// Format string. `{}` and `%s` are both interpolated with the
        /// current profile name.
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
            Some(Command::Doctor { json: false })
        ));
        assert!(matches!(
            Args::try_parse_from(["awswit", "doctor", "--json"])
                .unwrap()
                .command,
            Some(Command::Doctor { json: true })
        ));
    }
}
