use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use clap::CommandFactory;
use serde::Serialize;

use crate::activation::EnvironmentPatch;
use crate::catalog::{
    Catalog, CatalogFileKind, CatalogIssueKind, CredentialSource, EnvironmentSource, PathOrigin,
    Profile, SourceFile, SourcePaths, SsoMetadata,
};
use crate::cli::{
    ActivateArgs, Cli, Command, DoctorArgs, DoctorFormat, ExecArgs, ListArgs, ListFormat, Shell,
    SourceArgs,
};
use crate::error::AppError;
use crate::history::{PreferenceDelta, PreferenceHistory, PreferenceStore};
use crate::output;
use crate::process::{self, ExecutionPlan};
use crate::safety::CredentialEnvironment;
use crate::text_safety::{is_unambiguous_terminal_text, visible_terminal_text};

pub(crate) enum Completion {
    Exit(u8),
    Cancelled,
    Terminated(i32),
}

pub(crate) fn run(cli: Cli) -> Result<Completion, AppError> {
    match cli.command {
        Some(Command::Init { shell }) => emit_init(shell),
        Some(Command::Completions { shell }) => emit_completions(shell),
        Some(Command::Unset) => unset(),
        Some(Command::Activate(args)) => activate(args),
        Some(Command::Exec(args)) => execute(args),
        Some(Command::List(args)) => list(args),
        Some(Command::Doctor(args)) => doctor(args),
        None => activate(ActivateArgs::default()),
    }
}

fn unset() -> Result<Completion, AppError> {
    require_hook()?;
    emit_patch(EnvironmentPatch::unset())
}

fn activate(args: ActivateArgs) -> Result<Completion, AppError> {
    require_hook()?;

    let sources = resolve_sources(&args.sources)?;
    let catalog = load_catalog(sources.clone())?;

    let requested_profile = args.profile.or(args.named_profile);
    let (selection, mut delta, store) = match requested_profile {
        Some(requested) => {
            let profile = exact_profile(&catalog, &requested)?;
            (
                profile.name.clone(),
                PreferenceDelta::new(),
                available_preference_store(),
            )
        }
        None => {
            // Invalid profiles remain addressable by exact name so the caller
            // gets PROFILE_CONFIG_INVALID. An interactive picker, however,
            // has nothing useful to show when every discovered profile is
            // invalid; report that deterministically before probing the TTY.
            if !catalog.has_selectable_profiles() {
                return Err(AppError::NoProfiles);
            }
            let store = available_preference_store();
            let history = store
                .as_ref()
                .map_or_else(PreferenceHistory::default, load_preferences);
            match choose_interactively(&catalog, &history)? {
                TuiChoice::Selected { profile, delta } => (profile, delta, store),
                TuiChoice::Cancelled { delta } => {
                    if let Some(store) = store.as_ref() {
                        commit_preferences(store, &delta);
                    }
                    return Ok(Completion::Cancelled);
                }
                TuiChoice::Terminated(signal) => {
                    return Ok(Completion::Terminated(signal_number(signal)));
                }
            }
        }
    };
    let profile = catalog.get(&selection).ok_or(AppError::Internal {
        detail: "chooser returned a profile outside its catalog",
    })?;
    validate_profile_configuration(&catalog, profile)?;

    let conflicts = preflight(&catalog, profile, args.clear_credential_overrides)?;
    let region = args.region.as_deref().or(profile.region.as_deref());
    let patch = EnvironmentPatch::activate(
        &profile.name,
        region,
        propagated_source_path(&sources.config),
        propagated_source_path(&sources.credentials),
        &conflicts,
    )?;
    let frame = encode_patch(&patch)?;

    record_selection(&mut delta, &profile.name);
    if let Some(store) = store.as_ref() {
        commit_preferences(store, &delta);
    }
    emit_bytes(frame.as_bytes())
}

fn require_hook() -> Result<(), AppError> {
    if std::env::var_os("AWSWIT_HOOK").as_deref() == Some(OsStr::new("1")) {
        Ok(())
    } else {
        Err(AppError::HookRequired)
    }
}

fn execute(args: ExecArgs) -> Result<Completion, AppError> {
    let sources = resolve_sources(&args.sources)?;
    let catalog = load_catalog(sources.clone())?;
    let requested = args
        .profile
        .as_deref()
        .or(args.named_profile.as_deref())
        .ok_or(AppError::Internal {
            detail: "exec profile passed CLI validation without a value",
        })?;
    let profile = exact_profile(&catalog, requested)?;
    validate_profile_configuration(&catalog, profile)?;
    let conflicts = preflight(&catalog, profile, args.clear_credential_overrides)?;
    let region = args.region.as_deref().or(profile.region.as_deref());
    let patch = EnvironmentPatch::activate(
        &profile.name,
        region,
        propagated_source_path(&sources.config),
        propagated_source_path(&sources.credentials),
        &conflicts,
    )?;

    if let Some(store) = available_preference_store() {
        let mut delta = PreferenceDelta::new();
        record_selection(&mut delta, &profile.name);
        commit_preferences(&store, &delta);
    }

    let plan = ExecutionPlan::new(args.command, patch)?;
    process::execute(plan).map(Completion::Exit)
}

fn list(args: ListArgs) -> Result<Completion, AppError> {
    let catalog = load_catalog(resolve_sources(&args.sources)?)?;
    let bytes = match args.format {
        ListFormat::Names => newline_separated(catalog.selectable_names()),
        ListFormat::Completion => newline_separated(
            catalog
                .selectable_names()
                .filter(|name| is_unambiguous_terminal_text(name)),
        ),
        ListFormat::Json => {
            let report = ListReport {
                schema_version: 1,
                profiles: catalog.selectable_profiles().collect(),
            };
            output::pretty_json(&report)?
        }
        ListFormat::Human => human_list(&catalog).into_bytes(),
    };
    emit_bytes(&bytes)
}

fn newline_separated<'a>(values: impl Iterator<Item = &'a str>) -> Vec<u8> {
    let mut output = values.collect::<Vec<_>>().join("\n");
    if !output.is_empty() {
        output.push('\n');
    }
    output.into_bytes()
}

fn doctor(args: DoctorArgs) -> Result<Completion, AppError> {
    let sources = resolve_sources(&args.sources)?;
    let catalog = load_catalog(sources.clone())?;
    let credential_environment = CredentialEnvironment::capture();
    let current_profile = std::env::var("AWS_PROFILE")
        .ok()
        .or_else(|| std::env::var("AWS_DEFAULT_PROFILE").ok());
    let hook = std::env::var_os("AWSWIT_HOOK").as_deref() == Some(OsStr::new("1"));
    let report = DoctorReport {
        schema_version: 1,
        offline: true,
        sources: SourceReport::new(&sources),
        profile_count: catalog.len(),
        current_profile,
        hook_detected: hook,
        credential_overrides: credential_environment
            .present()
            .iter()
            .map(|variable| variable.name())
            .collect(),
        issues: catalog.issues().iter().map(IssueReport::new).collect(),
    };

    let bytes = match args.format {
        DoctorFormat::Json => output::pretty_json(&report)?,
        DoctorFormat::Human => report.human()?.into_bytes(),
    };
    emit_bytes(&bytes)
}

fn encode_patch(patch: &EnvironmentPatch) -> Result<String, AppError> {
    let frame = patch.encode()?;
    EnvironmentPatch::decode(&frame).map_err(AppError::from)?;
    Ok(frame)
}

fn emit_patch(patch: EnvironmentPatch) -> Result<Completion, AppError> {
    let frame = encode_patch(&patch)?;
    emit_bytes(frame.as_bytes())
}

fn emit_completions(shell: Shell) -> Result<Completion, AppError> {
    emit_bytes(&completion_artifact(shell))
}

fn completion_artifact(shell: Shell) -> Vec<u8> {
    let mut command = Cli::command();
    let mut bytes = Vec::new();
    let shell = match shell {
        Shell::Bash => clap_complete::Shell::Bash,
        Shell::Zsh => clap_complete::Shell::Zsh,
        Shell::Fish => clap_complete::Shell::Fish,
        Shell::Powershell => clap_complete::Shell::PowerShell,
    };
    clap_complete::generate(shell, &mut command, "awswit", &mut bytes);
    bytes
}

fn emit_init(shell: Shell) -> Result<Completion, AppError> {
    let mut bytes = Vec::new();
    match shell {
        Shell::Bash | Shell::Fish => bytes.extend(completion_artifact(shell)),
        Shell::Zsh => {
            bytes.extend(b"autoload -Uz compinit\n");
            bytes.extend(b"if (( ! $+functions[compdef] )); then compinit; fi\n");
            bytes.extend(b"if (( $+functions[compdef] )); then\n");
            bytes.extend(completion_artifact(shell));
            bytes.extend(b"fi\n");
        }
        // The PowerShell hook owns one combined completer because PowerShell
        // does not expose a prior native completer for safe delegation.
        Shell::Powershell => {}
    }
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        bytes.push(b'\n');
    }
    bytes.extend(init_script(shell).as_bytes());
    emit_bytes(&bytes)
}

fn emit_bytes(bytes: &[u8]) -> Result<Completion, AppError> {
    let _ = output::stdout(bytes)?;
    Ok(Completion::Exit(0))
}

fn init_script(shell: Shell) -> &'static str {
    match shell {
        Shell::Bash => include_str!("init/bash.sh"),
        Shell::Zsh => include_str!("init/zsh.sh"),
        Shell::Fish => include_str!("init/fish.fish"),
        Shell::Powershell => include_str!("init/powershell.ps1"),
    }
}

fn resolve_sources(args: &SourceArgs) -> Result<SourcePaths, AppError> {
    let config = explicit_source(args.config_file.as_ref(), "AWS_CONFIG_FILE");
    let credentials = explicit_source(
        args.credentials_file.as_ref(),
        "AWS_SHARED_CREDENTIALS_FILE",
    );
    let home = if config.is_none() || credentials.is_none() {
        Some(dirs::home_dir().ok_or(AppError::HomeUnavailable)?)
    } else {
        None
    };
    let config = match config {
        Some(source) => source,
        None => SourceFile::new(
            home.as_ref()
                .ok_or(AppError::Internal {
                    detail: "default config source had no home directory",
                })?
                .join(".aws")
                .join("config"),
            PathOrigin::Default,
        ),
    };
    let credentials = match credentials {
        Some(source) => source,
        None => SourceFile::new(
            home.as_ref()
                .ok_or(AppError::Internal {
                    detail: "default credentials source had no home directory",
                })?
                .join(".aws")
                .join("credentials"),
            PathOrigin::Default,
        ),
    };
    Ok(SourcePaths::new(
        absolute_source(config)?,
        absolute_source(credentials)?,
    ))
}

fn explicit_source(cli: Option<&PathBuf>, environment: &str) -> Option<SourceFile> {
    if let Some(path) = cli {
        Some(SourceFile::new(path.clone(), PathOrigin::CommandLine))
    } else {
        std::env::var_os(environment)
            .map(PathBuf::from)
            .map(|path| SourceFile::new(path, PathOrigin::Environment))
    }
}

fn absolute_source(mut source: SourceFile) -> Result<SourceFile, AppError> {
    // Keep the path lexical: canonicalization would require the source to
    // exist, change the reported/propagated identity, and pre-resolve platform
    // symlink/reparse policy. Absolutizing is sufficient to pin the path to the
    // invocation's working directory, including non-Unicode paths.
    source.path = std::path::absolute(&source.path)
        .map_err(|source| AppError::SourcePathResolution { source })?;
    Ok(source)
}

fn propagated_source_path(source: &SourceFile) -> Option<&Path> {
    // An environment-selected relative path is already inherited, but leaving
    // it relative would let a later `cd` in the activated parent shell point
    // AWS at a different file than the catalog actually read.
    source
        .propagates_resolved_path()
        .then_some(source.path.as_path())
}

fn load_catalog(sources: SourcePaths) -> Result<Catalog, AppError> {
    Catalog::load(sources).map_err(|error| AppError::Catalog {
        detail: error.to_string(),
    })
}

fn exact_profile<'a>(catalog: &'a Catalog, requested: &str) -> Result<&'a Profile, AppError> {
    catalog
        .get(requested)
        .ok_or_else(|| AppError::ProfileNotFound {
            requested: requested.to_owned(),
        })
}

fn validate_profile_configuration(catalog: &Catalog, profile: &Profile) -> Result<(), AppError> {
    if catalog.profile_is_activatable(&profile.name) {
        Ok(())
    } else {
        Err(AppError::ProfileConfigurationInvalid {
            profile: profile.name.clone(),
        })
    }
}

fn preflight(
    catalog: &Catalog,
    profile: &Profile,
    clear_credential_overrides: bool,
) -> Result<BTreeSet<crate::activation::CredentialVariable>, AppError> {
    let environment_source = catalog.environment_source(&profile.name);
    if environment_source == EnvironmentSource::InvalidChain {
        return Err(AppError::ProfileConfigurationInvalid {
            profile: profile.name.clone(),
        });
    }
    let assessment = CredentialEnvironment::capture().assess(environment_source);
    if assessment.is_safe() {
        return Ok(BTreeSet::new());
    }
    if clear_credential_overrides {
        return Ok(assessment.conflicts().clone());
    }
    Err(AppError::CredentialOverride {
        profile: profile.name.clone(),
        variables: assessment.conflicts().clone(),
    })
}

fn preference_store() -> Result<PreferenceStore, AppError> {
    let root = dirs::state_dir()
        .or_else(dirs::data_local_dir)
        .ok_or(AppError::HomeUnavailable)?;
    Ok(PreferenceStore::new(
        root.join("awswit").join("history.json"),
    ))
}

fn available_preference_store() -> Option<PreferenceStore> {
    match preference_store() {
        Ok(store) => Some(store),
        Err(error) => {
            output::warning("HISTORY_DISABLED", &error.to_string());
            None
        }
    }
}

fn load_preferences(store: &PreferenceStore) -> PreferenceHistory {
    match store.load() {
        Ok(history) => history,
        Err(error) => {
            output::warning("HISTORY_READ", &error.to_string());
            PreferenceHistory::default()
        }
    }
}

fn record_selection(delta: &mut PreferenceDelta, profile: &str) {
    let milliseconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(0);
    delta.record_selection(profile, milliseconds);
}

fn commit_preferences(store: &PreferenceStore, delta: &PreferenceDelta) {
    if let Err(error) = store.commit(delta) {
        output::warning("HISTORY_WRITE", &error.to_string());
    }
}

fn choose_interactively(
    catalog: &Catalog,
    history: &PreferenceHistory,
) -> Result<TuiChoice, AppError> {
    if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
        return Err(AppError::TtyRequired);
    }
    if !catalog.has_selectable_profiles() {
        return Err(AppError::NoProfiles);
    }

    let current = std::env::var("AWS_PROFILE")
        .ok()
        .or_else(|| std::env::var("AWS_DEFAULT_PROFILE").ok());
    let profiles = catalog
        .selectable_profiles()
        .map(|profile| crate::tui::ViewProfile {
            name: profile.name.clone(),
            region: profile.region.clone(),
            provider: view_provider(profile),
            source: profile.source_profile.clone().or_else(|| {
                profile
                    .credential_source
                    .as_ref()
                    .map(|source| source.as_configured_str().to_owned())
            }),
            account: profile.account_id().map(str::to_owned),
            role_arn: profile.role_arn.clone(),
            role_name: profile
                .sso
                .as_ref()
                .and_then(SsoMetadata::role_name)
                .map(str::to_owned),
            sso_session: match profile.sso.as_ref() {
                Some(SsoMetadata::Modern { session_name, .. }) => Some(session_name.clone()),
                _ => None,
            },
            current: current.as_deref() == Some(profile.name.as_str()),
            favorite: history.is_favorite(&profile.name),
            last_used_sequence: history
                .get(&profile.name)
                .map(|entry| entry.last_used_sequence()),
        })
        .collect();

    let result =
        crate::tui::run_picker(profiles).map_err(|source| AppError::Terminal { source })?;
    let mut delta = PreferenceDelta::new();
    for change in result.favorite_changes {
        if catalog.profile_is_activatable(&change.profile_name) {
            delta.set_favorite(change.profile_name, change.favorite);
        }
    }

    match result.termination {
        crate::tui::PickerTermination::Selected => {
            let profile = result.selection.ok_or(AppError::Internal {
                detail: "selected picker result had no profile",
            })?;
            Ok(TuiChoice::Selected { profile, delta })
        }
        crate::tui::PickerTermination::Cancelled => Ok(TuiChoice::Cancelled { delta }),
        crate::tui::PickerTermination::Signal(signal) => Ok(TuiChoice::Terminated(signal)),
    }
}

enum TuiChoice {
    Selected {
        profile: String,
        delta: PreferenceDelta,
    },
    Cancelled {
        delta: PreferenceDelta,
    },
    Terminated(crate::tui::PickerSignal),
}

fn signal_number(signal: crate::tui::PickerSignal) -> i32 {
    match signal {
        crate::tui::PickerSignal::Interrupt => signal_hook_number::INTERRUPT,
        #[cfg(unix)]
        crate::tui::PickerSignal::Terminate => signal_hook_number::TERMINATE,
        #[cfg(unix)]
        crate::tui::PickerSignal::Hangup => signal_hook_number::HANGUP,
    }
}

#[cfg(unix)]
mod signal_hook_number {
    pub(super) const INTERRUPT: i32 = signal_hook::consts::SIGINT;
    pub(super) const TERMINATE: i32 = signal_hook::consts::SIGTERM;
    pub(super) const HANGUP: i32 = signal_hook::consts::SIGHUP;
}

#[cfg(not(unix))]
mod signal_hook_number {
    pub(super) const INTERRUPT: i32 = 2;
}

fn view_provider(profile: &Profile) -> crate::tui::ViewProvider {
    match profile.sso {
        Some(SsoMetadata::Modern { .. }) => crate::tui::ViewProvider::SsoModern,
        Some(SsoMetadata::Legacy { .. }) => crate::tui::ViewProvider::SsoLegacy,
        None if profile.has_web_identity_token_file => crate::tui::ViewProvider::WebIdentity,
        None if profile.is_role() => crate::tui::ViewProvider::Role,
        None if profile.has_credential_process => crate::tui::ViewProvider::CredentialProcess,
        None if profile.has_login_session => crate::tui::ViewProvider::Login,
        None if profile.credential_source.is_some() => crate::tui::ViewProvider::CredentialSource,
        None if profile.static_credentials.any() => crate::tui::ViewProvider::StaticCredentials,
        None => crate::tui::ViewProvider::Unspecified,
    }
}

fn human_list(catalog: &Catalog) -> String {
    let mut output =
        String::from("PROFILE\tPROVIDER\tREGION\tSOURCE\tACCOUNT (configured metadata)\n");
    for profile in catalog.selectable_profiles() {
        output.push_str(&visible_terminal_text(&profile.name));
        output.push('\t');
        output.push_str(view_provider(profile).label());
        output.push('\t');
        output.push_str(&visible_terminal_text(
            profile.region.as_deref().unwrap_or("-"),
        ));
        output.push('\t');
        output.push_str(&visible_terminal_text(
            profile
                .source_profile
                .as_deref()
                .or(profile
                    .credential_source
                    .as_ref()
                    .map(CredentialSource::as_configured_str))
                .unwrap_or("-"),
        ));
        output.push('\t');
        output.push_str(&visible_terminal_text(profile.account_id().unwrap_or("-")));
        output.push('\n');
    }
    output
}

#[derive(Serialize)]
struct DoctorReport<'a> {
    schema_version: u8,
    offline: bool,
    sources: SourceReport,
    profile_count: usize,
    current_profile: Option<String>,
    hook_detected: bool,
    credential_overrides: Vec<&'static str>,
    issues: Vec<IssueReport<'a>>,
}

#[derive(Serialize)]
struct ListReport<'a> {
    schema_version: u8,
    profiles: Vec<&'a Profile>,
}

#[derive(Serialize)]
struct SourceReport {
    config: SourceFileReport,
    credentials: SourceFileReport,
}

impl SourceReport {
    fn new(sources: &SourcePaths) -> Self {
        Self {
            config: SourceFileReport::new(&sources.config),
            credentials: SourceFileReport::new(&sources.credentials),
        }
    }
}

#[derive(Serialize)]
struct SourceFileReport {
    path: String,
    path_is_unicode: bool,
    origin: PathOrigin,
}

impl SourceFileReport {
    fn new(source: &SourceFile) -> Self {
        Self {
            path: source.path.to_string_lossy().into_owned(),
            path_is_unicode: source.path.to_str().is_some(),
            origin: source.origin,
        }
    }
}

#[derive(Serialize)]
struct IssueReport<'a> {
    source: CatalogFileKind,
    path: String,
    path_is_unicode: bool,
    line: Option<usize>,
    kind: &'a CatalogIssueKind,
}

impl<'a> IssueReport<'a> {
    fn new(issue: &'a crate::catalog::CatalogIssue) -> Self {
        Self {
            source: issue.source,
            path: issue.path.to_string_lossy().into_owned(),
            path_is_unicode: issue.path.to_str().is_some(),
            line: issue.line,
            kind: &issue.kind,
        }
    }
}

impl DoctorReport<'_> {
    fn human(&self) -> Result<String, AppError> {
        let mut output = String::new();
        output.push_str("awswit doctor (offline; identity not verified)\n");
        output.push_str(&format!("profiles: {}\n", self.profile_count));
        output.push_str(&format!(
            "hook: {}\n",
            if self.hook_detected {
                "detected"
            } else {
                "not detected"
            }
        ));
        output.push_str(&format!(
            "current profile: {}\n",
            self.current_profile
                .as_deref()
                .map_or_else(|| "not set".to_owned(), safe_human_text)
        ));
        output.push_str(&format!(
            "config: {} ({:?})\n",
            safe_human_text(&self.sources.config.path),
            self.sources.config.origin
        ));
        output.push_str(&format!(
            "credentials: {} ({:?})\n",
            safe_human_text(&self.sources.credentials.path),
            self.sources.credentials.origin
        ));
        output.push_str(&format!(
            "credential overrides: {}\n",
            if self.credential_overrides.is_empty() {
                "none".to_owned()
            } else {
                self.credential_overrides.join(", ")
            }
        ));
        output.push_str(&format!("catalog issues: {}\n", self.issues.len()));
        for issue in &self.issues {
            let kind = serde_json::to_string(issue.kind).map_err(|error| AppError::Output {
                source: std::io::Error::other(error),
            })?;
            output.push_str("  - ");
            output.push_str(match issue.source {
                CatalogFileKind::Config => "config ",
                CatalogFileKind::Credentials => "credentials ",
            });
            output.push_str(&safe_human_text(&issue.path));
            if let Some(line) = issue.line {
                output.push(':');
                output.push_str(&line.to_string());
            }
            if !issue.path_is_unicode {
                output.push_str(" [path rendered lossily]");
            }
            output.push_str(": ");
            output.push_str(&safe_human_text(&kind));
            output.push('\n');
        }
        Ok(output)
    }
}

fn safe_human_text(value: &str) -> String {
    visible_terminal_text(value)
}
