use crate::aws::credentials::Credentials;

/// Supported shell types
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellType {
    Bash,
    Zsh,
    Fish,
    PowerShell,
}

impl ShellType {
    /// Detect the current shell from environment
    pub fn detect() -> Self {
        if let Ok(shell) = std::env::var("SHELL") {
            if shell.contains("fish") {
                return ShellType::Fish;
            }
            if shell.contains("zsh") {
                return ShellType::Zsh;
            }
        }

        // Check PSModulePath for PowerShell
        if std::env::var("PSModulePath").is_ok() {
            return ShellType::PowerShell;
        }

        // Default to Bash
        ShellType::Bash
    }

    /// Detect from explicit shell name
    pub fn from_name(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            "fish" => ShellType::Fish,
            "zsh" => ShellType::Zsh,
            "powershell" | "pwsh" => ShellType::PowerShell,
            _ => ShellType::Bash,
        }
    }
}

/// Generate export commands for credentials
pub fn generate_export_commands(
    creds: &Credentials,
    profile_name: &str,
    shell: &ShellType,
) -> String {
    let mut lines = Vec::new();

    match shell {
        ShellType::Bash | ShellType::Zsh => {
            lines.push(format!(
                "export AWS_ACCESS_KEY_ID={}",
                creds.access_key_id
            ));
            lines.push(format!(
                "export AWS_SECRET_ACCESS_KEY={}",
                creds.secret_access_key
            ));
            if let Some(ref token) = creds.session_token {
                lines.push(format!("export AWS_SESSION_TOKEN={}", token));
            } else {
                lines.push("unset AWS_SESSION_TOKEN".to_string());
            }
            if let Some(ref region) = creds.region {
                lines.push(format!("export AWS_REGION={}", region));
                lines.push(format!("export AWS_DEFAULT_REGION={}", region));
            }
            lines.push(format!("export AWSWIT_PROFILE={}", profile_name));
            if let Some(ref exp) = creds.expiration {
                lines.push(format!("export AWSWIT_EXPIRATION={}", exp.to_rfc3339()));
            }
        }
        ShellType::Fish => {
            lines.push(format!(
                "set -gx AWS_ACCESS_KEY_ID {}",
                creds.access_key_id
            ));
            lines.push(format!(
                "set -gx AWS_SECRET_ACCESS_KEY {}",
                creds.secret_access_key
            ));
            if let Some(ref token) = creds.session_token {
                lines.push(format!("set -gx AWS_SESSION_TOKEN {}", token));
            } else {
                lines.push("set -e AWS_SESSION_TOKEN".to_string());
            }
            if let Some(ref region) = creds.region {
                lines.push(format!("set -gx AWS_REGION {}", region));
                lines.push(format!("set -gx AWS_DEFAULT_REGION {}", region));
            }
            lines.push(format!("set -gx AWSWIT_PROFILE {}", profile_name));
            if let Some(ref exp) = creds.expiration {
                lines.push(format!("set -gx AWSWIT_EXPIRATION {}", exp.to_rfc3339()));
            }
        }
        ShellType::PowerShell => {
            lines.push(format!(
                "$env:AWS_ACCESS_KEY_ID = '{}'",
                creds.access_key_id
            ));
            lines.push(format!(
                "$env:AWS_SECRET_ACCESS_KEY = '{}'",
                creds.secret_access_key
            ));
            if let Some(ref token) = creds.session_token {
                lines.push(format!("$env:AWS_SESSION_TOKEN = '{}'", token));
            } else {
                lines.push("Remove-Item Env:\\AWS_SESSION_TOKEN -ErrorAction SilentlyContinue".to_string());
            }
            if let Some(ref region) = creds.region {
                lines.push(format!("$env:AWS_REGION = '{}'", region));
                lines.push(format!("$env:AWS_DEFAULT_REGION = '{}'", region));
            }
            lines.push(format!("$env:AWSWIT_PROFILE = '{}'", profile_name));
            if let Some(ref exp) = creds.expiration {
                lines.push(format!("$env:AWSWIT_EXPIRATION = '{}'", exp.to_rfc3339()));
            }
        }
    }

    lines.join("\n")
}

/// Generate unset commands to clear AWS environment variables
pub fn generate_unset_commands(shell: &ShellType) -> String {
    let vars = [
        "AWS_ACCESS_KEY_ID",
        "AWS_SECRET_ACCESS_KEY",
        "AWS_SESSION_TOKEN",
        "AWS_REGION",
        "AWS_DEFAULT_REGION",
        "AWSWIT_PROFILE",
        "AWSWIT_EXPIRATION",
    ];

    let mut lines = Vec::new();

    match shell {
        ShellType::Bash | ShellType::Zsh => {
            for var in &vars {
                lines.push(format!("unset {}", var));
            }
        }
        ShellType::Fish => {
            for var in &vars {
                lines.push(format!("set -e {}", var));
            }
        }
        ShellType::PowerShell => {
            for var in &vars {
                lines.push(format!(
                    "Remove-Item Env:\\{} -ErrorAction SilentlyContinue",
                    var
                ));
            }
        }
    }

    lines.join("\n")
}

/// Generate the shell eval wrapper function
pub fn generate_shell_wrapper(shell: &ShellType) -> String {
    match shell {
        ShellType::Bash | ShellType::Zsh => {
            r#"awswit() {
    local output
    output=$(command awswit "$@")
    local exit_code=$?
    if [ $exit_code -eq 0 ] && [ -n "$output" ]; then
        eval "$output"
    fi
    return $exit_code
}"#
            .to_string()
        }
        ShellType::Fish => {
            r#"function awswit
    set -l output (command awswit $argv)
    set -l exit_code $status
    if test $exit_code -eq 0; and test -n "$output"
        eval $output
    end
    return $exit_code
end"#
            .to_string()
        }
        ShellType::PowerShell => {
            r#"function awswit {
    $output = & (Get-Command awswit -CommandType Application).Source @args
    if ($LASTEXITCODE -eq 0 -and $output) {
        Invoke-Expression ($output -join "`n")
    }
}"#
            .to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_creds() -> Credentials {
        Credentials {
            access_key_id: "AKIAIOSFODNN7EXAMPLE".to_string(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string(),
            session_token: Some("FwoGZXIvYXdzEBY".to_string()),
            expiration: None,
            region: Some("us-east-1".to_string()),
        }
    }

    #[test]
    fn test_bash_export() {
        let output = generate_export_commands(&test_creds(), "dev", &ShellType::Bash);
        assert!(output.contains("export AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE"));
        assert!(output.contains("export AWS_REGION=us-east-1"));
        assert!(output.contains("export AWSWIT_PROFILE=dev"));
    }

    #[test]
    fn test_fish_export() {
        let output = generate_export_commands(&test_creds(), "dev", &ShellType::Fish);
        assert!(output.contains("set -gx AWS_ACCESS_KEY_ID AKIAIOSFODNN7EXAMPLE"));
        assert!(output.contains("set -gx AWS_REGION us-east-1"));
    }

    #[test]
    fn test_powershell_export() {
        let output = generate_export_commands(&test_creds(), "dev", &ShellType::PowerShell);
        assert!(output.contains("$env:AWS_ACCESS_KEY_ID = 'AKIAIOSFODNN7EXAMPLE'"));
    }

    #[test]
    fn test_unset_bash() {
        let output = generate_unset_commands(&ShellType::Bash);
        assert!(output.contains("unset AWS_ACCESS_KEY_ID"));
        assert!(output.contains("unset AWSWIT_PROFILE"));
    }

    #[test]
    fn test_unset_fish() {
        let output = generate_unset_commands(&ShellType::Fish);
        assert!(output.contains("set -e AWS_ACCESS_KEY_ID"));
    }

    #[test]
    fn test_shell_detect() {
        // Just test that it doesn't panic
        let _shell = ShellType::detect();
    }

    #[test]
    fn test_shell_from_name() {
        assert_eq!(ShellType::from_name("fish"), ShellType::Fish);
        assert_eq!(ShellType::from_name("zsh"), ShellType::Zsh);
        assert_eq!(ShellType::from_name("bash"), ShellType::Bash);
        assert_eq!(ShellType::from_name("powershell"), ShellType::PowerShell);
    }
}
