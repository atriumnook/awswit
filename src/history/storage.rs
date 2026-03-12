use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::AwswitError;

/// History entry for a profile
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// Profile name
    pub name: String,

    /// Last time this profile was used
    pub last_used: DateTime<Utc>,

    /// Number of times used
    pub use_count: u32,

    /// Is this a favorite profile
    pub is_favorite: bool,
}

/// Profile usage history storage
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProfileHistory {
    /// History entries by profile name
    entries: HashMap<String, HistoryEntry>,
}

impl ProfileHistory {
    /// Get the history file path
    fn history_path() -> Result<PathBuf, AwswitError> {
        crate::utils::paths::awswit_home_dir()
            .map(|p| p.join("history.json"))
            .map_err(|e| AwswitError::CacheError {
                message: e.to_string(),
            })
    }

    /// Load history from file
    pub fn load() -> Result<Self, AwswitError> {
        let path = Self::history_path()?;

        // Read directly instead of exists() check to avoid TOCTOU race
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => {
                return Err(AwswitError::CacheError {
                    message: format!("Failed to read history: {}", e),
                })
            }
        };

        match serde_json::from_str::<Self>(&content) {
            Ok(history) => Ok(history),
            Err(e) => {
                // Backup corrupt file before resetting to prevent data loss
                let backup_path = path.with_extension("json.corrupt");
                tracing::warn!(
                    "History file is corrupt ({}), backing up to {:?} and resetting",
                    e,
                    backup_path
                );
                if let Err(backup_err) = fs::copy(&path, &backup_path) {
                    tracing::warn!("Failed to backup corrupt history file: {}", backup_err);
                }
                Ok(Self::default())
            }
        }
    }

    /// Save history to file
    pub fn save(&self) -> Result<(), AwswitError> {
        let path = Self::history_path()?;

        // Ensure directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let content = serde_json::to_string_pretty(self)?;
        crate::utils::fs::atomic_write_restricted(&path, content.as_bytes())?;

        Ok(())
    }

    /// Record profile usage
    pub fn record_use(&mut self, profile_name: &str) {
        let now = Utc::now();

        if let Some(entry) = self.entries.get_mut(profile_name) {
            entry.last_used = now;
            entry.use_count += 1;
        } else {
            let entry = HistoryEntry {
                name: profile_name.to_string(),
                last_used: now,
                use_count: 1,
                is_favorite: false,
            };
            self.entries.insert(profile_name.to_string(), entry);
        }
    }

    /// Get history entry for a profile
    pub fn get(&self, profile_name: &str) -> Option<&HistoryEntry> {
        self.entries.get(profile_name)
    }

    /// Check if a profile is favorite
    pub fn is_favorite(&self, profile_name: &str) -> bool {
        self.entries
            .get(profile_name)
            .map(|e| e.is_favorite)
            .unwrap_or(false)
    }

    /// Set favorite status
    pub fn set_favorite(&mut self, profile_name: &str, is_favorite: bool) {
        if let Some(entry) = self.entries.get_mut(profile_name) {
            entry.is_favorite = is_favorite;
        } else if is_favorite {
            // Create entry for favoriting a profile that hasn't been used yet
            let entry = HistoryEntry {
                name: profile_name.to_string(),
                last_used: Utc::now(),
                use_count: 0,
                is_favorite: true,
            };
            self.entries.insert(profile_name.to_string(), entry);
        }
    }

    /// Get favorite profiles
    pub fn favorite_profiles(&self) -> Vec<&str> {
        self.entries
            .values()
            .filter(|e| e.is_favorite)
            .map(|e| e.name.as_str())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_use() {
        let mut history = ProfileHistory::default();

        history.record_use("test-profile");
        assert_eq!(history.get("test-profile").unwrap().use_count, 1);

        history.record_use("test-profile");
        assert_eq!(history.get("test-profile").unwrap().use_count, 2);
    }

    #[test]
    fn test_favorites() {
        let mut history = ProfileHistory::default();

        assert!(!history.is_favorite("test-profile"));

        history.set_favorite("test-profile", true);
        assert!(history.is_favorite("test-profile"));

        history.set_favorite("test-profile", false);
        assert!(!history.is_favorite("test-profile"));
    }
}
