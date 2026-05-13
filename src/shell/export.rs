/// Shell-quote a value by wrapping in single quotes and escaping embedded single quotes.
fn posix_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// PowerShell-quote a value by wrapping in single quotes and escaping embedded ones.
fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Reject values that would break the shell line protocol.
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

/// Environment variables awswit manages.
///
/// We deliberately set only the two variables the modern AWS SDK reads
/// first: `AWS_PROFILE` and `AWS_REGION`. The legacy `AWS_DEFAULT_*`
/// variants are intentionally not touched — the SDK prefers the
/// non-`DEFAULT` versions, and leaving the legacy ones alone avoids
/// surprising users who set them by hand.
pub const MANAGED_VARS: &[&str] = &["AWS_PROFILE", "AWS_REGION"];

#[derive(Debug, Clone, Copy)]
pub enum ShellType {
    Bash,
    Zsh,
    Fish,
    PowerShell,
}

impl ShellType {
    pub fn detect() -> Self {
        if let Ok(name) = std::env::var("AWSWIT_SHELL") {
            return Self::from_name(&name);
        }
        if let Ok(name) = std::env::var("SHELL") {
            return Self::from_name(&name);
        }
        if std::env::var("PSModulePath").is_ok() || cfg!(windows) {
            return ShellType::PowerShell;
        }
        ShellType::Bash
    }

    fn from_name(name: &str) -> Self {
        let lower = name.to_lowercase();
        if lower.contains("fish") {
            ShellType::Fish
        } else if lower.contains("zsh") {
            ShellType::Zsh
        } else if lower.contains("powershell") || lower.contains("pwsh") {
            ShellType::PowerShell
        } else {
            ShellType::Bash
        }
    }
}

/// Emit shell commands to set / unset awswit-managed environment variables.
///
/// Output is exclusively `set` / `unset` statements safe for `eval` — one per
/// line. Status messages are the caller's job to write to stderr.
pub struct ShellExporter {
    shell: ShellType,
}

impl ShellExporter {
    pub fn new() -> Self {
        Self {
            shell: ShellType::detect(),
        }
    }

    pub fn for_shell(shell: ShellType) -> Self {
        Self { shell }
    }

    /// Emit `set` commands for the selected profile and (optional) region.
    pub fn export(
        &self,
        profile: &str,
        region: Option<&str>,
    ) -> Result<String, crate::error::AwswitError> {
        validate_shell_value("Profile name", profile)?;
        if let Some(r) = region {
            validate_shell_value("Region", r)?;
        }

        let mut out = String::new();
        out.push_str(&self.set("AWS_PROFILE", profile));
        match region {
            Some(r) => out.push_str(&self.set("AWS_REGION", r)),
            None => out.push_str(&self.unset("AWS_REGION")),
        }
        Ok(out)
    }

    /// Emit `unset` commands for every managed variable.
    pub fn unset_all(&self) -> String {
        let mut out = String::new();
        for var in MANAGED_VARS {
            out.push_str(&self.unset(var));
        }
        out
    }

    fn set(&self, name: &str, value: &str) -> String {
        match self.shell {
            ShellType::Bash | ShellType::Zsh => {
                format!("export {}={}\n", name, posix_quote(value))
            }
            ShellType::Fish => format!("set -gx {} {}\n", name, posix_quote(value)),
            ShellType::PowerShell => format!("$env:{} = {}\n", name, ps_quote(value)),
        }
    }

    fn unset(&self, name: &str) -> String {
        match self.shell {
            ShellType::Bash | ShellType::Zsh => format!("unset {}\n", name),
            ShellType::Fish => format!("set -e {}\n", name),
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
    fn export_bash_with_region() {
        let out = ShellExporter::for_shell(ShellType::Bash)
            .export("prod", Some("ap-northeast-1"))
            .unwrap();
        assert_eq!(
            out,
            "export AWS_PROFILE='prod'\nexport AWS_REGION='ap-northeast-1'\n"
        );
    }

    #[test]
    fn export_bash_without_region_unsets_region() {
        let out = ShellExporter::for_shell(ShellType::Bash)
            .export("prod", None)
            .unwrap();
        assert_eq!(out, "export AWS_PROFILE='prod'\nunset AWS_REGION\n");
    }

    #[test]
    fn export_fish() {
        let out = ShellExporter::for_shell(ShellType::Fish)
            .export("prod", Some("us-east-1"))
            .unwrap();
        assert_eq!(
            out,
            "set -gx AWS_PROFILE 'prod'\nset -gx AWS_REGION 'us-east-1'\n"
        );
    }

    #[test]
    fn export_powershell() {
        let out = ShellExporter::for_shell(ShellType::PowerShell)
            .export("prod", Some("us-east-1"))
            .unwrap();
        assert_eq!(
            out,
            "$env:AWS_PROFILE = 'prod'\n$env:AWS_REGION = 'us-east-1'\n"
        );
    }

    #[test]
    fn unset_all_bash() {
        let out = ShellExporter::for_shell(ShellType::Bash).unset_all();
        assert_eq!(out, "unset AWS_PROFILE\nunset AWS_REGION\n");
    }

    #[test]
    fn unset_all_fish() {
        let out = ShellExporter::for_shell(ShellType::Fish).unset_all();
        assert_eq!(out, "set -e AWS_PROFILE\nset -e AWS_REGION\n");
    }

    #[test]
    fn unset_all_powershell() {
        let out = ShellExporter::for_shell(ShellType::PowerShell).unset_all();
        assert!(out.contains("Remove-Item Env:\\AWS_PROFILE"));
        assert!(out.contains("Remove-Item Env:\\AWS_REGION"));
    }

    #[test]
    fn rejects_newline_in_profile() {
        let r = ShellExporter::for_shell(ShellType::Bash).export("a\nb", None);
        assert!(r.is_err());
    }

    #[test]
    fn rejects_carriage_return_in_region() {
        let r = ShellExporter::for_shell(ShellType::Bash).export("a", Some("us\r"));
        assert!(r.is_err());
    }

    #[test]
    fn single_quote_escaped_posix() {
        let out = ShellExporter::for_shell(ShellType::Bash)
            .export("it's", None)
            .unwrap();
        assert!(out.contains("'it'\\''s'"));
    }

    #[test]
    fn single_quote_escaped_powershell() {
        let out = ShellExporter::for_shell(ShellType::PowerShell)
            .export("it's", None)
            .unwrap();
        assert!(out.contains("'it''s'"));
    }

    #[test]
    fn dollar_substitution_is_quoted() {
        let out = ShellExporter::for_shell(ShellType::Bash)
            .export("$(whoami)", None)
            .unwrap();
        assert!(out.contains("'$(whoami)'"));
    }

    #[test]
    fn unicode_profile_name() {
        let out = ShellExporter::for_shell(ShellType::Bash)
            .export("プロファイル", Some("ap-northeast-1"))
            .unwrap();
        assert!(out.contains("'プロファイル'"));
    }

    #[test]
    fn shell_detect_from_env() {
        // AWSWIT_SHELL takes precedence.
        unsafe {
            std::env::set_var("AWSWIT_SHELL", "fish");
        }
        assert!(matches!(ShellType::detect(), ShellType::Fish));
        unsafe {
            std::env::remove_var("AWSWIT_SHELL");
        }
    }
}
