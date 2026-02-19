use aws_credential_types::provider::SharedCredentialsProvider;
use aws_sdk_sts::config::Region;

use crate::aws::credentials::Credentials;
use crate::error::{AwswitError, Result};

/// AWS STS client wrapper
pub struct StsClient {
    client: aws_sdk_sts::Client,
}

impl StsClient {
    /// Create a new STS client with the given credentials
    pub async fn new(creds: &Credentials) -> Self {
        let credentials = aws_credential_types::Credentials::new(
            &creds.access_key_id,
            &creds.secret_access_key,
            creds.session_token.clone(),
            None,
            "awswit",
        );

        let region = creds
            .region
            .as_ref()
            .map(|r| Region::new(r.clone()))
            .unwrap_or_else(|| Region::new("us-east-1"));

        let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region(region)
            .credentials_provider(SharedCredentialsProvider::new(credentials))
            .load()
            .await;

        let client = aws_sdk_sts::Client::new(&config);

        Self { client }
    }

    /// Assume a role and return temporary credentials
    pub async fn assume_role(
        &self,
        role_arn: &str,
        session_name: &str,
        external_id: Option<&str>,
        duration_seconds: i32,
        mfa_serial: Option<&str>,
        mfa_token: Option<&str>,
        region: Option<&str>,
    ) -> Result<Credentials> {
        let mut request = self
            .client
            .assume_role()
            .role_arn(role_arn)
            .role_session_name(session_name)
            .duration_seconds(duration_seconds);

        if let Some(eid) = external_id {
            request = request.external_id(eid);
        }
        if let Some(serial) = mfa_serial {
            request = request.serial_number(serial);
        }
        if let Some(token) = mfa_token {
            request = request.token_code(token);
        }

        let response = request.send().await.map_err(|e| {
            let msg = format!("{}", e);
            if msg.contains("AccessDenied") {
                AwswitError::assume_role_failed(format!(
                    "Access denied when assuming role {}: {}",
                    role_arn, msg
                ))
            } else if msg.contains("MalformedPolicyDocument") {
                AwswitError::assume_role_failed(format!(
                    "Malformed policy for role {}: {}",
                    role_arn, msg
                ))
            } else if msg.contains("InvalidIdentityToken") || msg.contains("ValidationError") {
                AwswitError::invalid_mfa_token()
            } else {
                AwswitError::assume_role_failed(format!(
                    "Failed to assume role {}: {}",
                    role_arn, msg
                ))
            }
        })?;

        let sts_creds = response.credentials().ok_or_else(|| {
            AwswitError::assume_role_failed("No credentials in AssumeRole response")
        })?;

        let exp = sts_creds.expiration();
        let expiration = chrono::DateTime::from_timestamp(exp.secs(), exp.subsec_nanos() as u32);

        Ok(Credentials {
            access_key_id: sts_creds.access_key_id().to_string(),
            secret_access_key: sts_creds.secret_access_key().to_string(),
            session_token: Some(sts_creds.session_token().to_string()),
            expiration,
            region: region.map(|r| r.to_string()),
        })
    }

    /// Get a session token (for MFA-protected IAM users)
    pub async fn get_session_token(
        &self,
        mfa_serial: &str,
        mfa_token: &str,
        duration_seconds: i32,
        region: Option<&str>,
    ) -> Result<Credentials> {
        let response = self
            .client
            .get_session_token()
            .serial_number(mfa_serial)
            .token_code(mfa_token)
            .duration_seconds(duration_seconds)
            .send()
            .await
            .map_err(|e| {
                let msg = format!("{}", e);
                if msg.contains("AccessDenied") || msg.contains("ValidationError") {
                    AwswitError::invalid_mfa_token()
                } else {
                    AwswitError::Sts(format!("GetSessionToken failed: {}", msg))
                }
            })?;

        let sts_creds = response
            .credentials()
            .ok_or_else(|| AwswitError::Sts("No credentials in GetSessionToken response".into()))?;

        let exp = sts_creds.expiration();
        let expiration = chrono::DateTime::from_timestamp(exp.secs(), exp.subsec_nanos() as u32);

        Ok(Credentials {
            access_key_id: sts_creds.access_key_id().to_string(),
            secret_access_key: sts_creds.secret_access_key().to_string(),
            session_token: Some(sts_creds.session_token().to_string()),
            expiration,
            region: region.map(|r| r.to_string()),
        })
    }

    /// Get the caller identity
    pub async fn get_caller_identity(&self) -> Result<(String, String, String)> {
        let response = self
            .client
            .get_caller_identity()
            .send()
            .await
            .map_err(|e| AwswitError::Sts(format!("GetCallerIdentity failed: {}", e)))?;

        Ok((
            response.account().unwrap_or_default().to_string(),
            response.arn().unwrap_or_default().to_string(),
            response.user_id().unwrap_or_default().to_string(),
        ))
    }
}
