/// Shell-quote a value by wrapping in single quotes and escaping embedded single quotes
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Validate that a value is safe for shell output (no newlines or carriage returns).
fn validate_shell_value(label: &str, value: &str) -> Result<(), crate::error::AwswitError> {
    if value.contains('\n') || value.contains('\r') {
        return Err(crate::error::AwswitError::ShellError {
            message: format!(
                "{} contains newline characters, which is not allowed in shell output",
                label
            ),
        });
    }
    Ok(())
}

/// All environment variable names managed by awswit.
///
/// These must be kept in sync with the variable lists in:
/// - src/init/bash.sh (case statement + AWSWIT_UNSET handler)
/// - src/init/zsh.sh (case statement + AWSWIT_UNSET handler)
/// - src/init/fish.fish (switch statement + AWSWIT_UNSET handler)
/// - src/init/powershell.ps1 (switch statement + AWSWIT_UNSET handler)
const MANAGED_VARS: &[&str] = &[
    "AWS_PROFILE",
    "AWS_DEFAULT_PROFILE",
    "AWS_REGION",
    "AWS_DEFAULT_REGION",
    "AWSWIT_PROFILE",
];

/// Bindings for profile selection output.
fn profile_bindings(
    profile_name: &str,
    region: Option<&str>,
) -> Vec<(&'static str, Option<String>)> {
    vec![
        ("AWS_PROFILE", Some(profile_name.to_string())),
        ("AWS_DEFAULT_PROFILE", Some(profile_name.to_string())),
        ("AWS_REGION", region.map(|r| r.to_string())),
        ("AWS_DEFAULT_REGION", region.map(|r| r.to_string())),
        ("AWSWIT_PROFILE", Some(profile_name.to_string())),
    ]
}

/// Handles exporting profile selection to shell environment
pub struct ShellExporter {
    shell_type: ShellType,
}

#[derive(Debug, Clone, Copy)]
pub enum ShellType {
    Bash,
    Zsh,
    Fish,
    PowerShell,
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
        if let Ok(shell) = std::env::var("AWSWIT_SHELL") {
            return Self::parse_shell_name(&shell);
        }

        if let Ok(shell) = std::env::var("SHELL") {
            return Self::parse_shell_name(&shell);
        }

        if std::env::var("PSModulePath").is_ok() {
            return ShellType::PowerShell;
        }

        if cfg!(windows) {
            return ShellType::PowerShell;
        }

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
        } else {
            if !lower.contains("bash") && !lower.contains("sh") {
                tracing::warn!("Unknown shell '{}', falling back to Bash", name);
            }
            ShellType::Bash
        }
    }

    /// Generate bindings with validation and a formatting function.
    fn generate_bindings<F>(
        profile_name: &str,
        region: Option<&str>,
        format_fn: F,
    ) -> Result<String, crate::error::AwswitError>
    where
        F: Fn(&str, Option<&str>) -> String,
    {
        validate_shell_value("Profile name", profile_name)?;
        if let Some(r) = region {
            validate_shell_value("Region", r)?;
        }

        let bindings = profile_bindings(profile_name, region);
        let mut output = String::new();
        for (name, value) in &bindings {
            output.push_str(&format_fn(name, value.as_deref()));
        }
        Ok(output)
    }

    /// Generate export commands that can be displayed to the user (--show-commands).
    pub fn generate_export_commands(
        &self,
        profile_name: &str,
        region: Option<&str>,
    ) -> Result<String, crate::error::AwswitError> {
        Self::generate_bindings(profile_name, region, |name, value| match value {
            Some(val) => self.format_set(name, val),
            None => self.format_unset(name),
        })
    }

    /// Generate output for shell wrapper to eval.
    ///
    /// Values are validated to reject newlines and carriage returns, which could
    /// inject extra KEY=VALUE lines and corrupt the shell wrapper's parsing.
    pub fn generate_shell_output(
        &self,
        profile_name: &str,
        region: Option<&str>,
    ) -> Result<String, crate::error::AwswitError> {
        Self::generate_bindings(profile_name, region, |name, value| match value {
            Some(val) => format!("{}={}\n", name, val),
            None => format!("{}=\n", name),
        })
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

    #[test]
    fn test_posix_export() {
        let exporter = ShellExporter::for_shell(ShellType::Bash);
        let output = exporter
            .generate_export_commands("test-profile", Some("us-west-2"))
            .unwrap();

        assert!(output.contains("export AWS_PROFILE='test-profile'"));
        assert!(output.contains("export AWS_DEFAULT_PROFILE='test-profile'"));
        assert!(output.contains("export AWS_REGION='us-west-2'"));
        assert!(output.contains("export AWSWIT_PROFILE='test-profile'"));
    }

    #[test]
    fn test_fish_export() {
        let exporter = ShellExporter::for_shell(ShellType::Fish);
        let output = exporter
            .generate_export_commands("test-profile", Some("us-west-2"))
            .unwrap();

        assert!(output.contains("set -gx AWS_PROFILE 'test-profile'"));
    }

    #[test]
    fn test_powershell_export() {
        let exporter = ShellExporter::for_shell(ShellType::PowerShell);
        let output = exporter
            .generate_export_commands("test-profile", Some("us-west-2"))
            .unwrap();

        assert!(output.contains("$env:AWS_PROFILE = 'test-profile'"));
    }

    #[test]
    fn test_no_region_emits_unset() {
        let exporter = ShellExporter::for_shell(ShellType::Bash);
        let output = exporter.generate_export_commands("test", None).unwrap();
        assert!(output.contains("unset AWS_REGION"));
        assert!(output.contains("unset AWS_DEFAULT_REGION"));

        let exporter = ShellExporter::for_shell(ShellType::Fish);
        let output = exporter.generate_export_commands("test", None).unwrap();
        assert!(output.contains("set -e AWS_REGION"));

        let exporter = ShellExporter::for_shell(ShellType::PowerShell);
        let output = exporter.generate_export_commands("test", None).unwrap();
        assert!(output.contains("Remove-Item Env:\\AWS_REGION"));
    }

    #[test]
    fn test_unset_commands_all_shells() {
        for shell in [ShellType::Bash, ShellType::Fish, ShellType::PowerShell] {
            let exporter = ShellExporter::for_shell(shell);
            let output = exporter.generate_unset_commands();
            for var in MANAGED_VARS {
                assert!(output.contains(var), "Missing {} in {:?} unset", var, shell);
            }
        }
    }

    #[test]
    fn test_shell_output_format() {
        let exporter = ShellExporter::for_shell(ShellType::Bash);
        let output = exporter
            .generate_shell_output("prod", Some("ap-northeast-1"))
            .unwrap();
        assert!(output.contains("AWS_PROFILE=prod\n"));
        assert!(output.contains("AWS_DEFAULT_PROFILE=prod\n"));
        assert!(output.contains("AWS_REGION=ap-northeast-1\n"));
        assert!(output.contains("AWS_DEFAULT_REGION=ap-northeast-1\n"));
        assert!(output.contains("AWSWIT_PROFILE=prod\n"));
    }

    #[test]
    fn test_shell_output_no_region() {
        let exporter = ShellExporter::for_shell(ShellType::Bash);
        let output = exporter.generate_shell_output("prod", None).unwrap();
        assert!(output.contains("AWS_PROFILE=prod\n"));
        assert!(output.contains("AWS_REGION=\n"));
    }

    #[test]
    fn test_shell_output_rejects_newline_in_profile() {
        let exporter = ShellExporter::for_shell(ShellType::Bash);
        let result = exporter.generate_shell_output("test\ninjected", None);
        assert!(result.is_err());
    }

    #[test]
    fn test_shell_output_rejects_carriage_return_in_profile() {
        let exporter = ShellExporter::for_shell(ShellType::Bash);
        let result = exporter.generate_shell_output("test\rinjected", None);
        assert!(result.is_err());
    }

    #[test]
    fn test_shell_output_rejects_newline_in_region() {
        let exporter = ShellExporter::for_shell(ShellType::Bash);
        let result = exporter.generate_shell_output("prod", Some("us-east-1\nMALICIOUS=evil"));
        assert!(result.is_err());
    }

    #[test]
    fn test_export_commands_rejects_newline_in_profile() {
        let exporter = ShellExporter::for_shell(ShellType::Bash);
        let result = exporter.generate_export_commands("test\ninjected", None);
        assert!(result.is_err());
    }

    #[test]
    fn test_export_commands_rejects_newline_in_region() {
        let exporter = ShellExporter::for_shell(ShellType::Bash);
        let result = exporter.generate_export_commands("prod", Some("us-east-1\nMALICIOUS=evil"));
        assert!(result.is_err());
    }

    #[test]
    fn test_unset_output() {
        let exporter = ShellExporter::for_shell(ShellType::Bash);
        let output = exporter.generate_unset_output();
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
    fn test_shell_quote_special_chars() {
        let result = shell_quote("$HOME");
        assert_eq!(result, "'$HOME'");
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
    fn test_powershell_single_quote_escaped() {
        let exporter = ShellExporter::for_shell(ShellType::PowerShell);
        let output = exporter.format_set("TEST", "it's a test");
        assert!(output.contains("it''s a test"));
    }

    #[test]
    fn test_bash_single_quote_escaped() {
        let exporter = ShellExporter::for_shell(ShellType::Bash);
        let output = exporter.format_set("TEST", "it's a test");
        assert!(output.contains("'\\''"));
    }

    #[test]
    fn test_command_substitution_characters_are_quoted() {
        let exporter = ShellExporter::for_shell(ShellType::Bash);
        // $() and backticks should be safely quoted inside single quotes
        let output = exporter
            .generate_export_commands("$(whoami)", Some("us-east-1"))
            .unwrap();
        assert!(output.contains("'$(whoami)'"));

        let output = exporter.generate_export_commands("`whoami`", None).unwrap();
        assert!(output.contains("'`whoami`'"));
    }

    #[test]
    fn test_unicode_profile_name() {
        let exporter = ShellExporter::for_shell(ShellType::Bash);
        let output = exporter
            .generate_export_commands("プロファイル", Some("ap-northeast-1"))
            .unwrap();
        assert!(output.contains("'プロファイル'"));
    }

    #[test]
    fn test_long_profile_name() {
        let long_name: String = "a".repeat(256);
        let exporter = ShellExporter::for_shell(ShellType::Bash);
        let output = exporter.generate_export_commands(&long_name, None).unwrap();
        assert!(output.contains(&long_name));
    }
}
