//! Deterministic, credential-blind discovery of AWS shared-config profiles.
//!
//! The catalog owns the unpleasant parts of AWS INI discovery: partial-file
//! recovery, merging config and credentials sections, SSO-session joins, and
//! source-profile graph validation.  Callers only receive immutable profile
//! metadata and sanitized diagnostics.  Credential values and process command
//! text never cross this module's parser boundary.

mod parser;

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};

use serde::{Serialize, Serializer};
use thiserror::Error;

use parser::{ParsedConfig, ParsedCredentials, RawProfile, parse_config, parse_credentials};

/// How a shared-config path was selected before catalog loading began.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PathOrigin {
    Default,
    Environment,
    CommandLine,
}

impl PathOrigin {
    fn is_explicit(self) -> bool {
        !matches!(self, Self::Default)
    }
}

/// One already-resolved shared-config source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct SourceFile {
    pub(crate) path: PathBuf,
    pub(crate) origin: PathOrigin,
    /// Whether activation/exec must replace the inherited path value.
    /// Kept out of reports: `origin` remains the public diagnostic contract.
    #[serde(skip)]
    propagate_resolved_path: bool,
}

impl SourceFile {
    pub(crate) fn new(path: impl Into<PathBuf>, origin: PathOrigin) -> Self {
        let path = path.into();
        let propagate_resolved_path = match origin {
            PathOrigin::CommandLine => true,
            PathOrigin::Environment => !path.is_absolute(),
            PathOrigin::Default => false,
        };
        Self {
            path,
            origin,
            propagate_resolved_path,
        }
    }

    pub(crate) const fn propagates_resolved_path(&self) -> bool {
        self.propagate_resolved_path
    }
}

/// Config and credentials paths after `CLI > environment > default` resolution.
///
/// Path resolution deliberately lives outside this module.  That keeps a
/// catalog load reproducible and prevents it from observing a changing process
/// environment midway through a workflow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct SourcePaths {
    pub(crate) config: SourceFile,
    pub(crate) credentials: SourceFile,
}

impl SourcePaths {
    pub(crate) fn new(config: SourceFile, credentials: SourceFile) -> Self {
        Self {
            config,
            credentials,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CatalogFileKind {
    Config,
    Credentials,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MetadataField {
    Region,
    RoleArn,
    SourceProfile,
    CredentialSource,
    MfaSerial,
    CredentialProcess,
    LoginSession,
    WebIdentityTokenFile,
    AccessKeyId,
    SecretAccessKey,
    SessionToken,
    SsoSession,
    SsoStartUrl,
    SsoRegion,
    SsoRegistrationScopes,
    SsoAccountId,
    SsoRoleName,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SsoMode {
    Modern,
    Legacy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RequiredSsoField {
    Session,
    StartUrl,
    Region,
    AccountId,
    RoleName,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProviderGraphViolation {
    ConflictingCredentialProviders,
    MultipleRoleCredentialSources,
    RoleMissingCredentialSource,
    SourceProfileWithoutRoleArn,
    CredentialSourceWithoutRoleArn,
    WebIdentityWithoutRoleArn,
    SourceProfileNotCredentialCapable,
    EnvironmentCredentialSourceCannotBeSelectedSafely,
    UnknownCredentialSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StaticCredentialField {
    AccessKeyId,
    SecretAccessKey,
}

/// A diagnostic safe to render or serialize.
///
/// Variants contain only validated profile/session names and fixed enums.  Raw
/// INI text and values are intentionally absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub(crate) enum CatalogIssueKind {
    InvalidEncoding,
    LineTooLong,
    MalformedSection,
    MalformedProperty,
    PropertyOutsideSection,
    UnsafeProfileName,
    UnsafeSsoSessionName,
    InvalidMetadata {
        field: MetadataField,
    },
    DuplicateSection {
        name: String,
    },
    DuplicateMetadata {
        field: MetadataField,
    },
    MissingSourceProfile {
        profile: String,
        source_profile: String,
    },
    SourceProfileCycle {
        profiles: Vec<String>,
    },
    MissingSsoSession {
        profile: String,
        session: String,
    },
    IncompleteSso {
        profile: String,
        mode: SsoMode,
        missing: Vec<RequiredSsoField>,
    },
    InvalidProviderGraph {
        profile: String,
        violation: ProviderGraphViolation,
    },
    IncompleteStaticCredentials {
        profile: String,
        missing: Vec<StaticCredentialField>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct CatalogIssue {
    pub(crate) source: CatalogFileKind,
    pub(crate) path: PathBuf,
    pub(crate) line: Option<usize>,
    pub(crate) kind: CatalogIssueKind,
}

impl CatalogIssue {
    fn graph(path: &Path, line: Option<usize>, kind: CatalogIssueKind) -> Self {
        Self {
            source: CatalogFileKind::Config,
            path: path.to_path_buf(),
            line,
            kind,
        }
    }
}

/// Presence flags only; key material is never represented.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub(crate) struct StaticCredentialKeys {
    pub(crate) access_key_id: bool,
    pub(crate) secret_access_key: bool,
    pub(crate) session_token: bool,
}

impl StaticCredentialKeys {
    pub(crate) fn any(self) -> bool {
        self.access_key_id || self.secret_access_key || self.session_token
    }

    fn merge(&mut self, other: Self) {
        self.access_key_id |= other.access_key_id;
        self.secret_access_key |= other.secret_access_key;
        self.session_token |= other.session_token;
    }
}

/// SSO provider hints resolved from either modern or legacy configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub(crate) enum SsoMetadata {
    Modern {
        session_name: String,
        start_url: Option<String>,
        sso_region: Option<String>,
        registration_scopes: Option<String>,
        account_id: Option<String>,
        role_name: Option<String>,
    },
    Legacy {
        start_url: Option<String>,
        sso_region: Option<String>,
        account_id: Option<String>,
        role_name: Option<String>,
    },
}

impl SsoMetadata {
    pub(crate) fn account_id(&self) -> Option<&str> {
        match self {
            Self::Modern { account_id, .. } | Self::Legacy { account_id, .. } => {
                account_id.as_deref()
            }
        }
    }

    pub(crate) fn role_name(&self) -> Option<&str> {
        match self {
            Self::Modern { role_name, .. } | Self::Legacy { role_name, .. } => role_name.as_deref(),
        }
    }
}

/// A configured AWS `credential_source` value.
///
/// Known AWS values are typed so provider validation and environment safety do
/// not depend on string comparisons. A bounded, display-safe future value is
/// retained explicitly as `Unknown` so diagnostics remain forward-compatible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CredentialSource {
    Environment,
    EcsContainer,
    Ec2InstanceMetadata,
    Unknown(String),
}

impl CredentialSource {
    fn from_configured(value: String) -> Self {
        match value.as_str() {
            "Environment" => Self::Environment,
            "EcsContainer" => Self::EcsContainer,
            "Ec2InstanceMetadata" => Self::Ec2InstanceMetadata,
            _ => Self::Unknown(value),
        }
    }

    pub(crate) fn as_configured_str(&self) -> &str {
        match self {
            Self::Environment => "Environment",
            Self::EcsContainer => "EcsContainer",
            Self::Ec2InstanceMetadata => "Ec2InstanceMetadata",
            Self::Unknown(value) => value,
        }
    }

    fn environment_source(&self) -> EnvironmentSource {
        match self {
            Self::Environment => EnvironmentSource::UsesEnvironment,
            Self::EcsContainer => EnvironmentSource::UsesEcsContainer,
            Self::Ec2InstanceMetadata => EnvironmentSource::UsesEc2InstanceMetadata,
            Self::Unknown(_) => EnvironmentSource::InvalidChain,
        }
    }

    fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown(_))
    }
}

impl Serialize for CredentialSource {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_configured_str())
    }
}

/// Discovered AWS profile metadata. Selection additionally requires the
/// catalog-wide provider-chain proof performed by `Catalog`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Profile {
    pub(crate) name: String,
    pub(crate) region: Option<String>,
    pub(crate) role_arn: Option<String>,
    pub(crate) source_profile: Option<String>,
    pub(crate) credential_source: Option<CredentialSource>,
    pub(crate) has_mfa_serial: bool,
    pub(crate) has_credential_process: bool,
    pub(crate) has_login_session: bool,
    pub(crate) has_web_identity_token_file: bool,
    pub(crate) static_credentials: StaticCredentialKeys,
    pub(crate) has_config_section: bool,
    pub(crate) has_credentials_section: bool,
    pub(crate) sso: Option<SsoMetadata>,
    /// Local parser/provider completeness only. Source-chain validity is
    /// evaluated by `Catalog::profile_is_activatable`.
    #[serde(skip)]
    intrinsically_activatable: bool,
}

impl Profile {
    pub(crate) fn is_role(&self) -> bool {
        self.role_arn.is_some()
    }

    pub(crate) fn account_id(&self) -> Option<&str> {
        self.sso
            .as_ref()
            .and_then(SsoMetadata::account_id)
            .or_else(|| self.role_arn.as_deref().and_then(account_from_arn))
    }
}

fn account_from_arn(arn: &str) -> Option<&str> {
    let mut components = arn.splitn(6, ':');
    let prefix = components.next()?;
    let _partition = components.next()?;
    let service = components.next()?;
    let _region = components.next()?;
    let account = components.next()?;
    let _resource = components.next()?;
    (prefix == "arn"
        && service == "iam"
        && account.len() == 12
        && account.bytes().all(|byte| byte.is_ascii_digit()))
    .then_some(account)
}

/// Which ambient provider input a complete, acyclic source chain explicitly
/// requests, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EnvironmentSource {
    UsesEnvironment,
    UsesEcsContainer,
    UsesEc2InstanceMetadata,
    UsesLoginSession,
    DoesNotUseEnvironment,
    InvalidChain,
}

/// Immutable profile catalog with deterministic name ordering.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Catalog {
    sources: SourcePaths,
    profiles: BTreeMap<String, Profile>,
    issues: Vec<CatalogIssue>,
    #[serde(skip)]
    resolutions: BTreeMap<String, ProfileResolution>,
}

#[derive(Debug, Clone, Copy)]
struct ProfileResolution {
    activatable: bool,
    environment_source: EnvironmentSource,
    usable_as_source: bool,
}

impl Catalog {
    /// Load both shared-config files and build a validated catalog.
    ///
    /// A missing OS-default path is an empty source.  A path supplied by the
    /// environment or command line is intentional, so absence is a hard error.
    /// All other open/read failures are hard errors regardless of origin.
    pub(crate) fn load(sources: SourcePaths) -> Result<Self, CatalogError> {
        let mut config = load_config(&sources.config)?;
        let mut credentials = load_credentials(&sources.credentials)?;

        let mut issues = std::mem::take(&mut config.issues);
        issues.append(&mut credentials.issues);
        if issues.len() > parser::MAX_ISSUES {
            return Err(catalog_limit_error(&sources));
        }

        let mut names: BTreeSet<String> = config.profiles.keys().cloned().collect();
        names.extend(credentials.profiles.keys().cloned());
        if names.len().saturating_add(config.sso_sessions.len()) > parser::MAX_CATALOG_ENTRIES {
            return Err(catalog_limit_error(&sources));
        }

        let mut profiles = BTreeMap::new();
        for name in names {
            let raw = config.profiles.get(&name).cloned().unwrap_or_else(|| {
                let mut profile = RawProfile::new(name.clone());
                profile.has_config_section = false;
                profile
            });
            let credential_presence = credentials.profiles.get(&name);
            profiles.insert(
                name.clone(),
                materialize_profile(raw, credential_presence, &config, &sources, &mut issues),
            );
            if issues.len() > parser::MAX_ISSUES {
                return Err(catalog_limit_error(&sources));
            }
        }

        validate_source_graph(&profiles, &config, &sources.config.path, &mut issues);
        if issues.len() > parser::MAX_ISSUES
            || profiles.len().saturating_add(config.sso_sessions.len())
                > parser::MAX_CATALOG_ENTRIES
        {
            return Err(catalog_limit_error(&sources));
        }
        let resolutions = resolve_profiles(&profiles);
        append_source_capability_issues(
            &profiles,
            &resolutions,
            &config,
            &sources.config.path,
            &mut issues,
        );
        if issues.len() > parser::MAX_ISSUES {
            return Err(catalog_limit_error(&sources));
        }

        Ok(Self {
            sources,
            profiles,
            issues,
            resolutions,
        })
    }

    pub(crate) fn selectable_profiles(&self) -> impl Iterator<Item = &Profile> {
        self.profiles
            .values()
            .filter(|profile| self.profile_is_activatable(&profile.name))
    }

    pub(crate) fn selectable_names(&self) -> impl Iterator<Item = &str> {
        self.selectable_profiles()
            .map(|profile| profile.name.as_str())
    }

    pub(crate) fn has_selectable_profiles(&self) -> bool {
        self.selectable_profiles().next().is_some()
    }

    /// Exact, case-sensitive lookup.  Fuzzy resolution is intentionally absent.
    pub(crate) fn get(&self, name: &str) -> Option<&Profile> {
        self.profiles.get(name)
    }

    pub(crate) fn len(&self) -> usize {
        self.profiles.len()
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.profiles.is_empty()
    }

    pub(crate) fn issues(&self) -> &[CatalogIssue] {
        &self.issues
    }

    /// Prove that an exact profile and its entire `source_profile` chain are
    /// safe to activate from the catalog snapshot.
    ///
    /// This is deliberately fail-closed. A missing name, cycle, parser
    /// ambiguity, incomplete credential tuple, incomplete SSO configuration,
    /// or invalid provider graph at any visited node returns `false`.
    /// A `true` result proves catalog consistency only; it does not contact AWS
    /// or claim that credentials are present, current, or authorized.
    pub(crate) fn profile_is_activatable(&self, name: &str) -> bool {
        self.resolutions
            .get(name)
            .is_some_and(|resolution| resolution.activatable)
    }

    /// Return the provider proof for the selected profile's source chain.
    ///
    /// `InvalidChain` is fail-closed: callers must not exempt ambient static
    /// credentials when a source is missing or cyclic, even if one visited node
    /// happens to mention `Environment`.
    pub(crate) fn environment_source(&self, name: &str) -> EnvironmentSource {
        self.resolutions
            .get(name)
            .map_or(EnvironmentSource::InvalidChain, |resolution| {
                resolution.environment_source
            })
    }
}

fn catalog_limit_error(sources: &SourcePaths) -> CatalogError {
    CatalogError::Read {
        source_kind: CatalogFileKind::Config,
        path: sources.config.path.clone(),
        error: io::Error::new(
            io::ErrorKind::InvalidData,
            "AWS shared configuration exceeds a bounded catalog limit",
        ),
    }
}

fn resolve_profiles(profiles: &BTreeMap<String, Profile>) -> BTreeMap<String, ProfileResolution> {
    let mut resolved = BTreeMap::<String, ProfileResolution>::new();
    for start in profiles.keys() {
        if resolved.contains_key(start) {
            continue;
        }

        let mut path = Vec::new();
        let mut local = BTreeSet::new();
        let mut current = start.as_str();
        let mut downstream = loop {
            if let Some(resolution) = resolved.get(current).copied() {
                break resolution;
            }
            let Some(profile) = profiles.get(current) else {
                break invalid_resolution();
            };
            if !local.insert(current.to_owned()) {
                break invalid_resolution();
            }
            if !profile.intrinsically_activatable || provider_graph_violation(profile).is_some() {
                let invalid = invalid_resolution();
                resolved.insert(current.to_owned(), invalid);
                break invalid;
            }
            match profile.source_profile.as_deref() {
                Some(source)
                    if source == current && is_complete_static_self_source_role(profile) =>
                {
                    let top_level_only = ProfileResolution {
                        activatable: true,
                        environment_source: EnvironmentSource::DoesNotUseEnvironment,
                        usable_as_source: false,
                    };
                    resolved.insert(current.to_owned(), top_level_only);
                    break top_level_only;
                }
                Some(source) => {
                    path.push(current.to_owned());
                    current = source;
                }
                None => {
                    let terminal = terminal_resolution(profile);
                    resolved.insert(current.to_owned(), terminal);
                    break terminal;
                }
            }
        };
        for name in path.into_iter().rev() {
            let resolution = if downstream.activatable && downstream.usable_as_source {
                ProfileResolution {
                    activatable: true,
                    environment_source: downstream.environment_source,
                    usable_as_source: true,
                }
            } else {
                invalid_resolution()
            };
            resolved.insert(name, resolution);
            downstream = resolution;
        }
    }
    resolved
}

fn terminal_resolution(profile: &Profile) -> ProfileResolution {
    let environment_source = if profile.has_login_session {
        EnvironmentSource::UsesLoginSession
    } else {
        profile.credential_source.as_ref().map_or(
            EnvironmentSource::DoesNotUseEnvironment,
            CredentialSource::environment_source,
        )
    };
    ProfileResolution {
        activatable: true,
        environment_source,
        usable_as_source: has_signing_credential_provider(profile),
    }
}

fn has_signing_credential_provider(profile: &Profile) -> bool {
    profile.role_arn.is_some()
        || (profile.static_credentials.access_key_id
            && profile.static_credentials.secret_access_key)
        || profile.has_credential_process
        || profile.has_login_session
        || profile
            .sso
            .as_ref()
            .is_some_and(|sso| sso.account_id().is_some() && sso.role_name().is_some())
}

const fn invalid_resolution() -> ProfileResolution {
    ProfileResolution {
        activatable: false,
        environment_source: EnvironmentSource::InvalidChain,
        usable_as_source: false,
    }
}

fn load_config(source: &SourceFile) -> Result<ParsedConfig, CatalogError> {
    let Some(file) = open_source(source, CatalogFileKind::Config)? else {
        return Ok(ParsedConfig::default());
    };
    parse_config(BufReader::new(file), &source.path).map_err(|error| CatalogError::Read {
        source_kind: CatalogFileKind::Config,
        path: source.path.clone(),
        error,
    })
}

fn load_credentials(source: &SourceFile) -> Result<ParsedCredentials, CatalogError> {
    let Some(file) = open_source(source, CatalogFileKind::Credentials)? else {
        return Ok(ParsedCredentials::default());
    };
    parse_credentials(BufReader::new(file), &source.path).map_err(|error| CatalogError::Read {
        source_kind: CatalogFileKind::Credentials,
        path: source.path.clone(),
        error,
    })
}

fn open_source(source: &SourceFile, kind: CatalogFileKind) -> Result<Option<File>, CatalogError> {
    #[cfg(windows)]
    if path_uses_windows_device_namespace(&source.path) {
        return Err(CatalogError::Open {
            source_kind: kind,
            path: source.path.clone(),
            error: io::Error::new(
                io::ErrorKind::InvalidInput,
                "AWS shared configuration source uses a Windows device namespace",
            ),
        });
    }

    let mut options = OpenOptions::new();
    options.read(true);
    set_nonblocking_read_flags(&mut options);
    match options.open(&source.path) {
        Ok(file) => {
            let metadata = file.metadata().map_err(|error| CatalogError::Open {
                source_kind: kind,
                path: source.path.clone(),
                error,
            })?;
            if !metadata_is_regular(&metadata) {
                return Err(CatalogError::Open {
                    source_kind: kind,
                    path: source.path.clone(),
                    error: io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "AWS shared configuration source is not a regular file",
                    ),
                });
            }
            Ok(Some(file))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound && !source.origin.is_explicit() => {
            Ok(None)
        }
        Err(error) => Err(CatalogError::Open {
            source_kind: kind,
            path: source.path.clone(),
            error,
        }),
    }
}

#[cfg(unix)]
fn set_nonblocking_read_flags(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;

    // Follow the final symlink exactly as AWS clients do. The opened handle is
    // accepted only after `fstat` proves its target is a regular file. Keeping
    // O_NONBLOCK makes a symlink redirected to a FIFO/device fail promptly at
    // that post-open type check instead of stalling catalog discovery.
    options.custom_flags(nix::libc::O_NONBLOCK | nix::libc::O_CLOEXEC);
}

#[cfg(windows)]
fn set_nonblocking_read_flags(options: &mut OpenOptions) {
    use std::os::windows::fs::OpenOptionsExt;

    // Windows reparse points can target blocking/custom providers, and this
    // reader has no Unix-like O_NONBLOCK guarantee before the post-open type
    // check. Inspect the reparse point itself and fail closed for now.
    options.custom_flags(WINDOWS_OPEN_REPARSE_POINT);
}

#[cfg(not(any(unix, windows)))]
fn set_nonblocking_read_flags(_options: &mut OpenOptions) {}

#[cfg(windows)]
const WINDOWS_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

#[cfg(windows)]
fn path_uses_windows_device_namespace(path: &Path) -> bool {
    use std::path::{Component, Prefix};

    path.components().next().is_some_and(|component| {
        matches!(
            component,
            Component::Prefix(prefix)
                if matches!(prefix.kind(), Prefix::DeviceNS(_) | Prefix::Verbatim(_))
        )
    })
}

#[cfg(windows)]
fn metadata_is_regular(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    metadata.is_file() && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
}

#[cfg(not(windows))]
fn metadata_is_regular(metadata: &std::fs::Metadata) -> bool {
    metadata.is_file()
}

fn materialize_profile(
    mut raw: RawProfile,
    credentials: Option<&parser::CredentialPresence>,
    config: &ParsedConfig,
    sources: &SourcePaths,
    issues: &mut Vec<CatalogIssue>,
) -> Profile {
    let mut intrinsically_activatable = !raw.activation_ambiguous
        && credentials.is_none_or(|credentials| !credentials.activation_ambiguous);
    let config_static_credentials = raw.static_credentials;
    let credentials_static_credentials = credentials
        .map(|credentials| credentials.static_credentials)
        .unwrap_or_default();
    let mut static_credentials = config_static_credentials;
    let has_credential_process = raw.has_credential_process;
    let has_credentials_section = credentials.is_some();
    static_credentials.merge(credentials_static_credentials);

    // AWS consumers do not uniformly repair a partial static tuple with keys
    // from the other shared file. Validate each provider source independently
    // instead of manufacturing a complete provider from their union.
    let incomplete_config = append_incomplete_static_issue(
        &raw.name,
        config_static_credentials,
        CatalogFileKind::Config,
        &sources.config.path,
        issues,
    );
    let incomplete_credentials = append_incomplete_static_issue(
        &raw.name,
        credentials_static_credentials,
        CatalogFileKind::Credentials,
        &sources.credentials.path,
        issues,
    );
    if incomplete_config || incomplete_credentials {
        intrinsically_activatable = false;
    }

    let sso = if let Some(session_name) = raw.sso_session.take() {
        if raw.sso_start_url.is_some() || raw.sso_region.is_some() {
            intrinsically_activatable = false;
            issues.push(CatalogIssue::graph(
                &sources.config.path,
                raw.sso_session_line,
                CatalogIssueKind::InvalidProviderGraph {
                    profile: raw.name.clone(),
                    violation: ProviderGraphViolation::ConflictingCredentialProviders,
                },
            ));
        }
        let session = config.sso_sessions.get(&session_name);
        if session.is_none_or(|session| session.activation_ambiguous) {
            intrinsically_activatable = false;
        }
        if session.is_none() {
            issues.push(CatalogIssue::graph(
                &sources.config.path,
                raw.sso_session_line,
                CatalogIssueKind::MissingSsoSession {
                    profile: raw.name.clone(),
                    session: session_name.clone(),
                },
            ));
        }

        let start_url = session.and_then(|session| session.start_url.clone());
        let sso_region = session.and_then(|session| session.sso_region.clone());
        let registration_scopes = session.and_then(|session| session.registration_scopes.clone());
        let mut missing = Vec::new();
        if session.is_none() {
            missing.push(RequiredSsoField::Session);
        }
        if start_url.is_none() {
            missing.push(RequiredSsoField::StartUrl);
        }
        if sso_region.is_none() {
            missing.push(RequiredSsoField::Region);
        }
        match (raw.sso_account_id.is_some(), raw.sso_role_name.is_some()) {
            (true, false) => missing.push(RequiredSsoField::RoleName),
            (false, true) => missing.push(RequiredSsoField::AccountId),
            (false, false) | (true, true) => {}
        }
        if !missing.is_empty() {
            intrinsically_activatable = false;
            issues.push(CatalogIssue::graph(
                &sources.config.path,
                raw.sso_session_line,
                CatalogIssueKind::IncompleteSso {
                    profile: raw.name.clone(),
                    mode: SsoMode::Modern,
                    missing,
                },
            ));
        }

        Some(SsoMetadata::Modern {
            session_name,
            start_url,
            sso_region,
            registration_scopes,
            account_id: raw.sso_account_id.take(),
            role_name: raw.sso_role_name.take(),
        })
    } else {
        let has_legacy_sso = raw.sso_start_url.is_some()
            || raw.sso_region.is_some()
            || raw.sso_account_id.is_some()
            || raw.sso_role_name.is_some();
        has_legacy_sso.then(|| {
            let fields = [
                (raw.sso_start_url.is_none(), RequiredSsoField::StartUrl),
                (raw.sso_region.is_none(), RequiredSsoField::Region),
                (raw.sso_account_id.is_none(), RequiredSsoField::AccountId),
                (raw.sso_role_name.is_none(), RequiredSsoField::RoleName),
            ];
            let missing: Vec<_> = fields
                .into_iter()
                .filter_map(|(is_missing, field)| is_missing.then_some(field))
                .collect();
            if !missing.is_empty() {
                intrinsically_activatable = false;
                issues.push(CatalogIssue::graph(
                    &sources.config.path,
                    raw.first_config_line,
                    CatalogIssueKind::IncompleteSso {
                        profile: raw.name.clone(),
                        mode: SsoMode::Legacy,
                        missing,
                    },
                ));
            }
            SsoMetadata::Legacy {
                start_url: raw.sso_start_url.take(),
                sso_region: raw.sso_region.take(),
                account_id: raw.sso_account_id.take(),
                role_name: raw.sso_role_name.take(),
            }
        })
    };

    Profile {
        name: raw.name,
        region: raw.region,
        role_arn: raw.role_arn,
        source_profile: raw.source_profile,
        credential_source: raw.credential_source.map(CredentialSource::from_configured),
        has_mfa_serial: raw.has_mfa_serial,
        has_credential_process,
        has_login_session: raw.has_login_session,
        has_web_identity_token_file: raw.has_web_identity_token_file,
        static_credentials,
        has_config_section: raw.has_config_section,
        has_credentials_section,
        sso,
        intrinsically_activatable,
    }
}

fn validate_source_graph(
    profiles: &BTreeMap<String, Profile>,
    config: &ParsedConfig,
    config_path: &Path,
    issues: &mut Vec<CatalogIssue>,
) {
    for profile in profiles.values() {
        if let Some(violation) = provider_graph_violation(profile) {
            let line = config
                .profiles
                .get(&profile.name)
                .and_then(|raw| raw.credential_source_line);
            issues.push(CatalogIssue::graph(
                config_path,
                line,
                CatalogIssueKind::InvalidProviderGraph {
                    profile: profile.name.clone(),
                    violation,
                },
            ));
            if issues.len() > parser::MAX_ISSUES {
                return;
            }
        }

        let Some(source_profile) = profile.source_profile.as_ref() else {
            continue;
        };
        if !profiles.contains_key(source_profile) {
            let line = config
                .profiles
                .get(&profile.name)
                .and_then(|raw| raw.source_profile_line);
            issues.push(CatalogIssue::graph(
                config_path,
                line,
                CatalogIssueKind::MissingSourceProfile {
                    profile: profile.name.clone(),
                    source_profile: source_profile.clone(),
                },
            ));
            if issues.len() > parser::MAX_ISSUES {
                return;
            }
        }
    }

    let mut globally_seen = BTreeSet::new();
    let mut cycles = BTreeSet::new();
    for start in profiles.keys() {
        if globally_seen.contains(start) {
            continue;
        }

        let mut path = Vec::new();
        let mut local_positions = BTreeMap::new();
        let mut current = start.as_str();
        loop {
            if let Some(&cycle_start) = local_positions.get(current) {
                cycles.insert(canonical_cycle(&path[cycle_start..]));
                break;
            }
            if globally_seen.contains(current) {
                break;
            }

            local_positions.insert(current.to_owned(), path.len());
            path.push(current.to_owned());
            let Some(profile) = profiles.get(current) else {
                break;
            };
            let Some(next) = profile.source_profile.as_deref() else {
                break;
            };
            // Botocore permits a role to name itself as source_profile when
            // that same profile has a complete local static credential tuple.
            // It is a terminal provider, not a graph cycle.
            if next == current && is_complete_static_self_source_role(profile) {
                break;
            }
            if !profiles.contains_key(next) {
                break;
            }
            current = next;
        }
        globally_seen.extend(path);
    }

    for profiles in cycles {
        issues.push(CatalogIssue::graph(
            config_path,
            None,
            CatalogIssueKind::SourceProfileCycle { profiles },
        ));
        if issues.len() > parser::MAX_ISSUES {
            return;
        }
    }
}

fn append_source_capability_issues(
    profiles: &BTreeMap<String, Profile>,
    resolutions: &BTreeMap<String, ProfileResolution>,
    config: &ParsedConfig,
    config_path: &Path,
    issues: &mut Vec<CatalogIssue>,
) {
    for profile in profiles.values() {
        let Some(source_profile) = profile.source_profile.as_deref() else {
            continue;
        };
        if source_profile == profile.name {
            continue;
        }
        let Some(source_resolution) = resolutions.get(source_profile) else {
            continue;
        };
        if !source_resolution.activatable || source_resolution.usable_as_source {
            continue;
        }
        let line = config
            .profiles
            .get(&profile.name)
            .and_then(|raw| raw.source_profile_line);
        issues.push(CatalogIssue::graph(
            config_path,
            line,
            CatalogIssueKind::InvalidProviderGraph {
                profile: profile.name.clone(),
                violation: ProviderGraphViolation::SourceProfileNotCredentialCapable,
            },
        ));
        if issues.len() > parser::MAX_ISSUES {
            return;
        }
    }
}

fn provider_graph_violation(profile: &Profile) -> Option<ProviderGraphViolation> {
    if is_complete_static_self_source_role(profile) {
        return None;
    }
    let source_count = usize::from(profile.source_profile.is_some())
        + usize::from(profile.credential_source.is_some())
        + usize::from(profile.has_web_identity_token_file);
    if profile.credential_source.is_some() && profile.role_arn.is_none() {
        return Some(ProviderGraphViolation::CredentialSourceWithoutRoleArn);
    }
    if profile
        .credential_source
        .as_ref()
        .is_some_and(CredentialSource::is_unknown)
    {
        return Some(ProviderGraphViolation::UnknownCredentialSource);
    }
    if profile.role_arn.is_some() && source_count == 0 {
        return Some(ProviderGraphViolation::RoleMissingCredentialSource);
    }
    if source_count > 1 {
        return Some(ProviderGraphViolation::MultipleRoleCredentialSources);
    }
    if profile.source_profile.is_some() && profile.role_arn.is_none() {
        return Some(ProviderGraphViolation::SourceProfileWithoutRoleArn);
    }
    if profile.has_web_identity_token_file && profile.role_arn.is_none() {
        return Some(ProviderGraphViolation::WebIdentityWithoutRoleArn);
    }
    let top_level_provider_count = usize::from(profile.role_arn.is_some())
        + usize::from(profile.sso.is_some())
        + usize::from(profile.has_credential_process)
        + usize::from(profile.has_login_session)
        + usize::from(profile.static_credentials.any());
    if top_level_provider_count > 1 {
        return Some(ProviderGraphViolation::ConflictingCredentialProviders);
    }
    if profile.role_arn.is_some()
        && profile.credential_source == Some(CredentialSource::Environment)
    {
        return Some(ProviderGraphViolation::EnvironmentCredentialSourceCannotBeSelectedSafely);
    }
    None
}

fn is_complete_static_self_source_role(profile: &Profile) -> bool {
    profile.role_arn.is_some()
        && profile.source_profile.as_deref() == Some(profile.name.as_str())
        && profile.credential_source.is_none()
        && !profile.has_web_identity_token_file
        && profile.sso.is_none()
        && !profile.has_credential_process
        && !profile.has_login_session
        && profile.static_credentials.access_key_id
        && profile.static_credentials.secret_access_key
}

fn missing_static_fields(credentials: StaticCredentialKeys) -> Vec<StaticCredentialField> {
    if !credentials.any() || (credentials.access_key_id && credentials.secret_access_key) {
        return Vec::new();
    }

    let mut missing = Vec::new();
    if !credentials.access_key_id {
        missing.push(StaticCredentialField::AccessKeyId);
    }
    if !credentials.secret_access_key {
        missing.push(StaticCredentialField::SecretAccessKey);
    }
    missing
}

fn append_incomplete_static_issue(
    profile: &str,
    credentials: StaticCredentialKeys,
    source: CatalogFileKind,
    path: &Path,
    issues: &mut Vec<CatalogIssue>,
) -> bool {
    let missing = missing_static_fields(credentials);
    if missing.is_empty() {
        return false;
    }
    issues.push(CatalogIssue {
        source,
        path: path.to_path_buf(),
        line: None,
        kind: CatalogIssueKind::IncompleteStaticCredentials {
            profile: profile.to_owned(),
            missing,
        },
    });
    true
}

fn canonical_cycle(cycle: &[String]) -> Vec<String> {
    let Some((minimum_index, _)) = cycle.iter().enumerate().min_by_key(|(_, name)| *name) else {
        return Vec::new();
    };
    cycle[minimum_index..]
        .iter()
        .chain(&cycle[..minimum_index])
        .cloned()
        .collect()
}

#[derive(Debug, Error)]
pub(crate) enum CatalogError {
    #[error("could not open {source_kind:?} source {path:?}: {error}")]
    Open {
        source_kind: CatalogFileKind,
        path: PathBuf,
        #[source]
        error: io::Error,
    },
    #[error("could not read {source_kind:?} source {path:?}: {error}")]
    Read {
        source_kind: CatalogFileKind,
        path: PathBuf,
        #[source]
        error: io::Error,
    },
}

#[cfg(test)]
mod tests;
