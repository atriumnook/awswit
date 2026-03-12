use std::collections::HashMap;
use std::io::IsTerminal;

use clap::Parser;
use tracing::Level;
use tracing_subscriber::FmtSubscriber;

use awswit::aws::StsClient;
use awswit::cli::{Args, Command};
use awswit::config::AwswitConfig;
use awswit::context::AppContext;
use awswit::error::AwswitError;
use awswit::profile::{Profile, ProfileResolver};
use awswit::shell::ShellExporter;
use awswit::{autorefresh, aws, tui, utils};

#[tokio::main]
async fn main() {
    let args = Args::parse();

    // Setup logging - determine level once, init once
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

    tracing::subscriber::set_global_default(subscriber).expect("Failed to set tracing subscriber");

    // Run the main application
    match run(args).await {
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

async fn run(args: Args) -> Result<i32, AwswitError> {
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
            Command::Exec {
                profile,
                force_refresh,
                region,
                command,
            } => {
                return handle_exec(profile, *force_refresh, region.clone(), command).await;
            }
        }
    }

    // Handle version flag early (before loading config)
    if args.version {
        println!("awswit {}", env!("CARGO_PKG_VERSION"));
        return Ok(0);
    }

    // Early-exit flags that only need config (no profiles)
    if args.unset {
        handle_unset(&args)?;
        return Ok(0);
    }

    if args.kill_refresher {
        handle_kill_refresher(&args)?;
        return Ok(0);
    }

    // Build full application context
    let mut ctx = AppContext::build(args)?;

    // Handle list profiles
    if ctx.args.list_profiles.is_some() {
        handle_list_profiles(&ctx.profiles, &ctx.config)?;
        return Ok(0);
    }

    // Handle refresh autocomplete
    if ctx.args.refresh_autocomplete {
        handle_refresh_autocomplete(&ctx.profiles)?;
        return Ok(0);
    }

    // Determine target profile
    let use_fzf = ctx.args.use_fzf
        || std::env::var("AWSWIT_USE_FZF")
            .map(|v| tui::fzf::is_truthy(&v))
            .unwrap_or(false);

    let target_profile_name = if ctx.args.profile_name.is_none()
        && ctx.args.role_arn.is_none()
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

    // Show spinner while resolving credentials
    let spinner = tui::AwswitSpinner::assuming_role(&target_profile_name);

    // Resolve credentials (STS initialized lazily here)
    let resolver = ProfileResolver::new(&ctx.profiles, &ctx.config);
    let sts_client = StsClient::new().await;

    let credentials = match resolver
        .resolve_credentials(&target_profile_name, &ctx.args, &sts_client, &ctx.cache)
        .await
    {
        Ok(creds) => {
            spinner.finish_success(&format!("Assumed {}", target_profile_name));
            creds
        }
        Err(e) => {
            spinner.finish_error(&e.to_string());
            return Err(e);
        }
    };

    tracing::debug!("Got credentials, expiration: {:?}", credentials.expiration);

    // Record usage in history
    ctx.history.record_use(&target_profile_name);
    if let Err(e) = ctx.history.save() {
        tracing::warn!("Failed to save profile history: {}", e);
    }

    // Handle auto-refresh
    if ctx.args.auto_refresh {
        let requires_mfa = check_chain_requires_mfa(&ctx.profiles, &target_profile_name);
        autorefresh::start_auto_refresh(
            &target_profile_name,
            &ctx.args,
            &credentials,
            requires_mfa,
        )
        .await?;
    }

    // Emit credentials
    emit_credentials(&credentials, &target_profile_name, &ctx.args)?;

    Ok(0)
}

/// Check whether the profile's full source_profile chain requires MFA.
fn check_chain_requires_mfa(
    profiles: &HashMap<String, Profile>,
    target_profile_name: &str,
) -> bool {
    profiles
        .get(target_profile_name)
        .map(|p| {
            if p.requires_mfa() {
                return true;
            }
            let mut current_source = p.source_profile.as_deref();
            let mut visited = std::collections::HashSet::new();
            while let Some(src_name) = current_source {
                if !visited.insert(src_name) {
                    break;
                }
                if let Some(src_p) = profiles.get(src_name) {
                    if src_p.requires_mfa() {
                        return true;
                    }
                    current_source = src_p.source_profile.as_deref();
                } else {
                    break;
                }
            }
            false
        })
        .unwrap_or(false)
}

/// Emit credentials as shell output or export commands
fn emit_credentials(
    credentials: &aws::Credentials,
    profile_name: &str,
    args: &Args,
) -> Result<(), AwswitError> {
    let exporter = ShellExporter::new();
    if args.show_commands {
        print!(
            "{}",
            exporter.generate_export_commands(credentials, profile_name)
        );
    } else {
        tui::StatusLine::profile_assumed(
            profile_name,
            credentials
                .expiration
                .map(|e| e.format("%Y-%m-%d %H:%M:%S").to_string())
                .as_deref(),
        );

        print!(
            "{}",
            exporter.generate_shell_output(credentials, profile_name)?
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

fn handle_kill_refresher(args: &Args) -> Result<(), AwswitError> {
    if let Some(ref profile_name) = args.profile_name {
        autorefresh::stop_auto_refresh(profile_name)?;
        println!("Stopped auto-refresh for profile: {}", profile_name);
    } else {
        autorefresh::stop_all_auto_refresh()?;
        println!("Stopped all auto-refresh processes");
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
        "{:<20} {:<8} {:<15} {:<6} {:<12} {}",
        "PROFILE", "TYPE", "SOURCE", "MFA?", "REGION", "ACCOUNT"
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
        let mfa = if profile.mfa_serial.is_some() {
            "Yes"
        } else {
            "No"
        };
        let region = profile.region.as_deref().unwrap_or("-");

        let account = profile.get_account_id().unwrap_or_else(|| "-".to_string());

        let line = format!(
            "{:<20} {:<8} {:<15} {:<6} {:<12} {}",
            truncate(name, 20),
            profile_type,
            truncate(source, 15),
            mfa,
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

fn handle_refresh_autocomplete(profiles: &HashMap<String, Profile>) -> Result<(), AwswitError> {
    let mut names: Vec<_> = profiles.keys().collect();
    names.sort();
    for name in names {
        println!("{}", name);
    }
    Ok(())
}

fn determine_target_profile(
    args: &Args,
    profiles: &HashMap<String, Profile>,
    config: &AwswitConfig,
) -> Result<String, AwswitError> {
    if args.role_arn.is_some() {
        let name = args
            .session_name
            .clone()
            .or_else(|| {
                args.resolve_role_arn()
                    .and_then(|arn| arn.rsplit('/').next().map(|s| s.to_string()))
            })
            .unwrap_or_else(|| "cli-role".to_string());
        return Ok(name);
    }

    let profile_name = args
        .profile_name
        .clone()
        .or_else(|| std::env::var("AWS_PROFILE").ok())
        .or_else(|| std::env::var("AWS_DEFAULT_PROFILE").ok())
        .unwrap_or_else(|| "default".to_string());

    if profiles.contains_key(&profile_name) {
        return Ok(profile_name);
    }

    if config.fuzzy_match {
        if let Some(matched) = utils::fuzzy::find_closest_profile(&profile_name, profiles) {
            tracing::info!("Fuzzy matched '{}' to '{}'", profile_name, matched);
            return Ok(matched);
        }
    }

    Err(AwswitError::ProfileNotFound { name: profile_name })
}

fn handle_completions(shell: clap_complete::Shell) -> Result<(), AwswitError> {
    use clap::CommandFactory;
    let mut cmd = Args::command();
    clap_complete::generate(shell, &mut cmd, "awswit", &mut std::io::stdout());
    Ok(())
}

async fn handle_exec(
    profile: &str,
    force_refresh: bool,
    region: Option<String>,
    command: &[String],
) -> Result<i32, AwswitError> {
    let args = Args {
        profile_name: Some(profile.to_string()),
        force_refresh,
        region: region.clone(),
        ..Default::default()
    };
    let ctx = AppContext::build(args)?;
    let resolver = ProfileResolver::new(&ctx.profiles, &ctx.config);
    let sts_client = StsClient::new().await;

    let credentials = resolver
        .resolve_credentials(profile, &ctx.args, &sts_client, &ctx.cache)
        .await?;

    let (program, cmd_args) = command
        .split_first()
        .ok_or_else(|| AwswitError::ShellError {
            message: "No command specified".to_string(),
        })?;

    let status = std::process::Command::new(program)
        .args(cmd_args)
        .env("AWS_ACCESS_KEY_ID", &credentials.access_key_id)
        .env("AWS_SECRET_ACCESS_KEY", &credentials.secret_access_key)
        .env(
            "AWS_SESSION_TOKEN",
            credentials.session_token.as_deref().unwrap_or(""),
        )
        .env(
            "AWS_DEFAULT_REGION",
            credentials
                .region
                .as_deref()
                .or(region.as_deref())
                .unwrap_or(""),
        )
        .env("AWSWIT_PROFILE", profile)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status()
        .map_err(|e| AwswitError::ShellError {
            message: format!("Failed to execute command '{}': {}", program, e),
        })?;

    Ok(status.code().unwrap_or(1))
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
