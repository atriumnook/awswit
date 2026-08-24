use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, BufRead};
use std::path::{Path, PathBuf};

use super::{CatalogFileKind, CatalogIssue, CatalogIssueKind, MetadataField, StaticCredentialKeys};
use crate::text_safety::is_safe_terminal_text;

pub(super) const MAX_LINE_BYTES: usize = 64 * 1024;
const MAX_NAME_BYTES: usize = 1024;
const MAX_METADATA_BYTES: usize = 4 * 1024;
pub(super) const MAX_SOURCE_BYTES: u64 = 16 * 1024 * 1024;
pub(super) const MAX_ISSUES: usize = 1024;
pub(super) const MAX_CATALOG_ENTRIES: usize = 16 * 1024;

#[derive(Debug, Clone, Default)]
pub(super) struct ParsedConfig {
    pub(super) profiles: BTreeMap<String, RawProfile>,
    pub(super) sso_sessions: BTreeMap<String, RawSsoSession>,
    pub(super) issues: Vec<CatalogIssue>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct ParsedCredentials {
    pub(super) profiles: BTreeMap<String, CredentialPresence>,
    pub(super) issues: Vec<CatalogIssue>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct CredentialPresence {
    pub(super) static_credentials: StaticCredentialKeys,
    /// True when this named section cannot be interpreted unambiguously.
    pub(super) activation_ambiguous: bool,
    seen_metadata: BTreeSet<MetadataField>,
}

#[derive(Debug, Clone)]
pub(super) struct RawProfile {
    pub(super) name: String,
    pub(super) region: Option<String>,
    pub(super) role_arn: Option<String>,
    pub(super) source_profile: Option<String>,
    pub(super) credential_source: Option<String>,
    pub(super) has_mfa_serial: bool,
    pub(super) has_credential_process: bool,
    pub(super) has_login_session: bool,
    pub(super) has_web_identity_token_file: bool,
    pub(super) static_credentials: StaticCredentialKeys,
    pub(super) has_config_section: bool,
    pub(super) sso_session: Option<String>,
    pub(super) sso_start_url: Option<String>,
    pub(super) sso_region: Option<String>,
    pub(super) sso_account_id: Option<String>,
    pub(super) sso_role_name: Option<String>,
    pub(super) first_config_line: Option<usize>,
    pub(super) source_profile_line: Option<usize>,
    pub(super) credential_source_line: Option<usize>,
    pub(super) sso_session_line: Option<usize>,
    /// Parser recovery retained the profile name, but not a trustworthy
    /// complete interpretation of its section.
    pub(super) activation_ambiguous: bool,
    seen_metadata: BTreeSet<MetadataField>,
}

impl RawProfile {
    pub(super) fn new(name: String) -> Self {
        Self {
            name,
            region: None,
            role_arn: None,
            source_profile: None,
            credential_source: None,
            has_mfa_serial: false,
            has_credential_process: false,
            has_login_session: false,
            has_web_identity_token_file: false,
            static_credentials: StaticCredentialKeys::default(),
            has_config_section: true,
            sso_session: None,
            sso_start_url: None,
            sso_region: None,
            sso_account_id: None,
            sso_role_name: None,
            first_config_line: None,
            source_profile_line: None,
            credential_source_line: None,
            sso_session_line: None,
            activation_ambiguous: false,
            seen_metadata: BTreeSet::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct RawSsoSession {
    pub(super) start_url: Option<String>,
    pub(super) sso_region: Option<String>,
    pub(super) registration_scopes: Option<String>,
    pub(super) activation_ambiguous: bool,
    seen_metadata: BTreeSet<MetadataField>,
}

impl RawSsoSession {
    fn new() -> Self {
        Self {
            start_url: None,
            sso_region: None,
            registration_scopes: None,
            activation_ambiguous: false,
            seen_metadata: BTreeSet::new(),
        }
    }
}

#[derive(Debug, Clone)]
enum ConfigSection {
    None,
    Profile(String),
    SsoSession(String),
    Ignored,
    Quarantined,
}

#[derive(Debug, Clone)]
enum CredentialsSection {
    None,
    Profile(String),
    Quarantined,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PropertyOutcome {
    Applied,
    IgnoredUnknown,
    Invalid,
}

pub(super) fn parse_config<R: BufRead>(mut reader: R, path: &Path) -> io::Result<ParsedConfig> {
    let mut parsed = ParsedConfig::default();
    let mut section = ConfigSection::None;
    let mut line = Vec::with_capacity(512);
    let mut line_number = 0;
    let mut bytes_read = 0_u64;
    let mut continuation_indent: Option<usize> = None;

    while let Some(too_long) = read_bounded_line(&mut reader, &mut line, &mut bytes_read)? {
        line_number += 1;
        if parsed.issues.len() > MAX_ISSUES {
            return Err(input_limit_error());
        }
        if too_long {
            mark_config_section_ambiguous(&section, &mut parsed);
            parsed.issues.push(issue(
                CatalogFileKind::Config,
                path,
                line_number,
                CatalogIssueKind::LineTooLong,
            ));
            section = ConfigSection::Quarantined;
            continue;
        }

        let physical = strip_bom(line_number, &line);
        let indented = physical.first().is_some_and(u8::is_ascii_whitespace);
        let indent = physical
            .len()
            .saturating_sub(physical.trim_ascii_start().len());
        let bytes = trim_ascii(physical);
        if bytes.is_empty() || is_comment(bytes) {
            continue;
        }
        if continuation_indent.is_some_and(|parent| indented && indent > parent) {
            continue;
        }
        if bytes.first() == Some(&b'[') {
            continuation_indent = None;
            if parsed
                .profiles
                .len()
                .saturating_add(parsed.sso_sessions.len())
                >= MAX_CATALOG_ENTRIES
                && config_header_adds_entry(bytes, &parsed)
            {
                return Err(input_limit_error());
            }
            section = parse_config_section(bytes, path, line_number, &mut parsed);
            continue;
        }

        match &section {
            ConfigSection::Profile(name) => {
                let Some((key, value)) = parse_property(bytes) else {
                    if let Some(profile) = parsed.profiles.get_mut(name) {
                        profile.activation_ambiguous = true;
                    }
                    parsed.issues.push(issue(
                        CatalogFileKind::Config,
                        path,
                        line_number,
                        CatalogIssueKind::MalformedProperty,
                    ));
                    section = ConfigSection::Quarantined;
                    continue;
                };
                let Some(profile) = parsed.profiles.get_mut(name) else {
                    continue;
                };
                match apply_profile_property(
                    profile,
                    key,
                    value,
                    path,
                    line_number,
                    &mut parsed.issues,
                ) {
                    PropertyOutcome::Applied => continuation_indent = None,
                    PropertyOutcome::IgnoredUnknown => continuation_indent = Some(indent),
                    PropertyOutcome::Invalid => {
                        continuation_indent = None;
                        section = ConfigSection::Quarantined;
                    }
                }
            }
            ConfigSection::SsoSession(name) => {
                let Some((key, value)) = parse_property(bytes) else {
                    if let Some(session) = parsed.sso_sessions.get_mut(name) {
                        session.activation_ambiguous = true;
                    }
                    parsed.issues.push(issue(
                        CatalogFileKind::Config,
                        path,
                        line_number,
                        CatalogIssueKind::MalformedProperty,
                    ));
                    section = ConfigSection::Quarantined;
                    continue;
                };
                let Some(session) = parsed.sso_sessions.get_mut(name) else {
                    continue;
                };
                match apply_sso_property(session, key, value, path, line_number, &mut parsed.issues)
                {
                    PropertyOutcome::Applied => continuation_indent = None,
                    PropertyOutcome::IgnoredUnknown => continuation_indent = Some(indent),
                    PropertyOutcome::Invalid => {
                        continuation_indent = None;
                        section = ConfigSection::Quarantined;
                    }
                }
            }
            ConfigSection::None => parsed.issues.push(issue(
                CatalogFileKind::Config,
                path,
                line_number,
                CatalogIssueKind::PropertyOutsideSection,
            )),
            ConfigSection::Ignored | ConfigSection::Quarantined => {}
        }
    }

    if parsed.issues.len() > MAX_ISSUES {
        return Err(input_limit_error());
    }
    Ok(parsed)
}

pub(super) fn parse_credentials<R: BufRead>(
    mut reader: R,
    path: &Path,
) -> io::Result<ParsedCredentials> {
    let mut parsed = ParsedCredentials::default();
    let mut section = CredentialsSection::None;
    let mut line = Vec::with_capacity(512);
    let mut line_number = 0;
    let mut bytes_read = 0_u64;
    let mut continuation_indent: Option<usize> = None;

    while let Some(too_long) = read_bounded_line(&mut reader, &mut line, &mut bytes_read)? {
        line_number += 1;
        if parsed.issues.len() > MAX_ISSUES {
            return Err(input_limit_error());
        }
        if too_long {
            mark_credentials_section_ambiguous(&section, &mut parsed);
            parsed.issues.push(issue(
                CatalogFileKind::Credentials,
                path,
                line_number,
                CatalogIssueKind::LineTooLong,
            ));
            section = CredentialsSection::Quarantined;
            continue;
        }

        let physical = strip_bom(line_number, &line);
        let indented = physical.first().is_some_and(u8::is_ascii_whitespace);
        let indent = physical
            .len()
            .saturating_sub(physical.trim_ascii_start().len());
        let bytes = trim_ascii(physical);
        if bytes.is_empty() || is_comment(bytes) {
            continue;
        }
        if continuation_indent.is_some_and(|parent| indented && indent > parent) {
            continue;
        }
        if bytes.first() == Some(&b'[') {
            continuation_indent = None;
            if parsed.profiles.len() >= MAX_CATALOG_ENTRIES
                && credentials_header_adds_entry(bytes, &parsed)
            {
                return Err(input_limit_error());
            }
            section = parse_credentials_section(bytes, path, line_number, &mut parsed);
            continue;
        }

        match &section {
            CredentialsSection::Profile(name) => {
                let Some((key, value)) = parse_property(bytes) else {
                    if let Some(presence) = parsed.profiles.get_mut(name) {
                        presence.activation_ambiguous = true;
                    }
                    parsed.issues.push(issue(
                        CatalogFileKind::Credentials,
                        path,
                        line_number,
                        CatalogIssueKind::MalformedProperty,
                    ));
                    section = CredentialsSection::Quarantined;
                    continue;
                };
                let Some(presence) = parsed.profiles.get_mut(name) else {
                    continue;
                };
                match apply_credential_key(
                    presence,
                    key,
                    value,
                    path,
                    line_number,
                    &mut parsed.issues,
                ) {
                    PropertyOutcome::Applied => continuation_indent = None,
                    PropertyOutcome::IgnoredUnknown => continuation_indent = Some(indent),
                    PropertyOutcome::Invalid => {
                        continuation_indent = None;
                        section = CredentialsSection::Quarantined;
                    }
                }
            }
            CredentialsSection::None => parsed.issues.push(issue(
                CatalogFileKind::Credentials,
                path,
                line_number,
                CatalogIssueKind::PropertyOutsideSection,
            )),
            CredentialsSection::Quarantined => {}
        }
    }

    if parsed.issues.len() > MAX_ISSUES {
        return Err(input_limit_error());
    }
    Ok(parsed)
}

fn mark_config_section_ambiguous(section: &ConfigSection, parsed: &mut ParsedConfig) {
    match section {
        ConfigSection::Profile(name) => {
            if let Some(profile) = parsed.profiles.get_mut(name) {
                profile.activation_ambiguous = true;
            }
        }
        ConfigSection::SsoSession(name) => {
            if let Some(session) = parsed.sso_sessions.get_mut(name) {
                session.activation_ambiguous = true;
            }
        }
        ConfigSection::None | ConfigSection::Ignored | ConfigSection::Quarantined => {}
    }
}

fn mark_credentials_section_ambiguous(
    section: &CredentialsSection,
    parsed: &mut ParsedCredentials,
) {
    if let CredentialsSection::Profile(name) = section
        && let Some(profile) = parsed.profiles.get_mut(name)
    {
        profile.activation_ambiguous = true;
    }
}

fn config_header_adds_entry(line: &[u8], parsed: &ParsedConfig) -> bool {
    let Some(name_bytes) = section_name_bytes(line) else {
        return false;
    };
    let Ok(section_name) = std::str::from_utf8(name_bytes) else {
        return false;
    };
    if section_name == "default" {
        return !parsed.profiles.contains_key("default");
    }
    if let Some(name) = section_name.strip_prefix("profile ") {
        let name = name.trim_matches(is_ascii_space_char);
        return safe_name(name) && !parsed.profiles.contains_key(name);
    }
    if let Some(name) = section_name.strip_prefix("sso-session ") {
        let name = name.trim_matches(is_ascii_space_char);
        return safe_name(name) && !parsed.sso_sessions.contains_key(name);
    }
    false
}

fn credentials_header_adds_entry(line: &[u8], parsed: &ParsedCredentials) -> bool {
    let Some(name_bytes) = section_name_bytes(line) else {
        return false;
    };
    let Ok(name) = std::str::from_utf8(name_bytes) else {
        return false;
    };
    safe_name(name) && !parsed.profiles.contains_key(name)
}

fn parse_config_section(
    line: &[u8],
    path: &Path,
    line_number: usize,
    parsed: &mut ParsedConfig,
) -> ConfigSection {
    let Some(name_bytes) = section_name_bytes(line) else {
        parsed.issues.push(issue(
            CatalogFileKind::Config,
            path,
            line_number,
            CatalogIssueKind::MalformedSection,
        ));
        return ConfigSection::Quarantined;
    };
    let Ok(section_name) = std::str::from_utf8(name_bytes) else {
        parsed.issues.push(issue(
            CatalogFileKind::Config,
            path,
            line_number,
            CatalogIssueKind::InvalidEncoding,
        ));
        return ConfigSection::Quarantined;
    };

    if section_name == "default" {
        return insert_profile("default", path, line_number, parsed);
    }
    if let Some(name) = section_name.strip_prefix("profile ") {
        let name = name.trim_matches(is_ascii_space_char);
        if !safe_name(name) {
            parsed.issues.push(issue(
                CatalogFileKind::Config,
                path,
                line_number,
                CatalogIssueKind::UnsafeProfileName,
            ));
            return ConfigSection::Quarantined;
        }
        return insert_profile(name, path, line_number, parsed);
    }
    if let Some(name) = section_name.strip_prefix("sso-session ") {
        let name = name.trim_matches(is_ascii_space_char);
        if !safe_name(name) {
            parsed.issues.push(issue(
                CatalogFileKind::Config,
                path,
                line_number,
                CatalogIssueKind::UnsafeSsoSessionName,
            ));
            return ConfigSection::Quarantined;
        }
        if parsed.sso_sessions.contains_key(name) {
            parsed.issues.push(issue(
                CatalogFileKind::Config,
                path,
                line_number,
                CatalogIssueKind::DuplicateSection {
                    name: name.to_owned(),
                },
            ));
            if let Some(session) = parsed.sso_sessions.get_mut(name) {
                session.activation_ambiguous = true;
            }
            return ConfigSection::Quarantined;
        } else {
            parsed
                .sso_sessions
                .insert(name.to_owned(), RawSsoSession::new());
        }
        return ConfigSection::SsoSession(name.to_owned());
    }
    if section_name == "profile" {
        parsed.issues.push(issue(
            CatalogFileKind::Config,
            path,
            line_number,
            CatalogIssueKind::UnsafeProfileName,
        ));
        return ConfigSection::Quarantined;
    }
    if section_name == "sso-session" {
        parsed.issues.push(issue(
            CatalogFileKind::Config,
            path,
            line_number,
            CatalogIssueKind::UnsafeSsoSessionName,
        ));
        return ConfigSection::Quarantined;
    }

    // `services`, `plugins`, and future AWS section kinds are metadata, not
    // profiles.  Unknown sections are ignored for forward compatibility.
    ConfigSection::Ignored
}

fn insert_profile(
    name: &str,
    path: &Path,
    line_number: usize,
    parsed: &mut ParsedConfig,
) -> ConfigSection {
    if let Some(profile) = parsed.profiles.get_mut(name) {
        parsed.issues.push(issue(
            CatalogFileKind::Config,
            path,
            line_number,
            CatalogIssueKind::DuplicateSection {
                name: name.to_owned(),
            },
        ));
        profile.has_config_section = true;
        profile.activation_ambiguous = true;
        return ConfigSection::Quarantined;
    } else {
        let mut profile = RawProfile::new(name.to_owned());
        profile.first_config_line = Some(line_number);
        parsed.profiles.insert(name.to_owned(), profile);
    }
    ConfigSection::Profile(name.to_owned())
}

fn parse_credentials_section(
    line: &[u8],
    path: &Path,
    line_number: usize,
    parsed: &mut ParsedCredentials,
) -> CredentialsSection {
    let Some(name_bytes) = section_name_bytes(line) else {
        parsed.issues.push(issue(
            CatalogFileKind::Credentials,
            path,
            line_number,
            CatalogIssueKind::MalformedSection,
        ));
        return CredentialsSection::Quarantined;
    };
    let Ok(name) = std::str::from_utf8(name_bytes) else {
        parsed.issues.push(issue(
            CatalogFileKind::Credentials,
            path,
            line_number,
            CatalogIssueKind::InvalidEncoding,
        ));
        return CredentialsSection::Quarantined;
    };
    if !safe_name(name) {
        parsed.issues.push(issue(
            CatalogFileKind::Credentials,
            path,
            line_number,
            CatalogIssueKind::UnsafeProfileName,
        ));
        return CredentialsSection::Quarantined;
    }

    if parsed.profiles.contains_key(name) {
        parsed.issues.push(issue(
            CatalogFileKind::Credentials,
            path,
            line_number,
            CatalogIssueKind::DuplicateSection {
                name: name.to_owned(),
            },
        ));
        if let Some(profile) = parsed.profiles.get_mut(name) {
            profile.activation_ambiguous = true;
        }
        return CredentialsSection::Quarantined;
    } else {
        parsed
            .profiles
            .insert(name.to_owned(), CredentialPresence::default());
    }
    CredentialsSection::Profile(name.to_owned())
}

fn apply_profile_property(
    profile: &mut RawProfile,
    key: &[u8],
    value: &[u8],
    path: &Path,
    line_number: usize,
    issues: &mut Vec<CatalogIssue>,
) -> PropertyOutcome {
    let Some(key) = normalized_key(key) else {
        profile.activation_ambiguous = true;
        issues.push(issue(
            CatalogFileKind::Config,
            path,
            line_number,
            CatalogIssueKind::MalformedProperty,
        ));
        return PropertyOutcome::Invalid;
    };

    let field = match key.as_str() {
        "region" => Some(MetadataField::Region),
        "role_arn" => Some(MetadataField::RoleArn),
        "source_profile" => Some(MetadataField::SourceProfile),
        "credential_source" => Some(MetadataField::CredentialSource),
        "sso_session" => Some(MetadataField::SsoSession),
        "sso_start_url" => Some(MetadataField::SsoStartUrl),
        "sso_region" => Some(MetadataField::SsoRegion),
        "sso_account_id" => Some(MetadataField::SsoAccountId),
        "sso_role_name" => Some(MetadataField::SsoRoleName),
        _ => None,
    };

    if let Some(field) = field {
        if !profile.seen_metadata.insert(field) {
            profile.activation_ambiguous = true;
            issues.push(issue(
                CatalogFileKind::Config,
                path,
                line_number,
                CatalogIssueKind::DuplicateMetadata { field },
            ));
            return PropertyOutcome::Invalid;
        }
        let mut parsed_value = safe_metadata(value);
        if matches!(
            field,
            MetadataField::SourceProfile | MetadataField::SsoSession
        ) && parsed_value
            .as_deref()
            .is_some_and(|value| !safe_name(value))
        {
            parsed_value = None;
        }
        if parsed_value.is_none() {
            profile.activation_ambiguous = true;
            issues.push(issue(
                CatalogFileKind::Config,
                path,
                line_number,
                CatalogIssueKind::InvalidMetadata { field },
            ));
            return PropertyOutcome::Invalid;
        }
        match field {
            MetadataField::Region => profile.region = parsed_value,
            MetadataField::RoleArn => profile.role_arn = parsed_value,
            MetadataField::SourceProfile => {
                profile.source_profile = parsed_value;
                profile.source_profile_line = Some(line_number);
            }
            MetadataField::CredentialSource => {
                profile.credential_source = parsed_value;
                profile.credential_source_line = Some(line_number);
            }
            MetadataField::SsoSession => {
                profile.sso_session = parsed_value;
                profile.sso_session_line = Some(line_number);
            }
            MetadataField::SsoStartUrl => profile.sso_start_url = parsed_value,
            MetadataField::SsoRegion => profile.sso_region = parsed_value,
            MetadataField::SsoRegistrationScopes => return PropertyOutcome::Invalid,
            MetadataField::SsoAccountId => profile.sso_account_id = parsed_value,
            MetadataField::SsoRoleName => profile.sso_role_name = parsed_value,
            MetadataField::MfaSerial
            | MetadataField::CredentialProcess
            | MetadataField::LoginSession
            | MetadataField::WebIdentityTokenFile
            | MetadataField::AccessKeyId
            | MetadataField::SecretAccessKey
            | MetadataField::SessionToken => return PropertyOutcome::Invalid,
        }
        return PropertyOutcome::Applied;
    }

    // Presence-only keys are tracked without decoding or retaining their values.
    // Duplicate provider keys are ambiguous even though their contents remain secret.
    if let Some(field) = presence_field(&key) {
        if !profile.seen_metadata.insert(field) {
            profile.activation_ambiguous = true;
            issues.push(issue(
                CatalogFileKind::Config,
                path,
                line_number,
                CatalogIssueKind::DuplicateMetadata { field },
            ));
            return PropertyOutcome::Invalid;
        }
        if !valid_presence_value(field, value) {
            profile.activation_ambiguous = true;
            issues.push(issue(
                CatalogFileKind::Config,
                path,
                line_number,
                CatalogIssueKind::InvalidMetadata { field },
            ));
            return PropertyOutcome::Invalid;
        }
        apply_profile_presence(profile, field);
        return PropertyOutcome::Applied;
    }
    PropertyOutcome::IgnoredUnknown
}

fn apply_sso_property(
    session: &mut RawSsoSession,
    key: &[u8],
    value: &[u8],
    path: &Path,
    line_number: usize,
    issues: &mut Vec<CatalogIssue>,
) -> PropertyOutcome {
    let Some(key) = normalized_key(key) else {
        session.activation_ambiguous = true;
        issues.push(issue(
            CatalogFileKind::Config,
            path,
            line_number,
            CatalogIssueKind::MalformedProperty,
        ));
        return PropertyOutcome::Invalid;
    };
    let field = match key.as_str() {
        "sso_start_url" => MetadataField::SsoStartUrl,
        "sso_region" => MetadataField::SsoRegion,
        "sso_registration_scopes" => MetadataField::SsoRegistrationScopes,
        _ => return PropertyOutcome::IgnoredUnknown,
    };
    if !session.seen_metadata.insert(field) {
        session.activation_ambiguous = true;
        issues.push(issue(
            CatalogFileKind::Config,
            path,
            line_number,
            CatalogIssueKind::DuplicateMetadata { field },
        ));
        return PropertyOutcome::Invalid;
    }
    let parsed_value = safe_metadata(value);
    if parsed_value.is_none() {
        session.activation_ambiguous = true;
        issues.push(issue(
            CatalogFileKind::Config,
            path,
            line_number,
            CatalogIssueKind::InvalidMetadata { field },
        ));
        return PropertyOutcome::Invalid;
    }
    if field == MetadataField::SsoStartUrl {
        session.start_url = parsed_value;
    } else if field == MetadataField::SsoRegion {
        session.sso_region = parsed_value;
    } else {
        session.registration_scopes = parsed_value;
    }
    PropertyOutcome::Applied
}

fn apply_credential_key(
    presence: &mut CredentialPresence,
    key: &[u8],
    value: &[u8],
    path: &Path,
    line_number: usize,
    issues: &mut Vec<CatalogIssue>,
) -> PropertyOutcome {
    let Some(key) = normalized_key(key) else {
        presence.activation_ambiguous = true;
        issues.push(issue(
            CatalogFileKind::Credentials,
            path,
            line_number,
            CatalogIssueKind::MalformedProperty,
        ));
        return PropertyOutcome::Invalid;
    };
    let field = match key.as_str() {
        "aws_access_key_id" => Some(MetadataField::AccessKeyId),
        "aws_secret_access_key" => Some(MetadataField::SecretAccessKey),
        "aws_session_token" | "aws_security_token" => Some(MetadataField::SessionToken),
        // AWS only defines these three credential keys in the credentials file.
        // Other settings are forward-compatible unknowns here, even when the
        // same spelling has meaning in the shared config file.
        _ => None,
    };
    let Some(field) = field else {
        return PropertyOutcome::IgnoredUnknown;
    };
    if !presence.seen_metadata.insert(field) {
        presence.activation_ambiguous = true;
        issues.push(issue(
            CatalogFileKind::Credentials,
            path,
            line_number,
            CatalogIssueKind::DuplicateMetadata { field },
        ));
        return PropertyOutcome::Invalid;
    }
    if !valid_secret_presence_value(value) {
        presence.activation_ambiguous = true;
        issues.push(issue(
            CatalogFileKind::Credentials,
            path,
            line_number,
            CatalogIssueKind::InvalidMetadata { field },
        ));
        return PropertyOutcome::Invalid;
    }
    match field {
        MetadataField::AccessKeyId => presence.static_credentials.access_key_id = true,
        MetadataField::SecretAccessKey => presence.static_credentials.secret_access_key = true,
        MetadataField::SessionToken => presence.static_credentials.session_token = true,
        _ => {}
    }
    PropertyOutcome::Applied
}

fn presence_field(key: &str) -> Option<MetadataField> {
    match key {
        "mfa_serial" => Some(MetadataField::MfaSerial),
        "credential_process" => Some(MetadataField::CredentialProcess),
        "login_session" => Some(MetadataField::LoginSession),
        "web_identity_token_file" => Some(MetadataField::WebIdentityTokenFile),
        "aws_access_key_id" => Some(MetadataField::AccessKeyId),
        "aws_secret_access_key" => Some(MetadataField::SecretAccessKey),
        "aws_session_token" | "aws_security_token" => Some(MetadataField::SessionToken),
        _ => None,
    }
}

fn apply_profile_presence(profile: &mut RawProfile, field: MetadataField) {
    match field {
        MetadataField::MfaSerial => profile.has_mfa_serial = true,
        MetadataField::CredentialProcess => profile.has_credential_process = true,
        MetadataField::LoginSession => profile.has_login_session = true,
        MetadataField::WebIdentityTokenFile => profile.has_web_identity_token_file = true,
        MetadataField::AccessKeyId => profile.static_credentials.access_key_id = true,
        MetadataField::SecretAccessKey => profile.static_credentials.secret_access_key = true,
        MetadataField::SessionToken => profile.static_credentials.session_token = true,
        _ => {}
    }
}

fn valid_presence_value(field: MetadataField, value: &[u8]) -> bool {
    if matches!(
        field,
        MetadataField::AccessKeyId | MetadataField::SecretAccessKey | MetadataField::SessionToken
    ) {
        return valid_secret_presence_value(value);
    }
    !value.is_empty()
        && value.len() <= MAX_METADATA_BYTES
        && std::str::from_utf8(value).is_ok_and(is_safe_terminal_text)
}

fn valid_secret_presence_value(value: &[u8]) -> bool {
    // STS explicitly gives session tokens no fixed maximum. The streaming
    // physical-line budget already bounds memory, and credential bytes are
    // neither retained nor displayed, so do not impose the metadata limit.
    !value.is_empty() && std::str::from_utf8(value).is_ok_and(is_safe_terminal_text)
}

/// Return `Some(too_long)` for every physical line.  Once the configured bound
/// is reached, the rest of that line is drained without further allocation.
fn read_bounded_line<R: BufRead>(
    reader: &mut R,
    output: &mut Vec<u8>,
    source_bytes: &mut u64,
) -> io::Result<Option<bool>> {
    output.clear();
    let mut saw_input = false;
    let mut too_long = false;
    let mut physical_bytes = 0_usize;

    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok(saw_input.then_some(too_long));
        }
        saw_input = true;

        let newline = available.iter().position(|byte| *byte == b'\n');
        let take = newline.map_or(available.len(), |position| position + 1);
        physical_bytes = physical_bytes.saturating_add(take);
        if physical_bytes > MAX_LINE_BYTES {
            too_long = true;
        }
        *source_bytes = source_bytes.saturating_add(u64::try_from(take).unwrap_or(u64::MAX));
        if *source_bytes > MAX_SOURCE_BYTES {
            return Err(input_limit_error());
        }
        let content_len = newline.unwrap_or(take);
        if output.len() < MAX_LINE_BYTES {
            let remaining = MAX_LINE_BYTES.saturating_sub(output.len());
            let copy = content_len.min(remaining);
            output.extend_from_slice(&available[..copy]);
            if copy < content_len {
                too_long = true;
            }
        }
        reader.consume(take);
        if newline.is_some() {
            return Ok(Some(too_long));
        }
    }
}

fn input_limit_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "AWS shared configuration exceeds the bounded parser budget",
    )
}

fn section_name_bytes(line: &[u8]) -> Option<&[u8]> {
    let closing = line.iter().position(|byte| *byte == b']')?;
    if line.first() != Some(&b'[') || closing == 1 {
        return None;
    }
    let trailing = trim_ascii(&line[closing + 1..]);
    if !trailing.is_empty() && !is_comment(trailing) {
        return None;
    }
    Some(trim_ascii(&line[1..closing]))
}

fn parse_property(line: &[u8]) -> Option<(&[u8], &[u8])> {
    let delimiter = line.iter().position(|byte| matches!(*byte, b'=' | b':'))?;
    let key = trim_ascii(&line[..delimiter]);
    (!key.is_empty()).then_some((key, trim_ascii(&line[delimiter + 1..])))
}

fn normalized_key(key: &[u8]) -> Option<String> {
    if key.is_empty()
        || !key
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'_' | b'-' | b'.'))
    {
        return None;
    }
    Some(
        key.iter()
            .map(u8::to_ascii_lowercase)
            .map(char::from)
            .collect(),
    )
}

fn safe_metadata(value: &[u8]) -> Option<String> {
    if value.is_empty() || value.len() > MAX_METADATA_BYTES {
        return None;
    }
    let value = std::str::from_utf8(value).ok()?;
    is_safe_terminal_text(value).then(|| value.to_owned())
}

fn safe_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= MAX_NAME_BYTES && is_safe_terminal_text(name)
}

fn strip_bom(line_number: usize, line: &[u8]) -> &[u8] {
    if line_number == 1 {
        line.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(line)
    } else {
        line
    }
}

fn trim_ascii(mut value: &[u8]) -> &[u8] {
    while value.first().is_some_and(|byte| byte.is_ascii_whitespace()) {
        value = &value[1..];
    }
    while value.last().is_some_and(|byte| byte.is_ascii_whitespace()) {
        value = &value[..value.len() - 1];
    }
    value
}

fn is_comment(line: &[u8]) -> bool {
    matches!(line.first(), Some(b'#' | b';'))
}

fn is_ascii_space_char(character: char) -> bool {
    character.is_ascii_whitespace()
}

fn issue(
    source: CatalogFileKind,
    path: &Path,
    line: usize,
    kind: CatalogIssueKind,
) -> CatalogIssue {
    CatalogIssue {
        source,
        path: PathBuf::from(path),
        line: Some(line),
        kind,
    }
}
