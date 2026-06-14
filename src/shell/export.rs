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
/// We deliberately *set* only `AWS_PROFILE` and `AWS_REGION` — the modern
/// AWS SDK prefers the non-`DEFAULT` form and we don't want to introduce
/// new variables onto the user's environment.
///
/// We *clear* the legacy `AWS_DEFAULT_*` variants on every switch (and on
/// `awswit -u`), because the SDK falls back to them when the modern
/// variables are missing or unset. CI images, corporate dotfiles, and
/// previous `aws configure` runs frequently leave them set; without
/// clearing, `awswit -u && aws ...` can silently hit the previous account.
///
/// The set kept here drives `--unset` (`unset_all`); the per-switch path
/// uses the same list to emit "unset" lines after the "export" lines.
pub const MANAGED_VARS: &[&str] = &[
    "AWS_PROFILE",
    "AWS_DEFAULT_PROFILE",
    "AWS_REGION",
    "AWS_DEFAULT_REGION",
];

/// Credential variables that the AWS SDK reads *before* it ever looks at
/// `AWS_PROFILE`. We clear these on every switch (and on `awswit -u`) so the
/// selected profile actually takes effect.
///
/// In the SDK / CLI credential-resolution chain, explicit environment
/// credentials win over `AWS_PROFILE`. So if a previous `aws sso login`,
/// aws-vault subshell, or awsume run left `AWS_ACCESS_KEY_ID` /
/// `AWS_SESSION_TOKEN` in the environment, then `awswit prod` would set
/// `AWS_PROFILE=prod` yet `aws s3 ls` would silently keep using the *stale*
/// credentials. Clearing them makes awswit a drop-in awsume-style switcher:
/// after `awswit prod`, plain `aws ...` runs as the prod profile.
///
/// awswit still never *creates* credentials — it only removes conflicting
/// ones and lets the SDK resolve the profile (IAM keys, SSO, role assumption,
/// `credential_process`, …) on the next call. `AWS_SECURITY_TOKEN` is the
/// legacy alias for `AWS_SESSION_TOKEN` still honored by some SDKs;
/// `AWS_CREDENTIAL_EXPIRATION` is informational metadata some tools export.
pub const CREDENTIAL_VARS: &[&str] = &[
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "AWS_SECURITY_TOKEN",
    "AWS_CREDENTIAL_EXPIRATION",
];

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

    /// Emit `set` commands for the selected profile and (optional) region,
    /// plus `unset` for the legacy `AWS_DEFAULT_*` fallbacks and any inherited
    /// credential variables.
    ///
    /// Without explicitly unsetting `AWS_DEFAULT_PROFILE` and
    /// `AWS_DEFAULT_REGION`, the AWS SDK can still resolve credentials/region
    /// from them, so `awswit prod` followed by `aws sts get-caller-identity`
    /// could silently hit the previous account if it had `AWS_DEFAULT_PROFILE`
    /// set. Likewise, inherited `AWS_ACCESS_KEY_ID` / `AWS_SESSION_TOKEN`
    /// outrank `AWS_PROFILE`, so we clear them too — see the `MANAGED_VARS` and
    /// `CREDENTIAL_VARS` doc-comments.
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
        out.push_str(&self.unset("AWS_DEFAULT_PROFILE"));
        match region {
            Some(r) => out.push_str(&self.set("AWS_REGION", r)),
            None => out.push_str(&self.unset("AWS_REGION")),
        }
        out.push_str(&self.unset("AWS_DEFAULT_REGION"));
        // Clear inherited credentials so the freshly selected profile wins the
        // SDK resolution chain (env credentials outrank AWS_PROFILE).
        for var in CREDENTIAL_VARS {
            out.push_str(&self.unset(var));
        }
        Ok(out)
    }

    /// Emit `unset` commands for every managed variable, including inherited
    /// credentials, so `awswit -u` returns to a clean, unauthenticated state.
    pub fn unset_all(&self) -> String {
        let mut out = String::new();
        for var in MANAGED_VARS.iter().chain(CREDENTIAL_VARS) {
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
    fn export_bash_with_region_also_clears_legacy_defaults_and_credentials() {
        let out = ShellExporter::for_shell(ShellType::Bash)
            .export("prod", Some("ap-northeast-1"))
            .unwrap();
        assert_eq!(
            out,
            "export AWS_PROFILE='prod'\n\
             unset AWS_DEFAULT_PROFILE\n\
             export AWS_REGION='ap-northeast-1'\n\
             unset AWS_DEFAULT_REGION\n\
             unset AWS_ACCESS_KEY_ID\n\
             unset AWS_SECRET_ACCESS_KEY\n\
             unset AWS_SESSION_TOKEN\n\
             unset AWS_SECURITY_TOKEN\n\
             unset AWS_CREDENTIAL_EXPIRATION\n"
        );
    }

    #[test]
    fn export_bash_without_region_unsets_region_legacy_defaults_and_credentials() {
        let out = ShellExporter::for_shell(ShellType::Bash)
            .export("prod", None)
            .unwrap();
        assert_eq!(
            out,
            "export AWS_PROFILE='prod'\n\
             unset AWS_DEFAULT_PROFILE\n\
             unset AWS_REGION\n\
             unset AWS_DEFAULT_REGION\n\
             unset AWS_ACCESS_KEY_ID\n\
             unset AWS_SECRET_ACCESS_KEY\n\
             unset AWS_SESSION_TOKEN\n\
             unset AWS_SECURITY_TOKEN\n\
             unset AWS_CREDENTIAL_EXPIRATION\n"
        );
    }

    #[test]
    fn export_clears_inherited_credentials_for_all_shells() {
        for shell in [ShellType::Bash, ShellType::Fish, ShellType::PowerShell] {
            let out = ShellExporter::for_shell(shell)
                .export("prod", Some("us-east-1"))
                .unwrap();
            for v in CREDENTIAL_VARS {
                assert!(out.contains(v), "{:?} export missing clear of {}", shell, v);
            }
        }
    }

    #[test]
    fn export_fish() {
        let out = ShellExporter::for_shell(ShellType::Fish)
            .export("prod", Some("us-east-1"))
            .unwrap();
        assert!(out.contains("set -gx AWS_PROFILE 'prod'"));
        assert!(out.contains("set -gx AWS_REGION 'us-east-1'"));
        assert!(out.contains("set -e AWS_DEFAULT_PROFILE"));
        assert!(out.contains("set -e AWS_DEFAULT_REGION"));
    }

    #[test]
    fn export_powershell() {
        let out = ShellExporter::for_shell(ShellType::PowerShell)
            .export("prod", Some("us-east-1"))
            .unwrap();
        assert!(out.contains("$env:AWS_PROFILE = 'prod'"));
        assert!(out.contains("$env:AWS_REGION = 'us-east-1'"));
        assert!(out.contains("Remove-Item Env:\\AWS_DEFAULT_PROFILE"));
        assert!(out.contains("Remove-Item Env:\\AWS_DEFAULT_REGION"));
    }

    #[test]
    fn unset_all_bash_clears_managed_vars_and_credentials() {
        let out = ShellExporter::for_shell(ShellType::Bash).unset_all();
        assert_eq!(
            out,
            "unset AWS_PROFILE\n\
             unset AWS_DEFAULT_PROFILE\n\
             unset AWS_REGION\n\
             unset AWS_DEFAULT_REGION\n\
             unset AWS_ACCESS_KEY_ID\n\
             unset AWS_SECRET_ACCESS_KEY\n\
             unset AWS_SESSION_TOKEN\n\
             unset AWS_SECURITY_TOKEN\n\
             unset AWS_CREDENTIAL_EXPIRATION\n"
        );
    }

    #[test]
    fn unset_all_fish() {
        let out = ShellExporter::for_shell(ShellType::Fish).unset_all();
        for v in MANAGED_VARS.iter().chain(CREDENTIAL_VARS) {
            assert!(out.contains(&format!("set -e {}", v)), "missing {}", v);
        }
    }

    #[test]
    fn unset_all_powershell() {
        let out = ShellExporter::for_shell(ShellType::PowerShell).unset_all();
        for v in MANAGED_VARS.iter().chain(CREDENTIAL_VARS) {
            assert!(
                out.contains(&format!("Remove-Item Env:\\{}", v)),
                "missing {}",
                v
            );
        }
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
