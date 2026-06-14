use std::collections::HashMap;

use crate::cli::Args;
use crate::config::AwsFiles;
use crate::error::AwswitError;
use crate::history::{ProfileHistory, load_history};
use crate::profile::Profile;

/// Aggregate of everything we read on startup.
///
/// We deliberately do **not** load an awswit-specific config file: every
/// runtime knob is a CLI flag or an environment variable, so users don't
/// have to chase a config file when behavior surprises them.
pub struct AppContext {
    pub args: Args,
    pub profiles: HashMap<String, Profile>,
    pub history: ProfileHistory,
}

impl AppContext {
    pub fn build(args: Args) -> Result<Self, AwswitError> {
        let config_path = args
            .config_file
            .clone()
            .or_else(|| std::env::var("AWS_CONFIG_FILE").ok())
            .unwrap_or_else(|| default_aws_path("config"));

        let credentials_path = std::env::var("AWS_SHARED_CREDENTIALS_FILE")
            .ok()
            .unwrap_or_else(|| default_aws_path("credentials"));

        let aws_files = AwsFiles::load(&config_path, &credentials_path)?;
        let profiles = aws_files.merge_profiles();
        tracing::debug!("loaded {} profiles", profiles.len());

        let history = load_history().unwrap_or_else(|e| {
            tracing::warn!("failed to load history ({}); starting empty", e);
            ProfileHistory::default()
        });

        Ok(Self {
            args,
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
