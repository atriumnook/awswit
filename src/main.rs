use std::collections::HashMap;
use std::io::{self, IsTerminal, Write};

use clap::Parser;
use tracing::Level;
use tracing_subscriber::FmtSubscriber;

use awswit::cli::{Args, Command};
use awswit::context::AppContext;
use awswit::error::AwswitError;
use awswit::profile::Profile;
use awswit::shell::ShellExporter;
use awswit::{tui, utils};

fn main() {
    let args = Args::parse();

    let level = if args.debug {
        Level::DEBUG
    } else if args.verbose {
        Level::INFO
    } else {
        Level::WARN
    };

    let subscriber = FmtSubscriber::builder()
        .with_max_level(level)
        .with_target(false)
        .with_writer(io::stderr)
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);

    match run(args) {
        Ok(_) => {}
        Err(AwswitError::UserCancelled) => std::process::exit(130),
        Err(e) => {
            eprintln!("awswit: {}", e);
            std::process::exit(1);
        }
    }
}

fn run(args: Args) -> Result<(), AwswitError> {
    // Subcommands run without touching ~/.aws.
    if let Some(cmd) = args.command.clone() {
        return match cmd {
            Command::Init { shell } => print_init_script(&shell),
            Command::Completions { shell } => print_completions(shell),
        };
    }

    if args.version {
        println!("awswit {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    if args.unset {
        return emit_unset(&args);
    }

    let ctx = AppContext::build(args)?;

    if ctx.args.list {
        return list_profiles(&ctx.profiles, ctx.args.json);
    }

    // Warn loudly if we're switching profile inside an aws-vault session — the
    // resulting env is almost certainly not what the user wants.
    if let Ok(vault_profile) = std::env::var("AWS_VAULT") {
        eprintln!(
            "awswit: warning: AWS_VAULT={} is set. Changing AWS_PROFILE inside an aws-vault \
             session can leave stale session credentials in your environment.",
            vault_profile
        );
    }

    let target = resolve_target_profile(&ctx)?;

    let mut history = ctx.history;
    history.record_use(&target);
    if let Err(e) = awswit::history::save_history(&history) {
        tracing::warn!("Failed to persist history: {}", e);
    }

    let region = ctx
        .args
        .region
        .as_deref()
        .or_else(|| ctx.profiles.get(&target).and_then(|p| p.region.as_deref()));

    emit_export(&target, region, &ctx.args)
}

/// Decide which profile name we are switching to.
///
/// Priority: TUI / fzf if interactive → CLI arg → `$AWS_PROFILE` →
/// `"default"`. Names that aren't an exact match fall through to fuzzy
/// matching, which logs a WARN noting the substitution.
fn resolve_target_profile(ctx: &AppContext) -> Result<String, AwswitError> {
    let want_picker = ctx.args.profile_name.is_none()
        && !ctx.args.no_interactive
        && std::io::stdout().is_terminal();

    if want_picker {
        return pick_profile(ctx);
    }

    let candidate = ctx
        .args
        .profile_name
        .clone()
        .or_else(|| std::env::var("AWS_PROFILE").ok())
        .unwrap_or_else(|| "default".to_string());

    if ctx.profiles.contains_key(&candidate) {
        return Ok(candidate);
    }

    let allow_fuzzy = std::env::var("AWSWIT_NO_FUZZY").is_err();
    if allow_fuzzy
        && let Some(matched) = utils::fuzzy::find_closest_profile(&candidate, &ctx.profiles)
    {
        eprintln!(
            "awswit: fuzzy-matched '{}' to '{}'. Set AWSWIT_NO_FUZZY=1 to disable.",
            candidate, matched
        );
        return Ok(matched);
    }

    Err(AwswitError::ProfileNotFound { name: candidate })
}

fn pick_profile(ctx: &AppContext) -> Result<String, AwswitError> {
    let use_fzf = ctx.args.use_fzf
        || std::env::var("AWSWIT_USE_FZF")
            .map(|v| tui::fzf::is_truthy(&v))
            .unwrap_or(false);

    if use_fzf {
        return tui::fzf::select_with_fzf(&ctx.profiles, &ctx.history);
    }

    let picker = tui::ProfilePicker::new(&ctx.profiles).with_history(ctx.history.clone());
    match picker.run() {
        Ok(tui::picker::PickerResult::Selected(name)) => Ok(name),
        Ok(tui::picker::PickerResult::Cancelled) => Err(AwswitError::UserCancelled),
        Err(e) => Err(AwswitError::ShellError {
            message: format!("Picker error: {}", e),
        }),
    }
}

fn emit_export(profile: &str, region: Option<&str>, args: &Args) -> Result<(), AwswitError> {
    let exporter = ShellExporter::new();
    let payload = exporter.export(profile, region)?;

    if args.shell_export {
        // eval-mode: payload on stdout, nothing else.
        io::stdout().write_all(payload.as_bytes())?;
    } else {
        // Direct-invocation mode: show the user what would happen.
        eprintln!("awswit: switched to {}", profile);
        if let Some(r) = region {
            eprintln!("awswit: region {}", r);
        }
        io::stdout().write_all(payload.as_bytes())?;
    }
    Ok(())
}

fn emit_unset(args: &Args) -> Result<(), AwswitError> {
    let exporter = ShellExporter::new();
    let payload = exporter.unset_all();
    if !args.shell_export {
        eprintln!("awswit: unset");
    }
    io::stdout().write_all(payload.as_bytes())?;
    Ok(())
}

fn list_profiles(profiles: &HashMap<String, Profile>, json: bool) -> Result<(), AwswitError> {
    let mut names: Vec<&String> = profiles.keys().collect();
    names.sort();

    let stdout = io::stdout();
    let mut out = stdout.lock();

    if json {
        let entries: Vec<_> = names
            .iter()
            .map(|n| {
                let p = &profiles[*n];
                serde_json::json!({
                    "name": n,
                    "type": classify(p),
                    "source": p.source_profile.as_deref().or(p.credential_source.as_deref()),
                    "region": p.region,
                    "account": p.get_account_id(),
                })
            })
            .collect();
        serde_json::to_writer_pretty(&mut out, &entries)?;
        writeln!(out)?;
        return Ok(());
    }

    if !out.is_terminal() {
        // Tab-separated, no headers — easy for awk/cut.
        for name in &names {
            let p = &profiles[*name];
            writeln!(
                out,
                "{}\t{}\t{}\t{}\t{}",
                name,
                classify(p),
                p.source_profile
                    .as_deref()
                    .or(p.credential_source.as_deref())
                    .unwrap_or(""),
                p.region.as_deref().unwrap_or(""),
                p.get_account_id().unwrap_or_default(),
            )?;
        }
        return Ok(());
    }

    // Pretty table.
    let name_width = names.iter().map(|n| n.len()).max().unwrap_or(7).max(7);
    writeln!(
        out,
        "{:<width$}  {:<5}  {:<20}  {:<14}  ACCOUNT",
        "PROFILE",
        "TYPE",
        "SOURCE",
        "REGION",
        width = name_width
    )?;
    for name in &names {
        let p = &profiles[*name];
        writeln!(
            out,
            "{:<width$}  {:<5}  {:<20}  {:<14}  {}",
            name,
            classify(p),
            p.source_profile
                .as_deref()
                .or(p.credential_source.as_deref())
                .unwrap_or("-"),
            p.region.as_deref().unwrap_or("-"),
            p.get_account_id().unwrap_or_else(|| "-".into()),
            width = name_width,
        )?;
    }
    Ok(())
}

fn classify(p: &Profile) -> &'static str {
    if p.is_sso_profile() {
        "SSO"
    } else if p.is_role_profile() {
        "Role"
    } else {
        "User"
    }
}

fn print_completions(shell: clap_complete::Shell) -> Result<(), AwswitError> {
    use clap::CommandFactory;
    let mut cmd = Args::command();
    clap_complete::generate(shell, &mut cmd, "awswit", &mut io::stdout());
    Ok(())
}

fn print_init_script(shell: &str) -> Result<(), AwswitError> {
    let script = match shell.to_lowercase().as_str() {
        "bash" => include_str!("init/bash.sh"),
        "zsh" => include_str!("init/zsh.sh"),
        "fish" => include_str!("init/fish.fish"),
        "powershell" | "pwsh" => include_str!("init/powershell.ps1"),
        other => {
            return Err(AwswitError::ShellError {
                message: format!(
                    "unsupported shell '{}' (expected: bash, zsh, fish, powershell)",
                    other
                ),
            });
        }
    };
    print!("{}", script);
    Ok(())
}
