use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::error::AwswitError;

/// awswit configuration stored in ~/.awswit/config.toml
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AwswitConfig {
    /// Enable colored output
    pub colors: bool,

    /// Enable fuzzy profile name matching
    #[serde(rename = "fuzzy-match")]
    pub fuzzy_match: bool,

    /// Default role duration in seconds (0 = use AWS default)
    #[serde(rename = "role-duration")]
    pub role_duration: i32,

    /// Default AWS region
    pub region: Option<String>,

    /// Default role session name
    #[serde(rename = "role-session-name")]
    pub role_session_name: Option<String>,

    /// Custom session token duration in seconds
    #[serde(rename = "session-token-duration")]
    pub session_token_duration: Option<i32>,
}

impl Default for AwswitConfig {
    fn default() -> Self {
        Self {
            colors: !cfg!(windows), // Disabled on Windows by default
            fuzzy_match: true,
            role_duration: 0,
            region: None,
            role_session_name: None,
            session_token_duration: None,
        }
    }
}

impl AwswitConfig {
    /// Get the config file path (TOML)
    pub fn config_path() -> Result<PathBuf, AwswitError> {
        crate::utils::paths::awswit_home_dir()
            .map(|p| p.join("config.toml"))
            .map_err(|e| AwswitError::ConfigFileError {
                message: e.to_string(),
            })
    }

    /// Get the legacy YAML config file path
    fn legacy_yaml_path() -> Result<PathBuf, AwswitError> {
        crate::utils::paths::awswit_home_dir()
            .map(|p| p.join("config.yaml"))
            .map_err(|e| AwswitError::ConfigFileError {
                message: e.to_string(),
            })
    }

    /// Load config from file, or return default if not found.
    /// Prefers config.toml; falls back to config.yaml with a deprecation warning.
    pub fn load() -> Result<Self, AwswitError> {
        let toml_path = Self::config_path()?;

        // Try TOML first
        match fs::read_to_string(&toml_path) {
            Ok(content) => {
                let config: Self = toml::from_str(&content)?;
                return Ok(config);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Fall through to YAML fallback
            }
            Err(e) => {
                return Err(AwswitError::ConfigFileError {
                    message: format!("Failed to read config: {}", e),
                });
            }
        }

        // Try legacy YAML fallback
        let yaml_path = Self::legacy_yaml_path()?;
        match fs::read_to_string(&yaml_path) {
            Ok(content) => {
                eprintln!("Warning: ~/.awswit/config.yaml is deprecated. Rename to config.toml.");
                let config: Self = toml::from_str(&content).map_err(|_| {
                    AwswitError::ConfigFileError {
                        message: "Failed to parse config.yaml. Please convert to TOML format and rename to config.toml.".to_string(),
                    }
                })?;
                Ok(config)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::debug!("awswit config not found, using defaults");
                Ok(Self::default())
            }
            Err(e) => Err(AwswitError::ConfigFileError {
                message: format!("Failed to read config: {}", e),
            }),
        }
    }

    /// Save config to file
    pub fn save(&self) -> Result<(), AwswitError> {
        let path = Self::config_path()?;

        // Ensure directory exists with restrictive permissions
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
            }
        }

        let content = toml::to_string_pretty(self)?;
        crate::utils::fs::atomic_write_restricted(&path, content.as_bytes())?;

        tracing::debug!("Saved config to {:?}", path);
        Ok(())
    }

    /// Set a config value by key
    pub fn set_value(&mut self, key: &str, value: &str) -> Result<(), AwswitError> {
        match key {
            "colors" => {
                self.colors = value.parse().map_err(|_| AwswitError::ValidationError {
                    message: format!("Invalid boolean: {}", value),
                })?;
            }
            "fuzzy-match" => {
                self.fuzzy_match = value.parse().map_err(|_| AwswitError::ValidationError {
                    message: format!("Invalid boolean: {}", value),
                })?;
            }
            "role-duration" => {
                let duration: i32 = value.parse().map_err(|_| AwswitError::ValidationError {
                    message: format!("Invalid number: {}", value),
                })?;
                if duration != 0 && !(900..=43200).contains(&duration) {
                    return Err(AwswitError::ValidationError {
                        message: format!(
                            "role-duration must be between 900 and 43200 seconds, got {}",
                            duration
                        ),
                    });
                }
                self.role_duration = duration;
            }
            "region" => {
                self.region = Some(value.to_string());
            }
            "role-session-name" => {
                self.role_session_name = Some(value.to_string());
            }
            "session-token-duration" => {
                let duration: i32 = value.parse().map_err(|_| AwswitError::ValidationError {
                    message: format!("Invalid number: {}", value),
                })?;
                if !(900..=129600).contains(&duration) {
                    return Err(AwswitError::ValidationError {
                        message: format!(
                            "session-token-duration must be between 900 and 129600 seconds, got {}",
                            duration
                        ),
                    });
                }
                self.session_token_duration = Some(duration);
            }
            _ => {
                return Err(AwswitError::ValidationError {
                    message: format!("Unknown config key: {}", key),
                });
            }
        }
        Ok(())
    }

    /// Get a config value by key
    pub fn get_value(&self, key: &str) -> Option<String> {
        match key {
            "colors" => Some(self.colors.to_string()),
            "fuzzy-match" => Some(self.fuzzy_match.to_string()),
            "role-duration" => Some(self.role_duration.to_string()),
            "region" => self.region.clone(),
            "role-session-name" => self.role_session_name.clone(),
            "session-token-duration" => self.session_token_duration.map(|d| d.to_string()),
            _ => None,
        }
    }

    /// Reset a config value to default
    pub fn reset_value(&mut self, key: &str) -> Result<(), AwswitError> {
        let default = Self::default();
        match key {
            "colors" => self.colors = default.colors,
            "fuzzy-match" => self.fuzzy_match = default.fuzzy_match,
            "role-duration" => self.role_duration = default.role_duration,
            "region" => self.region = None,
            "role-session-name" => self.role_session_name = None,
            "session-token-duration" => self.session_token_duration = None,
            _ => {
                return Err(AwswitError::ValidationError {
                    message: format!("Unknown config key: {}", key),
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = AwswitConfig::default();
        assert!(config.fuzzy_match);
        assert_eq!(config.role_duration, 0);
        assert!(config.session_token_duration.is_none());
    }

    #[test]
    fn test_set_get_value() {
        let mut config = AwswitConfig::default();

        config.set_value("role-duration", "3600").unwrap();
        assert_eq!(config.get_value("role-duration"), Some("3600".to_string()));

        config.set_value("fuzzy-match", "false").unwrap();
        assert_eq!(config.get_value("fuzzy-match"), Some("false".to_string()));

        config.set_value("session-token-duration", "43200").unwrap();
        assert_eq!(
            config.get_value("session-token-duration"),
            Some("43200".to_string())
        );
    }

    #[test]
    fn test_toml_serialization() {
        let config = AwswitConfig::default();
        let toml_str = toml::to_string_pretty(&config).unwrap();
        assert!(toml_str.contains("colors"));
        assert!(toml_str.contains("fuzzy-match"));
    }

    #[test]
    fn test_toml_deserialization() {
        let toml_str = "colors = true\n\"fuzzy-match\" = false\n";
        let config: AwswitConfig = toml::from_str(toml_str).unwrap();
        assert!(config.colors);
        assert!(!config.fuzzy_match);
    }

    #[test]
    fn test_session_token_duration_boundary_low_reject() {
        let mut config = AwswitConfig::default();
        assert!(config.set_value("session-token-duration", "899").is_err());
    }

    #[test]
    fn test_session_token_duration_boundary_low_accept() {
        let mut config = AwswitConfig::default();
        assert!(config.set_value("session-token-duration", "900").is_ok());
        assert_eq!(config.session_token_duration, Some(900));
    }

    #[test]
    fn test_session_token_duration_boundary_high_accept() {
        let mut config = AwswitConfig::default();
        assert!(config.set_value("session-token-duration", "129600").is_ok());
        assert_eq!(config.session_token_duration, Some(129600));
    }

    #[test]
    fn test_session_token_duration_boundary_high_reject() {
        let mut config = AwswitConfig::default();
        assert!(config
            .set_value("session-token-duration", "129601")
            .is_err());
    }

    #[test]
    fn test_session_token_duration_non_numeric() {
        let mut config = AwswitConfig::default();
        assert!(config.set_value("session-token-duration", "abc").is_err());
    }

    #[test]
    fn test_unknown_toml_keys_rejected_on_load() {
        let toml_str =
            "colors = true\n\"fuzzy-match\" = false\nunknown-plugin-key = \"some-value\"\n";
        let result = toml::from_str::<AwswitConfig>(toml_str);
        assert!(result.is_err());
    }

    #[test]
    fn test_valid_toml_keys_accepted_on_load() {
        let toml_str = "colors = true\n\"fuzzy-match\" = false\n";
        let config: AwswitConfig = toml::from_str(toml_str).unwrap();
        assert!(config.colors);
        assert!(!config.fuzzy_match);
    }

    #[test]
    fn test_set_unknown_key_returns_error() {
        let mut config = AwswitConfig::default();
        assert!(config.set_value("nonexistent-key", "value").is_err());
    }

    #[test]
    fn test_reset_unknown_key_returns_error() {
        let mut config = AwswitConfig::default();
        assert!(config.reset_value("nonexistent-key").is_err());
    }
}
