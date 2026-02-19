use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{AwswitError, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebugConfig {
    #[serde(default = "default_session_token_duration")]
    pub session_token_duration: i64,
}

fn default_session_token_duration() -> i64 {
    43200
}

impl Default for DebugConfig {
    fn default() -> Self {
        Self {
            session_token_duration: default_session_token_duration(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AwswitConfig {
    #[serde(default = "default_true")]
    pub colors: bool,

    #[serde(default = "default_true", rename = "fuzzy-match")]
    pub fuzzy_match: bool,

    #[serde(default = "default_role_duration", rename = "role-duration")]
    pub role_duration: i32,

    #[serde(default, rename = "role-session-name")]
    pub role_session_name: Option<String>,

    #[serde(default)]
    pub region: Option<String>,

    #[serde(default)]
    pub debug: DebugConfig,
}

fn default_true() -> bool {
    true
}

fn default_role_duration() -> i32 {
    3600
}

impl Default for AwswitConfig {
    fn default() -> Self {
        Self {
            colors: true,
            fuzzy_match: true,
            role_duration: default_role_duration(),
            role_session_name: None,
            region: None,
            debug: DebugConfig::default(),
        }
    }
}

impl AwswitConfig {
    /// Get the awswit configuration directory path
    pub fn config_dir() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("~"))
            .join(".awswit")
    }

    /// Get the awswit configuration file path
    pub fn config_path() -> PathBuf {
        Self::config_dir().join("config.yaml")
    }

    /// Load configuration from the default path
    pub fn load() -> Result<Self> {
        let path = Self::config_path();
        if !path.exists() {
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(&path).map_err(|e| {
            AwswitError::config_file_error(format!(
                "Failed to read config file {}: {}",
                path.display(),
                e
            ))
        })?;

        let config: Self = serde_yaml::from_str(&content).map_err(|e| {
            AwswitError::config_file_error(format!(
                "Failed to parse config file {}: {}",
                path.display(),
                e
            ))
        })?;

        Ok(config)
    }

    /// Ensure the configuration directory exists with proper permissions
    pub fn ensure_config_dir() -> Result<PathBuf> {
        let dir = Self::config_dir();
        if !dir.exists() {
            std::fs::create_dir_all(&dir)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
            }
        }
        Ok(dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = AwswitConfig::default();
        assert!(config.colors);
        assert!(config.fuzzy_match);
        assert_eq!(config.role_duration, 3600);
        assert!(config.role_session_name.is_none());
        assert!(config.region.is_none());
    }

    #[test]
    fn test_deserialize_config() {
        let yaml = r#"
colors: true
fuzzy-match: true
role-duration: 7200
region: us-west-2
role-session-name: my-session
debug:
  session_token_duration: 21600
"#;
        let config: AwswitConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(config.colors);
        assert!(config.fuzzy_match);
        assert_eq!(config.role_duration, 7200);
        assert_eq!(config.region.as_deref(), Some("us-west-2"));
        assert_eq!(config.role_session_name.as_deref(), Some("my-session"));
        assert_eq!(config.debug.session_token_duration, 21600);
    }
}
