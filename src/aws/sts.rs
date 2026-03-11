use std::time::Duration;

use aws_sdk_sts::Client;
use chrono::{DateTime, Utc};
use tokio::time::timeout;

use crate::aws::Credentials;
use crate::error::AwswitError;

const STS_TIMEOUT_SECS: u64 = 30;

/// Parameters for AssumeRole calls, replacing the 8-argument method signature
#[derive(Default)]
#[allow(dead_code)]
pub struct AssumeRoleParams<'a> {
    pub source_credentials: Option<&'a Credentials>,
    pub role_arn: &'a str,
    pub session_name: &'a str,
    pub external_id: Option<&'a str>,
    pub region: Option<&'a str>,
    pub duration_seconds: Option<i32>,
    pub mfa_serial: Option<&'a str>,
    pub mfa_token: Option<&'a str>,
}

/// AWS STS client wrapper
pub struct StsClient {
    default_client: Client,
}

impl StsClient {
    /// Create a new STS client
    pub async fn new() -> Self {
        let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;

        // Warn if no region is explicitly configured (silent fallback breaks GovCloud/China)
        if config.region().is_none() {
            tracing::warn!(
                "No AWS region configured. Falling back to SDK default (us-east-1). \
                 This may not work for GovCloud or China regions. \
                 Set AWS_REGION or configure a region in your profile."
            );
        }

        let client = Client::new(&config);
        Self {
            default_client: client,
        }
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

        let mut config_builder =
            aws_config::defaults(aws_config::BehaviorVersion::latest()).credentials_provider(creds);

        if let Some(r) = region {
            config_builder = config_builder.region(aws_config::Region::new(r.to_string()));
        }

        let config = config_builder.load().await;
        Ok(Client::new(&config))
    }

    /// Assume a role
    #[allow(clippy::too_many_arguments)]
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

        let response = timeout(Duration::from_secs(STS_TIMEOUT_SECS), request.send())
            .await
            .map_err(|_| AwswitError::StsTimeout {
                seconds: STS_TIMEOUT_SECS,
            })?
            .map_err(|e| {
                tracing::debug!("AssumeRole SDK error: {}", e);
                AwswitError::AssumeRoleFailed {
                    message: sanitize_sdk_error(&e),
                }
            })?;

        let aws_creds = response
            .credentials()
            .ok_or_else(|| AwswitError::AssumeRoleFailed {
                message: "No credentials in response".to_string(),
            })?;

        let expiration = parse_sts_expiration(aws_creds.expiration())?;

        Ok(Credentials {
            access_key_id: aws_creds.access_key_id().to_string(),
            secret_access_key: aws_creds.secret_access_key().to_string(),
            session_token: Some(aws_creds.session_token().to_string()),
            expiration: Some(expiration),
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

        let client =
            Self::client_with_credentials(source_credentials, source_credentials.region.as_deref())
                .await?;

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

        let response = timeout(Duration::from_secs(STS_TIMEOUT_SECS), request.send())
            .await
            .map_err(|_| AwswitError::StsTimeout {
                seconds: STS_TIMEOUT_SECS,
            })?
            .map_err(|e| {
                tracing::debug!("GetSessionToken SDK error: {}", e);
                AwswitError::GetSessionTokenFailed {
                    message: sanitize_sdk_error(&e),
                }
            })?;

        let aws_creds =
            response
                .credentials()
                .ok_or_else(|| AwswitError::GetSessionTokenFailed {
                    message: "No credentials in response".to_string(),
                })?;

        let expiration = parse_sts_expiration(aws_creds.expiration())?;

        Ok(Credentials {
            access_key_id: aws_creds.access_key_id().to_string(),
            secret_access_key: aws_creds.secret_access_key().to_string(),
            session_token: Some(aws_creds.session_token().to_string()),
            expiration: Some(expiration),
            region: source_credentials.region.clone(),
        })
    }
}

/// Parse STS expiration timestamp, returning error instead of silently ignoring
fn parse_sts_expiration(
    e: &aws_sdk_sts::primitives::DateTime,
) -> Result<DateTime<Utc>, AwswitError> {
    DateTime::<Utc>::from_timestamp(e.secs(), e.subsec_nanos()).ok_or_else(|| {
        tracing::warn!(
            "Failed to parse STS expiration: secs={}, nanos={}",
            e.secs(),
            e.subsec_nanos()
        );
        AwswitError::InvalidStsTimestamp
    })
}

/// Extract only error code/message from SDK error, hiding request IDs and internal details
fn sanitize_sdk_error<E: std::error::Error>(err: &aws_sdk_sts::error::SdkError<E>) -> String {
    match err {
        aws_sdk_sts::error::SdkError::ServiceError(service_err) => {
            format!("{}", service_err.err())
        }
        aws_sdk_sts::error::SdkError::TimeoutError(_) => "Request timed out".to_string(),
        aws_sdk_sts::error::SdkError::DispatchFailure(_) => "Failed to connect to AWS".to_string(),
        _ => "AWS request failed".to_string(),
    }
}
