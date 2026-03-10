use aws_sdk_sts::Client;
use chrono::{DateTime, Utc};

use crate::aws::Credentials;
use crate::error::AwswitError;

/// AWS STS client wrapper
pub struct StsClient {
    default_client: Client,
}

impl StsClient {
    /// Create a new STS client
    pub async fn new() -> Self {
        let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
        let client = Client::new(&config);
        Self { default_client: client }
    }

    /// Create an STS client with specific credentials
    async fn client_with_credentials(
        credentials: &Credentials,
        region: Option<&str>,
    ) -> Result<Client, AwswitError> {
        use aws_credential_types::Credentials as AwsCredentials;

        let creds = AwsCredentials::new(
            &credentials.access_key_id,
            &credentials.secret_access_key,
            credentials.session_token.clone(),
            credentials.expiration.map(|e| e.into()),
            "awswit",
        );

        let mut config_builder = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .credentials_provider(creds);

        if let Some(r) = region {
            config_builder = config_builder.region(aws_config::Region::new(r.to_string()));
        }

        let config = config_builder.load().await;
        Ok(Client::new(&config))
    }

    /// Assume a role
    pub async fn assume_role(
        &self,
        source_credentials: Option<&Credentials>,
        role_arn: &str,
        session_name: &str,
        external_id: Option<&str>,
        region: Option<&str>,
        duration_seconds: Option<i32>,
        mfa_serial: Option<&str>,
        mfa_token: Option<&str>,
    ) -> Result<Credentials, AwswitError> {
        tracing::debug!("Assuming role: {}", role_arn);
        tracing::debug!("Session name: {}", session_name);

        let client = if let Some(creds) = source_credentials {
            Self::client_with_credentials(creds, region).await?
        } else {
            self.default_client.clone()
        };

        let mut request = client
            .assume_role()
            .role_arn(role_arn)
            .role_session_name(session_name);

        if let Some(ext_id) = external_id {
            request = request.external_id(ext_id);
        }

        if let Some(duration) = duration_seconds {
            request = request.duration_seconds(duration);
        }

        if let Some(serial) = mfa_serial {
            request = request.serial_number(serial);
        }

        if let Some(token) = mfa_token {
            request = request.token_code(token);
        }

        let response = request.send().await
            .map_err(|e| AwswitError::AssumeRoleFailed(e.to_string()))?;

        let aws_creds = response.credentials()
            .ok_or_else(|| AwswitError::AssumeRoleFailed("No credentials in response".to_string()))?;

        let e = aws_creds.expiration();
        let expiration = Some(
            DateTime::<Utc>::from_timestamp(e.secs(), e.subsec_nanos())
                .unwrap_or_else(|| {
                    tracing::warn!("Failed to parse STS expiration timestamp (secs={}, nanos={}), falling back to current time", e.secs(), e.subsec_nanos());
                    Utc::now()
                })
        );

        Ok(Credentials {
            access_key_id: aws_creds.access_key_id().to_string(),
            secret_access_key: aws_creds.secret_access_key().to_string(),
            session_token: Some(aws_creds.session_token().to_string()),
            expiration,
            region: region.map(String::from),
        })
    }

    /// Get session token (with optional MFA)
    pub async fn get_session_token(
        &self,
        source_credentials: &Credentials,
        mfa_serial: Option<&str>,
        mfa_token: Option<&str>,
        duration_seconds: Option<i32>,
    ) -> Result<Credentials, AwswitError> {
        tracing::debug!("Getting session token");

        let client = Self::client_with_credentials(
            source_credentials,
            source_credentials.region.as_deref(),
        ).await?;

        let mut request = client.get_session_token();

        if let Some(serial) = mfa_serial {
            request = request.serial_number(serial);
        }

        if let Some(token) = mfa_token {
            request = request.token_code(token);
        }

        if let Some(duration) = duration_seconds {
            request = request.duration_seconds(duration);
        }

        let response = request.send().await
            .map_err(|e| AwswitError::GetSessionTokenFailed(e.to_string()))?;

        let aws_creds = response.credentials()
            .ok_or_else(|| AwswitError::GetSessionTokenFailed("No credentials in response".to_string()))?;

        let e = aws_creds.expiration();
        let expiration = Some(
            DateTime::<Utc>::from_timestamp(e.secs(), e.subsec_nanos())
                .unwrap_or_else(|| {
                    tracing::warn!("Failed to parse STS expiration timestamp (secs={}, nanos={}), falling back to current time", e.secs(), e.subsec_nanos());
                    Utc::now()
                })
        );

        Ok(Credentials {
            access_key_id: aws_creds.access_key_id().to_string(),
            secret_access_key: aws_creds.secret_access_key().to_string(),
            session_token: Some(aws_creds.session_token().to_string()),
            expiration,
            region: source_credentials.region.clone(),
        })
    }

    /// Get caller identity
    pub async fn get_caller_identity(
        &self,
        credentials: Option<&Credentials>,
    ) -> Result<CallerIdentity, AwswitError> {
        let client = if let Some(creds) = credentials {
            Self::client_with_credentials(creds, creds.region.as_deref()).await?
        } else {
            self.default_client.clone()
        };

        let response = client.get_caller_identity().send().await
            .map_err(|e| AwswitError::AwsSdkError(e.to_string()))?;

        Ok(CallerIdentity {
            account: response.account().map(String::from),
            arn: response.arn().map(String::from),
            user_id: response.user_id().map(String::from),
        })
    }
}

/// Result of GetCallerIdentity
#[derive(Debug, Clone)]
pub struct CallerIdentity {
    pub account: Option<String>,
    pub arn: Option<String>,
    pub user_id: Option<String>,
}
