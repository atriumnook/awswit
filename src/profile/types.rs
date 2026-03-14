use serde::{Deserialize, Serialize};

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
}

impl Profile {
    /// Check if this is a role profile (has role_arn)
    pub fn is_role_profile(&self) -> bool {
        self.role_arn.is_some()
    }

    /// Check if this profile uses SSO
    pub fn is_sso_profile(&self) -> bool {
        self.sso_start_url.is_some() && self.sso_account_id.is_some()
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
                let parts: Vec<&str> = arn.split(':').collect();
                if parts.len() >= 5 {
                    Some(parts[4].to_string())
                } else {
                    None
                }
            })
            .or_else(|| {
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
    fn test_get_account_id() {
        let profile = Profile {
            role_arn: Some("arn:aws:iam::123456789012:role/TestRole".to_string()),
            ..Default::default()
        };
        assert_eq!(profile.get_account_id(), Some("123456789012".to_string()));
    }

    #[test]
    fn test_is_sso_profile() {
        let mut profile = Profile::default();
        assert!(!profile.is_sso_profile());

        profile.sso_start_url = Some("https://example.awsapps.com/start".to_string());
        assert!(!profile.is_sso_profile());

        profile.sso_account_id = Some("123456789012".to_string());
        assert!(profile.is_sso_profile());
    }
}
