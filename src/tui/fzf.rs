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
    names.sort_by(|a, b| {
        let a_entry = history.get(a);
        let b_entry = history.get(b);
        let a_fav = a_entry.map(|e| e.is_favorite).unwrap_or(false);
        let b_fav = b_entry.map(|e| e.is_favorite).unwrap_or(false);

        b_fav
            .cmp(&a_fav)
            .then_with(|| {
                let a_score = a_entry.map(|e| e.frecency_score(now)).unwrap_or(0.0);
                let b_score = b_entry.map(|e| e.frecency_score(now)).unwrap_or(0.0);
                b_score
                    .partial_cmp(&a_score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.cmp(b))
    });

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

    if let Some(mut stdin) = child.stdin.take() {
        if let Err(e) = stdin.write_all(input.as_bytes()) {
            tracing::warn!("Failed to write to fzf stdin: {}", e);
        }
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
}
