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

    /// Common INI file loader: reads sections matching `extract_profile_name`
    /// and returns them as `Profile`s.
    ///
    /// File permissions are intentionally not validated here. Securing
    /// `~/.aws/credentials` is the responsibility of the AWS SDK / aws-vault
    /// / the user — awswit parses the file contents but only retains section
    /// names and metadata in the returned profiles, ignoring credential keys.
    fn load_ini_file(
        path: &str,
        label: &str,
        extract_profile_name: fn(&str) -> Option<String>,
    ) -> Result<HashMap<String, Profile>, AwswitError> {
        let path = shellexpand::tilde(path).to_string();

        if !Path::new(&path).exists() {
            tracing::debug!("{} not found: {}", label, path);
            return Ok(HashMap::new());
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
            if let Some(profile_name) = extract_profile_name(&section_name) {
                let profile = Self::section_to_profile(&ini, &section_name);
                profiles.insert(profile_name, profile);
            }
        }

        tracing::debug!("Loaded {} profiles from {}", profiles.len(), label);
        Ok(profiles)
    }

    /// Load the AWS config file (~/.aws/config)
    fn load_config_file(path: &str) -> Result<HashMap<String, Profile>, AwswitError> {
        Self::load_ini_file(path, "Config file", |section_name| {
            if section_name.starts_with("profile ") {
                Some(section_name.strip_prefix("profile ").unwrap().to_string())
            } else if section_name == "default" {
                Some("default".to_string())
            } else {
                None // Skip non-profile sections (like "sso-session")
            }
        })
    }

    /// Load the AWS credentials file (~/.aws/credentials)
    fn load_credentials_file(path: &str) -> Result<HashMap<String, Profile>, AwswitError> {
        Self::load_ini_file(path, "Credentials file", |section_name| {
            Some(section_name.to_string())
        })
    }

    /// Convert an INI section to a Profile
    fn section_to_profile(ini: &Ini, section: &str) -> Profile {
        let get = |key: &str| -> Option<String> { ini.get(section, key) };

        Profile {
            name: section
                .strip_prefix("profile ")
                .unwrap_or(section)
                .to_string(),
            role_arn: get("role_arn"),
            source_profile: get("source_profile"),
            credential_source: get("credential_source"),
            mfa_serial: get("mfa_serial"),
            region: get("region"),
            // SSO fields
            sso_start_url: get("sso_start_url"),
            sso_region: get("sso_region"),
            sso_account_id: get("sso_account_id"),
            sso_role_name: get("sso_role_name"),
        }
    }

    /// Merge config and credentials profiles.
    ///
    /// Profiles defined only in `~/.aws/credentials` are exposed by name so the
    /// picker can list them; awswit does not read or store the actual key
    /// material — the AWS SDK resolves credentials at runtime.
    pub fn merge_profiles(&self) -> HashMap<String, Profile> {
        let mut merged: HashMap<String, Profile> = self.config_profiles.clone();

        for (name, cred_profile) in &self.credentials_profiles {
            merged
                .entry(name.clone())
                .or_insert_with(|| cred_profile.clone());
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

        assert!(merged.contains_key("default"));
        let default = &merged["default"];
        assert_eq!(default.region, Some("us-east-1".to_string()));
    }
}
