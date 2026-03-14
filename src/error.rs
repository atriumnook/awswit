use thiserror::Error;

pub type Result<T> = std::result::Result<T, AwswitError>;

#[derive(Error, Debug)]
pub enum AwswitError {
    #[error("[E001] Profile not found: {name}")]
    ProfileNotFound { name: String },

    #[error("[E012] Config file error: {message}")]
    ConfigFileError { message: String },

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

    #[error("[E025] Operation cancelled by user")]
    UserCancelled,
}
