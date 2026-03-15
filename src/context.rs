use std::collections::HashMap;

use crate::cli::Args;
use crate::config::{AwsFiles, AwswitConfig};
use crate::error::AwswitError;
use crate::history::{ProfileHistory, load_history};
use crate::profile::Profile;

/// Aggregates all application state loaded at startup.
pub struct AppContext {
    pub args: Args,
    pub config: AwswitConfig,
    pub profiles: HashMap<String, Profile>,
    pub history: ProfileHistory,
}

impl AppContext {
    /// Load config, AWS files, profiles, and history from the standard locations.
    pub fn build(args: Args) -> Result<Self, AwswitError> {
        let config = AwswitConfig::load()?;
        tracing::debug!("Loaded awswit config: {:?}", config);

        let config_file = args
            .config_file
            .clone()
            .or_else(|| std::env::var("AWS_CONFIG_FILE").ok())
            .unwrap_or_else(|| default_aws_path("config"));

        let credentials_file = std::env::var("AWS_SHARED_CREDENTIALS_FILE")
            .ok()
            .unwrap_or_else(|| default_aws_path("credentials"));

        let aws_files = AwsFiles::load(&config_file, &credentials_file)?;
        let profiles = aws_files.merge_profiles();
        tracing::debug!("Loaded {} profiles", profiles.len());

        let history = load_history().unwrap_or_else(|e| {
            tracing::warn!("Failed to load profile history: {}", e);
            eprintln!("Warning: Failed to load profile history: {}. Favorites and recent profiles may be missing.", e);
            ProfileHistory::default()
        });

        Ok(Self {
            args,
            config,
            profiles,
            history,
        })
    }
}

fn default_aws_path(filename: &str) -> String {
    dirs::home_dir()
        .map(|h| h.join(".aws").join(filename).to_string_lossy().to_string())
        .unwrap_or_else(|| format!("~/.aws/{}", filename))
}
