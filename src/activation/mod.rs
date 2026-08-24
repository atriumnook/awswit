//! Shell-neutral, transactional environment patches.
//!
//! The activation protocol is deliberately small.  The executable writes one
//! complete frame to stdout and the generated shell hook validates the whole
//! frame before changing its environment.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::Path;

use crate::text_safety::is_terminal_control;

const HEADER_ACTIVATE: &str = "AWSWIT-PATCH 1 ACTIVATE";
const HEADER_UNSET: &str = "AWSWIT-PATCH 1 UNSET";
const COMMIT: &str = "AWSWIT-COMMIT";

const PROFILE_VARIABLES: [&str; 3] = ["AWS_PROFILE", "AWS_DEFAULT_PROFILE", "AWSWIT_PROFILE"];
const REGION_VARIABLES: [&str; 2] = ["AWS_REGION", "AWS_DEFAULT_REGION"];

/// Environment variables which can supersede an AWS profile's credential
/// provider.  Values are intentionally never represented by this type.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum CredentialVariable {
    AccessKeyId,
    AccessKeyLegacy,
    AmazonAccessKeyId,
    SecretAccessKey,
    SecretKeyLegacy,
    AmazonSecretAccessKey,
    SessionToken,
    SecurityToken,
    AmazonSessionToken,
    WebIdentityTokenFile,
    RoleArn,
    RoleSessionName,
    ContainerCredentialsRelativeUri,
    ContainerCredentialsFullUri,
    ContainerAuthorizationToken,
    ContainerAuthorizationTokenFile,
    Ec2MetadataServiceEndpoint,
    LoginCacheDirectory,
    CredentialProfilesFileLegacy,
    BedrockBearerToken,
}

impl CredentialVariable {
    pub(crate) const ALL: [Self; 20] = [
        Self::AccessKeyId,
        Self::AccessKeyLegacy,
        Self::AmazonAccessKeyId,
        Self::SecretAccessKey,
        Self::SecretKeyLegacy,
        Self::AmazonSecretAccessKey,
        Self::SessionToken,
        Self::SecurityToken,
        Self::AmazonSessionToken,
        Self::WebIdentityTokenFile,
        Self::RoleArn,
        Self::RoleSessionName,
        Self::ContainerCredentialsRelativeUri,
        Self::ContainerCredentialsFullUri,
        Self::ContainerAuthorizationToken,
        Self::ContainerAuthorizationTokenFile,
        Self::Ec2MetadataServiceEndpoint,
        Self::LoginCacheDirectory,
        Self::CredentialProfilesFileLegacy,
        Self::BedrockBearerToken,
    ];

    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::AccessKeyId => "AWS_ACCESS_KEY_ID",
            Self::AccessKeyLegacy => "AWS_ACCESS_KEY",
            Self::AmazonAccessKeyId => "AMAZON_ACCESS_KEY_ID",
            Self::SecretAccessKey => "AWS_SECRET_ACCESS_KEY",
            Self::SecretKeyLegacy => "AWS_SECRET_KEY",
            Self::AmazonSecretAccessKey => "AMAZON_SECRET_ACCESS_KEY",
            Self::SessionToken => "AWS_SESSION_TOKEN",
            Self::SecurityToken => "AWS_SECURITY_TOKEN",
            Self::AmazonSessionToken => "AMAZON_SESSION_TOKEN",
            Self::WebIdentityTokenFile => "AWS_WEB_IDENTITY_TOKEN_FILE",
            Self::RoleArn => "AWS_ROLE_ARN",
            Self::RoleSessionName => "AWS_ROLE_SESSION_NAME",
            Self::ContainerCredentialsRelativeUri => "AWS_CONTAINER_CREDENTIALS_RELATIVE_URI",
            Self::ContainerCredentialsFullUri => "AWS_CONTAINER_CREDENTIALS_FULL_URI",
            Self::ContainerAuthorizationToken => "AWS_CONTAINER_AUTHORIZATION_TOKEN",
            Self::ContainerAuthorizationTokenFile => "AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE",
            Self::Ec2MetadataServiceEndpoint => "AWS_EC2_METADATA_SERVICE_ENDPOINT",
            Self::LoginCacheDirectory => "AWS_LOGIN_CACHE_DIRECTORY",
            Self::CredentialProfilesFileLegacy => "AWS_CREDENTIAL_PROFILES_FILE",
            Self::BedrockBearerToken => "AWS_BEARER_TOKEN_BEDROCK",
        }
    }

    pub(crate) fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|variable| variable.name() == name)
    }

    pub(crate) const fn is_container(self) -> bool {
        matches!(
            self,
            Self::ContainerCredentialsRelativeUri
                | Self::ContainerCredentialsFullUri
                | Self::ContainerAuthorizationToken
                | Self::ContainerAuthorizationTokenFile
        )
    }

    pub(crate) const fn is_ec2_metadata(self) -> bool {
        matches!(self, Self::Ec2MetadataServiceEndpoint)
    }

    pub(crate) const fn is_login_cache(self) -> bool {
        matches!(self, Self::LoginCacheDirectory)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PatchKind {
    Activate,
    Unset,
}

/// A validated operation exposed as a read-only view to process adapters.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OperationRef<'a> {
    Set {
        name: &'static str,
        value: &'a OsStr,
    },
    Unset {
        name: &'static str,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Operation {
    Set { name: &'static str, value: OsString },
    Unset { name: &'static str },
}

/// A coherent, allow-listed patch.  Private fields prevent callers from
/// constructing partial profile or region updates.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EnvironmentPatch {
    kind: PatchKind,
    operations: Vec<Operation>,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum PatchError {
    EmptyValue { field: &'static str },
    UnsafeValue { field: &'static str },
    NonUnicodeValue { variable: &'static str },
    InvalidFrame,
}

impl fmt::Display for PatchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyValue { field } => write!(formatter, "{field} must not be empty"),
            Self::UnsafeValue { field } => {
                write!(formatter, "{field} contains a control character")
            }
            Self::NonUnicodeValue { variable } => {
                write!(
                    formatter,
                    "{variable} cannot be represented by the shell protocol"
                )
            }
            Self::InvalidFrame => formatter.write_str("invalid activation frame"),
        }
    }
}

impl std::error::Error for PatchError {}

impl EnvironmentPatch {
    pub(crate) fn activate(
        profile: &str,
        region: Option<&str>,
        config_file: Option<&Path>,
        credentials_file: Option<&Path>,
        clear: &BTreeSet<CredentialVariable>,
    ) -> Result<Self, PatchError> {
        validate_value("profile", profile)?;
        if let Some(region) = region {
            validate_value("region", region)?;
        }

        let mut operations = Vec::with_capacity(7 + clear.len());
        for name in PROFILE_VARIABLES {
            operations.push(Operation::Set {
                name,
                value: OsString::from(profile),
            });
        }
        for name in REGION_VARIABLES {
            match region {
                Some(value) => operations.push(Operation::Set {
                    name,
                    value: OsString::from(value),
                }),
                None => operations.push(Operation::Unset { name }),
            }
        }
        if let Some(value) = config_file {
            operations.push(Operation::Set {
                name: "AWS_CONFIG_FILE",
                value: value.as_os_str().to_owned(),
            });
        }
        if let Some(value) = credentials_file {
            operations.push(Operation::Set {
                name: "AWS_SHARED_CREDENTIALS_FILE",
                value: value.as_os_str().to_owned(),
            });
        }
        operations.extend(clear.iter().map(|variable| Operation::Unset {
            name: variable.name(),
        }));

        Ok(Self {
            kind: PatchKind::Activate,
            operations,
        })
    }

    pub(crate) fn unset() -> Self {
        let operations = PROFILE_VARIABLES
            .into_iter()
            .chain(REGION_VARIABLES)
            .map(|name| Operation::Unset { name })
            .collect();
        Self {
            kind: PatchKind::Unset,
            operations,
        }
    }

    #[cfg(test)]
    const fn kind(&self) -> PatchKind {
        self.kind
    }

    pub(crate) fn operations(&self) -> impl Iterator<Item = OperationRef<'_>> {
        self.operations.iter().map(|operation| match operation {
            Operation::Set { name, value } => OperationRef::Set { name, value },
            Operation::Unset { name } => OperationRef::Unset { name },
        })
    }

    /// Encode a frame whose final record is an explicit commit marker.
    pub(crate) fn encode(&self) -> Result<String, PatchError> {
        let mut frame = String::new();
        frame.push_str(match self.kind {
            PatchKind::Activate => HEADER_ACTIVATE,
            PatchKind::Unset => HEADER_UNSET,
        });
        frame.push('\n');
        for operation in &self.operations {
            match operation {
                Operation::Set { name, value } => {
                    let value = value
                        .to_str()
                        .ok_or(PatchError::NonUnicodeValue { variable: name })?;
                    if contains_unsafe_character(value) {
                        return Err(PatchError::UnsafeValue { field: name });
                    }
                    frame.push_str("SET ");
                    frame.push_str(name);
                    frame.push('=');
                    frame.push_str(value);
                    frame.push('\n');
                }
                Operation::Unset { name } => {
                    frame.push_str("UNSET ");
                    frame.push_str(name);
                    frame.push('\n');
                }
            }
        }
        frame.push_str(COMMIT);
        frame.push('\n');
        Ok(frame)
    }

    /// Decode and semantically validate a frame.  Production shell adapters
    /// implement the same closed grammar; this decoder is also a conformance
    /// oracle for tests.
    pub(crate) fn decode(frame: &str) -> Result<Self, PatchError> {
        if !frame.ends_with('\n') || frame.contains('\r') || frame.contains('\0') {
            return Err(PatchError::InvalidFrame);
        }
        let mut lines = frame.lines();
        let kind = match lines.next() {
            Some(HEADER_ACTIVATE) => PatchKind::Activate,
            Some(HEADER_UNSET) => PatchKind::Unset,
            _ => return Err(PatchError::InvalidFrame),
        };

        let mut records = BTreeMap::<&str, Option<&str>>::new();
        let mut ordered_records = Vec::<(&str, Option<&str>)>::new();
        let mut committed = false;
        for line in lines {
            if committed {
                return Err(PatchError::InvalidFrame);
            }
            if line == COMMIT {
                committed = true;
                continue;
            }
            if let Some(record) = line.strip_prefix("SET ") {
                let (name, value) = record.split_once('=').ok_or(PatchError::InvalidFrame)?;
                if value.is_empty()
                    || !is_settable(name)
                    || contains_unsafe_character(value)
                    || records.insert(name, Some(value)).is_some()
                {
                    return Err(PatchError::InvalidFrame);
                }
                ordered_records.push((name, Some(value)));
            } else if let Some(name) = line.strip_prefix("UNSET ") {
                if !is_unsettable_for(kind, name) || records.insert(name, None).is_some() {
                    return Err(PatchError::InvalidFrame);
                }
                ordered_records.push((name, None));
            } else {
                return Err(PatchError::InvalidFrame);
            }
        }
        if !committed {
            return Err(PatchError::InvalidFrame);
        }

        validate_semantics(kind, &records)?;
        let operations = ordered_records
            .into_iter()
            .map(|(name, value)| {
                let name = canonical_name(name).ok_or(PatchError::InvalidFrame)?;
                Ok(match value {
                    Some(value) => Operation::Set {
                        name,
                        value: OsString::from(value),
                    },
                    None => Operation::Unset { name },
                })
            })
            .collect::<Result<Vec<_>, PatchError>>()?;
        Ok(Self { kind, operations })
    }
}

fn validate_value(field: &'static str, value: &str) -> Result<(), PatchError> {
    if value.is_empty() {
        return Err(PatchError::EmptyValue { field });
    }
    if contains_unsafe_character(value) {
        return Err(PatchError::UnsafeValue { field });
    }
    Ok(())
}

fn contains_unsafe_character(value: &str) -> bool {
    value.chars().any(is_terminal_control)
}

fn is_settable(name: &str) -> bool {
    PROFILE_VARIABLES.contains(&name)
        || REGION_VARIABLES.contains(&name)
        || matches!(name, "AWS_CONFIG_FILE" | "AWS_SHARED_CREDENTIALS_FILE")
}

fn is_unsettable_for(kind: PatchKind, name: &str) -> bool {
    match kind {
        PatchKind::Activate => {
            REGION_VARIABLES.contains(&name) || CredentialVariable::from_name(name).is_some()
        }
        PatchKind::Unset => PROFILE_VARIABLES.contains(&name) || REGION_VARIABLES.contains(&name),
    }
}

fn canonical_name(name: &str) -> Option<&'static str> {
    PROFILE_VARIABLES
        .into_iter()
        .chain(REGION_VARIABLES)
        .chain(["AWS_CONFIG_FILE", "AWS_SHARED_CREDENTIALS_FILE"])
        .find(|candidate| *candidate == name)
        .or_else(|| CredentialVariable::from_name(name).map(CredentialVariable::name))
}

fn validate_semantics(
    kind: PatchKind,
    records: &BTreeMap<&str, Option<&str>>,
) -> Result<(), PatchError> {
    match kind {
        PatchKind::Activate => {
            let profile = records
                .get("AWS_PROFILE")
                .copied()
                .flatten()
                .ok_or(PatchError::InvalidFrame)?;
            if PROFILE_VARIABLES
                .into_iter()
                .any(|name| records.get(name).copied().flatten() != Some(profile))
            {
                return Err(PatchError::InvalidFrame);
            }
            let first_region = records.get("AWS_REGION").ok_or(PatchError::InvalidFrame)?;
            let second_region = records
                .get("AWS_DEFAULT_REGION")
                .ok_or(PatchError::InvalidFrame)?;
            if first_region != second_region {
                return Err(PatchError::InvalidFrame);
            }
            for (&name, value) in records {
                if CredentialVariable::from_name(name).is_some() && value.is_some() {
                    return Err(PatchError::InvalidFrame);
                }
            }
        }
        PatchKind::Unset => {
            if records.len() != PROFILE_VARIABLES.len() + REGION_VARIABLES.len()
                || PROFILE_VARIABLES
                    .into_iter()
                    .chain(REGION_VARIABLES)
                    .any(|name| records.get(name) != Some(&None))
            {
                return Err(PatchError::InvalidFrame);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_preserves_shell_metacharacters_as_data() {
        let profile = "team $() `tick` 'single' \"double\" = 東京";
        let patch = EnvironmentPatch::activate(
            profile,
            Some("ap-northeast-1"),
            Some(Path::new("/tmp/config $() = 東京")),
            None,
            &BTreeSet::from([CredentialVariable::AccessKeyId]),
        )
        .unwrap();
        let encoded = patch.encode().unwrap();
        let decoded = EnvironmentPatch::decode(&encoded).unwrap();

        assert_eq!(decoded.kind(), PatchKind::Activate);
        assert!(decoded.operations().any(|operation| {
            operation
                == OperationRef::Set {
                    name: "AWS_PROFILE",
                    value: OsStr::new(profile),
                }
        }));
        assert!(decoded.operations().any(|operation| {
            operation
                == OperationRef::Unset {
                    name: "AWS_ACCESS_KEY_ID",
                }
        }));
    }

    #[test]
    fn activation_always_updates_profile_and_region_coherently() {
        let patch = EnvironmentPatch::activate("prod", None, None, None, &BTreeSet::new()).unwrap();
        let frame = patch.encode().unwrap();

        assert!(frame.contains("SET AWS_PROFILE=prod\n"));
        assert!(frame.contains("SET AWS_DEFAULT_PROFILE=prod\n"));
        assert!(frame.contains("SET AWSWIT_PROFILE=prod\n"));
        assert!(frame.contains("UNSET AWS_REGION\n"));
        assert!(frame.contains("UNSET AWS_DEFAULT_REGION\n"));
        assert_eq!(EnvironmentPatch::decode(&frame).unwrap(), patch);
    }

    #[test]
    fn rejects_control_and_directional_characters() {
        for value in [
            "bad\nname",
            "bad\tname",
            "bad\u{1b}name",
            "bad\u{061c}name",
            "bad\u{200e}name",
            "bad\u{200f}name",
            "bad\u{202e}name",
            "bad\u{2066}name",
        ] {
            assert!(matches!(
                EnvironmentPatch::activate(value, None, None, None, &BTreeSet::new()),
                Err(PatchError::UnsafeValue { .. })
            ));
        }
    }

    #[test]
    fn decoder_rejects_truncated_duplicate_unknown_and_incoherent_frames() {
        let valid = EnvironmentPatch::activate("prod", None, None, None, &BTreeSet::new())
            .unwrap()
            .encode()
            .unwrap();
        let cases = [
            valid.trim_end_matches("AWSWIT-COMMIT\n").to_owned(),
            valid.replace(
                "SET AWS_PROFILE=prod\n",
                "SET AWS_PROFILE=prod\nSET AWS_PROFILE=other\n",
            ),
            valid.replace("AWSWIT-COMMIT", "SET AWS_UNKNOWN=x\nAWSWIT-COMMIT"),
            valid.replace(
                "SET AWS_DEFAULT_PROFILE=prod",
                "SET AWS_DEFAULT_PROFILE=other",
            ),
            valid.replace(
                "UNSET AWS_DEFAULT_REGION",
                "SET AWS_DEFAULT_REGION=eu-west-1",
            ),
        ];

        for frame in cases {
            assert_eq!(
                EnvironmentPatch::decode(&frame),
                Err(PatchError::InvalidFrame),
                "unexpectedly accepted {frame:?}"
            );
        }
    }

    #[test]
    fn unset_frame_has_only_owned_profile_and_region_variables() {
        let patch = EnvironmentPatch::unset();
        let frame = patch.encode().unwrap();
        assert_eq!(EnvironmentPatch::decode(&frame).unwrap(), patch);
        assert!(!frame.contains("AWS_ACCESS_KEY_ID"));
        assert!(!frame.contains("AWS_CONFIG_FILE"));
    }

    #[test]
    fn activate_frame_cannot_unset_shared_config_paths() {
        let frame = EnvironmentPatch::activate("prod", None, None, None, &BTreeSet::new())
            .unwrap()
            .encode()
            .unwrap()
            .replace(
                "UNSET AWS_REGION",
                "UNSET AWS_CONFIG_FILE\nUNSET AWS_REGION",
            );
        assert_eq!(
            EnvironmentPatch::decode(&frame),
            Err(PatchError::InvalidFrame)
        );
    }
}
