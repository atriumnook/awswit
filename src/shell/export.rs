use crate::aws::Credentials;

/// Shell-quote a value by wrapping in single quotes and escaping embedded single quotes
fn shell_quote(s: &str) -> String {
    // Replace ' with '\'' (end quote, escaped quote, start quote)
    format!("'{}'", s.replace('\'', "'\\''"))
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
        // Check AWSUME_SHELL first (set by shell wrapper)
        if let Ok(shell) = std::env::var("AWSUME_SHELL") {
            return Self::parse_shell_name(&shell);
        }

        // Check SHELL environment variable
        if let Ok(shell) = std::env::var("SHELL") {
            return Self::parse_shell_name(&shell);
        }

        // Windows detection
        if cfg!(windows) {
            if std::env::var("PSModulePath").is_ok() {
                return ShellType::PowerShell;
            }
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

    /// Generate export commands that can be displayed to the user
    pub fn generate_export_commands(&self, credentials: &Credentials, profile_name: &str) -> String {
        match self.shell_type {
            ShellType::Bash | ShellType::Zsh => {
                self.generate_posix_export(credentials, profile_name)
            }
            ShellType::Fish => {
                self.generate_fish_export(credentials, profile_name)
            }
            ShellType::PowerShell => {
                self.generate_powershell_export(credentials, profile_name)
            }
            ShellType::Cmd => {
                self.generate_cmd_export(credentials, profile_name)
            }
        }
    }

    /// Generate output for shell wrapper to eval
    pub fn generate_shell_output(&self, credentials: &Credentials, profile_name: &str) -> String {
        // The shell wrapper will source this output
        // Format: key=value pairs, one per line
        // The shell wrapper converts these to appropriate export commands
        let mut output = String::new();

        output.push_str(&format!("AWS_ACCESS_KEY_ID={}\n", &credentials.access_key_id));
        output.push_str(&format!("AWS_SECRET_ACCESS_KEY={}\n", &credentials.secret_access_key));

        if let Some(ref token) = credentials.session_token {
            output.push_str(&format!("AWS_SESSION_TOKEN={}\n", token));
            output.push_str(&format!("AWS_SECURITY_TOKEN={}\n", token)); // Legacy
        }

        if let Some(ref region) = credentials.region {
            output.push_str(&format!("AWS_REGION={}\n", region));
            output.push_str(&format!("AWS_DEFAULT_REGION={}\n", region));
        }

        output.push_str(&format!("AWSWIT_PROFILE={}\n", profile_name));

        if let Some(expiration) = credentials.expiration {
            output.push_str(&format!(
                "AWSWIT_EXPIRATION={}\n",
                expiration.format("%Y-%m-%dT%H:%M:%S")
            ));
        }

        output
    }

    /// Generate unset commands for display
    pub fn generate_unset_commands(&self) -> String {
        match self.shell_type {
            ShellType::Bash | ShellType::Zsh => {
                self.generate_posix_unset()
            }
            ShellType::Fish => {
                self.generate_fish_unset()
            }
            ShellType::PowerShell => {
                self.generate_powershell_unset()
            }
            ShellType::Cmd => {
                self.generate_cmd_unset()
            }
        }
    }

    /// Generate unset output for shell wrapper
    pub fn generate_unset_output(&self) -> String {
        // Special marker to indicate unset
        "AWSWIT_UNSET=1\n".to_string()
    }

    // POSIX shell (bash, zsh)
    fn generate_posix_export(&self, creds: &Credentials, profile: &str) -> String {
        let mut output = String::new();

        output.push_str(&format!("export AWS_ACCESS_KEY_ID={}\n", shell_quote(&creds.access_key_id)));
        output.push_str(&format!("export AWS_SECRET_ACCESS_KEY={}\n", shell_quote(&creds.secret_access_key)));

        if let Some(ref token) = creds.session_token {
            output.push_str(&format!("export AWS_SESSION_TOKEN={}\n", shell_quote(token)));
            output.push_str(&format!("export AWS_SECURITY_TOKEN={}\n", shell_quote(token)));
        }

        if let Some(ref region) = creds.region {
            output.push_str(&format!("export AWS_REGION={}\n", shell_quote(region)));
            output.push_str(&format!("export AWS_DEFAULT_REGION={}\n", shell_quote(region)));
        }

        output.push_str(&format!("export AWSWIT_PROFILE={}\n", shell_quote(profile)));

        if let Some(exp) = creds.expiration {
            output.push_str(&format!(
                "export AWSWIT_EXPIRATION={}\n",
                shell_quote(&exp.format("%Y-%m-%dT%H:%M:%S").to_string())
            ));
        }

        output
    }

    fn generate_posix_unset(&self) -> String {
        r#"unset AWS_ACCESS_KEY_ID
unset AWS_SECRET_ACCESS_KEY
unset AWS_SESSION_TOKEN
unset AWS_SECURITY_TOKEN
unset AWS_REGION
unset AWS_DEFAULT_REGION
unset AWS_PROFILE
unset AWS_DEFAULT_PROFILE
unset AWSWIT_PROFILE
unset AWSWIT_EXPIRATION
"#.to_string()
    }

    // Fish shell
    fn generate_fish_export(&self, creds: &Credentials, profile: &str) -> String {
        let mut output = String::new();

        output.push_str(&format!("set -gx AWS_ACCESS_KEY_ID {}\n", shell_quote(&creds.access_key_id)));
        output.push_str(&format!("set -gx AWS_SECRET_ACCESS_KEY {}\n", shell_quote(&creds.secret_access_key)));

        if let Some(ref token) = creds.session_token {
            output.push_str(&format!("set -gx AWS_SESSION_TOKEN {}\n", shell_quote(token)));
            output.push_str(&format!("set -gx AWS_SECURITY_TOKEN {}\n", shell_quote(token)));
        }

        if let Some(ref region) = creds.region {
            output.push_str(&format!("set -gx AWS_REGION {}\n", shell_quote(region)));
            output.push_str(&format!("set -gx AWS_DEFAULT_REGION {}\n", shell_quote(region)));
        }

        output.push_str(&format!("set -gx AWSWIT_PROFILE {}\n", shell_quote(profile)));

        if let Some(exp) = creds.expiration {
            output.push_str(&format!(
                "set -gx AWSWIT_EXPIRATION {}\n",
                shell_quote(&exp.format("%Y-%m-%dT%H:%M:%S").to_string())
            ));
        }

        output
    }

    fn generate_fish_unset(&self) -> String {
        r#"set -e AWS_ACCESS_KEY_ID
set -e AWS_SECRET_ACCESS_KEY
set -e AWS_SESSION_TOKEN
set -e AWS_SECURITY_TOKEN
set -e AWS_REGION
set -e AWS_DEFAULT_REGION
set -e AWS_PROFILE
set -e AWS_DEFAULT_PROFILE
set -e AWSWIT_PROFILE
set -e AWSWIT_EXPIRATION
"#.to_string()
    }

    // PowerShell
    fn generate_powershell_export(&self, creds: &Credentials, profile: &str) -> String {
        let mut output = String::new();

        // PowerShell escapes single quotes by doubling them
        let ps_quote = |s: &str| -> String {
            format!("'{}'", s.replace('\'', "''"))
        };

        output.push_str(&format!("$env:AWS_ACCESS_KEY_ID = {}\n", ps_quote(&creds.access_key_id)));
        output.push_str(&format!("$env:AWS_SECRET_ACCESS_KEY = {}\n", ps_quote(&creds.secret_access_key)));

        if let Some(ref token) = creds.session_token {
            output.push_str(&format!("$env:AWS_SESSION_TOKEN = {}\n", ps_quote(token)));
            output.push_str(&format!("$env:AWS_SECURITY_TOKEN = {}\n", ps_quote(token)));
        }

        if let Some(ref region) = creds.region {
            output.push_str(&format!("$env:AWS_REGION = {}\n", ps_quote(region)));
            output.push_str(&format!("$env:AWS_DEFAULT_REGION = {}\n", ps_quote(region)));
        }

        output.push_str(&format!("$env:AWSWIT_PROFILE = {}\n", ps_quote(profile)));

        if let Some(exp) = creds.expiration {
            output.push_str(&format!(
                "$env:AWSWIT_EXPIRATION = {}\n",
                ps_quote(&exp.format("%Y-%m-%dT%H:%M:%S").to_string())
            ));
        }

        output
    }

    fn generate_powershell_unset(&self) -> String {
        r#"Remove-Item Env:\AWS_ACCESS_KEY_ID -ErrorAction SilentlyContinue
Remove-Item Env:\AWS_SECRET_ACCESS_KEY -ErrorAction SilentlyContinue
Remove-Item Env:\AWS_SESSION_TOKEN -ErrorAction SilentlyContinue
Remove-Item Env:\AWS_SECURITY_TOKEN -ErrorAction SilentlyContinue
Remove-Item Env:\AWS_REGION -ErrorAction SilentlyContinue
Remove-Item Env:\AWS_DEFAULT_REGION -ErrorAction SilentlyContinue
Remove-Item Env:\AWS_PROFILE -ErrorAction SilentlyContinue
Remove-Item Env:\AWS_DEFAULT_PROFILE -ErrorAction SilentlyContinue
Remove-Item Env:\AWSWIT_PROFILE -ErrorAction SilentlyContinue
Remove-Item Env:\AWSWIT_EXPIRATION -ErrorAction SilentlyContinue
"#.to_string()
    }

    // Windows Command Prompt
    fn generate_cmd_export(&self, creds: &Credentials, profile: &str) -> String {
        let mut output = String::new();

        // CMD set doesn't need quoting for values (everything after = is the value)
        // but we need to escape special CMD chars: & | < > ^ %
        let cmd_escape = |s: &str| -> String {
            s.replace('^', "^^")
                .replace('&', "^&")
                .replace('|', "^|")
                .replace('<', "^<")
                .replace('>', "^>")
                .replace('%', "%%")
        };

        output.push_str(&format!("set AWS_ACCESS_KEY_ID={}\n", cmd_escape(&creds.access_key_id)));
        output.push_str(&format!("set AWS_SECRET_ACCESS_KEY={}\n", cmd_escape(&creds.secret_access_key)));

        if let Some(ref token) = creds.session_token {
            output.push_str(&format!("set AWS_SESSION_TOKEN={}\n", cmd_escape(token)));
            output.push_str(&format!("set AWS_SECURITY_TOKEN={}\n", cmd_escape(token)));
        }

        if let Some(ref region) = creds.region {
            output.push_str(&format!("set AWS_REGION={}\n", cmd_escape(region)));
            output.push_str(&format!("set AWS_DEFAULT_REGION={}\n", cmd_escape(region)));
        }

        output.push_str(&format!("set AWSWIT_PROFILE={}\n", cmd_escape(profile)));

        if let Some(exp) = creds.expiration {
            output.push_str(&format!(
                "set AWSWIT_EXPIRATION={}\n",
                exp.format("%Y-%m-%dT%H:%M:%S")
            ));
        }

        output
    }

    fn generate_cmd_unset(&self) -> String {
        r#"set AWS_ACCESS_KEY_ID=
set AWS_SECRET_ACCESS_KEY=
set AWS_SESSION_TOKEN=
set AWS_SECURITY_TOKEN=
set AWS_REGION=
set AWS_DEFAULT_REGION=
set AWS_PROFILE=
set AWS_DEFAULT_PROFILE=
set AWSWIT_PROFILE=
set AWSWIT_EXPIRATION=
"#.to_string()
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
}
