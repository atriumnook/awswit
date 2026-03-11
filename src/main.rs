use std::io::IsTerminal;

use clap::Parser;
use tracing::Level;
use tracing_subscriber::FmtSubscriber;

use awswit::aws::StsClient;
use awswit::cache::CacheManager;
use awswit::cli::{Args, Command};
use awswit::config::{AwsFiles, AwswitConfig};
use awswit::error::AwswitError;
use awswit::profile::ProfileResolver;
use awswit::shell::ShellExporter;
use awswit::{autorefresh, aws, history, profile, tui, utils};

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
    if let Err(e) = run(args).await {
        if matches!(e, AwswitError::UserCancelled) {
            std::process::exit(0);
        }
        eprintln!("{}", e);
        std::process::exit(1);
    }
}

async fn run(args: Args) -> Result<(), AwswitError> {
    tracing::debug!("Starting awswit with args: {:?}", args);

    // Handle subcommands
    if let Some(ref command) = args.command {
        match command {
            Command::Init { shell } => {
                return handle_init(shell);
            }
        }
    }

    // Handle version flag early (before loading config)
    if args.version {
        println!("awswit {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    // Load awswit configuration
    let awswit_config = AwswitConfig::load()?;
    tracing::debug!("Loaded awswit config: {:?}", awswit_config);

    // Handle unset flag
    if args.unset {
        return handle_unset(&args);
    }

    // Handle kill refresher
    if args.kill_refresher {
        return handle_kill_refresher(&args).await;
    }

    // Handle --role-arn early - no need to load profiles if just assuming a direct role ARN
    // (moved below profile loading since we may still need profiles for --source-profile)

    // Load AWS config and credentials files
    let credentials_file = match args
        .credentials_file
        .clone()
        .or_else(|| std::env::var("AWS_SHARED_CREDENTIALS_FILE").ok())
    {
        Some(path) => path,
        None => {
            let home = dirs::home_dir().ok_or_else(|| AwswitError::ShellError {
                message: "Could not determine home directory".to_string(),
            })?;
            home.join(".aws")
                .join("credentials")
                .to_string_lossy()
                .to_string()
        }
    };

    let config_file = match args
        .config_file
        .clone()
        .or_else(|| std::env::var("AWS_CONFIG_FILE").ok())
    {
        Some(path) => path,
        None => {
            let home = dirs::home_dir().ok_or_else(|| AwswitError::ShellError {
                message: "Could not determine home directory".to_string(),
            })?;
            home.join(".aws")
                .join("config")
                .to_string_lossy()
                .to_string()
        }
    };

    let aws_files = AwsFiles::load(&config_file, &credentials_file)?;
    let profiles = aws_files.merge_profiles();
    tracing::debug!("Loaded {} profiles", profiles.len());

    // Handle list profiles
    if args.list_profiles.is_some() {
        return handle_list_profiles(&profiles, &args, &awswit_config);
    }

    // Handle refresh autocomplete
    if args.refresh_autocomplete {
        return handle_refresh_autocomplete(&profiles);
    }

    // Load history once and reuse
    let mut profile_history = history::ProfileHistory::load().unwrap_or_else(|e| {
        tracing::warn!("Failed to load profile history: {}", e);
        history::ProfileHistory::default()
    });

    // Determine target profile - use interactive mode if no profile specified
    let target_profile_name = if args.profile_name.is_none()
        && args.role_arn.is_none()
        && !args.interactive_disabled()
        && std::io::stdout().is_terminal()
    {
        // Launch interactive picker - pass reference, not clone
        let picker = tui::ProfilePicker::new(&profiles).with_history(profile_history.clone());

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
    } else {
        determine_target_profile(&args, &profiles, &awswit_config)?
    };

    tracing::info!("Target profile: {}", target_profile_name);

    // Show spinner while resolving credentials
    let spinner = tui::AwswitSpinner::assuming_role(&target_profile_name);

    // Resolve the profile chain and get credentials
    // StsClient is initialized lazily here, just before it's needed, to avoid
    // unnecessary AWS SDK initialization when operations don't require STS.
    let resolver = ProfileResolver::new(&profiles, &awswit_config);
    let cache_manager = CacheManager::new()?;
    let sts_client = StsClient::new().await;

    let credentials = match resolver
        .resolve_credentials(&target_profile_name, &args, &sts_client, &cache_manager)
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
    profile_history.record_use(&target_profile_name);
    if let Err(e) = profile_history.save() {
        tracing::warn!("Failed to save profile history: {}", e);
    }

    // Handle auto-refresh
    if args.auto_refresh {
        // Determine if the profile chain requires MFA
        let requires_mfa = profiles
            .get(&target_profile_name)
            .map(|p| {
                if p.requires_mfa() {
                    return true;
                }
                // Check source profile chain for MFA
                if let Some(ref src) = p.source_profile {
                    if let Some(src_p) = profiles.get(src) {
                        return src_p.requires_mfa();
                    }
                }
                false
            })
            .unwrap_or(false);
        autorefresh::start_auto_refresh(&target_profile_name, &args, &credentials, requires_mfa)
            .await?;
    }

    // Emit credentials
    emit_credentials(&credentials, &target_profile_name, &args)?;

    Ok(())
}

/// Emit credentials as shell output or export commands
fn emit_credentials(
    credentials: &aws::Credentials,
    profile_name: &str,
    args: &Args,
) -> Result<(), AwswitError> {
    let exporter = ShellExporter::new();
    if args.show_commands {
        // Print export commands to stdout (not stderr) so `> file` works
        print!(
            "{}",
            exporter.generate_export_commands(credentials, profile_name)
        );
    } else {
        // Show nice status message on stderr
        tui::StatusLine::profile_assumed(
            profile_name,
            credentials
                .expiration
                .map(|e| e.format("%Y-%m-%d %H:%M:%S").to_string())
                .as_deref(),
        );

        // Output in a format the shell wrapper can eval
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

async fn handle_kill_refresher(args: &Args) -> Result<(), AwswitError> {
    if let Some(ref profile_name) = args.profile_name {
        autorefresh::stop_auto_refresh(profile_name).await?;
        println!("Stopped auto-refresh for profile: {}", profile_name);
    } else {
        autorefresh::stop_all_auto_refresh().await?;
        println!("Stopped all auto-refresh processes");
    }
    Ok(())
}

fn handle_list_profiles(
    profiles: &std::collections::HashMap<String, profile::Profile>,
    args: &Args,
    config: &AwswitConfig,
) -> Result<(), AwswitError> {
    use colored::Colorize;

    let show_more = args
        .list_profiles
        .as_ref()
        .map(|s| s == "more")
        .unwrap_or(false);
    let use_colors = config.colors && !cfg!(windows);

    // Print to stdout so `| less` and `> file` work
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

        let account = if show_more {
            "Fetching...".to_string()
        } else {
            profile
                .role_arn
                .as_ref()
                .and_then(|arn| extract_account_from_arn(arn))
                .unwrap_or_else(|| "Unavailable".to_string())
        };

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

fn extract_account_from_arn(arn: &str) -> Option<String> {
    let parts: Vec<&str> = arn.split(':').collect();
    if parts.len() >= 5 {
        Some(parts[4].to_string())
    } else {
        None
    }
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.chars().count() > max_len {
        let truncated: String = s.chars().take(max_len.saturating_sub(3)).collect();
        format!("{}...", truncated)
    } else {
        s.to_string()
    }
}

fn handle_refresh_autocomplete(
    profiles: &std::collections::HashMap<String, profile::Profile>,
) -> Result<(), AwswitError> {
    let mut names: Vec<_> = profiles.keys().collect();
    names.sort();
    for name in names {
        println!("{}", name);
    }
    Ok(())
}

fn determine_target_profile(
    args: &Args,
    profiles: &std::collections::HashMap<String, profile::Profile>,
    config: &AwswitConfig,
) -> Result<String, AwswitError> {
    // Early guard: --role-arn doesn't need a profile name
    if args.role_arn.is_some() {
        let name = args
            .session_name
            .clone()
            .or_else(|| {
                args.resolve_role_arn().and_then(|arn| {
                    arn.rsplit('/').next().map(|s| s.to_string())
                })
            })
            .unwrap_or_else(|| "cli-role".to_string());
        return Ok(name);
    }

    // Get profile name from args or default
    let profile_name = args
        .profile_name
        .clone()
        .or_else(|| std::env::var("AWS_PROFILE").ok())
        .or_else(|| std::env::var("AWS_DEFAULT_PROFILE").ok())
        .unwrap_or_else(|| "default".to_string());

    // Check if profile exists
    if profiles.contains_key(&profile_name) {
        return Ok(profile_name);
    }

    // Try fuzzy matching if enabled
    if config.fuzzy_match {
        if let Some(matched) = utils::fuzzy::find_closest_profile(&profile_name, profiles) {
            tracing::info!("Fuzzy matched '{}' to '{}'", profile_name, matched);
            return Ok(matched);
        }
    }

    Err(AwswitError::ProfileNotFound { name: profile_name })
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
