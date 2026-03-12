//! Shared credential file operations for autorefresh.
//!
//! Centralizes INI-safe validation, locking, and atomic writes to
//! `~/.aws/credentials` so that both `daemon.rs` and `runner.rs` use
//! a single, validated code path.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::aws::Credentials;
use crate::error::AwswitError;

/// Validate that a profile name is safe for use in INI section headers.
/// Allowed characters: alphanumeric, `_`, `-`, `.`.
pub fn validate_profile_name(name: &str) -> Result<(), AwswitError> {
    if name.is_empty() {
        return Err(AwswitError::ValidationError {
            message: "Profile name cannot be empty".to_string(),
        });
    }
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        return Err(AwswitError::ValidationError {
            message: format!(
                "Profile name '{}' contains invalid characters (allowed: alphanumeric, _, -, .)",
                name
            ),
        });
    }
    Ok(())
}

/// Validate that a credential value is safe for INI files.
/// Rejects values that could inject section headers or contain control characters.
pub fn validate_credential_value(key: &str, value: &str) -> Result<(), AwswitError> {
    if value.starts_with('[') || value.chars().any(|c| c.is_control()) {
        return Err(AwswitError::ValidationError {
            message: format!(
                "Credential value for '{}' contains invalid characters",
                key
            ),
        });
    }
    Ok(())
}

/// Get the default `~/.aws/credentials` path.
pub fn get_aws_credentials_path() -> Result<PathBuf, AwswitError> {
    crate::utils::paths::aws_credentials_path().map_err(|e| AwswitError::AutoRefreshError {
        message: e.to_string(),
    })
}

/// Lock timeout for credential file operations.
const LOCK_TIMEOUT: Duration = Duration::from_secs(30);

/// Acquire an exclusive lock on the credentials file with timeout.
/// Returns the lock file handle (lock released on Drop).
pub fn lock_aws_credentials_file(creds_path: &Path) -> Result<fs::File, AwswitError> {
    let mut lock_path = creds_path.as_os_str().to_owned();
    lock_path.push(".lock");
    let lock_path = PathBuf::from(lock_path);

    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let lock_file = opts.open(&lock_path)?;

    lock_with_timeout(&lock_file, LOCK_TIMEOUT).map_err(|e| AwswitError::AutoRefreshError {
        message: format!("Failed to acquire credentials lock: {}", e),
    })?;

    Ok(lock_file)
}

/// Try to acquire an exclusive lock with exponential backoff and timeout.
fn lock_with_timeout(file: &fs::File, timeout: Duration) -> io::Result<()> {
    crate::utils::fs::lock_exclusive_with_timeout(file, timeout)
}

/// Remove an INI section from credential file content.
fn remove_credentials_section(content: &str, profile_name: &str) -> String {
    let section_header = format!("[{}]", profile_name);
    let mut new_lines = Vec::new();
    let mut skip = false;

    for line in content.lines() {
        if line.starts_with('[') {
            skip = line.trim() == section_header;
        }
        if !skip {
            new_lines.push(line);
        }
    }

    let mut new_content = new_lines.join("\n");
    if !new_content.ends_with('\n') && !new_content.is_empty() {
        new_content.push('\n');
    }

    new_content
}

/// Write credentials for a profile to the credentials file.
/// Converts Credentials to key/value and delegates to write_credentials_from_output.
pub fn write_credentials(
    creds_path: &Path,
    profile_name: &str,
    creds: &Credentials,
) -> Result<(), AwswitError> {
    let expiration_str = creds.expiration.map(|e| e.to_rfc3339());
    write_credentials_from_output(
        creds_path,
        profile_name,
        &creds.access_key_id,
        &creds.secret_access_key,
        creds.session_token.as_deref(),
        expiration_str.as_deref(),
    )
}

/// Write credentials from parsed key=value output (used by runner's refresh_profile).
/// Validates profile name and all values before writing.
pub fn write_credentials_from_output(
    creds_path: &Path,
    profile_name: &str,
    access_key: &str,
    secret_key: &str,
    session_token: Option<&str>,
    expiration: Option<&str>,
) -> Result<(), AwswitError> {
    validate_profile_name(profile_name)?;
    validate_credential_value("aws_access_key_id", access_key)?;
    validate_credential_value("aws_secret_access_key", secret_key)?;
    if let Some(token) = session_token {
        validate_credential_value("aws_session_token", token)?;
    }
    if let Some(exp) = expiration {
        validate_credential_value("awswit_expiration", exp)?;
    }

    let _lock_file = lock_aws_credentials_file(creds_path)?;

    let content = match fs::read_to_string(creds_path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            return Err(AwswitError::AutoRefreshError {
                message: format!("Failed to read credentials file {}: {}", creds_path.display(), e),
            });
        }
    };
    let mut new_content = remove_credentials_section(&content, profile_name);

    let mut new_section = format!(
        "[{}]\n\
        aws_access_key_id = {}\n\
        aws_secret_access_key = {}\n",
        profile_name, access_key, secret_key,
    );
    if let Some(token) = session_token {
        new_section.push_str(&format!("aws_session_token = {}\n", token));
    }
    new_section.push_str("autoawswit = true\n");
    if let Some(exp) = expiration {
        new_section.push_str(&format!("awswit_expiration = {}\n", exp));
    }

    new_content.push_str(&new_section);

    crate::utils::fs::atomic_write_restricted(creds_path, new_content.as_bytes())?;

    Ok(())
}

/// Remove multiple profile sections from the credentials file in a single lock-read-write cycle.
pub fn remove_credentials_batch(
    creds_path: &Path,
    profile_names: &[String],
) -> Result<(), AwswitError> {
    if profile_names.is_empty() {
        return Ok(());
    }

    let _lock_file = lock_aws_credentials_file(creds_path)?;

    let mut content = match fs::read_to_string(creds_path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    for name in profile_names {
        content = remove_credentials_section(&content, name);
    }

    crate::utils::fs::atomic_write_restricted(creds_path, content.as_bytes())?;
    Ok(())
}

/// Remove a profile section from the credentials file.
pub fn remove_credentials(creds_path: &Path, profile_name: &str) -> Result<(), AwswitError> {
    let _lock_file = lock_aws_credentials_file(creds_path)?;

    let content = match fs::read_to_string(creds_path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    let new_content = remove_credentials_section(&content, profile_name);

    crate::utils::fs::atomic_write_restricted(creds_path, new_content.as_bytes())?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_credentials_path() -> (TempDir, PathBuf) {
        let temp_dir = TempDir::new().unwrap();
        let creds_dir = temp_dir.path().join(".aws");
        fs::create_dir_all(&creds_dir).unwrap();
        (temp_dir, creds_dir.join("credentials"))
    }

    #[test]
    fn validate_profile_name_accepts_valid() {
        assert!(validate_profile_name("autoawswit-dev").is_ok());
        assert!(validate_profile_name("my_profile.1").is_ok());
    }

    #[test]
    fn validate_profile_name_rejects_empty() {
        assert!(validate_profile_name("").is_err());
    }

    #[test]
    fn validate_profile_name_rejects_section_injection() {
        assert!(validate_profile_name("evil]\n[injected").is_err());
        assert!(validate_profile_name("evil\nkey=val").is_err());
    }

    #[test]
    fn validate_profile_name_rejects_brackets() {
        assert!(validate_profile_name("[evil]").is_err());
    }

    #[test]
    fn validate_credential_value_accepts_valid() {
        assert!(validate_credential_value("key", "AKIAIOSFODNN7EXAMPLE").is_ok());
    }

    #[test]
    fn validate_credential_value_rejects_section_header() {
        assert!(validate_credential_value("key", "[injected]").is_err());
    }

    #[test]
    fn validate_credential_value_rejects_control_chars() {
        assert!(validate_credential_value("key", "value\ninjected").is_err());
    }

    #[test]
    fn write_and_remove_credentials() {
        let (_temp, creds_path) = create_test_credentials_path();
        fs::write(
            &creds_path,
            "[default]\naws_access_key_id = ORIGINAL\n",
        )
        .unwrap();

        let creds = Credentials {
            access_key_id: "AKIATEST".to_string(),
            secret_access_key: "secret".to_string(),
            session_token: Some("token".to_string()),
            expiration: None,
            region: None,
        };

        write_credentials(&creds_path, "autoawswit-test", &creds).unwrap();

        let content = fs::read_to_string(&creds_path).unwrap();
        assert!(content.contains("[default]\naws_access_key_id = ORIGINAL\n"));
        assert!(content.contains("[autoawswit-test]\n"));
        assert!(content.contains("aws_access_key_id = AKIATEST\n"));

        remove_credentials(&creds_path, "autoawswit-test").unwrap();

        let content = fs::read_to_string(&creds_path).unwrap();
        assert!(content.contains("[default]"));
        assert!(!content.contains("[autoawswit-test]"));
    }

    #[test]
    fn write_credentials_rejects_ini_injection_in_profile() {
        let (_temp, creds_path) = create_test_credentials_path();
        fs::write(&creds_path, "").unwrap();

        let creds = Credentials {
            access_key_id: "AKIATEST".to_string(),
            secret_access_key: "secret".to_string(),
            session_token: None,
            expiration: None,
            region: None,
        };

        let result = write_credentials(&creds_path, "evil]\n[inject", &creds);
        assert!(result.is_err());
    }

    #[test]
    fn write_credentials_to_empty_file() {
        let (_temp, creds_path) = create_test_credentials_path();
        fs::write(&creds_path, "").unwrap();

        let creds = Credentials {
            access_key_id: "AKIATEST".to_string(),
            secret_access_key: "secret".to_string(),
            session_token: None,
            expiration: None,
            region: None,
        };

        write_credentials(&creds_path, "autoawswit-test", &creds).unwrap();

        let content = fs::read_to_string(&creds_path).unwrap();
        assert!(content.contains("[autoawswit-test]"));
        assert!(content.contains("aws_access_key_id = AKIATEST"));
    }

    #[test]
    fn write_credentials_handles_crlf_input() {
        let (_temp, creds_path) = create_test_credentials_path();
        // Write content with CRLF line endings
        fs::write(
            &creds_path,
            "[default]\r\naws_access_key_id = ORIGINAL\r\n",
        )
        .unwrap();

        let creds = Credentials {
            access_key_id: "AKIATEST".to_string(),
            secret_access_key: "secret".to_string(),
            session_token: None,
            expiration: None,
            region: None,
        };

        write_credentials(&creds_path, "autoawswit-test", &creds).unwrap();

        let content = fs::read_to_string(&creds_path).unwrap();
        assert!(content.contains("[autoawswit-test]"));
        assert!(content.contains("aws_access_key_id = AKIATEST"));
    }

    #[test]
    fn remove_credentials_batch_removes_multiple() {
        let (_temp, creds_path) = create_test_credentials_path();
        fs::write(
            &creds_path,
            "[default]\naws_access_key_id = KEEP\n[autoawswit-a]\naws_access_key_id = A\n[autoawswit-b]\naws_access_key_id = B\n[other]\naws_access_key_id = OTHER\n",
        )
        .unwrap();

        let names = vec!["autoawswit-a".to_string(), "autoawswit-b".to_string()];
        remove_credentials_batch(&creds_path, &names).unwrap();

        let content = fs::read_to_string(&creds_path).unwrap();
        assert!(content.contains("[default]"));
        assert!(content.contains("[other]"));
        assert!(!content.contains("[autoawswit-a]"));
        assert!(!content.contains("[autoawswit-b]"));
    }

    #[test]
    fn remove_credentials_batch_empty_list() {
        let (_temp, creds_path) = create_test_credentials_path();
        fs::write(&creds_path, "[default]\naws_access_key_id = KEEP\n").unwrap();

        remove_credentials_batch(&creds_path, &[]).unwrap();

        let content = fs::read_to_string(&creds_path).unwrap();
        assert!(content.contains("[default]"));
    }

    #[test]
    fn remove_credentials_nonexistent_file() {
        let (_temp, creds_path) = create_test_credentials_path();
        // Don't create the file
        let result = remove_credentials(&creds_path, "anything");
        assert!(result.is_ok());
    }

    #[test]
    fn remove_section_preserves_surrounding_content() {
        let content = "[before]\nkey1 = val1\n[target]\nkey2 = val2\n[after]\nkey3 = val3\n";
        let result = remove_credentials_section(content, "target");
        assert!(result.contains("[before]"));
        assert!(result.contains("key1 = val1"));
        assert!(!result.contains("[target]"));
        assert!(!result.contains("key2 = val2"));
        assert!(result.contains("[after]"));
        assert!(result.contains("key3 = val3"));
    }
}
