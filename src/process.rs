use std::ffi::OsString;
use std::io;
#[cfg(windows)]
use std::process::Command;

use crate::activation::{EnvironmentPatch, OperationRef};
use crate::error::AppError;

/// A child-only execution plan.  Its constructor requires an already-validated
/// environment patch, keeping safety preflight out of the process adapter.
pub(crate) struct ExecutionPlan {
    executable: OsString,
    arguments: Vec<OsString>,
    patch: EnvironmentPatch,
}

impl ExecutionPlan {
    pub(crate) fn new(
        mut command: Vec<OsString>,
        patch: EnvironmentPatch,
    ) -> Result<Self, AppError> {
        if command.is_empty() {
            return Err(AppError::Internal {
                detail: "an execution plan requires a command",
            });
        }
        let executable = command.remove(0);
        if executable.is_empty() {
            return Err(AppError::ProcessNotFound);
        }
        Ok(Self {
            executable,
            arguments: command,
            patch,
        })
    }

    #[cfg(windows)]
    fn command(&self) -> Result<Command, AppError> {
        if is_windows_batch_file(&self.executable) {
            // `std::process::Command` implicitly routes .bat/.cmd files through
            // cmd.exe.  That would turn untrusted argv into shell syntax and
            // violate the shell-free execution contract.
            return Err(AppError::ProcessDenied);
        }
        let mut command = Command::new(&self.executable);
        command.args(&self.arguments);
        for operation in self.patch.operations() {
            match operation {
                OperationRef::Set { name, value } => {
                    command.env(name, value);
                }
                OperationRef::Unset { name } => {
                    command.env_remove(name);
                }
            }
        }
        Ok(command)
    }

    #[cfg(unix)]
    fn into_unix_parts(self) -> (OsString, Vec<OsString>, EnvironmentPatch) {
        (self.executable, self.arguments, self.patch)
    }
}

/// Execute without a shell. On Unix a successful call does not return because
/// `exec` replaces awswit, naturally preserving streams, exit status, and
/// signal semantics.
#[cfg(unix)]
pub(crate) fn execute(plan: ExecutionPlan) -> Result<u8, AppError> {
    use std::ffi::{CString, OsStr};
    use std::os::unix::ffi::OsStrExt;

    use nix::unistd::execve;

    let (executable, arguments, patch) = plan.into_unix_parts();
    let mut argv = Vec::with_capacity(arguments.len().saturating_add(1));
    // The resolved path selects the inode, while argv[0] remains exactly what
    // the caller supplied, matching direct process-launch semantics.
    argv.push(c_string(&executable)?);
    for argument in arguments {
        argv.push(c_string(&argument)?);
    }

    let mut environment: Vec<(OsString, OsString)> = std::env::vars_os().collect();
    for operation in patch.operations() {
        match operation {
            OperationRef::Set { name, value } => {
                environment.retain(|(key, _)| key != OsStr::new(name));
                environment.push((OsString::from(name), value.to_owned()));
            }
            OperationRef::Unset { name } => {
                environment.retain(|(key, _)| key != OsStr::new(name));
            }
        }
    }
    let environment = environment
        .into_iter()
        .map(|(key, value)| {
            let mut entry = key;
            entry.push("=");
            entry.push(value);
            c_string(&entry)
        })
        .collect::<Result<Vec<_>, AppError>>()?;

    let error = if executable.as_bytes().contains(&b'/') {
        let executable_path = c_string(&executable)?;
        match execve(&executable_path, &argv, &environment) {
            Ok(never) => match never {},
            Err(error) => error,
        }
    } else {
        let Some(search) = std::env::var_os("PATH") else {
            return Err(AppError::ProcessNotFound);
        };
        let mut permission_denied = false;
        let mut terminal_error = None;
        for directory in std::env::split_paths(&search) {
            let candidate = directory.join(&executable);
            let candidate = c_string(candidate.as_os_str())?;
            let error = match execve(&candidate, &argv, &environment) {
                Ok(never) => match never {},
                Err(error) => error,
            };
            match error {
                nix::errno::Errno::EACCES => permission_denied = true,
                nix::errno::Errno::ENOENT | nix::errno::Errno::ENOTDIR => {}
                other => {
                    terminal_error = Some(other);
                    break;
                }
            }
        }
        if let Some(error) = terminal_error {
            error
        } else if permission_denied {
            nix::errno::Errno::EACCES
        } else {
            nix::errno::Errno::ENOENT
        }
    };
    fn c_string(value: &OsStr) -> Result<CString, AppError> {
        CString::new(value.as_bytes()).map_err(|_| AppError::Process {
            source: io::Error::new(
                io::ErrorKind::InvalidInput,
                "executable arguments and environment must not contain NUL",
            ),
        })
    }

    Err(classify_spawn_error(io::Error::from(error)))
}

/// Windows cannot replace the current process using the standard library, so
/// it inherits the console, waits, and forwards the child's numeric exit code.
#[cfg(windows)]
pub(crate) fn execute(plan: ExecutionPlan) -> Result<u8, AppError> {
    let status = plan.command()?.status().map_err(classify_spawn_error)?;
    Ok(portable_windows_exit_code(status.code()))
}

#[cfg(windows)]
fn is_windows_batch_file(executable: &std::ffi::OsStr) -> bool {
    use std::os::windows::ffi::OsStrExt;

    let mut name: Vec<u16> = executable.encode_wide().collect();
    while name
        .last()
        .is_some_and(|unit| *unit == u16::from(b' ') || *unit == u16::from(b'.'))
    {
        name.pop();
    }
    let extension = name
        .iter()
        .rposition(|unit| *unit == b'.' as u16)
        .map_or(&[][..], |index| &name[index + 1..]);
    fn is_extension(actual: &[u16], expected: &[u8]) -> bool {
        actual.len() == expected.len()
            && actual.iter().zip(expected).all(|(actual, expected)| {
                let lowercase = if (*actual >= b'A' as u16) && (*actual <= b'Z' as u16) {
                    actual.saturating_add((b'a' - b'A') as u16)
                } else {
                    *actual
                };
                lowercase == *expected as u16
            })
    }

    is_extension(extension, b"bat") || is_extension(extension, b"cmd")
}

#[cfg(windows)]
fn portable_windows_exit_code(code: Option<i32>) -> u8 {
    match code {
        Some(code @ 0..=255) => code as u8,
        Some(_) | None => 1,
    }
}

fn classify_spawn_error(source: io::Error) -> AppError {
    match source.kind() {
        io::ErrorKind::NotFound => AppError::ProcessNotFound,
        io::ErrorKind::PermissionDenied => AppError::ProcessDenied,
        #[cfg(unix)]
        _ if source.raw_os_error() == Some(nix::errno::Errno::ENOTDIR as i32) => {
            AppError::ProcessNotFound
        }
        #[cfg(unix)]
        _ if source.raw_os_error() == Some(nix::errno::Errno::ENOEXEC as i32) => {
            AppError::ProcessDenied
        }
        #[cfg(windows)]
        _ if matches!(source.raw_os_error(), Some(191 | 192 | 193 | 216)) => {
            // Invalid signature, invalid executable, bad executable format, or
            // machine-type mismatch are all the Windows equivalent of ENOEXEC.
            AppError::ProcessDenied
        }
        _ => AppError::Process { source },
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    #[cfg(windows)]
    use std::ffi::OsStr;

    use super::*;

    #[test]
    fn plan_rejects_an_empty_command() {
        let patch = EnvironmentPatch::activate("test", None, None, None, &BTreeSet::new()).unwrap();
        assert!(matches!(
            ExecutionPlan::new(Vec::new(), patch),
            Err(AppError::Internal { .. })
        ));
    }

    #[test]
    fn plan_rejects_an_empty_executable() {
        let patch = EnvironmentPatch::activate("test", None, None, None, &BTreeSet::new()).unwrap();
        assert!(matches!(
            ExecutionPlan::new(vec![OsString::new()], patch),
            Err(AppError::ProcessNotFound)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn unix_non_directory_path_component_is_command_not_found() {
        let classified = classify_spawn_error(io::Error::from_raw_os_error(
            nix::errno::Errno::ENOTDIR as i32,
        ));
        assert!(matches!(classified, AppError::ProcessNotFound));
        assert_eq!(classified.exit_code(), 127);
    }

    #[cfg(windows)]
    #[test]
    fn windows_rejects_implicit_batch_shell_execution() {
        assert!(is_windows_batch_file(OsStr::new("script.bat")));
        assert!(is_windows_batch_file(OsStr::new("SCRIPT.CMD... ")));
        assert!(!is_windows_batch_file(OsStr::new("cmd.exe")));
        assert!(!is_windows_batch_file(OsStr::new("script.ps1")));
    }

    #[cfg(windows)]
    #[test]
    fn windows_exit_codes_never_turn_failure_into_success() {
        assert_eq!(portable_windows_exit_code(Some(0)), 0);
        assert_eq!(portable_windows_exit_code(Some(255)), 255);
        assert_eq!(portable_windows_exit_code(Some(-1)), 1);
        assert_eq!(portable_windows_exit_code(Some(256)), 1);
        assert_eq!(portable_windows_exit_code(None), 1);
    }

    #[cfg(windows)]
    #[test]
    fn windows_invalid_executable_formats_map_to_126() {
        for raw_error in [191, 192, 193, 216] {
            let classified = classify_spawn_error(io::Error::from_raw_os_error(raw_error));
            assert!(matches!(classified, AppError::ProcessDenied));
            assert_eq!(classified.exit_code(), 126);
        }
    }
}
