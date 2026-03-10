use std::io::IsTerminal;

use clap::Parser;
use tracing::Level;
use tracing_subscriber::FmtSubscriber;

mod cli;
mod config;
mod profile;
mod aws;
mod cache;
mod shell;
mod autorefresh;
mod utils;
mod error;
mod tui;
mod history;

use cli::{Args, Command};
use config::{AwswitConfig, AwsFiles};
use profile::ProfileResolver;
use aws::StsClient;
use cache::CacheManager;
use shell::ShellExporter;
use error::AwswitError;

#[tokio::main]
async fn main() {
    let args = Args::parse();

    // Setup logging
    setup_logging(&args);

    // Run the main application
    if let Err(e) = run(args).await {
        // Output error in a format the shell wrapper can handle
        eprintln!("{}", e);
        std::process::exit(1);
    }
}

fn setup_logging(args: &Args) {
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

    tracing::subscriber::set_global_default(subscriber)
        .expect("Failed to set tracing subscriber");
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

    // Load awswit configuration
    let awswit_config = AwswitConfig::load()?;
    tracing::debug!("Loaded awswit config: {:?}", awswit_config);

    // Handle version flag
    if args.version {
        println!("awswit {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    // Handle config management
    if let Some(ref config_options) = args.config {
        return handle_config_command(&awswit_config, config_options);
    }

    // Handle unset flag
    if args.unset {
        return handle_unset(&args);
    }

    // Handle kill refresher
    if args.kill_refresher {
        return handle_kill_refresher(&args).await;
    }

    // Load AWS config and credentials files
    let credentials_file = match args.credentials_file.clone()
        .or_else(|| std::env::var("AWS_SHARED_CREDENTIALS_FILE").ok())
    {
        Some(path) => path,
        None => {
            let home = dirs::home_dir()
                .ok_or_else(|| AwswitError::ShellError("Could not determine home directory".to_string()))?;
            home.join(".aws").join("credentials").to_string_lossy().to_string()
        }
    };

    let config_file = match args.config_file.clone()
        .or_else(|| std::env::var("AWS_CONFIG_FILE").ok())
    {
        Some(path) => path,
        None => {
            let home = dirs::home_dir()
                .ok_or_else(|| AwswitError::ShellError("Could not determine home directory".to_string()))?;
            home.join(".aws").join("config").to_string_lossy().to_string()
        }
    };

    let aws_files = AwsFiles::load(&config_file, &credentials_file)?;
    let profiles = aws_files.merge_profiles();
    tracing::debug!("Loaded {} profiles", profiles.len());

    // Handle list profiles
    if args.list_profiles.is_some() {
        return handle_list_profiles(&profiles, &args, &awswit_config).await;
    }

    // Handle refresh autocomplete
    if args.refresh_autocomplete {
        return handle_refresh_autocomplete(&profiles);
    }

    // Load history
    let mut profile_history = history::ProfileHistory::load().unwrap_or_default();

    // Determine target profile - use interactive mode if no profile specified
    let target_profile_name = if args.profile_name.is_none() 
        && args.role_arn.is_none() 
        && !args.interactive_disabled()
        && std::io::stdout().is_terminal()
    {
        // Launch interactive picker
        let picker = tui::ProfilePicker::new(profiles.clone())
            .with_history(profile_history.clone());
        
        match picker.run() {
            Ok(tui::picker::PickerResult::Selected(name)) => name,
            Ok(tui::picker::PickerResult::Cancelled) => {
                return Ok(());
            }
            Err(e) => {
                return Err(AwswitError::ShellError(format!("Picker error: {}", e)));
            }
        }
    } else {
        determine_target_profile(&args, &profiles, &awswit_config)?
    };
    
    tracing::info!("Target profile: {}", target_profile_name);

    // Show spinner while resolving credentials
    let spinner = tui::AwswitSpinner::assuming_role(&target_profile_name);

    // Resolve the profile chain and get credentials
    let resolver = ProfileResolver::new(&profiles, &awswit_config);
    let cache_manager = CacheManager::new()?;
    let sts_client = StsClient::new().await;

    let credentials = match resolver.resolve_credentials(
        &target_profile_name,
        &args,
        &sts_client,
        &cache_manager,
    ).await {
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
    let _ = profile_history.save();

    // Handle auto-refresh
    if args.auto_refresh {
        autorefresh::start_auto_refresh(&target_profile_name, &args, &credentials).await?;
    }

    // Handle output profile
    if let Some(ref output_profile) = args.output_profile {
        handle_output_profile(output_profile, &credentials, &credentials_file)?;
    }

    // Export credentials to shell
    let exporter = ShellExporter::new();
    if args.show_commands {
        // Print export commands for manual use
        let commands = exporter.generate_export_commands(&credentials, &target_profile_name);
        println!("{}", commands);
    } else {
        // Show nice status message
        tui::StatusLine::profile_assumed(
            &target_profile_name,
            credentials.expiration.map(|e| e.format("%Y-%m-%d %H:%M:%S").to_string()).as_deref(),
        );
        
        // Output in a format the shell wrapper can eval
        let output = exporter.generate_shell_output(&credentials, &target_profile_name);
        print!("{}", output);
    }

    Ok(())
}

fn handle_config_command(config: &AwswitConfig, options: &[String]) -> Result<(), AwswitError> {
    if options.is_empty() {
        // List all config
        println!("{}", serde_yaml::to_string(config)?);
        return Ok(());
    }

    match options.first().map(|s| s.as_str()) {
        Some("set") if options.len() >= 3 => {
            let key = &options[1];
            let value = &options[2];
            let mut new_config = config.clone();
            new_config.set_value(key, value)?;
            new_config.save()?;
            println!("Set {} = {}", key, value);
        }
        Some("get") if options.len() >= 2 => {
            let key = &options[1];
            if let Some(value) = config.get_value(key) {
                println!("{}", value);
            } else {
                return Err(AwswitError::ConfigKeyNotFound(key.clone()));
            }
        }
        Some("reset") | Some("clear") if options.len() >= 2 => {
            let key = &options[1];
            let mut new_config = config.clone();
            new_config.reset_value(key)?;
            new_config.save()?;
            println!("Reset {} to default", key);
        }
        Some("list") | None => {
            println!("{}", serde_yaml::to_string(config)?);
        }
        _ => {
            return Err(AwswitError::InvalidConfigCommand(options.join(" ")));
        }
    }

    Ok(())
}

fn handle_unset(args: &Args) -> Result<(), AwswitError> {
    let exporter = ShellExporter::new();
    if args.show_commands {
        let commands = exporter.generate_unset_commands();
        println!("{}", commands);
    } else {
        let output = exporter.generate_unset_output();
        print!("{}", output);
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

async fn handle_list_profiles(
    profiles: &std::collections::HashMap<String, profile::Profile>,
    args: &Args,
    config: &AwswitConfig,
) -> Result<(), AwswitError> {
    use colored::Colorize;

    let show_more = args.list_profiles.as_ref().map(|s| s == "more").unwrap_or(false);
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
        let profile_type = if profile.role_arn.is_some() { "Role" } else { "User" };
        let source = profile.source_profile.as_deref()
            .or(profile.credential_source.as_deref())
            .unwrap_or("None");
        let mfa = if profile.mfa_serial.is_some() { "Yes" } else { "No" };
        let region = profile.region.as_deref().unwrap_or("-");
        
        let account = if show_more {
            // Would need to make STS call here
            "Fetching...".to_string()
        } else {
            profile.role_arn.as_ref()
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
    // arn:aws:iam::123456789012:role/RoleName
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
    // If role ARN is provided directly, we don't need a profile name
    if args.role_arn.is_some() {
        return Ok("cli-role".to_string());
    }

    // Get profile name from args or default
    let profile_name = args.profile_name.clone()
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

    Err(AwswitError::ProfileNotFound(profile_name))
}

fn handle_init(shell: &str) -> Result<(), AwswitError> {
    let script = match shell.to_lowercase().as_str() {
        "bash" | "zsh" => include_str!("init/bash.sh"),
        "fish" => include_str!("init/fish.fish"),
        "powershell" | "pwsh" => include_str!("init/powershell.ps1"),
        _ => {
            return Err(AwswitError::ShellError(format!(
                "Unsupported shell: {}. Supported shells: bash, zsh, fish, powershell",
                shell
            )));
        }
    };
    print!("{}", script);
    Ok(())
}

fn handle_output_profile(
    output_profile: &str,
    credentials: &aws::Credentials,
    credentials_file: &str,
) -> Result<(), AwswitError> {
    use std::fs;
    use fs2::FileExt;

    // Acquire exclusive lock before reading/writing credentials file
    let lock_path = format!("{}.lock", credentials_file);
    let lock_file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .open(&lock_path)?;
    lock_file.lock_exclusive()?;

    let content = fs::read_to_string(credentials_file).unwrap_or_default();

    // Remove existing section if present to avoid duplicates
    let section_header = format!("[{}]", output_profile);
    let lines: Vec<&str> = content.lines().collect();
    let mut new_lines = Vec::new();
    let mut skip = false;

    for line in &lines {
        if line.starts_with('[') {
            skip = line.trim() == section_header;
        }
        if !skip {
            new_lines.push(*line);
        }
    }

    let mut new_content = new_lines.join("\n");
    if !new_content.ends_with('\n') && !new_content.is_empty() {
        new_content.push('\n');
    }

    let profile_content = format!(
        "[{}]\n\
        aws_access_key_id = {}\n\
        aws_secret_access_key = {}\n\
        aws_session_token = {}\n\
        manager = awswit\n\
        awswit_expiration = {}\n",
        output_profile,
        credentials.access_key_id,
        credentials.secret_access_key,
        credentials.session_token.as_deref().unwrap_or(""),
        credentials.expiration.map(|e| e.to_rfc3339()).unwrap_or_default()
    );

    new_content.push_str(&profile_content);
    fs::write(credentials_file, new_content)?;

    lock_file.unlock()?;

    tracing::info!("Wrote credentials to profile: {}", output_profile);
    Ok(())
}
