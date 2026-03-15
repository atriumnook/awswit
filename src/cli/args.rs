use clap::{Parser, Subcommand};

/// awswit: A fast, modern AWS profile switcher with interactive TUI
#[derive(Parser, Clone, Default, Debug)]
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

    /// Show the commands to set the credentials
    #[arg(short = 's', long = "show-commands")]
    pub show_commands: bool,

    /// Unset your AWS environment variables
    #[arg(short = 'u', long = "unset")]
    pub unset: bool,

    /// List available profiles
    #[arg(short = 'l', long = "list-profiles")]
    pub list_profiles: bool,

    /// Path to config file
    #[arg(long = "config-file", value_name = "config_file")]
    pub config_file: Option<String>,

    /// AWS region to use
    #[arg(long = "region", value_name = "region")]
    pub region: Option<String>,

    /// Display INFO level logs
    #[arg(long = "info")]
    pub info: bool,

    /// Display DEBUG level logs
    #[arg(long = "debug")]
    pub debug: bool,

    /// Disable interactive mode (use when piping or scripting)
    #[arg(long = "no-interactive", short = 'n')]
    pub no_interactive: bool,

    /// Use external fzf for profile selection
    #[arg(long = "fzf")]
    pub use_fzf: bool,
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

    /// Generate static shell completions
    ///
    /// This generates tab-completion scripts for your shell.
    ///
    /// Usage:
    ///   awswit completions bash > /etc/bash_completion.d/awswit
    ///   awswit completions zsh > ~/.zfunc/_awswit
    ///   awswit completions fish > ~/.config/fish/completions/awswit.fish
    Completions {
        /// Shell type
        shell: clap_complete::Shell,
    },
}

impl Args {
    /// Check if interactive mode is disabled
    pub fn interactive_disabled(&self) -> bool {
        self.no_interactive
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn test_parse_defaults() {
        let args = Args::try_parse_from(["awswit"]).unwrap();
        assert!(!args.version);
        assert!(!args.unset);
        assert!(args.profile_name.is_none());
    }

    #[test]
    fn test_parse_profile_name() {
        let args = Args::try_parse_from(["awswit", "prod"]).unwrap();
        assert_eq!(args.profile_name, Some("prod".to_string()));
    }

    #[test]
    fn test_parse_unset() {
        let args = Args::try_parse_from(["awswit", "--unset"]).unwrap();
        assert!(args.unset);
    }

    #[test]
    fn test_parse_fzf() {
        let args = Args::try_parse_from(["awswit", "--fzf"]).unwrap();
        assert!(args.use_fzf);
    }
}
