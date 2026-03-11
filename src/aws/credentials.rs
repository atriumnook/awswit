use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// AWS credentials with optional expiration
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Credentials {
    /// AWS Access Key ID
    pub access_key_id: String,

    /// AWS Secret Access Key
    pub secret_access_key: String,

    /// Session token (for temporary credentials)
    pub session_token: Option<String>,

    /// Expiration time for temporary credentials
    pub expiration: Option<DateTime<Utc>>,

    /// AWS region
    pub region: Option<String>,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let masked_key = if self.access_key_id.is_ascii() && self.access_key_id.len() > 4 {
            format!(
                "{}...{}",
                &self.access_key_id[..4],
                &self.access_key_id[self.access_key_id.len() - 4..]
            )
        } else {
            "[REDACTED]".to_string()
        };
        f.debug_struct("Credentials")
            .field("access_key_id", &masked_key)
            .field("secret_access_key", &"[REDACTED]")
            .field(
                "session_token",
                &self.session_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("expiration", &self.expiration)
            .field("region", &self.region)
            .finish()
    }
}

impl Credentials {
    /// Check if credentials are expired
    pub fn is_expired(&self) -> bool {
        if let Some(expiration) = self.expiration {
            expiration < Utc::now()
        } else {
            false
        }
    }

    /// Get time until expiration
    pub fn time_until_expiration(&self) -> Option<chrono::Duration> {
        self.expiration.map(|exp| exp - Utc::now())
    }

    /// Format expiration for display
    pub fn expiration_string(&self) -> String {
        if let Some(exp) = self.expiration {
            exp.format("%Y-%m-%d %H:%M:%S").to_string()
        } else {
            "Never".to_string()
        }
    }

    /// Set the region
    pub fn with_region(mut self, region: Option<String>) -> Self {
        self.region = region;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_expired() {
        let mut creds = Credentials::default();

        // No expiration - not expired
        assert!(!creds.is_expired());

        // Future expiration - not expired
        creds.expiration = Some(Utc::now() + chrono::Duration::hours(1));
        assert!(!creds.is_expired());

        // Past expiration - expired
        creds.expiration = Some(Utc::now() - chrono::Duration::hours(1));
        assert!(creds.is_expired());
    }

}
