use configparser::ini::Ini;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::error::AwswitError;
use crate::profile::Profile;

/// Handles loading and parsing of AWS config and credentials files
#[derive(Debug, Default)]
pub struct AwsFiles {
    /// Profiles from ~/.aws/config
    pub config_profiles: HashMap<String, Profile>,
    /// Profiles from ~/.aws/credentials
    pub credentials_profiles: HashMap<String, Profile>,
}

impl AwsFiles {
    /// Load AWS config and credentials files
    pub fn load(config_path: &str, credentials_path: &str) -> Result<Self, AwswitError> {
        let config_profiles = Self::load_config_file(config_path)?;
        let credentials_profiles = Self::load_credentials_file(credentials_path)?;

        Ok(Self {
            config_profiles,
            credentials_profiles,
        })
    }

    /// Load the AWS config file (~/.aws/config)
    fn load_config_file(path: &str) -> Result<HashMap<String, Profile>, AwswitError> {
        let path = shellexpand::tilde(path).to_string();

        if !Path::new(&path).exists() {
            tracing::debug!("Config file not found: {}", path);
            return Ok(HashMap::new());
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if let Ok(meta) = fs::metadata(&path) {
                if meta.mode() & 0o002 != 0 {
                    tracing::warn!(
                        "AWS config file {} is world-writable (mode {:o}). This is a security risk.",
                        path,
                        meta.mode() & 0o777
                    );
                }
            }
        }

        let content = fs::read_to_string(&path).map_err(|e| AwswitError::ConfigFileError {
            message: format!("Failed to read {}: {}", path, e),
        })?;

        let mut ini = Ini::new_cs(); // Case sensitive
        ini.read(content)
            .map_err(|e| AwswitError::ConfigFileError {
                message: format!("Failed to parse {}: {}", path, e),
            })?;

        let mut profiles = HashMap::new();

        for section_name in ini.sections() {
            // Config file uses "profile xxx" format (except for "default")
            let profile_name = if section_name.starts_with("profile ") {
                section_name.strip_prefix("profile ").unwrap().to_string()
            } else if section_name == "default" {
                "default".to_string()
            } else {
                // Skip non-profile sections (like "sso-session")
                continue;
            };

            let profile = Self::section_to_profile(&ini, &section_name);
            profiles.insert(profile_name, profile);
        }

        tracing::debug!("Loaded {} profiles from config file", profiles.len());
        Ok(profiles)
    }

    /// Load the AWS credentials file (~/.aws/credentials)
    fn load_credentials_file(path: &str) -> Result<HashMap<String, Profile>, AwswitError> {
        let path = shellexpand::tilde(path).to_string();

        if !Path::new(&path).exists() {
            tracing::debug!("Credentials file not found: {}", path);
            return Ok(HashMap::new());
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if let Ok(meta) = fs::metadata(&path) {
                let mode = meta.mode() & 0o777;
                if mode & 0o002 != 0 {
                    return Err(AwswitError::ConfigFileError {
                        message: format!(
                            "AWS credentials file {} is world-writable (mode {:o}). \
                             Fix with: chmod 600 {}",
                            path, mode, path
                        ),
                    });
                }
                if mode & 0o044 != 0 {
                    tracing::warn!(
                        "AWS credentials file {} is readable by group/others (mode {:o}). \
                         Recommended permissions are 0600.",
                        path,
                        mode
                    );
                }
            }
        }

        let content = fs::read_to_string(&path).map_err(|e| AwswitError::ConfigFileError {
            message: format!("Failed to read {}: {}", path, e),
        })?;

        let mut ini = Ini::new_cs();
        ini.read(content)
            .map_err(|e| AwswitError::ConfigFileError {
                message: format!("Failed to parse {}: {}", path, e),
            })?;

        let mut profiles = HashMap::new();

        for section_name in ini.sections() {
            // Credentials file uses direct profile names
            let profile = Self::section_to_profile(&ini, &section_name);
            profiles.insert(section_name.clone(), profile);
        }

        tracing::debug!("Loaded {} profiles from credentials file", profiles.len());
        Ok(profiles)
    }

    /// Convert an INI section to a Profile
    fn section_to_profile(ini: &Ini, section: &str) -> Profile {
        let get = |key: &str| -> Option<String> { ini.get(section, key) };

        Profile {
            name: section
                .strip_prefix("profile ")
                .unwrap_or(section)
                .to_string(),
            aws_access_key_id: get("aws_access_key_id"),
            aws_secret_access_key: get("aws_secret_access_key"),
            aws_session_token: get("aws_session_token"),
            role_arn: get("role_arn"),
            source_profile: get("source_profile"),
            credential_source: get("credential_source"),
            credential_process: get("credential_process"),
            mfa_serial: get("mfa_serial"),
            external_id: get("external_id"),
            role_session_name: get("role_session_name"),
            duration_seconds: get("duration_seconds").and_then(|s| match s.parse() {
                Ok(v) => Some(v),
                Err(e) => {
                    tracing::warn!(
                        "Failed to parse duration_seconds '{}' in section '{}': {}",
                        s,
                        section,
                        e
                    );
                    None
                }
            }),
            region: get("region"),
            output: get("output"),
            // SSO fields
            sso_start_url: get("sso_start_url"),
            sso_region: get("sso_region"),
            sso_account_id: get("sso_account_id"),
            sso_role_name: get("sso_role_name"),
            // Web identity
            web_identity_token_file: get("web_identity_token_file"),
            // Additional awswit-specific fields
            manager: get("manager"),
            awswit_cache_name: get("awswit_cache_name"),
            autoawswit: get("autoawswit").map(|s| s == "true"),
        }
    }

    /// Merge config and credentials profiles
    /// Credentials file takes precedence for credential fields
    pub fn merge_profiles(&self) -> HashMap<String, Profile> {
        let mut merged = HashMap::new();

        // Start with config profiles
        for (name, profile) in &self.config_profiles {
            merged.insert(name.clone(), profile.clone());
        }

        // Merge credentials profiles
        for (name, cred_profile) in &self.credentials_profiles {
            if let Some(existing) = merged.get_mut(name) {
                // Merge credentials into existing profile
                existing.merge_credentials(cred_profile);
            } else {
                // Add new profile from credentials
                merged.insert(name.clone(), cred_profile.clone());
            }
        }

        merged
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn create_temp_config() -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        write!(
            file,
            r#"
[default]
region = us-east-1

[profile dev]
role_arn = arn:aws:iam::123456789012:role/DevRole
source_profile = default
mfa_serial = arn:aws:iam::123456789012:mfa/user
region = us-west-2

[profile prod]
role_arn = arn:aws:iam::987654321098:role/ProdRole
source_profile = dev
external_id = abc123
"#
        )
        .unwrap();
        file
    }

    fn create_temp_credentials() -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        write!(
            file,
            r#"
[default]
aws_access_key_id = AKIAIOSFODNN7EXAMPLE
aws_secret_access_key = wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY
"#
        )
        .unwrap();
        file
    }

    #[test]
    fn test_load_config() {
        let config_file = create_temp_config();
        let creds_file = create_temp_credentials();

        let aws_files = AwsFiles::load(
            config_file.path().to_str().unwrap(),
            creds_file.path().to_str().unwrap(),
        )
        .unwrap();

        assert!(aws_files.config_profiles.contains_key("dev"));
        assert!(aws_files.config_profiles.contains_key("prod"));

        let dev = &aws_files.config_profiles["dev"];
        assert_eq!(
            dev.role_arn,
            Some("arn:aws:iam::123456789012:role/DevRole".to_string())
        );
        assert_eq!(dev.source_profile, Some("default".to_string()));
    }

    #[test]
    fn test_merge_profiles() {
        let config_file = create_temp_config();
        let creds_file = create_temp_credentials();

        let aws_files = AwsFiles::load(
            config_file.path().to_str().unwrap(),
            creds_file.path().to_str().unwrap(),
        )
        .unwrap();

        let merged = aws_files.merge_profiles();

        let default = &merged["default"];
        assert_eq!(
            default.aws_access_key_id,
            Some("AKIAIOSFODNN7EXAMPLE".to_string())
        );
        assert_eq!(default.region, Some("us-east-1".to_string()));
    }
}
