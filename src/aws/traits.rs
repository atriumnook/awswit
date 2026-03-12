use async_trait::async_trait;

use crate::aws::Credentials;
use crate::error::AwswitError;

/// Abstraction over AWS STS operations for testability.
#[async_trait]
pub trait StsOperations: Send + Sync {
    /// Assume a role
    #[allow(clippy::too_many_arguments)]
    async fn assume_role(
        &self,
        source_credentials: Option<&Credentials>,
        role_arn: &str,
        session_name: &str,
        external_id: Option<&str>,
        region: Option<&str>,
        duration_seconds: Option<i32>,
        mfa_serial: Option<&str>,
        mfa_token: Option<&str>,
    ) -> Result<Credentials, AwswitError>;

    /// Get session token (with optional MFA)
    async fn get_session_token(
        &self,
        source_credentials: &Credentials,
        mfa_serial: Option<&str>,
        mfa_token: Option<&str>,
        duration_seconds: Option<i32>,
    ) -> Result<Credentials, AwswitError>;
}
