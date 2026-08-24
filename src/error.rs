use std::collections::BTreeSet;
use std::fmt;
use std::io;

use crate::activation::{CredentialVariable, PatchError};
use crate::text_safety::sanitized;

#[derive(Debug)]
pub(crate) enum AppError {
    HookRequired,
    TtyRequired,
    ProfileNotFound {
        requested: String,
    },
    NoProfiles,
    ProfileConfigurationInvalid {
        profile: String,
    },
    CredentialOverride {
        profile: String,
        variables: BTreeSet<CredentialVariable>,
    },
    Catalog {
        detail: String,
    },
    Terminal {
        source: io::Error,
    },
    Patch(PatchError),
    ProcessNotFound,
    ProcessDenied,
    Process {
        source: io::Error,
    },
    Output {
        source: io::Error,
    },
    SourcePathResolution {
        source: io::Error,
    },
    HomeUnavailable,
    Internal {
        detail: &'static str,
    },
}

impl AppError {
    pub(crate) const fn code(&self) -> &'static str {
        match self {
            Self::HookRequired => "HOOK_REQUIRED",
            Self::TtyRequired => "TTY_REQUIRED",
            Self::ProfileNotFound { .. } => "PROFILE_NOT_FOUND",
            Self::NoProfiles => "NO_PROFILES",
            Self::ProfileConfigurationInvalid { .. } => "PROFILE_CONFIG_INVALID",
            Self::CredentialOverride { .. } => "CREDENTIAL_OVERRIDE",
            Self::Catalog { .. } => "CONFIG_READ",
            Self::Terminal { .. } => "TERMINAL_FAILURE",
            Self::Patch(_) => "PATCH_INVALID",
            Self::ProcessNotFound => "COMMAND_NOT_FOUND",
            Self::ProcessDenied => "COMMAND_NOT_EXECUTABLE",
            Self::Process { .. } => "COMMAND_FAILED",
            Self::Output { .. } => "OUTPUT_FAILED",
            Self::SourcePathResolution { .. } => "SOURCE_PATH_INVALID",
            Self::HomeUnavailable => "HOME_UNAVAILABLE",
            Self::Internal { .. } => "INTERNAL",
        }
    }

    pub(crate) const fn exit_code(&self) -> u8 {
        match self {
            Self::ProcessNotFound => 127,
            Self::ProcessDenied => 126,
            _ => 1,
        }
    }

    pub(crate) fn hint(&self) -> Option<&'static str> {
        match self {
            Self::HookRequired => Some(
                "load `awswit init bash|zsh|fish|powershell` for the current shell; use `awswit exec` for one command",
            ),
            Self::TtyRequired => {
                Some("provide an exact profile name or run from an interactive terminal")
            }
            Self::CredentialOverride { .. } => Some(
                "review the variables, then retry with --clear-credential-overrides only if removal is intended",
            ),
            Self::ProfileConfigurationInvalid { .. } => {
                Some("run `awswit doctor` and repair the selected profile or its provider chain")
            }
            Self::ProfileNotFound { .. } => {
                Some("run `awswit list --format names` and pass one exact profile name")
            }
            Self::NoProfiles => Some(
                "check AWS config/credentials paths with `awswit doctor`, then add a valid profile",
            ),
            Self::Catalog { .. } => Some(
                "run `awswit doctor` with the same source options and repair file access or bounded input errors",
            ),
            Self::Terminal { .. } => Some(
                "restore the terminal with `stty sane`, then retry from an interactive terminal",
            ),
            Self::Patch(_) => Some("update the executable and generated shell hook together"),
            Self::ProcessNotFound => Some("check the executable name and inherited PATH"),
            Self::ProcessDenied => Some(
                "use a native executable with execute permission; invoke a shell explicitly only when intended",
            ),
            Self::Process { .. } => Some("check the executable format and operating-system error"),
            Self::Output { .. } => {
                Some("check the output destination and available filesystem or pipe access")
            }
            Self::SourcePathResolution { .. } => Some(
                "use an absolute AWS source path or retry from an accessible working directory",
            ),
            Self::HomeUnavailable => {
                Some("set a valid user home/state directory or pass explicit AWS source paths")
            }
            Self::Internal { .. } => Some("report this invariant failure with the awswit version"),
        }
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HookRequired => formatter
                .write_str("activation cannot modify this parent shell without the awswit hook"),
            Self::TtyRequired => {
                formatter.write_str("interactive selection requires terminal input and display")
            }
            Self::ProfileNotFound { requested } => {
                write!(
                    formatter,
                    "profile {} was not found; exact matching is required",
                    quoted(requested)
                )
            }
            Self::NoProfiles => formatter.write_str("no selectable AWS profiles were found"),
            Self::ProfileConfigurationInvalid { profile } => write!(
                formatter,
                "profile {} or its provider chain has invalid configuration",
                quoted(profile)
            ),
            Self::CredentialOverride { profile, variables } => {
                write!(
                    formatter,
                    "profile {} may not control the AWS identity while these variables are set: ",
                    quoted(profile)
                )?;
                for (index, variable) in variables.iter().enumerate() {
                    if index > 0 {
                        formatter.write_str(", ")?;
                    }
                    formatter.write_str(variable.name())?;
                }
                formatter.write_str("; their values were neither retained nor printed")
            }
            Self::Catalog { detail } => {
                write!(
                    formatter,
                    "could not build the AWS profile catalog: {}",
                    sanitized(detail)
                )
            }
            Self::Terminal { source } => {
                write!(
                    formatter,
                    "interactive terminal failed: {}",
                    sanitized(&source.to_string())
                )
            }
            Self::Patch(source) => write!(
                formatter,
                "could not create a safe activation patch: {source}"
            ),
            Self::ProcessNotFound => formatter.write_str("command executable was not found"),
            Self::ProcessDenied => formatter.write_str("command executable could not be started"),
            Self::Process { source } => {
                write!(
                    formatter,
                    "command failed to start: {}",
                    sanitized(&source.to_string())
                )
            }
            Self::Output { source } => write!(
                formatter,
                "could not write command output: {}",
                sanitized(&source.to_string())
            ),
            Self::SourcePathResolution { source } => write!(
                formatter,
                "could not resolve an AWS source path against the working directory: {}",
                sanitized(&source.to_string())
            ),
            Self::HomeUnavailable => {
                formatter.write_str("the user configuration directory could not be determined")
            }
            Self::Internal { detail } => write!(formatter, "internal invariant failed: {detail}"),
        }
    }
}

impl std::error::Error for AppError {}

impl From<PatchError> for AppError {
    fn from(source: PatchError) -> Self {
        Self::Patch(source)
    }
}

fn quoted(value: &str) -> String {
    format!("\"{}\"", sanitized(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untrusted_names_cannot_forge_terminal_diagnostics() {
        let error = AppError::ProfileNotFound {
            requested: "prod\n\u{1b}[31mforged".to_owned(),
        };
        let rendered = error.to_string();
        assert!(!rendered.contains('\n'));
        assert!(!rendered.contains('\u{1b}'));
        assert!(rendered.contains("\\n"));
        assert!(rendered.contains("\\u{1b}"));
    }
}
