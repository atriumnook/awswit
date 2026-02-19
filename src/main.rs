mod autorefresh;
mod aws;
mod cache;
mod cli;
mod config;
mod error;
mod history;
mod profile;
mod shell;
mod tui;
mod utils;

use std::io::Write;

use clap::Parser;
use colored::Colorize;

use crate::cache::CacheManager;
use crate::cli::Args;
use crate::config::AwswitConfig;
use crate::error::AwswitError;
use crate::history::HistoryStorage;
use crate::profile::ProfileResolver;
use crate::shell::export::ShellType;
use crate::tui::{print_error, print_info, print_success};

#[tokio::main]
async fn main() {
    // Check if running as daemon
    if std::env::var("AWSWIT_DAEMON_MODE").is_ok() {
        let profile = std::env::var("AWSWIT_DAEMON_PROFILE").unwrap_or_default();
        if !profile.is_empty() {
            if let Err(e) = autorefresh::run_daemon_loop(&profile).await {
                eprintln!("Daemon error: {}", e);
            }
        }
        return;
    }

    let args = Args::parse();

    // Initialize logging
    if args.debug {
        tracing_subscriber::fmt()
            .with_env_filter("awswit=debug")
            .with_writer(std::io::stderr)
            .init();
    } else if args.info {
        tracing_subscriber::fmt()
            .with_env_filter("awswit=info")
            .with_writer(std::io::stderr)
            .init();
    }

    if let Err(e) = run(args).await {
        match e {
            AwswitError::UserCancelled => {
                print_info("Cancelled.");
                std::process::exit(0);
            }
            _ => {
                print_error(&format!("{}", e));
                std::process::exit(1);
            }
        }
    }
}

async fn run(args: Args) -> error::Result<()> {
    let awswit_config = AwswitConfig::load()?;

    // Handle version display (clap handles this, but just in case)
    // Handle completion generation
    if let Some(ref shell_name) = args.completion {
        let shell_type = ShellType::from_name(shell_name);
        println!("{}", shell::generate_completion(&shell_type));
        return Ok(());
    }

    // Handle shell wrapper generation
    // (Users call: eval "$(awswit --completion bash)")

    // Handle unset
    if args.unset {
        let shell_type = ShellType::detect();
        println!("{}", shell::generate_unset_commands(&shell_type));
        print_success("AWS environment variables cleared");
        return Ok(());
    }

    // Handle kill refresher
    if args.kill_refresher {
        autorefresh::kill_daemon()?;
        print_success("Auto-refresh daemon stopped");
        return Ok(());
    }

    // Load profiles
    let profiles = config::load_profiles(
        args.config_file.as_deref(),
        args.credentials_file.as_deref(),
    )?;

    if profiles.is_empty() {
        return Err(AwswitError::config_file_error(
            "No AWS profiles found. Check ~/.aws/config and ~/.aws/credentials",
        ));
    }

    // Handle list profiles
    if let Some(ref mode) = args.list_profiles {
        list_profiles(&profiles, mode, &awswit_config)?;
        return Ok(());
    }

    // Handle favorite toggle
    if let Some(ref profile_name) = args.favorite {
        let mut history = HistoryStorage::load()?;
        let is_fav = history.toggle_favorite(profile_name)?;
        if is_fav {
            print_success(&format!("Added '{}' to favorites", profile_name));
        } else {
            print_info(&format!("Removed '{}' from favorites", profile_name));
        }
        return Ok(());
    }

    // Determine the profile to use
    let profile_name = if let Some(ref name) = args.profile_name {
        name.clone()
    } else if let Some(ref role_arn) = args.role_arn {
        // Create an ad-hoc profile for direct role ARN
        resolve_role_arn_profile(&args, role_arn, &profiles, &awswit_config).await?;
        return Ok(());
    } else if !args.no_interactive && atty::is(atty::Stream::Stdin) {
        // Launch interactive picker
        let history = HistoryStorage::load()?;
        let mut picker = tui::Picker::new(
            profiles.clone(),
            history,
            awswit_config.colors,
            awswit_config.fuzzy_match,
        );
        picker.run()?
    } else {
        return Err(AwswitError::Other(
            "No profile specified. Use 'awswit <profile>' or run interactively.".to_string(),
        ));
    };

    // Verify profile exists
    if !profiles.contains_key(&profile_name) {
        // Try fuzzy matching
        let all_names: Vec<String> = profiles.keys().cloned().collect();
        let matches = utils::fuzzy_match(&profile_name, &all_names);
        if !matches.is_empty() {
            let suggestions: Vec<&str> = matches.iter().take(5).map(|m| m.name.as_str()).collect();
            return Err(AwswitError::profile_not_found(format!(
                "{}. Did you mean: {}?",
                profile_name,
                suggestions.join(", ")
            )));
        }
        return Err(AwswitError::profile_not_found(&profile_name));
    }

    // Get MFA token if needed
    let mfa_token = get_mfa_token(&args, &profiles, &profile_name)?;

    // Resolve credentials
    let spinner = tui::create_spinner(&format!("Resolving credentials for '{}'...", profile_name));

    let cache_manager = CacheManager::new()?;
    let resolver = ProfileResolver::new(
        profiles.clone(),
        cache_manager,
        awswit_config.clone(),
        args.refresh,
    );

    let credentials = resolver
        .resolve(&profile_name, mfa_token.as_deref())
        .await?;

    spinner.finish_and_clear();

    // Handle credential_process output format
    if args.credential_process {
        let json = credentials.to_credential_process_json();
        println!("{}", serde_json::to_string_pretty(&json)?);
        return Ok(());
    }

    // Generate export commands
    let shell_type = ShellType::detect();
    let export_cmds =
        shell::generate_export_commands(&credentials, &profile_name, &shell_type);

    if args.show_commands {
        // Print to stderr for display, don't output for eval
        eprintln!("{}", export_cmds);
        return Ok(());
    }

    // Output export commands (to be eval'd by shell wrapper)
    println!("{}", export_cmds);

    // Record usage history
    let mut history = HistoryStorage::load()?;
    history.record_usage(&profile_name)?;

    // Print status info to stderr
    print_success(&format!("Switched to profile: {}", profile_name.bold()));
    if let Some(remaining) = credentials.remaining_time() {
        print_info(&format!("Credentials expire in: {}", remaining));
    }

    // Start auto-refresh daemon if requested
    if args.auto_refresh {
        if let Some(profile) = profiles.get(&profile_name) {
            if profile.autoawswit.unwrap_or(false) || args.auto_refresh {
                autorefresh::start_daemon(&profile_name)?;
                print_info("Auto-refresh daemon started");
            }
        }
    }

    Ok(())
}

fn list_profiles(
    profiles: &std::collections::HashMap<String, profile::Profile>,
    mode: &str,
    _config: &AwswitConfig,
) -> error::Result<()> {
    let mut names: Vec<&String> = profiles.keys().collect();
    names.sort();

    let history = HistoryStorage::load()?;

    if mode == "more" {
        // Detailed view
        for name in &names {
            let profile = &profiles[*name];
            let ptype = profile.profile_type();
            let fav = if history.is_favorite(name) { "★" } else { " " };
            let region = profile.region.as_deref().unwrap_or("-");
            let role_arn = profile
                .role_arn
                .as_deref()
                .unwrap_or("-");

            eprintln!(
                "{} {:30} {:20} {:12} {}",
                fav,
                name,
                ptype,
                region,
                role_arn
            );
        }
    } else {
        // Simple view: just names (stdout for completion scripts)
        for name in &names {
            println!("{}", name);
        }
    }

    Ok(())
}

fn get_mfa_token(
    args: &Args,
    profiles: &std::collections::HashMap<String, profile::Profile>,
    profile_name: &str,
) -> error::Result<Option<String>> {
    if let Some(ref token) = args.mfa_token {
        return Ok(Some(token.clone()));
    }

    let profile = profiles.get(profile_name);
    let needs_mfa = profile.map(|p| p.requires_mfa()).unwrap_or(false);

    // Also check source profile for MFA
    let source_needs_mfa = profile
        .and_then(|p| p.source_profile.as_ref())
        .and_then(|sp| profiles.get(sp))
        .map(|sp| sp.requires_mfa())
        .unwrap_or(false);

    if needs_mfa || source_needs_mfa {
        let mfa_serial = profile
            .and_then(|p| p.mfa_serial.as_ref())
            .or_else(|| {
                profile
                    .and_then(|p| p.source_profile.as_ref())
                    .and_then(|sp| profiles.get(sp))
                    .and_then(|sp| sp.mfa_serial.as_ref())
            });

        if let Some(serial) = mfa_serial {
            if atty::is(atty::Stream::Stdin) {
                eprint!("MFA token for {}: ", serial);
                std::io::stderr().flush()?;

                let mut token = String::new();
                std::io::stdin().read_line(&mut token)?;
                let token = token.trim().to_string();

                if token.is_empty() {
                    return Err(AwswitError::mfa_required(serial));
                }

                return Ok(Some(token));
            } else {
                return Err(AwswitError::mfa_required(serial));
            }
        }
    }

    Ok(None)
}

async fn resolve_role_arn_profile(
    args: &Args,
    role_arn: &str,
    profiles: &std::collections::HashMap<String, profile::Profile>,
    config: &AwswitConfig,
) -> error::Result<()> {
    // Resolve source profile
    let source_profile_name = args
        .source_profile
        .as_deref()
        .unwrap_or("default");

    if !profiles.contains_key(source_profile_name) {
        return Err(AwswitError::profile_not_found(source_profile_name));
    }

    let mfa_token = get_mfa_token(args, profiles, source_profile_name)?;

    let cache_manager = CacheManager::new()?;
    let resolver = ProfileResolver::new(
        profiles.clone(),
        cache_manager,
        config.clone(),
        args.refresh,
    );

    // First resolve the source credentials
    let source_creds = resolver
        .resolve(source_profile_name, mfa_token.as_deref())
        .await?;

    // Then assume the role
    let sts = aws::StsClient::new(&source_creds).await;

    let session_name = args
        .session_name
        .clone()
        .unwrap_or_else(|| "awswit-direct".to_string());

    let duration = args
        .role_duration
        .unwrap_or(config.role_duration);

    let region = args
        .region
        .as_deref()
        .or(config.region.as_deref());

    let credentials = sts
        .assume_role(
            role_arn,
            &session_name,
            args.external_id.as_deref(),
            duration,
            None,
            None,
            region,
        )
        .await?;

    if args.credential_process {
        let json = credentials.to_credential_process_json();
        println!("{}", serde_json::to_string_pretty(&json)?);
        return Ok(());
    }

    let shell_type = ShellType::detect();
    let profile_display = format!("direct:{}", role_arn);
    let export_cmds =
        shell::generate_export_commands(&credentials, &profile_display, &shell_type);

    if args.show_commands {
        eprintln!("{}", export_cmds);
        return Ok(());
    }

    println!("{}", export_cmds);

    print_success(&format!("Assumed role: {}", role_arn));
    if let Some(remaining) = credentials.remaining_time() {
        print_info(&format!("Credentials expire in: {}", remaining));
    }

    Ok(())
}
