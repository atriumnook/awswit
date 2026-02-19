use thiserror::Error;

/// Application error codes matching the specification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// E001: Specified profile not found
    ProfileNotFound,
    /// E002: Cached credentials expired
    CredentialsExpired,
    /// E003: AssumeRole API call failed
    AssumeRoleFailed,
    /// E004: MFA token required
    MfaRequired,
    /// E005: MFA token invalid
    InvalidMfaToken,
    /// E006: Role chain too deep (> 10)
    RoleChainTooDeep,
    /// E007: Config file read error
    ConfigFileError,
    /// E008: Cache operation error
    CacheError,
    /// E009: Shell integration error
    ShellError,
    /// E010: Auto-refresh duration limit exceeded
    AutoRefreshDurationLimit,
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let code = match self {
            ErrorCode::ProfileNotFound => "E001",
            ErrorCode::CredentialsExpired => "E002",
            ErrorCode::AssumeRoleFailed => "E003",
            ErrorCode::MfaRequired => "E004",
            ErrorCode::InvalidMfaToken => "E005",
            ErrorCode::RoleChainTooDeep => "E006",
            ErrorCode::ConfigFileError => "E007",
            ErrorCode::CacheError => "E008",
            ErrorCode::ShellError => "E009",
            ErrorCode::AutoRefreshDurationLimit => "E010",
        };
        write!(f, "{}", code)
    }
}

#[derive(Debug, Error)]
pub enum AwswitError {
    #[error("[{code}] Profile not found: {name}")]
    ProfileNotFound { code: ErrorCode, name: String },

    #[error("[{code}] Credentials expired for profile: {profile}")]
    CredentialsExpired { code: ErrorCode, profile: String },

    #[error("[{code}] AssumeRole failed: {message}")]
    AssumeRoleFailed { code: ErrorCode, message: String },

    #[error("[{code}] MFA token required for: {mfa_serial}")]
    MfaRequired { code: ErrorCode, mfa_serial: String },

    #[error("[{code}] Invalid MFA token")]
    InvalidMfaToken { code: ErrorCode },

    #[error("[{code}] Role chain too deep (max 10): {chain}")]
    RoleChainTooDeep { code: ErrorCode, chain: String },

    #[error("[{code}] Config file error: {message}")]
    ConfigFileError { code: ErrorCode, message: String },

    #[error("[{code}] Cache error: {message}")]
    CacheError { code: ErrorCode, message: String },

    #[error("[{code}] Shell error: {message}")]
    ShellError { code: ErrorCode, message: String },

    #[error("[{code}] Auto-refresh duration limit exceeded")]
    AutoRefreshDurationLimit { code: ErrorCode },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("YAML error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("AWS STS error: {0}")]
    Sts(String),

    #[error("User cancelled")]
    UserCancelled,

    #[error("{0}")]
    Other(String),
}

impl AwswitError {
    pub fn profile_not_found(name: impl Into<String>) -> Self {
        AwswitError::ProfileNotFound {
            code: ErrorCode::ProfileNotFound,
            name: name.into(),
        }
    }

    pub fn credentials_expired(profile: impl Into<String>) -> Self {
        AwswitError::CredentialsExpired {
            code: ErrorCode::CredentialsExpired,
            profile: profile.into(),
        }
    }

    pub fn assume_role_failed(message: impl Into<String>) -> Self {
        AwswitError::AssumeRoleFailed {
            code: ErrorCode::AssumeRoleFailed,
            message: message.into(),
        }
    }

    pub fn mfa_required(mfa_serial: impl Into<String>) -> Self {
        AwswitError::MfaRequired {
            code: ErrorCode::MfaRequired,
            mfa_serial: mfa_serial.into(),
        }
    }

    pub fn invalid_mfa_token() -> Self {
        AwswitError::InvalidMfaToken {
            code: ErrorCode::InvalidMfaToken,
        }
    }

    pub fn role_chain_too_deep(chain: impl Into<String>) -> Self {
        AwswitError::RoleChainTooDeep {
            code: ErrorCode::RoleChainTooDeep,
            chain: chain.into(),
        }
    }

    pub fn config_file_error(message: impl Into<String>) -> Self {
        AwswitError::ConfigFileError {
            code: ErrorCode::ConfigFileError,
            message: message.into(),
        }
    }

    pub fn cache_error(message: impl Into<String>) -> Self {
        AwswitError::CacheError {
            code: ErrorCode::CacheError,
            message: message.into(),
        }
    }

    pub fn shell_error(message: impl Into<String>) -> Self {
        AwswitError::ShellError {
            code: ErrorCode::ShellError,
            message: message.into(),
        }
    }

    pub fn auto_refresh_duration_limit() -> Self {
        AwswitError::AutoRefreshDurationLimit {
            code: ErrorCode::AutoRefreshDurationLimit,
        }
    }
}

pub type Result<T> = std::result::Result<T, AwswitError>;
