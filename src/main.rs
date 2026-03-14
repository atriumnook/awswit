use std::collections::HashMap;
use std::io::IsTerminal;

use clap::Parser;
use tracing::Level;
use tracing_subscriber::FmtSubscriber;

use awswit::cli::{Args, Command};
use awswit::config::AwswitConfig;
use awswit::context::AppContext;
use awswit::error::AwswitError;
use awswit::profile::Profile;
use awswit::shell::ShellExporter;
use awswit::{tui, utils};

fn main() {
    let args = Args::parse();

    let level = if args.debug {
        Level::DEBUG
    } else if args.info {
        Level::INFO
    } else {
        Level::WARN
    };

    let subscriber = FmtSubscriber::builder()
        .with_max_level(level)
        .with_target(false)
        .with_thread_ids(false)
        .with_file(false)
        .with_line_number(false)
        .finish();

    if let Err(e) = tracing::subscriber::set_global_default(subscriber) {
        eprintln!("Warning: Failed to set tracing subscriber: {}", e);
    }

    match run(args) {
        Ok(code) => {
            if code != 0 {
                std::process::exit(code);
            }
        }
        Err(e) => {
            if matches!(e, AwswitError::UserCancelled) {
                std::process::exit(130);
            }
            eprintln!("{}", e);
            std::process::exit(1);
        }
    }
}

fn run(args: Args) -> Result<i32, AwswitError> {
    tracing::debug!("Starting awswit with args: {:?}", args);

    // Handle subcommands
    if let Some(ref command) = args.command {
        match command {
            Command::Init { shell } => {
                handle_init(shell)?;
                return Ok(0);
            }
            Command::Completions { shell } => {
                handle_completions(*shell)?;
                return Ok(0);
            }
        }
    }

    // Handle version flag early
    if args.version {
        println!("awswit {}", env!("CARGO_PKG_VERSION"));
        return Ok(0);
    }

    // Early-exit: unset
    if args.unset {
        handle_unset(&args)?;
        return Ok(0);
    }

    // Build full application context
    let mut ctx = AppContext::build(args)?;

    // Handle list profiles
    if ctx.args.list_profiles.is_some() {
        handle_list_profiles(&ctx.profiles, &ctx.config)?;
        return Ok(0);
    }

    // Determine target profile
    let use_fzf = ctx.args.use_fzf
        || std::env::var("AWSWIT_USE_FZF")
            .map(|v| tui::fzf::is_truthy(&v))
            .unwrap_or(false);

    let target_profile_name = if ctx.args.profile_name.is_none()
        && !ctx.args.interactive_disabled()
        && std::io::stdout().is_terminal()
    {
        if use_fzf {
            tui::fzf::select_with_fzf(&ctx.profiles, &ctx.history)?
        } else {
            let picker = tui::ProfilePicker::new(&ctx.profiles).with_history(ctx.history.clone());

            match picker.run() {
                Ok(tui::picker::PickerResult::Selected(name)) => name,
                Ok(tui::picker::PickerResult::Cancelled) => {
                    return Err(AwswitError::UserCancelled);
                }
                Err(e) => {
                    return Err(AwswitError::ShellError {
                        message: format!("Picker error: {}", e),
                    });
                }
            }
        }
    } else {
        determine_target_profile(&ctx.args, &ctx.profiles, &ctx.config)?
    };

    tracing::info!("Target profile: {}", target_profile_name);

    // Record usage in history
    ctx.history.record_use(&target_profile_name);
    if let Err(e) = ctx.history.save() {
        tracing::warn!("Failed to save profile history: {}", e);
    }

    // Look up the profile to get its region
    let profile_region = ctx
        .profiles
        .get(&target_profile_name)
        .and_then(|p| p.region.as_deref());

    // Use --region flag if provided, otherwise use the profile's region
    let region = ctx.args.region.as_deref().or(profile_region);

    // Emit profile selection
    emit_profile(&target_profile_name, region, &ctx.args)?;

    Ok(0)
}

/// Emit profile selection as shell output or export commands
fn emit_profile(
    profile_name: &str,
    region: Option<&str>,
    args: &Args,
) -> Result<(), AwswitError> {
    let exporter = ShellExporter::new();
    if args.show_commands {
        print!("{}", exporter.generate_export_commands(profile_name, region)?);
    } else {
        tui::StatusLine::profile_switched(profile_name);
        print!(
            "{}",
            exporter.generate_shell_output(profile_name, region)?
        );
    }

    Ok(())
}

fn handle_unset(args: &Args) -> Result<(), AwswitError> {
    let exporter = ShellExporter::new();
    if args.show_commands {
        print!("{}", exporter.generate_unset_commands());
    } else {
        print!("{}", exporter.generate_unset_output());
    }
    Ok(())
}

fn handle_list_profiles(
    profiles: &HashMap<String, Profile>,
    config: &AwswitConfig,
) -> Result<(), AwswitError> {
    use crossterm::style::Stylize;

    let use_colors = config.colors && !cfg!(windows);

    println!();
    let header = "========================AWS Profiles==========================";
    if use_colors {
        println!("{}", header.cyan().bold());
    } else {
        println!("{}", header);
    }

    let header_line = format!(
        "{:<20} {:<8} {:<15} {:<12} {}",
        "PROFILE", "TYPE", "SOURCE", "REGION", "ACCOUNT"
    );
    if use_colors {
        println!("{}", header_line.white().bold());
    } else {
        println!("{}", header_line);
    }

    let mut profile_names: Vec<_> = profiles.keys().collect();
    profile_names.sort();

    for name in profile_names {
        let profile = &profiles[name];
        let profile_type = if profile.role_arn.is_some() {
            "Role"
        } else {
            "User"
        };
        let source = profile
            .source_profile
            .as_deref()
            .or(profile.credential_source.as_deref())
            .unwrap_or("None");
        let region = profile.region.as_deref().unwrap_or("-");
        let account = profile.get_account_id().unwrap_or_else(|| "-".to_string());

        let line = format!(
            "{:<20} {:<8} {:<15} {:<12} {}",
            truncate(name, 20),
            profile_type,
            truncate(source, 15),
            truncate(region, 12),
            account
        );
        println!("{}", line);
    }

    Ok(())
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.chars().count() > max_len {
        let truncated: String = s.chars().take(max_len.saturating_sub(3)).collect();
        format!("{}...", truncated)
    } else {
        s.to_string()
    }
}

fn determine_target_profile(
    args: &Args,
    profiles: &HashMap<String, Profile>,
    config: &AwswitConfig,
) -> Result<String, AwswitError> {
    let profile_name = args
        .profile_name
        .clone()
        .or_else(|| std::env::var("AWS_PROFILE").ok())
        .or_else(|| std::env::var("AWS_DEFAULT_PROFILE").ok())
        .unwrap_or_else(|| "default".to_string());

    if profiles.contains_key(&profile_name) {
        return Ok(profile_name);
    }

    if config.fuzzy_match
        && let Some(matched) = utils::fuzzy::find_closest_profile(&profile_name, profiles)
    {
        tracing::info!("Fuzzy matched '{}' to '{}'", profile_name, matched);
        return Ok(matched);
    }

    Err(AwswitError::ProfileNotFound { name: profile_name })
}

fn handle_completions(shell: clap_complete::Shell) -> Result<(), AwswitError> {
    use clap::CommandFactory;
    let mut cmd = Args::command();
    clap_complete::generate(shell, &mut cmd, "awswit", &mut std::io::stdout());
    Ok(())
}

fn handle_init(shell: &str) -> Result<(), AwswitError> {
    let script = match shell.to_lowercase().as_str() {
        "bash" => include_str!("init/bash.sh"),
        "zsh" => include_str!("init/zsh.sh"),
        "fish" => include_str!("init/fish.fish"),
        "powershell" | "pwsh" => include_str!("init/powershell.ps1"),
        _ => {
            return Err(AwswitError::ShellError {
                message: format!(
                    "Unsupported shell: {}. Supported shells: bash, zsh, fish, powershell",
                    shell
                ),
            });
        }
    };
    print!("{}", script);
    Ok(())
}
