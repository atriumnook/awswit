use std::collections::HashMap;
use std::io::Write;
use std::process::{Command, Stdio};

use crate::error::AwswitError;
use crate::history::ProfileHistory;
use crate::profile::Profile;

/// Check if a string value is "truthy" (1, true, yes, on — case insensitive).
pub fn is_truthy(val: &str) -> bool {
    ["1", "true", "yes", "on"]
        .iter()
        .any(|candidate| val.eq_ignore_ascii_case(candidate))
}

/// Options that allow arbitrary command execution via fzf and must be blocked.
const BLOCKED_FZF_OPTIONS: &[&str] = &[
    "--preview",
    "--bind",
    "--execute",
    "--execute-silent",
    "--reload",
    "--transform",
    "--preview-window", // can contain execute(...) action
];

/// Check if a token is a blocked fzf option (handles both `--opt` and `--opt=value` forms).
fn is_blocked_fzf_option(token: &str) -> bool {
    let normalized = token.to_ascii_lowercase();
    BLOCKED_FZF_OPTIONS
        .iter()
        .any(|blocked| normalized == *blocked || normalized.starts_with(&format!("{}=", blocked)))
}

fn build_fzf_args(extra_opts: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "--prompt".to_string(),
        "AWS Profile> ".to_string(),
        "--height".to_string(),
        "40%".to_string(),
        "--reverse".to_string(),
        "--no-sort".to_string(),
    ];

    if let Some(opts) = extra_opts {
        for token in opts.split_whitespace() {
            if is_blocked_fzf_option(token) {
                tracing::warn!(
                    "Ignoring blocked fzf option from AWSWIT_FZF_OPTS: {}",
                    token
                );
                continue;
            }
            args.push(token.to_string());
        }
    }

    args
}

/// Select a profile using external fzf.
pub fn select_with_fzf(
    profiles: &HashMap<String, Profile>,
    history: &ProfileHistory,
) -> Result<String, AwswitError> {
    let now = chrono::Utc::now();

    // Sort: favorite first, then frecency desc, then name asc
    let mut names: Vec<&String> = profiles.keys().collect();
    names.sort_by(|a, b| history.compare_by_frecency(a, b, now));

    let input: String = names.iter().map(|n| format!("{}\n", n)).collect();

    let fzf_args = build_fzf_args(std::env::var("AWSWIT_FZF_OPTS").ok().as_deref());

    let mut child = Command::new("fzf")
        .args(&fzf_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                AwswitError::ShellError {
                    message: "fzf not found in PATH. Install fzf or use the default picker."
                        .to_string(),
                }
            } else {
                AwswitError::ShellError {
                    message: format!("Failed to start fzf: {}", e),
                }
            }
        })?;

    // stdin is dropped here after write_all, closing the pipe and sending EOF to fzf.
    // This must happen before wait_with_output() to avoid deadlock.
    if let Some(mut stdin) = child.stdin.take()
        && let Err(e) = stdin.write_all(input.as_bytes())
    {
        tracing::warn!("Failed to write to fzf stdin: {}", e);
    }

    let output = child
        .wait_with_output()
        .map_err(|e| AwswitError::ShellError {
            message: format!("Failed to wait for fzf: {}", e),
        })?;

    match output.status.code() {
        Some(0) => {
            let selected = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if selected.is_empty() {
                Err(AwswitError::UserCancelled)
            } else {
                Ok(selected)
            }
        }
        Some(130) | Some(1) => Err(AwswitError::UserCancelled),
        Some(code) => Err(AwswitError::ShellError {
            message: format!("fzf exited with code {}", code),
        }),
        None => Err(AwswitError::ShellError {
            message: "fzf terminated by signal".to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_truthy_positive() {
        assert!(is_truthy("1"));
        assert!(is_truthy("true"));
        assert!(is_truthy("TRUE"));
        assert!(is_truthy("True"));
        assert!(is_truthy("yes"));
        assert!(is_truthy("YES"));
        assert!(is_truthy("on"));
        assert!(is_truthy("ON"));
    }

    #[test]
    fn is_truthy_negative() {
        assert!(!is_truthy("0"));
        assert!(!is_truthy("false"));
        assert!(!is_truthy("no"));
        assert!(!is_truthy("off"));
        assert!(!is_truthy(""));
        assert!(!is_truthy("maybe"));
        assert!(!is_truthy("2"));
    }

    #[test]
    fn build_fzf_args_without_extra_opts() {
        assert_eq!(
            build_fzf_args(None),
            vec![
                "--prompt",
                "AWS Profile> ",
                "--height",
                "40%",
                "--reverse",
                "--no-sort",
            ]
        );
    }

    #[test]
    fn build_fzf_args_with_extra_opts() {
        assert_eq!(
            build_fzf_args(Some("--ansi --cycle")),
            vec![
                "--prompt",
                "AWS Profile> ",
                "--height",
                "40%",
                "--reverse",
                "--no-sort",
                "--ansi",
                "--cycle",
            ]
        );
    }

    #[test]
    fn build_fzf_args_blocks_dangerous_options() {
        // --preview, --bind, --execute etc. should be stripped
        let args = build_fzf_args(Some(
            "--ansi --preview 'cat {}' --bind 'enter:execute(rm -rf /)' --cycle",
        ));
        assert!(args.contains(&"--ansi".to_string()));
        assert!(args.contains(&"--cycle".to_string()));
        assert!(!args.iter().any(|a| a.starts_with("--preview")));
        assert!(!args.iter().any(|a| a.starts_with("--bind")));
    }

    #[test]
    fn build_fzf_args_blocks_option_with_equals() {
        let args = build_fzf_args(Some("--preview=cat --bind=enter:abort"));
        assert!(!args.iter().any(|a| a.starts_with("--preview")));
        assert!(!args.iter().any(|a| a.starts_with("--bind")));
    }

    #[test]
    fn is_blocked_fzf_option_cases() {
        assert!(is_blocked_fzf_option("--preview"));
        assert!(is_blocked_fzf_option("--PREVIEW"));
        assert!(is_blocked_fzf_option("--preview=cat"));
        assert!(is_blocked_fzf_option("--bind"));
        assert!(is_blocked_fzf_option("--execute"));
        assert!(is_blocked_fzf_option("--execute-silent"));
        assert!(is_blocked_fzf_option("--reload"));
        assert!(is_blocked_fzf_option("--transform"));
        assert!(!is_blocked_fzf_option("--ansi"));
        assert!(!is_blocked_fzf_option("--height"));
        assert!(!is_blocked_fzf_option("--cycle"));
    }
}
