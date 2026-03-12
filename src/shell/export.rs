use crate::aws::Credentials;

/// Shell-quote a value by wrapping in single quotes and escaping embedded single quotes
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// All environment variable names managed by awswit.
///
/// These must be kept in sync with the variable lists in:
/// - src/init/bash.sh (case statement + AWSWIT_UNSET handler)
/// - src/init/zsh.sh (case statement + AWSWIT_UNSET handler)
/// - src/init/fish.fish (switch statement + AWSWIT_UNSET handler)
/// - src/init/powershell.ps1 (switch statement + AWSWIT_UNSET handler)
const MANAGED_VARS: &[&str] = &[
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "AWS_SECURITY_TOKEN",
    "AWS_REGION",
    "AWS_DEFAULT_REGION",
    "AWS_PROFILE",
    "AWS_DEFAULT_PROFILE",
    "AWSWIT_PROFILE",
    "AWSWIT_EXPIRATION",
];

fn credential_bindings(
    creds: &Credentials,
    profile_name: &str,
) -> Vec<(&'static str, Option<String>)> {
    let expiration_str = creds.expiration.map(|exp| exp.to_rfc3339());

    vec![
        ("AWS_ACCESS_KEY_ID", Some(creds.access_key_id.clone())),
        (
            "AWS_SECRET_ACCESS_KEY",
            Some(creds.secret_access_key.clone()),
        ),
        ("AWS_SESSION_TOKEN", creds.session_token.clone()),
        // AWS_SECURITY_TOKEN is the legacy name for AWS_SESSION_TOKEN.
        // Some older AWS SDKs and tools (e.g., boto2, legacy Java SDK) only read
        // this variable. This can be removed once boto2 usage is negligible.
        ("AWS_SECURITY_TOKEN", creds.session_token.clone()),
        ("AWS_REGION", creds.region.clone()),
        ("AWS_DEFAULT_REGION", creds.region.clone()),
        // Unset AWS_PROFILE and AWS_DEFAULT_PROFILE to prevent conflict with the
        // directly-exported credential environment variables above. If AWS_PROFILE
        // remained set, the AWS SDK would resolve credentials from the named profile
        // in ~/.aws/credentials instead of using the exported env vars.
        ("AWS_PROFILE", None),
        ("AWS_DEFAULT_PROFILE", None),
        ("AWSWIT_PROFILE", Some(profile_name.to_string())),
        ("AWSWIT_EXPIRATION", expiration_str),
    ]
}

/// Handles exporting credentials to shell environment
pub struct ShellExporter {
    shell_type: ShellType,
}

#[derive(Debug, Clone, Copy)]
pub enum ShellType {
    Bash,
    Zsh,
    Fish,
    PowerShell,
    Cmd,
}

impl ShellExporter {
    /// Create a new shell exporter, detecting the current shell
    pub fn new() -> Self {
        let shell_type = Self::detect_shell();
        Self { shell_type }
    }

    /// Create exporter for a specific shell
    pub fn for_shell(shell_type: ShellType) -> Self {
        Self { shell_type }
    }

    /// Detect the current shell type
    fn detect_shell() -> ShellType {
        // Check AWSWIT_SHELL first (set by shell wrapper)
        if let Ok(shell) = std::env::var("AWSWIT_SHELL") {
            return Self::parse_shell_name(&shell);
        }

        // Check SHELL environment variable before PSModulePath — .NET SDK sets
        // PSModulePath on Linux even for bash/zsh users, causing false positives.
        if let Ok(shell) = std::env::var("SHELL") {
            return Self::parse_shell_name(&shell);
        }

        // Check PSModulePath (reliable on Windows / when SHELL is unset)
        if std::env::var("PSModulePath").is_ok() {
            return ShellType::PowerShell;
        }

        // Windows detection
        if cfg!(windows) {
            return ShellType::Cmd;
        }

        // Default to bash
        ShellType::Bash
    }

    fn parse_shell_name(name: &str) -> ShellType {
        let lower = name.to_lowercase();
        if lower.contains("fish") {
            ShellType::Fish
        } else if lower.contains("zsh") {
            ShellType::Zsh
        } else if lower.contains("powershell") || lower.contains("pwsh") {
            ShellType::PowerShell
        } else if lower.contains("cmd") {
            ShellType::Cmd
        } else {
            ShellType::Bash
        }
    }

    /// Generate export commands that can be displayed to the user.
    ///
    /// Values containing newlines or carriage returns are skipped with a warning,
    /// as they could inject additional shell commands.
    pub fn generate_export_commands(
        &self,
        credentials: &Credentials,
        profile_name: &str,
    ) -> String {
        let bindings = credential_bindings(credentials, profile_name);
        let mut output = String::new();

        for (name, value) in &bindings {
            match value {
                Some(val) => {
                    if val.contains('\n') || val.contains('\r') {
                        tracing::warn!(
                            "Skipping export of {} — value contains newline characters",
                            name
                        );
                        continue;
                    }
                    output.push_str(&self.format_set(name, val));
                }
                None => {
                    // Unset variables with no value (e.g., region when new profile has none)
                    output.push_str(&self.format_unset(name));
                }
            }
        }

        output
    }

    /// Generate output for shell wrapper to eval
    ///
    /// Values are validated to reject newlines and carriage returns, which could
    /// inject extra KEY=VALUE lines and corrupt the shell wrapper's parsing.
    pub fn generate_shell_output(
        &self,
        credentials: &Credentials,
        profile_name: &str,
    ) -> Result<String, crate::error::AwswitError> {
        // Validate profile_name for newlines/carriage returns (same check as credential values)
        if profile_name.contains('\n') || profile_name.contains('\r') {
            return Err(crate::error::AwswitError::ShellError {
                message:
                    "Profile name contains newline characters, which is not allowed in shell output"
                        .to_string(),
            });
        }

        let bindings = credential_bindings(credentials, profile_name);
        let mut output = String::new();

        for (name, value) in &bindings {
            match value {
                Some(val) => {
                    if val.contains('\n') || val.contains('\r') {
                        return Err(crate::error::AwswitError::ShellError {
                            message: format!(
                                "Value for {} contains newline characters, which is not allowed in shell output",
                                name
                            ),
                        });
                    }
                    output.push_str(&format!("{}={}\n", name, val));
                }
                None => {
                    // Empty value signals unset to shell wrapper
                    output.push_str(&format!("{}=\n", name));
                }
            }
        }

        Ok(output)
    }

    /// Generate unset commands for display
    pub fn generate_unset_commands(&self) -> String {
        let mut output = String::new();
        for var in MANAGED_VARS {
            output.push_str(&self.format_unset(var));
        }
        output
    }

    /// Generate unset output for shell wrapper
    pub fn generate_unset_output(&self) -> String {
        "AWSWIT_UNSET=1\n".to_string()
    }

    /// Format a set/export command for the detected shell
    fn format_set(&self, name: &str, value: &str) -> String {
        match self.shell_type {
            ShellType::Bash | ShellType::Zsh => {
                format!("export {}={}\n", name, shell_quote(value))
            }
            ShellType::Fish => {
                format!("set -gx {} {}\n", name, shell_quote(value))
            }
            ShellType::PowerShell => {
                let ps_value = format!("'{}'", value.replace('\'', "''"));
                format!("$env:{} = {}\n", name, ps_value)
            }
            ShellType::Cmd => {
                let escaped = value
                    .replace('^', "^^")
                    .replace('&', "^&")
                    .replace('|', "^|")
                    .replace('<', "^<")
                    .replace('>', "^>")
                    .replace('%', "%%");
                format!("set {}={}\n", name, escaped)
            }
        }
    }

    /// Format an unset command for the detected shell
    fn format_unset(&self, name: &str) -> String {
        match self.shell_type {
            ShellType::Bash | ShellType::Zsh => {
                format!("unset {}\n", name)
            }
            ShellType::Fish => {
                format!("set -e {}\n", name)
            }
            ShellType::PowerShell => {
                format!("Remove-Item Env:\\{} -ErrorAction SilentlyContinue\n", name)
            }
            ShellType::Cmd => {
                format!("set {}=\n", name)
            }
        }
    }
}

impl Default for ShellExporter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn test_credentials() -> Credentials {
        Credentials {
            access_key_id: "AKIATEST123".to_string(),
            secret_access_key: "secretkey123".to_string(),
            session_token: Some("token123".to_string()),
            expiration: Some(Utc::now()),
            region: Some("us-west-2".to_string()),
        }
    }

    #[test]
    fn test_posix_export() {
        let exporter = ShellExporter::for_shell(ShellType::Bash);
        let creds = test_credentials();
        let output = exporter.generate_export_commands(&creds, "test-profile");

        assert!(output.contains("export AWS_ACCESS_KEY_ID='AKIATEST123'"));
        assert!(output.contains("export AWS_SECRET_ACCESS_KEY='secretkey123'"));
        assert!(output.contains("export AWS_SESSION_TOKEN='token123'"));
        assert!(output.contains("export AWS_REGION='us-west-2'"));
    }

    #[test]
    fn test_fish_export() {
        let exporter = ShellExporter::for_shell(ShellType::Fish);
        let creds = test_credentials();
        let output = exporter.generate_export_commands(&creds, "test-profile");

        assert!(output.contains("set -gx AWS_ACCESS_KEY_ID 'AKIATEST123'"));
    }

    #[test]
    fn test_powershell_export() {
        let exporter = ShellExporter::for_shell(ShellType::PowerShell);
        let creds = test_credentials();
        let output = exporter.generate_export_commands(&creds, "test-profile");

        assert!(output.contains("$env:AWS_ACCESS_KEY_ID = 'AKIATEST123'"));
    }

    #[test]
    fn test_no_region_emits_unset() {
        let creds = Credentials {
            access_key_id: "AKIATEST123".to_string(),
            secret_access_key: "secretkey123".to_string(),
            session_token: None,
            expiration: None,
            region: None,
        };

        let exporter = ShellExporter::for_shell(ShellType::Bash);
        let output = exporter.generate_export_commands(&creds, "test");
        assert!(output.contains("unset AWS_REGION"));
        assert!(output.contains("unset AWS_DEFAULT_REGION"));
        assert!(output.contains("unset AWS_SESSION_TOKEN"));
        assert!(output.contains("unset AWS_SECURITY_TOKEN"));

        let exporter = ShellExporter::for_shell(ShellType::Fish);
        let output = exporter.generate_export_commands(&creds, "test");
        assert!(output.contains("set -e AWS_REGION"));

        let exporter = ShellExporter::for_shell(ShellType::PowerShell);
        let output = exporter.generate_export_commands(&creds, "test");
        assert!(output.contains("Remove-Item Env:\\AWS_REGION"));
    }

    #[test]
    fn test_unset_commands_all_shells() {
        for shell in [
            ShellType::Bash,
            ShellType::Fish,
            ShellType::PowerShell,
            ShellType::Cmd,
        ] {
            let exporter = ShellExporter::for_shell(shell);
            let output = exporter.generate_unset_commands();
            // All managed vars should appear
            for var in MANAGED_VARS {
                assert!(output.contains(var), "Missing {} in {:?} unset", var, shell);
            }
        }
    }

    #[test]
    fn test_cmd_special_chars_escaped() {
        let exporter = ShellExporter::for_shell(ShellType::Cmd);
        let output = exporter.format_set("TEST", "val&ue|with<special>chars^and%percent");
        assert!(output.contains("^&"));
        assert!(output.contains("^|"));
        assert!(output.contains("^<"));
        assert!(output.contains("^>"));
        assert!(output.contains("^^"));
        assert!(output.contains("%%"));
    }

    #[test]
    fn test_powershell_single_quote_escaped() {
        let exporter = ShellExporter::for_shell(ShellType::PowerShell);
        let output = exporter.format_set("TEST", "it's a test");
        // PowerShell escapes single quotes by doubling them
        assert!(output.contains("it''s a test"));
    }

    #[test]
    fn test_bash_single_quote_escaped() {
        let exporter = ShellExporter::for_shell(ShellType::Bash);
        let output = exporter.format_set("TEST", "it's a test");
        // Shell escapes by ending quote, backslash quote, resume quote
        assert!(output.contains("'\\''"));
    }

    #[test]
    fn test_fish_export_format() {
        let exporter = ShellExporter::for_shell(ShellType::Fish);
        let output = exporter.format_set("TEST", "value");
        assert_eq!(output, "set -gx TEST 'value'\n");
    }

    #[test]
    fn test_shell_output_rejects_newline_in_value() {
        let creds = Credentials {
            access_key_id: "AKIATEST".to_string(),
            secret_access_key: "secret\ninjected".to_string(),
            session_token: None,
            expiration: None,
            region: None,
        };
        let exporter = ShellExporter::for_shell(ShellType::Bash);
        let result = exporter.generate_shell_output(&creds, "test");
        assert!(result.is_err());
    }

    #[test]
    fn test_init_scripts_contain_all_managed_vars() {
        let bash = include_str!("../init/bash.sh");
        let zsh = include_str!("../init/zsh.sh");
        let fish = include_str!("../init/fish.fish");
        let ps1 = include_str!("../init/powershell.ps1");

        for var in MANAGED_VARS {
            assert!(bash.contains(var), "bash.sh missing {}", var);
            assert!(zsh.contains(var), "zsh.sh missing {}", var);
            assert!(fish.contains(var), "fish.fish missing {}", var);
            assert!(ps1.contains(var), "powershell.ps1 missing {}", var);
        }
    }

    #[test]
    fn test_export_commands_skips_newline_value() {
        let creds = Credentials {
            access_key_id: "AKIATEST".to_string(),
            secret_access_key: "secret\ninjected".to_string(),
            session_token: None,
            expiration: None,
            region: None,
        };
        let exporter = ShellExporter::for_shell(ShellType::Bash);
        let output = exporter.generate_export_commands(&creds, "test");
        // Should skip the injected value, not include it
        assert!(!output.contains("injected"));
    }

    #[test]
    fn test_shell_output_handles_utf8_profile_name() {
        let creds = Credentials {
            access_key_id: "AKIATEST".to_string(),
            secret_access_key: "secret".to_string(),
            session_token: None,
            expiration: None,
            region: None,
        };
        let exporter = ShellExporter::for_shell(ShellType::Bash);
        let result = exporter.generate_shell_output(&creds, "test-\u{65E5}\u{672C}\u{8A9E}");
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.contains("AWSWIT_PROFILE=test-\u{65E5}\u{672C}\u{8A9E}"));
    }

    #[test]
    fn test_shell_output_handles_long_session_token() {
        let long_token = "A".repeat(1000);
        let creds = Credentials {
            access_key_id: "AKIATEST".to_string(),
            secret_access_key: "secret".to_string(),
            session_token: Some(long_token.clone()),
            expiration: None,
            region: None,
        };
        let exporter = ShellExporter::for_shell(ShellType::Bash);
        let result = exporter.generate_shell_output(&creds, "test");
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.contains(&format!("AWS_SESSION_TOKEN={}", long_token)));
    }

    #[test]
    fn test_shell_output_rejects_carriage_return_in_profile() {
        let creds = Credentials {
            access_key_id: "AKIATEST".to_string(),
            secret_access_key: "secret".to_string(),
            session_token: None,
            expiration: None,
            region: None,
        };
        let exporter = ShellExporter::for_shell(ShellType::Bash);
        let result = exporter.generate_shell_output(&creds, "test\rinjected");
        assert!(result.is_err());
    }

    #[test]
    fn test_shell_output_no_version_header() {
        let creds = Credentials {
            access_key_id: "AKIATEST".to_string(),
            secret_access_key: "secret".to_string(),
            session_token: None,
            expiration: None,
            region: None,
        };
        let exporter = ShellExporter::for_shell(ShellType::Bash);
        let output = exporter.generate_shell_output(&creds, "test").unwrap();
        assert!(
            !output.contains("AWSWIT_VERSION="),
            "Shell output should not contain version header"
        );
    }

    #[test]
    fn test_unset_output() {
        let exporter = ShellExporter::for_shell(ShellType::Bash);
        let output = exporter.generate_unset_output();
        assert!(!output.contains("AWSWIT_VERSION="));
        assert!(output.contains("AWSWIT_UNSET=1"));
    }

    #[test]
    fn test_shell_quote_empty_string() {
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn test_shell_quote_with_single_quote() {
        let result = shell_quote("it's");
        assert_eq!(result, "'it'\\''s'");
    }

    #[test]
    fn test_shell_quote_consecutive_single_quotes() {
        let result = shell_quote("a''b");
        assert_eq!(result, "'a'\\'''\\''b'");
    }

    #[test]
    fn test_shell_quote_special_chars() {
        let result = shell_quote("$HOME");
        // Should be wrapped in single quotes, preventing expansion
        assert_eq!(result, "'$HOME'");

        let result = shell_quote("$(whoami)");
        assert_eq!(result, "'$(whoami)'");

        let result = shell_quote("foo bar");
        assert_eq!(result, "'foo bar'");

        let result = shell_quote("a`cmd`b");
        assert_eq!(result, "'a`cmd`b'");
    }
}
