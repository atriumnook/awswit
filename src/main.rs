use std::collections::HashMap;
use std::io::{self, IsTerminal, Write};
use std::process::{Command as ProcCommand, Stdio};

use clap::Parser;
use tracing::Level;
use tracing_subscriber::FmtSubscriber;

use awswit::cli::{Args, Command};
use awswit::context::AppContext;
use awswit::error::AwswitError;
use awswit::history::ProfileHistory;
use awswit::profile::Profile;
use awswit::shell::ShellExporter;
use awswit::sso;
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
        Ok(code) => std::process::exit(code),
        Err(AwswitError::UserCancelled) => std::process::exit(130),
        Err(e) => {
            eprintln!("awswit: {}", e);
            std::process::exit(1);
        }
    }
}

fn run(args: Args) -> Result<i32, AwswitError> {
    if let Some(cmd) = args.command.clone() {
        return match cmd {
            Command::Init { shell } => print_init_script(&shell).map(|_| 0),
            Command::Completions { shell } => print_completions(shell).map(|_| 0),
            Command::Exec {
                profile,
                region,
                cmd,
            } => exec_command(args, profile, region, cmd),
            Command::Pick => pick(args),
            Command::Which => which(args),
            Command::Doctor { json } => doctor(args, json),
            Command::Prompt { format, default } => prompt(&format, &default).map(|_| 0),
        };
    }

    if args.version {
        println!("awswit {}", env!("CARGO_PKG_VERSION"));
        return Ok(0);
    }
    if args.unset {
        return emit_unset(&args).map(|_| 0);
    }

    // Fast path for shell tab-completion: only read section headers from
    // ~/.aws/config, skip history and SSO cache. Tab-completion is invoked
    // on every keystroke and ran the full AppContext::build before this,
    // which made every TAB hit NFS / large SSO cache directories.
    if args.list && args.names_only {
        return list_names_only(&args).map(|_| 0);
    }

    let ctx = AppContext::build(args)?;
    if ctx.args.list {
        return list_profiles(&ctx.profiles, &ctx.history, ctx.args.json).map(|_| 0);
    }
    switch_profile(ctx).map(|_| 0)
}

/// Tab-completion fast path. Avoid `AppContext::build` so we never touch
/// history or the SSO cache here.
fn list_names_only(args: &Args) -> Result<(), AwswitError> {
    let config_path = args
        .config_file
        .clone()
        .or_else(|| std::env::var("AWS_CONFIG_FILE").ok())
        .unwrap_or_else(|| {
            dirs::home_dir()
                .map(|h| h.join(".aws").join("config").to_string_lossy().to_string())
                .unwrap_or_else(|| "~/.aws/config".to_string())
        });

    let names = awswit::config::AwsFiles::fast_profile_names(&config_path);
    let stdout = io::stdout();
    let mut out = stdout.lock();
    for name in names {
        writeln!(out, "{}", name)?;
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────
//   Default action: switch profile in the current shell
// ─────────────────────────────────────────────────────────────────────

fn switch_profile(mut ctx: AppContext) -> Result<(), AwswitError> {
    warn_if_aws_vault();

    let (target, history) = resolve_target_profile(&ctx)?;
    ctx.history = history;
    ctx.history.record_use(&target);
    if let Err(e) = awswit::history::save_history(&ctx.history) {
        tracing::warn!("failed to persist history: {}", e);
    }

    let region = ctx
        .args
        .region
        .as_deref()
        .or_else(|| ctx.profiles.get(&target).and_then(|p| p.region.as_deref()));

    emit_export(&target, region, &ctx.args)
}

/// Resolve the profile to switch to, returning any history updates the user
/// made along the way (e.g. favorite toggles inside the picker).
///
/// The two return values are kept together so the single save site in
/// `switch_profile` always sees the latest history — previously the picker
/// saved on-toggle while main saved a stale snapshot afterwards, silently
/// reverting favorite changes.
fn resolve_target_profile(ctx: &AppContext) -> Result<(String, ProfileHistory), AwswitError> {
    let want_picker = ctx.args.profile_name.is_none()
        && !ctx.args.no_interactive
        && std::io::stdout().is_terminal();

    if want_picker {
        let outcome = pick_profile(ctx)?;
        match outcome.selected {
            Some(name) => return Ok((name, outcome.history)),
            None => return Err(AwswitError::UserCancelled),
        }
    }

    let history = ctx.history.clone();

    // In scripted / `-n` mode we refuse to invent a target: a stray `awswit -n`
    // in a CI step or with `$AWS_PROFILE` accidentally unset would otherwise
    // silently switch you to `default`, which is often the root/admin account.
    // Interactive bare invocation on a non-tty (output piped) still gets the
    // `default` fallback, matching what AWS SDKs themselves do.
    let candidate = match (
        ctx.args.profile_name.clone(),
        std::env::var("AWS_PROFILE").ok(),
        ctx.args.no_interactive,
    ) {
        (Some(p), _, _) => p,
        (None, Some(env), _) => env,
        (None, None, true) => {
            return Err(AwswitError::ShellError {
                message: "-n / --no-interactive requires a PROFILE argument or $AWS_PROFILE".into(),
            });
        }
        (None, None, false) => "default".to_string(),
    };

    if ctx.profiles.contains_key(&candidate) {
        return Ok((candidate, history));
    }

    // Auto-fuzzy substitution is a convenience for interactive use. In
    // scripted / `-n` mode it's a footgun — a CI typo would silently target
    // a different AWS account — so we surface "did you mean…?" suggestions
    // instead of substituting.
    let interactive_mode = !ctx.args.no_interactive;
    let allow_fuzzy = interactive_mode && std::env::var("AWSWIT_NO_FUZZY").is_err();
    if allow_fuzzy
        && let Some(matched) = utils::fuzzy::find_closest_profile(&candidate, &ctx.profiles)
    {
        eprintln!(
            "awswit: fuzzy-matched '{}' to '{}'. Set AWSWIT_NO_FUZZY=1 to disable.",
            candidate, matched
        );
        return Ok((matched, history));
    }

    Err(profile_not_found_with_hint(&candidate, &ctx.profiles))
}

/// Build a ProfileNotFound error whose message includes up to three
/// candidate suggestions, ranked by Levenshtein distance.
fn profile_not_found_with_hint(name: &str, profiles: &HashMap<String, Profile>) -> AwswitError {
    let suggestions = utils::fuzzy::nearest_n(name, profiles, 3);
    // Filter out anything far from the input (heuristic: more than half the
    // input's length in edit distance is almost certainly noise). Compare in
    // *character* counts, not bytes — `levenshtein` counts chars, so using
    // `str::len()` here is wrong for non-ASCII names (a Japanese profile
    // like "プロ" has byte len 6 but only 2 chars).
    let name_chars = name.chars().count();
    let suggested = suggestions
        .into_iter()
        .filter(|s| {
            let threshold = name_chars.max(s.chars().count()).div_ceil(2);
            strsim::levenshtein(&name.to_lowercase(), &s.to_lowercase()) <= threshold
        })
        .collect::<Vec<_>>();

    let hint = if suggested.is_empty() {
        String::new()
    } else {
        format!(" — did you mean: {}?", suggested.join(", "))
    };

    AwswitError::ProfileNotFound {
        name: format!("{}{}", name, hint),
    }
}

fn pick_profile(ctx: &AppContext) -> Result<tui::picker::PickerOutcome, AwswitError> {
    let use_fzf = ctx.args.use_fzf
        || std::env::var("AWSWIT_USE_FZF")
            .map(|v| tui::fzf::is_truthy(&v))
            .unwrap_or(false);

    if use_fzf {
        let name = tui::fzf::select_with_fzf(&ctx.profiles, &ctx.history)?;
        return Ok(tui::picker::PickerOutcome {
            selected: Some(name),
            history: ctx.history.clone(),
        });
    }

    let picker = tui::ProfilePicker::new(&ctx.profiles).with_history(ctx.history.clone());
    // `io::Error` already converts via `#[from]`; forwarding it preserves
    // the kind (`PermissionDenied`, `NotConnected`, etc.) so users can tell
    // "no /dev/tty" from "the picker crashed".
    picker.run().map_err(AwswitError::from)
}

fn warn_if_aws_vault() {
    if let Ok(vault_profile) = std::env::var("AWS_VAULT") {
        eprintln!(
            "awswit: warning: AWS_VAULT={} is set. Changing AWS_PROFILE inside an aws-vault \
             session can leave stale session credentials in your environment.",
            vault_profile
        );
    }
}

fn emit_export(profile: &str, region: Option<&str>, args: &Args) -> Result<(), AwswitError> {
    let exporter = ShellExporter::new();
    let payload = exporter.export(profile, region)?;

    if args.shell_export {
        io::stdout().write_all(payload.as_bytes())?;
    } else {
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

// ─────────────────────────────────────────────────────────────────────
//   `awswit -l` / `awswit -l --json`
// ─────────────────────────────────────────────────────────────────────

fn list_profiles(
    profiles: &HashMap<String, Profile>,
    history: &awswit::history::ProfileHistory,
    json: bool,
) -> Result<(), AwswitError> {
    let mut names: Vec<&String> = profiles.keys().collect();
    names.sort();

    let stdout = io::stdout();
    let mut out = stdout.lock();

    if json {
        let entries: Vec<_> = names
            .iter()
            .map(|n| {
                let p = &profiles[*n];
                let h = history.get(n);
                serde_json::json!({
                    "name": n,
                    "type": classify(p),
                    "source": p.source_profile.as_deref().or(p.credential_source.as_deref()),
                    "region": p.region,
                    "account": p.get_account_id(),
                    "favorite": history.is_favorite(n),
                    "use_count": h.map(|e| e.use_count).unwrap_or(0),
                    "last_used": h.map(|e| e.last_used.to_rfc3339()),
                })
            })
            .collect();
        serde_json::to_writer_pretty(&mut out, &entries)?;
        writeln!(out)?;
        return Ok(());
    }

    if !out.is_terminal() {
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

// ─────────────────────────────────────────────────────────────────────
//   `awswit exec PROFILE -- CMD ARGS...`
// ─────────────────────────────────────────────────────────────────────

fn exec_command(
    mut args: Args,
    profile: String,
    region_override: Option<String>,
    cmd: Vec<String>,
) -> Result<i32, AwswitError> {
    if cmd.is_empty() {
        return Err(AwswitError::ShellError {
            message: "exec requires a command after `--`".into(),
        });
    }
    // Build ctx without a TUI ever firing.
    args.no_interactive = true;
    args.profile_name = Some(profile.clone());
    let ctx = AppContext::build(args)?;

    if !ctx.profiles.contains_key(&profile) {
        return Err(profile_not_found_with_hint(&profile, &ctx.profiles));
    }

    let region =
        region_override.or_else(|| ctx.profiles.get(&profile).and_then(|p| p.region.clone()));

    // Record frecency *before* spawning the child: a long-running or killed
    // command (an hour-long `aws s3 sync`, a Ctrl-C'd shell) should still
    // bump usage stats — otherwise frecency only records commands that
    // ran to completion, which defeats its purpose.
    let mut history = ctx.history;
    history.record_use(&profile);
    if let Err(e) = awswit::history::save_history(&history) {
        tracing::warn!("failed to persist history: {}", e);
    }

    let mut child = ProcCommand::new(&cmd[0]);
    child.args(&cmd[1..]);
    child.env("AWS_PROFILE", &profile);
    child.env_remove("AWS_DEFAULT_PROFILE");
    if let Some(r) = &region {
        child.env("AWS_REGION", r);
    } else {
        child.env_remove("AWS_REGION");
    }
    child.env_remove("AWS_DEFAULT_REGION");

    child.stdin(Stdio::inherit());
    child.stdout(Stdio::inherit());
    child.stderr(Stdio::inherit());

    let status = child
        .spawn()
        .map_err(|e| AwswitError::ShellError {
            message: format!("failed to spawn `{}`: {}", cmd[0], e),
        })?
        .wait()
        .map_err(|e| AwswitError::ShellError {
            message: format!("wait failed for `{}`: {}", cmd[0], e),
        })?;

    Ok(status.code().unwrap_or(1))
}

// ─────────────────────────────────────────────────────────────────────
//   `awswit pick`
// ─────────────────────────────────────────────────────────────────────

/// Open the picker, print the selection to stdout, exit 130 on cancel.
///
/// This is the pipeline-composition entry point — it never touches the
/// parent shell's environment. The status chatter we emit for the
/// default switch path is suppressed here so `$(awswit pick)` captures
/// only the profile name.
fn pick(args: Args) -> Result<i32, AwswitError> {
    let ctx = AppContext::build(args)?;
    let outcome = pick_profile(&ctx)?;
    match outcome.selected {
        Some(name) => {
            let mut history = outcome.history;
            history.record_use(&name);
            if let Err(e) = awswit::history::save_history(&history) {
                tracing::warn!("failed to persist history: {}", e);
            }
            println!("{}", name);
            Ok(0)
        }
        None => Err(AwswitError::UserCancelled),
    }
}

// ─────────────────────────────────────────────────────────────────────
//   `awswit which`
// ─────────────────────────────────────────────────────────────────────

fn which(args: Args) -> Result<i32, AwswitError> {
    let current = std::env::var("AWS_PROFILE").ok();
    let region_env = std::env::var("AWS_REGION").ok();
    let vault = std::env::var("AWS_VAULT").ok();

    let stdout = io::stdout();
    let mut out = stdout.lock();

    let Some(profile_name) = current else {
        writeln!(out, "AWS_PROFILE: (unset)")?;
        return Ok(0);
    };

    writeln!(out, "AWS_PROFILE: {}", profile_name)?;
    if let Some(r) = &region_env {
        writeln!(out, "AWS_REGION:  {}", r)?;
    }
    if let Some(v) = &vault {
        writeln!(out, "AWS_VAULT:   {} (running inside aws-vault session)", v)?;
    }

    // Load profile config and annotate.
    let ctx = match AppContext::build(args) {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!("which: skipping config lookup: {}", e);
            return Ok(0);
        }
    };

    let Some(profile) = ctx.profiles.get(&profile_name) else {
        writeln!(out)?;
        writeln!(
            out,
            "warning: profile '{}' is not defined in ~/.aws/config",
            profile_name
        )?;
        // Surface this to CI / precmd guards — `which` exits non-zero when
        // the environment claims a profile the config doesn't define.
        return Ok(1);
    };

    writeln!(out)?;
    writeln!(out, "type:        {}", classify(profile))?;
    if let Some(r) = &profile.region {
        writeln!(out, "region:      {}", r)?;
    }
    if let Some(a) = profile.get_account_id() {
        writeln!(out, "account:     {}", a)?;
    }
    if let Some(arn) = &profile.role_arn {
        writeln!(out, "role:        {}", arn)?;
    }
    if let Some(src) = &profile.source_profile {
        writeln!(out, "source:      {}", src)?;
    }
    if let Some(start) = &profile.sso_start_url {
        writeln!(out, "sso_start:   {}", start)?;
        // Check SSO token expiry.
        let sessions = sso::load_sessions();
        match sso::find_session(&sessions, start, profile.sso_region.as_deref()) {
            Some(s) if !s.is_expired(chrono::Utc::now()) => {
                writeln!(
                    out,
                    "sso_token:   valid until {}",
                    s.expires_at.format("%Y-%m-%d %H:%M UTC")
                )?;
            }
            Some(_) => {
                writeln!(
                    out,
                    "sso_token:   EXPIRED — run `aws sso login --profile {}`",
                    profile_name
                )?;
            }
            None => {
                writeln!(
                    out,
                    "sso_token:   not cached — run `aws sso login --profile {}`",
                    profile_name
                )?;
            }
        }
    }
    if profile.mfa_serial.is_some() {
        writeln!(out, "mfa:         required")?;
    }
    Ok(0)
}

// ─────────────────────────────────────────────────────────────────────
//   `awswit doctor`
// ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiagLevel {
    Error,
    Warning,
    Info,
}

struct Diagnostic {
    level: DiagLevel,
    profile: Option<String>,
    message: String,
}

fn doctor(args: Args, json: bool) -> Result<i32, AwswitError> {
    let ctx = AppContext::build(args)?;
    let now = chrono::Utc::now();
    let sso_sessions = sso::load_sessions();

    let mut diags: Vec<Diagnostic> = Vec::new();

    if ctx.profiles.is_empty() {
        diags.push(Diagnostic {
            level: DiagLevel::Warning,
            profile: None,
            message: "no profiles found in ~/.aws/config".into(),
        });
    }

    for (name, p) in &ctx.profiles {
        // Shell-unsafe profile name — these get filtered out of
        // --names-only and the tab-completion wordlist, but they're
        // also a sign that another tool wrote the file with an
        // unrendered template variable or worse. Surface them as errors.
        if !is_safe_profile_name(name) {
            diags.push(Diagnostic {
                level: DiagLevel::Error,
                profile: Some(name.clone()),
                message: "profile name contains characters that are unsafe in shell \
                          contexts (`$`, backtick, whitespace, etc.) — rename the section \
                          header in ~/.aws/config; tab-completion silently skips this name"
                    .into(),
            });
        }

        // Source-profile chain must resolve and must not loop.
        if let Some(src) = &p.source_profile {
            if !ctx.profiles.contains_key(src) {
                diags.push(Diagnostic {
                    level: DiagLevel::Error,
                    profile: Some(name.clone()),
                    message: format!(
                        "source_profile = '{}' references a profile that does not exist",
                        src
                    ),
                });
            } else if let Some(cycle) = find_source_profile_cycle(name, &ctx.profiles) {
                diags.push(Diagnostic {
                    level: DiagLevel::Error,
                    profile: Some(name.clone()),
                    message: format!(
                        "source_profile chain forms a cycle: {} — the AWS SDK will loop \
                         forever resolving credentials",
                        cycle.join(" -> ")
                    ),
                });
            }
        }

        // Role profile should have either source_profile or credential_source.
        if p.role_arn.is_some() && p.source_profile.is_none() && p.credential_source.is_none() {
            diags.push(Diagnostic {
                level: DiagLevel::Warning,
                profile: Some(name.clone()),
                message: "role profile has neither source_profile nor credential_source — \
                          add e.g. `source_profile = main` or \
                          `credential_source = Environment|Ec2InstanceMetadata|EcsContainer`"
                    .into(),
            });
        }

        // MFA serial should look like an ARN.
        if let Some(mfa) = &p.mfa_serial
            && !mfa.starts_with("arn:")
        {
            diags.push(Diagnostic {
                level: DiagLevel::Warning,
                profile: Some(name.clone()),
                message: format!("mfa_serial = '{}' does not look like an IAM MFA ARN", mfa),
            });
        }

        // Region format sanity-check — catches `us-east-1a` (AZ instead of
        // region), `eu-west` (missing trailing digit), and other typos.
        if let Some(region) = &p.region
            && !looks_like_aws_region(region)
        {
            diags.push(Diagnostic {
                level: DiagLevel::Warning,
                profile: Some(name.clone()),
                message: format!(
                    "region = '{}' does not look like an AWS region (expected e.g. \
                     us-east-1, ap-northeast-3, eu-central-1)",
                    region
                ),
            });
        }

        // SSO profiles need an unexpired cached token.
        if let Some(start) = &p.sso_start_url {
            match sso::find_session(&sso_sessions, start, p.sso_region.as_deref()) {
                None => diags.push(Diagnostic {
                    level: DiagLevel::Warning,
                    profile: Some(name.clone()),
                    message: format!(
                        "no SSO token cached — run `aws sso login --profile {}`",
                        name
                    ),
                }),
                Some(s) if s.is_expired(now) => diags.push(Diagnostic {
                    level: DiagLevel::Error,
                    profile: Some(name.clone()),
                    message: format!(
                        "SSO token expired at {} — run `aws sso login --profile {}`",
                        s.expires_at.format("%Y-%m-%d %H:%M UTC"),
                        name
                    ),
                }),
                _ => {}
            }
        }
    }

    diags.sort_by_key(|d| (d.level == DiagLevel::Info, d.level == DiagLevel::Warning));
    let errors = diags.iter().filter(|d| d.level == DiagLevel::Error).count();
    let warnings = diags
        .iter()
        .filter(|d| d.level == DiagLevel::Warning)
        .count();

    let stdout = io::stdout();
    let mut out = stdout.lock();

    if json {
        let payload = serde_json::json!({
            "profiles_checked": ctx.profiles.len(),
            "errors": errors,
            "warnings": warnings,
            "diagnostics": diags.iter().map(|d| serde_json::json!({
                "level": match d.level {
                    DiagLevel::Error => "error",
                    DiagLevel::Warning => "warning",
                    DiagLevel::Info => "info",
                },
                "profile": d.profile,
                "message": d.message,
            })).collect::<Vec<_>>(),
        });
        serde_json::to_writer_pretty(&mut out, &payload)?;
        writeln!(out)?;
        return Ok(if errors > 0 { 1 } else { 0 });
    }

    if diags.is_empty() {
        writeln!(
            out,
            "awswit doctor: no issues found ({} profiles)",
            ctx.profiles.len()
        )?;
        return Ok(0);
    }

    for d in &diags {
        let tag = match d.level {
            DiagLevel::Error => "ERROR",
            DiagLevel::Warning => "warn ",
            DiagLevel::Info => "info ",
        };
        match &d.profile {
            Some(p) => writeln!(out, "{} [{}] {}", tag, p, d.message)?,
            None => writeln!(out, "{}        {}", tag, d.message)?,
        }
    }

    writeln!(
        out,
        "\nawswit doctor: {} {}, {} {}",
        errors,
        pluralize(errors, "error", "errors"),
        warnings,
        pluralize(warnings, "warning", "warnings"),
    )?;
    Ok(if errors > 0 { 1 } else { 0 })
}

fn pluralize(n: usize, singular: &'static str, plural: &'static str) -> &'static str {
    if n == 1 { singular } else { plural }
}

/// Profile-name safety check used by `doctor`. Mirrors the filter in
/// `AwsFiles::fast_profile_names` so the two paths agree about what is
/// surfaceable as a candidate.
fn is_safe_profile_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | '@' | '+' | '=')
        })
}

/// Walk the source_profile chain from `start` and detect a cycle. Returns
/// the cycle path as a vec of profile names if one exists.
fn find_source_profile_cycle(
    start: &str,
    profiles: &HashMap<String, Profile>,
) -> Option<Vec<String>> {
    use std::collections::HashSet;
    let mut seen: HashSet<&str> = HashSet::new();
    let mut path: Vec<String> = Vec::new();
    let mut cursor = start;
    while seen.insert(cursor) {
        path.push(cursor.to_string());
        match profiles
            .get(cursor)
            .and_then(|p| p.source_profile.as_deref())
        {
            Some(next) if profiles.contains_key(next) => cursor = next,
            _ => return None,
        }
    }
    // We re-entered a profile — append the closing edge to make the cycle
    // visible: a -> b -> a.
    path.push(cursor.to_string());
    Some(path)
}

/// True if `r` looks like an AWS region identifier: `<two letters>-<word>-<digit>`.
///
/// Catches accidental Availability Zones (`us-east-1a`), missing trailing
/// digits (`eu-west`), and outright typos (`useast1`). Permissive enough
/// to admit every real region AWS has shipped (gov, cn, isob partitions).
fn looks_like_aws_region(r: &str) -> bool {
    let parts: Vec<&str> = r.split('-').collect();
    if parts.len() < 3 {
        return false;
    }
    let last = parts[parts.len() - 1];
    // Last token must be one or more digits and nothing else (the AZ form
    // `us-east-1a` ends in `1a`, which fails this check).
    if last.is_empty() || !last.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    // All other tokens are lowercase alphanumerics — also catches mixed
    // case typos like "US-East-1".
    parts[..parts.len() - 1]
        .iter()
        .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_lowercase()))
}

// ─────────────────────────────────────────────────────────────────────
//   `awswit prompt`
// ─────────────────────────────────────────────────────────────────────

fn prompt(format: &str, default: &str) -> Result<(), AwswitError> {
    let current = std::env::var("AWS_PROFILE").ok();
    let out = match current {
        Some(name) if !name.is_empty() => format.replace("{}", &name).replace("%s", &name),
        _ => default.to_string(),
    };
    print!("{}", out);
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────
//   Helpers
// ─────────────────────────────────────────────────────────────────────

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
