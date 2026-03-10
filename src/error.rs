use thiserror::Error;

#[derive(Error, Debug)]
pub enum AwswitError {
    #[error("Profile not found: {0}")]
    ProfileNotFound(String),

    #[error("Invalid profile '{profile_name}': {message}")]
    InvalidProfile {
        profile_name: String,
        message: String,
    },

    #[error("Source profile not found: {0}")]
    SourceProfileNotFound(String),

    #[error("Role chain cycle detected: {0}")]
    RoleChainCycle(String),

    #[error("Missing required profile key: {key} in profile {profile_name}")]
    MissingProfileKey {
        profile_name: String,
        key: String,
    },

    #[error("Invalid credential source: {0}")]
    InvalidCredentialSource(String),

    #[error("Failed to assume role: {0}")]
    AssumeRoleFailed(String),

    #[error("Failed to get session token: {0}")]
    GetSessionTokenFailed(String),

    #[error("MFA token required")]
    MfaTokenRequired,

    #[error("Invalid MFA token")]
    InvalidMfaToken,

    #[error("Credentials expired")]
    CredentialsExpired,

    #[error("Cache error: {0}")]
    CacheError(String),

    #[error("Config file error: {0}")]
    ConfigFileError(String),

    #[error("Config key not found: {0}")]
    ConfigKeyNotFound(String),

    #[error("Invalid config command: {0}")]
    InvalidConfigCommand(String),

    #[error("AWS SDK error: {0}")]
    AwsSdkError(String),

    #[error("Credential process failed: {0}")]
    CredentialProcessFailed(String),

    #[error("Auto-refresh error: {0}")]
    AutoRefreshError(String),

    #[error("Shell error: {0}")]
    ShellError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("YAML parse error: {0}")]
    YamlError(#[from] serde_yaml::Error),

    #[error("JSON parse error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("Validation error: {0}")]
    ValidationError(String),

    #[error("Cannot use auto-refresh with custom role duration > 1 hour")]
    AutoRefreshDurationLimit,

    #[error("Environment variable error: {0}")]
    EnvError(String),
}

impl AwswitError {
    pub fn invalid_profile(profile_name: impl Into<String>, message: impl Into<String>) -> Self {
        Self::InvalidProfile {
            profile_name: profile_name.into(),
            message: message.into(),
        }
    }

    pub fn missing_key(profile_name: impl Into<String>, key: impl Into<String>) -> Self {
        Self::MissingProfileKey {
            profile_name: profile_name.into(),
            key: key.into(),
        }
    }
}

// Convert AWS SDK errors
impl<E: std::error::Error> From<aws_sdk_sts::error::SdkError<E>> for AwswitError {
    fn from(err: aws_sdk_sts::error::SdkError<E>) -> Self {
        AwswitError::AwsSdkError(err.to_string())
    }
}
