use serde::{Deserialize, Serialize};

/// Represents the type of an AWS profile
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileType {
    /// Profile with direct IAM user credentials
    User,
    /// Profile that assumes a role
    Role,
    /// Profile using SSO
    Sso,
    /// Profile using credential_process
    CredentialProcess,
    /// Profile using credential_source (e.g., EC2 instance role)
    CredentialSource,
}

impl std::fmt::Display for ProfileType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProfileType::User => write!(f, "User"),
            ProfileType::Role => write!(f, "Role"),
            ProfileType::Sso => write!(f, "SSO"),
            ProfileType::CredentialProcess => write!(f, "CredentialProcess"),
            ProfileType::CredentialSource => write!(f, "CredentialSource"),
        }
    }
}

/// AWS profile definition
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Profile {
    /// Profile name
    pub name: String,

    // Authentication
    pub aws_access_key_id: Option<String>,
    pub aws_secret_access_key: Option<String>,
    pub aws_session_token: Option<String>,

    // Role configuration
    pub role_arn: Option<String>,
    pub source_profile: Option<String>,
    pub credential_source: Option<String>,
    pub external_id: Option<String>,
    pub role_session_name: Option<String>,
    pub duration_seconds: Option<i32>,

    // MFA
    pub mfa_serial: Option<String>,

    // Other AWS settings
    pub region: Option<String>,
    pub output: Option<String>,
    pub credential_process: Option<String>,

    // SSO
    pub sso_start_url: Option<String>,
    pub sso_region: Option<String>,
    pub sso_account_id: Option<String>,
    pub sso_role_name: Option<String>,

    // awswit-specific
    pub manager: Option<String>,
    pub autoawswit: Option<bool>,
}

impl Profile {
    pub fn new(name: String) -> Self {
        Self {
            name,
            ..Default::default()
        }
    }

    /// Determine the profile type based on its configuration
    pub fn profile_type(&self) -> ProfileType {
        if self.sso_start_url.is_some() {
            ProfileType::Sso
        } else if self.credential_process.is_some() {
            ProfileType::CredentialProcess
        } else if self.credential_source.is_some() {
            ProfileType::CredentialSource
        } else if self.role_arn.is_some() {
            ProfileType::Role
        } else {
            ProfileType::User
        }
    }

    /// Check if this profile has direct credentials
    pub fn has_credentials(&self) -> bool {
        self.aws_access_key_id.is_some() && self.aws_secret_access_key.is_some()
    }

    /// Check if this profile requires MFA
    pub fn requires_mfa(&self) -> bool {
        self.mfa_serial.is_some()
    }

    /// Get a display string showing profile details for preview
    pub fn detail_lines(&self) -> Vec<(String, String)> {
        let mut lines = vec![("Type".to_string(), self.profile_type().to_string())];

        if let Some(ref arn) = self.role_arn {
            lines.push(("Role ARN".to_string(), arn.clone()));
        }
        if let Some(ref sp) = self.source_profile {
            lines.push(("Source Profile".to_string(), sp.clone()));
        }
        if let Some(ref mfa) = self.mfa_serial {
            lines.push(("MFA Serial".to_string(), mfa.clone()));
        }
        if let Some(ref region) = self.region {
            lines.push(("Region".to_string(), region.clone()));
        }
        if let Some(ref cs) = self.credential_source {
            lines.push(("Credential Source".to_string(), cs.clone()));
        }
        if let Some(ref cp) = self.credential_process {
            lines.push(("Credential Process".to_string(), cp.clone()));
        }
        if let Some(ref sso_url) = self.sso_start_url {
            lines.push(("SSO Start URL".to_string(), sso_url.clone()));
        }
        if let Some(ref sso_account) = self.sso_account_id {
            lines.push(("SSO Account ID".to_string(), sso_account.clone()));
        }
        if let Some(ref sso_role) = self.sso_role_name {
            lines.push(("SSO Role Name".to_string(), sso_role.clone()));
        }
        if self.has_credentials() {
            lines.push(("Credentials".to_string(), "Direct (IAM User)".to_string()));
        }
        if let Some(dur) = self.duration_seconds {
            lines.push(("Duration".to_string(), format!("{}s", dur)));
        }

        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profile_type_user() {
        let mut profile = Profile::new("test".to_string());
        profile.aws_access_key_id = Some("AKID".to_string());
        profile.aws_secret_access_key = Some("SECRET".to_string());
        assert_eq!(profile.profile_type(), ProfileType::User);
    }

    #[test]
    fn test_profile_type_role() {
        let mut profile = Profile::new("test".to_string());
        profile.role_arn = Some("arn:aws:iam::123456789012:role/test".to_string());
        profile.source_profile = Some("default".to_string());
        assert_eq!(profile.profile_type(), ProfileType::Role);
    }

    #[test]
    fn test_profile_type_sso() {
        let mut profile = Profile::new("test".to_string());
        profile.sso_start_url = Some("https://my-sso.awsapps.com/start".to_string());
        assert_eq!(profile.profile_type(), ProfileType::Sso);
    }

    #[test]
    fn test_profile_type_credential_process() {
        let mut profile = Profile::new("test".to_string());
        profile.credential_process = Some("/usr/bin/my-cred-process".to_string());
        assert_eq!(profile.profile_type(), ProfileType::CredentialProcess);
    }

    #[test]
    fn test_has_credentials() {
        let mut profile = Profile::new("test".to_string());
        assert!(!profile.has_credentials());
        profile.aws_access_key_id = Some("AKID".to_string());
        assert!(!profile.has_credentials());
        profile.aws_secret_access_key = Some("SECRET".to_string());
        assert!(profile.has_credentials());
    }

    #[test]
    fn test_requires_mfa() {
        let mut profile = Profile::new("test".to_string());
        assert!(!profile.requires_mfa());
        profile.mfa_serial = Some("arn:aws:iam::123456789012:mfa/user".to_string());
        assert!(profile.requires_mfa());
    }
}
