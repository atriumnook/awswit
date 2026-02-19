use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Temporary AWS credentials
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: Option<String>,
    pub expiration: Option<DateTime<Utc>>,
    pub region: Option<String>,
}

impl Credentials {
    /// Check if these credentials have expired
    pub fn is_expired(&self) -> bool {
        if let Some(expiration) = self.expiration {
            // Consider expired if less than 60 seconds remaining
            Utc::now() >= expiration - chrono::Duration::seconds(60)
        } else {
            false
        }
    }

    /// Get remaining time until expiration as a human-readable string
    pub fn remaining_time(&self) -> Option<String> {
        self.expiration.map(|exp| {
            let remaining = exp - Utc::now();
            if remaining.num_seconds() <= 0 {
                "expired".to_string()
            } else if remaining.num_hours() > 0 {
                format!("{}h {}m", remaining.num_hours(), remaining.num_minutes() % 60)
            } else if remaining.num_minutes() > 0 {
                format!("{}m {}s", remaining.num_minutes(), remaining.num_seconds() % 60)
            } else {
                format!("{}s", remaining.num_seconds())
            }
        })
    }

    /// Format as credential_process JSON output
    pub fn to_credential_process_json(&self) -> serde_json::Value {
        let mut json = serde_json::json!({
            "Version": 1,
            "AccessKeyId": self.access_key_id,
            "SecretAccessKey": self.secret_access_key,
        });

        if let Some(ref token) = self.session_token {
            json["SessionToken"] = serde_json::Value::String(token.clone());
        }
        if let Some(ref exp) = self.expiration {
            json["Expiration"] = serde_json::Value::String(exp.to_rfc3339());
        }

        json
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_credentials_not_expired() {
        let creds = Credentials {
            access_key_id: "AKID".to_string(),
            secret_access_key: "SECRET".to_string(),
            session_token: None,
            expiration: Some(Utc::now() + chrono::Duration::hours(1)),
            region: None,
        };
        assert!(!creds.is_expired());
    }

    #[test]
    fn test_credentials_expired() {
        let creds = Credentials {
            access_key_id: "AKID".to_string(),
            secret_access_key: "SECRET".to_string(),
            session_token: None,
            expiration: Some(Utc::now() - chrono::Duration::hours(1)),
            region: None,
        };
        assert!(creds.is_expired());
    }

    #[test]
    fn test_credentials_no_expiration() {
        let creds = Credentials {
            access_key_id: "AKID".to_string(),
            secret_access_key: "SECRET".to_string(),
            session_token: None,
            expiration: None,
            region: None,
        };
        assert!(!creds.is_expired());
    }

    #[test]
    fn test_remaining_time() {
        let creds = Credentials {
            access_key_id: "AKID".to_string(),
            secret_access_key: "SECRET".to_string(),
            session_token: None,
            expiration: Some(Utc::now() + chrono::Duration::hours(1)),
            region: None,
        };
        let remaining = creds.remaining_time().unwrap();
        assert!(remaining.contains("59m") || remaining.contains("1h"));
    }

    #[test]
    fn test_credential_process_json() {
        let creds = Credentials {
            access_key_id: "AKID".to_string(),
            secret_access_key: "SECRET".to_string(),
            session_token: Some("TOKEN".to_string()),
            expiration: None,
            region: None,
        };
        let json = creds.to_credential_process_json();
        assert_eq!(json["Version"], 1);
        assert_eq!(json["AccessKeyId"], "AKID");
        assert_eq!(json["SecretAccessKey"], "SECRET");
        assert_eq!(json["SessionToken"], "TOKEN");
    }
}
