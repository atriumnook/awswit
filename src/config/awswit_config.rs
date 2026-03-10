use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::error::AwswitError;

/// awswit configuration stored in ~/.awswit/config.yaml
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
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

    /// Debug settings
    #[serde(default)]
    pub debug: DebugConfig,

    /// Plugin-specific configurations (preserved as-is)
    #[serde(flatten)]
    pub extra: std::collections::HashMap<String, serde_yaml::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DebugConfig {
    /// Custom session token duration
    #[serde(rename = "session_token_duration")]
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
            debug: DebugConfig::default(),
            extra: std::collections::HashMap::new(),
        }
    }
}

impl AwswitConfig {
    /// Get the config file path
    pub fn config_path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_default()
            .join(".awswit")
            .join("config.yaml")
    }

    /// Load config from file, or return default if not found
    pub fn load() -> Result<Self, AwswitError> {
        let path = Self::config_path();
        
        if !path.exists() {
            tracing::debug!("awswit config not found, using defaults");
            return Ok(Self::default());
        }

        let content = fs::read_to_string(&path)
            .map_err(|e| AwswitError::ConfigFileError(format!("Failed to read config: {}", e)))?;

        let config: Self = serde_yaml::from_str(&content)?;
        Ok(config)
    }

    /// Save config to file
    pub fn save(&self) -> Result<(), AwswitError> {
        let path = Self::config_path();
        
        // Ensure directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let content = serde_yaml::to_string(self)?;
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&path)?;
            file.write_all(content.as_bytes())?;
        }
        
        tracing::debug!("Saved config to {:?}", path);
        Ok(())
    }

    /// Set a config value by key
    pub fn set_value(&mut self, key: &str, value: &str) -> Result<(), AwswitError> {
        match key {
            "colors" => {
                self.colors = value.parse()
                    .map_err(|_| AwswitError::ValidationError(format!("Invalid boolean: {}", value)))?;
            }
            "fuzzy-match" => {
                self.fuzzy_match = value.parse()
                    .map_err(|_| AwswitError::ValidationError(format!("Invalid boolean: {}", value)))?;
            }
            "role-duration" => {
                self.role_duration = value.parse()
                    .map_err(|_| AwswitError::ValidationError(format!("Invalid number: {}", value)))?;
            }
            "region" => {
                self.region = Some(value.to_string());
            }
            "role-session-name" => {
                self.role_session_name = Some(value.to_string());
            }
            _ => {
                // Store in extra for plugins
                self.extra.insert(key.to_string(), serde_yaml::Value::String(value.to_string()));
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
            _ => self.extra.get(key).map(|v| format!("{:?}", v)),
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
            _ => {
                self.extra.remove(key);
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
    }

    #[test]
    fn test_set_get_value() {
        let mut config = AwswitConfig::default();
        
        config.set_value("role-duration", "3600").unwrap();
        assert_eq!(config.get_value("role-duration"), Some("3600".to_string()));
        
        config.set_value("fuzzy-match", "false").unwrap();
        assert_eq!(config.get_value("fuzzy-match"), Some("false".to_string()));
    }

    #[test]
    fn test_yaml_serialization() {
        let config = AwswitConfig::default();
        let yaml = serde_yaml::to_string(&config).unwrap();
        assert!(yaml.contains("colors"));
        assert!(yaml.contains("fuzzy-match"));
    }
}
