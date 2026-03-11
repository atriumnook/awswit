use serde::{Deserialize, Serialize};

/// Valid credential sources
pub const VALID_CREDENTIAL_SOURCES: &[&str] = &["Environment", "Ec2InstanceMetadata", "EcsContainer"];

/// Represents an AWS profile from config/credentials files
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub aws_access_key_id: Option<String>,
    pub aws_secret_access_key: Option<String>,
    pub aws_session_token: Option<String>,
    pub role_arn: Option<String>,
    pub source_profile: Option<String>,
    pub credential_source: Option<String>,
    pub credential_process: Option<String>,
    pub mfa_serial: Option<String>,
    pub external_id: Option<String>,
    pub role_session_name: Option<String>,
    pub duration_seconds: Option<i32>,
    pub region: Option<String>,
    pub output: Option<String>,

    // SSO fields
    pub sso_start_url: Option<String>,
    pub sso_region: Option<String>,
    pub sso_account_id: Option<String>,
    pub sso_role_name: Option<String>,

    // Web identity
    pub web_identity_token_file: Option<String>,

    // awswit-specific
    pub manager: Option<String>,
    pub awswit_cache_name: Option<String>,
    pub autoawswit: Option<bool>,
}

impl Profile {
    /// Check if this is a role profile (has role_arn)
    pub fn is_role_profile(&self) -> bool {
        self.role_arn.is_some()
    }

    /// Check if this profile requires MFA
    pub fn requires_mfa(&self) -> bool {
        self.mfa_serial.is_some()
    }

    /// Check if this profile uses credential_process
    pub fn uses_credential_process(&self) -> bool {
        self.credential_process.is_some()
    }

    /// Check if this profile uses SSO
    pub fn is_sso_profile(&self) -> bool {
        self.sso_start_url.is_some() && self.sso_account_id.is_some()
    }

    /// Check if credential_source is valid
    pub fn has_valid_credential_source(&self) -> bool {
        self.credential_source
            .as_ref()
            .map(|cs| VALID_CREDENTIAL_SOURCES.contains(&cs.as_str()))
            .unwrap_or(false)
    }

    /// Validate the profile configuration
    pub fn validate(&self) -> Result<(), ProfileValidationError> {
        // Role profiles must have source_profile OR credential_source OR credential_process
        if self.is_role_profile() {
            let has_source = self.source_profile.is_some();
            let has_cred_source = self.credential_source.is_some();
            let has_cred_process = self.credential_process.is_some();

            if !has_source && !has_cred_source && !has_cred_process {
                return Err(ProfileValidationError::MissingSourceForRole);
            }

            // source_profile and credential_source are mutually exclusive
            if has_source && has_cred_source {
                return Err(ProfileValidationError::ConflictingCredentialSource);
            }

            // Validate credential_source value
            if has_cred_source && !self.has_valid_credential_source() {
                let cs = self.credential_source.as_deref().unwrap_or_default();
                return Err(ProfileValidationError::InvalidCredentialSource(
                    cs.to_string(),
                ));
            }
        }

        // User profiles need access keys (unless using credential_process)
        if !self.is_role_profile()
            && !self.uses_credential_process()
            && (self.aws_access_key_id.is_none() || self.aws_secret_access_key.is_none())
        {
            // Allow if using credential_source
            if !self.has_valid_credential_source() {
                return Err(ProfileValidationError::MissingAccessKeys);
            }
        }

        Ok(())
    }

    /// Merge credentials from another profile
    pub fn merge_credentials(&mut self, other: &Profile) {
        if other.aws_access_key_id.is_some() {
            self.aws_access_key_id = other.aws_access_key_id.clone();
        }
        if other.aws_secret_access_key.is_some() {
            self.aws_secret_access_key = other.aws_secret_access_key.clone();
        }
        if other.aws_session_token.is_some() {
            self.aws_session_token = other.aws_session_token.clone();
        }
    }

    /// Extract account ID from role ARN
    pub fn get_account_id(&self) -> Option<String> {
        self.role_arn
            .as_ref()
            .and_then(|arn| {
                // arn:aws:iam::123456789012:role/RoleName
                let parts: Vec<&str> = arn.split(':').collect();
                if parts.len() >= 5 {
                    Some(parts[4].to_string())
                } else {
                    None
                }
            })
            .or_else(|| {
                // Try from mfa_serial
                self.mfa_serial.as_ref().and_then(|serial| {
                    let parts: Vec<&str> = serial.split(':').collect();
                    if parts.len() >= 5 {
                        Some(parts[4].to_string())
                    } else {
                        None
                    }
                })
            })
    }
}

/// Profile validation errors
#[derive(Debug, Clone)]
pub enum ProfileValidationError {
    MissingSourceForRole,
    ConflictingCredentialSource,
    InvalidCredentialSource(String),
    UnsupportedCredentialSource(String),
    MissingAccessKeys,
}

impl std::fmt::Display for ProfileValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingSourceForRole => {
                write!(f, "role profiles must contain one of credential_source, source_profile, or credential_process")
            }
            Self::ConflictingCredentialSource => {
                write!(
                    f,
                    "credential_source and source_profile are mutually exclusive"
                )
            }
            Self::InvalidCredentialSource(cs) => {
                write!(f, "unsupported credential_source: {}", cs)
            }
            Self::UnsupportedCredentialSource(cs) => {
                write!(
                    f,
                    "credential_source '{}' is not supported by awswit. Use the AWS CLI default credential chain instead.",
                    cs
                )
            }
            Self::MissingAccessKeys => {
                write!(f, "missing aws_access_key_id or aws_secret_access_key")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_role_profile() {
        let mut profile = Profile::default();
        assert!(!profile.is_role_profile());

        profile.role_arn = Some("arn:aws:iam::123456789012:role/Test".to_string());
        assert!(profile.is_role_profile());
    }

    #[test]
    fn test_requires_mfa() {
        let mut profile = Profile::default();
        assert!(!profile.requires_mfa());

        profile.mfa_serial = Some("arn:aws:iam::123456789012:mfa/user".to_string());
        assert!(profile.requires_mfa());
    }

    #[test]
    fn test_get_account_id() {
        let profile = Profile {
            role_arn: Some("arn:aws:iam::123456789012:role/TestRole".to_string()),
            ..Default::default()
        };
        assert_eq!(profile.get_account_id(), Some("123456789012".to_string()));
    }

    #[test]
    fn test_validate_role_profile() {
        let mut profile = Profile {
            role_arn: Some("arn:aws:iam::123456789012:role/Test".to_string()),
            ..Default::default()
        };

        // Should fail without source
        assert!(profile.validate().is_err());

        // Should pass with source_profile
        profile.source_profile = Some("default".to_string());
        assert!(profile.validate().is_ok());
    }

    #[test]
    fn test_validate_credential_source_environment() {
        let profile = Profile {
            role_arn: Some("arn:aws:iam::123456789012:role/Test".to_string()),
            credential_source: Some("Environment".to_string()),
            ..Default::default()
        };
        assert!(profile.validate().is_ok());
    }

    #[test]
    fn test_validate_credential_source_ec2() {
        let profile = Profile {
            role_arn: Some("arn:aws:iam::123456789012:role/Test".to_string()),
            credential_source: Some("Ec2InstanceMetadata".to_string()),
            ..Default::default()
        };
        assert!(profile.validate().is_ok());
    }

    #[test]
    fn test_validate_credential_source_ecs() {
        let profile = Profile {
            role_arn: Some("arn:aws:iam::123456789012:role/Test".to_string()),
            credential_source: Some("EcsContainer".to_string()),
            ..Default::default()
        };
        assert!(profile.validate().is_ok());
    }

    #[test]
    fn test_validate_source_and_credential_source_conflict() {
        let profile = Profile {
            role_arn: Some("arn:aws:iam::123456789012:role/Test".to_string()),
            source_profile: Some("default".to_string()),
            credential_source: Some("Environment".to_string()),
            ..Default::default()
        };
        let err = profile.validate().unwrap_err();
        matches!(err, ProfileValidationError::ConflictingCredentialSource);
    }

    #[test]
    fn test_validate_credential_process_role() {
        let profile = Profile {
            role_arn: Some("arn:aws:iam::123456789012:role/Test".to_string()),
            credential_process: Some("my-cred-process".to_string()),
            ..Default::default()
        };
        assert!(profile.validate().is_ok());
    }

    #[test]
    fn test_validate_user_profile_missing_keys() {
        let profile = Profile::default();
        let err = profile.validate().unwrap_err();
        matches!(err, ProfileValidationError::MissingAccessKeys);
    }

    #[test]
    fn test_validate_user_with_credential_process() {
        let profile = Profile {
            credential_process: Some("my-cred-process".to_string()),
            ..Default::default()
        };
        assert!(profile.validate().is_ok());
    }

    #[test]
    fn test_is_sso_profile() {
        let mut profile = Profile::default();
        assert!(!profile.is_sso_profile());

        profile.sso_start_url = Some("https://example.awsapps.com/start".to_string());
        assert!(!profile.is_sso_profile()); // Needs account_id too

        profile.sso_account_id = Some("123456789012".to_string());
        assert!(profile.is_sso_profile());
    }
}
