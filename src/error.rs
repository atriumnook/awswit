use thiserror::Error;

pub type Result<T> = std::result::Result<T, AwswitError>;

#[derive(Error, Debug)]
pub enum AwswitError {
    #[error("[E001] Profile not found: {name}")]
    ProfileNotFound { name: String },

    #[error("[E002] Invalid profile '{profile_name}': {message}")]
    InvalidProfile {
        profile_name: String,
        message: String,
    },

    #[error("[E003] Source profile not found: {name}")]
    SourceProfileNotFound { name: String },

    #[error("[E004] Role chain cycle detected: {chain}")]
    RoleChainCycle { chain: String },

    #[error("[E005] Missing required profile key: {key} in profile {profile_name}")]
    MissingProfileKey { profile_name: String, key: String },

    #[error("[E006] Invalid credential source: {name}")]
    InvalidCredentialSource { name: String },

    #[error("[E007] Failed to assume role: {message}")]
    AssumeRoleFailed { message: String },

    #[error("[E008] Failed to get session token: {message}")]
    GetSessionTokenFailed { message: String },

    #[error("[E009] MFA token required")]
    MfaTokenRequired,

    #[error("[E010] Invalid MFA token: {message}")]
    InvalidMfaToken { message: String },

    #[error("[E011] Cache error: {message}")]
    CacheError { message: String },

    #[error("[E012] Config file error: {message}")]
    ConfigFileError { message: String },

    #[error("[E013] Config key not found: {key}")]
    ConfigKeyNotFound { key: String },

    #[error("[E014] Invalid config command: {command}")]
    InvalidConfigCommand { command: String },

    #[error("[E015] AWS STS error: {message}")]
    AwsSdkError { message: String },

    #[error("[E016] Credential process failed: {message}")]
    CredentialProcessFailed { message: String },

    #[error("[E017] Auto-refresh error: {message}")]
    AutoRefreshError { message: String },

    #[error("[E018] Shell error: {message}")]
    ShellError { message: String },

    #[error("[E019] IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("[E020] TOML parse error: {0}")]
    TomlDeError(#[from] toml::de::Error),

    #[error("[E028] TOML serialize error: {0}")]
    TomlSerError(#[from] toml::ser::Error),

    #[error("[E021] JSON parse error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("[E022] Validation error: {message}")]
    ValidationError { message: String },

    #[error("[E023] Cannot use auto-refresh with custom role duration > 1 hour")]
    AutoRefreshDurationLimit,

    #[error("[E024] Environment variable error: {message}")]
    EnvError { message: String },

    #[error("[E025] Operation cancelled by user")]
    UserCancelled,

    #[error("[E099] {0}")]
    Other(String),

    #[error("[E026] STS request timed out after {seconds}s")]
    StsTimeout { seconds: u64 },

    #[error("[E027] Invalid STS expiration timestamp")]
    InvalidStsTimestamp,
}

// Convert AWS SDK errors - sanitize for security
impl<E: std::error::Error> From<aws_sdk_sts::error::SdkError<E>> for AwswitError {
    fn from(err: aws_sdk_sts::error::SdkError<E>) -> Self {
        tracing::debug!("AWS SDK error details: {}", err);
        let message = match &err {
            aws_sdk_sts::error::SdkError::ServiceError(service_err) => {
                format!("{}", service_err.err())
            }
            aws_sdk_sts::error::SdkError::TimeoutError(_) => "Request timed out".to_string(),
            aws_sdk_sts::error::SdkError::DispatchFailure(_) => {
                "Failed to connect to AWS".to_string()
            }
            _ => "AWS request failed".to_string(),
        };
        AwswitError::AwsSdkError { message }
    }
}
