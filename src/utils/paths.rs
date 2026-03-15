use std::path::PathBuf;

/// Get the awswit home directory (~/.awswit)
pub fn awswit_home_dir() -> Result<PathBuf, std::io::Error> {
    let home = dirs::home_dir().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Could not determine home directory",
        )
    })?;
    Ok(home.join(".awswit"))
}

/// Get the AWS credentials file path.
///
/// Checks `AWS_SHARED_CREDENTIALS_FILE` environment variable first,
/// falling back to `~/.aws/credentials`.
pub fn aws_credentials_path() -> Result<PathBuf, std::io::Error> {
    if let Ok(path) = std::env::var("AWS_SHARED_CREDENTIALS_FILE")
        && !path.is_empty()
    {
        return Ok(PathBuf::from(path));
    }
    let home = dirs::home_dir().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Could not determine home directory",
        )
    })?;
    Ok(home.join(".aws").join("credentials"))
}

/// Get the AWS config file path.
///
/// Checks `AWS_CONFIG_FILE` environment variable first,
/// falling back to `~/.aws/config`.
pub fn aws_config_path() -> Result<PathBuf, std::io::Error> {
    if let Ok(path) = std::env::var("AWS_CONFIG_FILE")
        && !path.is_empty()
    {
        return Ok(PathBuf::from(path));
    }
    let home = dirs::home_dir().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Could not determine home directory",
        )
    })?;
    Ok(home.join(".aws").join("config"))
}
