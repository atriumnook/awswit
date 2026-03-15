use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

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

    /// Default AWS region
    pub region: Option<String>,
}

impl Default for AwswitConfig {
    fn default() -> Self {
        Self {
            colors: !cfg!(windows),
            fuzzy_match: true,
            region: None,
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

    /// Load config from file, or return default if not found.
    pub fn load() -> Result<Self, AwswitError> {
        let toml_path = Self::config_path()?;
        Self::load_toml_if_exists(&toml_path)
    }

    fn load_toml_if_exists(path: &Path) -> Result<Self, AwswitError> {
        match fs::read_to_string(path) {
            Ok(content) => Ok(toml::from_str(&content)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::debug!("awswit config not found, using defaults");
                Ok(Self::default())
            }
            Err(e) => Err(AwswitError::ConfigFileError {
                message: format!("Failed to read config: {}", e),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = AwswitConfig::default();
        assert!(config.fuzzy_match);
        assert!(config.region.is_none());
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
    fn test_unknown_toml_keys_rejected_on_load() {
        let toml_str =
            "colors = true\n\"fuzzy-match\" = false\nunknown-plugin-key = \"some-value\"\n";
        let result = toml::from_str::<AwswitConfig>(toml_str);
        assert!(result.is_err());
    }

    #[test]
    fn test_load_missing_file_returns_default() {
        let temp = tempfile::tempdir().unwrap();
        let toml_path = temp.path().join("config.toml");
        let config = AwswitConfig::load_toml_if_exists(&toml_path).unwrap();
        assert!(config.fuzzy_match);
    }
}
