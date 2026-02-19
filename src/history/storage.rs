use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::AwswitConfig;
use crate::error::{AwswitError, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub name: String,
    pub last_used: DateTime<Utc>,
    pub use_count: u32,
    pub is_favorite: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HistoryData {
    pub entries: HashMap<String, HistoryEntry>,
    #[serde(default)]
    pub favorites: Vec<String>,
}

/// Manages usage history for profiles
pub struct HistoryStorage {
    path: PathBuf,
    data: HistoryData,
}

impl HistoryStorage {
    /// Load history from disk or create empty
    pub fn load() -> Result<Self> {
        let path = AwswitConfig::config_dir().join("history.json");

        let data = if path.exists() {
            let content = std::fs::read_to_string(&path).map_err(|e| {
                AwswitError::config_file_error(format!("Failed to read history: {}", e))
            })?;
            serde_json::from_str(&content).unwrap_or_else(|e| {
                tracing::warn!(
                    "Failed to parse history file {}: {}. Starting with empty history.",
                    path.display(),
                    e
                );
                HistoryData::default()
            })
        } else {
            HistoryData::default()
        };

        Ok(Self { path, data })
    }

    /// Record a profile usage
    pub fn record_usage(&mut self, profile_name: &str) -> Result<()> {
        let entry = self
            .data
            .entries
            .entry(profile_name.to_string())
            .or_insert_with(|| HistoryEntry {
                name: profile_name.to_string(),
                last_used: Utc::now(),
                use_count: 0,
                is_favorite: false,
            });

        entry.last_used = Utc::now();
        entry.use_count += 1;

        self.save()
    }

    /// Toggle favorite status for a profile
    pub fn toggle_favorite(&mut self, profile_name: &str) -> Result<bool> {
        let entry = self
            .data
            .entries
            .entry(profile_name.to_string())
            .or_insert_with(|| HistoryEntry {
                name: profile_name.to_string(),
                last_used: Utc::now(),
                use_count: 0,
                is_favorite: false,
            });

        entry.is_favorite = !entry.is_favorite;
        let is_fav = entry.is_favorite;

        if is_fav {
            if !self.data.favorites.contains(&profile_name.to_string()) {
                self.data.favorites.push(profile_name.to_string());
            }
        } else {
            self.data
                .favorites
                .retain(|f| f != profile_name);
        }

        self.save()?;
        Ok(is_fav)
    }

    /// Check if a profile is a favorite
    pub fn is_favorite(&self, profile_name: &str) -> bool {
        self.data
            .entries
            .get(profile_name)
            .map(|e| e.is_favorite)
            .unwrap_or(false)
    }

    /// Get history entry for a profile
    pub fn get(&self, profile_name: &str) -> Option<&HistoryEntry> {
        self.data.entries.get(profile_name)
    }

    /// Get all entries sorted by a combination of favorite status, use count, and recency
    pub fn sorted_profile_names(&self, all_profiles: &[String]) -> Vec<String> {
        let mut profiles: Vec<String> = all_profiles.to_vec();

        profiles.sort_by(|a, b| {
            let a_entry = self.data.entries.get(a);
            let b_entry = self.data.entries.get(b);

            // Favorites first
            let a_fav = a_entry.map(|e| e.is_favorite).unwrap_or(false);
            let b_fav = b_entry.map(|e| e.is_favorite).unwrap_or(false);
            if a_fav != b_fav {
                return b_fav.cmp(&a_fav);
            }

            // Then by use count (descending)
            let a_count = a_entry.map(|e| e.use_count).unwrap_or(0);
            let b_count = b_entry.map(|e| e.use_count).unwrap_or(0);
            if a_count != b_count {
                return b_count.cmp(&a_count);
            }

            // Then by last used (most recent first)
            let a_time = a_entry.map(|e| e.last_used);
            let b_time = b_entry.map(|e| e.last_used);
            match (b_time, a_time) {
                (Some(bt), Some(at)) => bt.cmp(&at),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a.cmp(b),
            }
        });

        profiles
    }

    /// Save history to disk
    fn save(&self) -> Result<()> {
        let dir = self.path.parent().unwrap();
        if !dir.exists() {
            AwswitConfig::ensure_config_dir()?;
        }

        let content = serde_json::to_string_pretty(&self.data)?;
        std::fs::write(&self.path, content).map_err(|e| {
            AwswitError::config_file_error(format!("Failed to write history: {}", e))
        })?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_storage(dir: &tempfile::TempDir) -> HistoryStorage {
        HistoryStorage {
            path: dir.path().join("history.json"),
            data: HistoryData::default(),
        }
    }

    #[test]
    fn test_record_usage() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut storage = test_storage(&dir);

        storage.record_usage("test-profile").unwrap();
        let entry = storage.get("test-profile").unwrap();
        assert_eq!(entry.use_count, 1);
        assert!(!entry.is_favorite);

        storage.record_usage("test-profile").unwrap();
        let entry = storage.get("test-profile").unwrap();
        assert_eq!(entry.use_count, 2);
    }

    #[test]
    fn test_toggle_favorite() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut storage = test_storage(&dir);

        let is_fav = storage.toggle_favorite("test-profile").unwrap();
        assert!(is_fav);
        assert!(storage.is_favorite("test-profile"));

        let is_fav = storage.toggle_favorite("test-profile").unwrap();
        assert!(!is_fav);
        assert!(!storage.is_favorite("test-profile"));
    }

    #[test]
    fn test_sorted_profiles() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut storage = test_storage(&dir);

        // Record usage: profile-b used more
        storage.record_usage("profile-a").unwrap();
        storage.record_usage("profile-b").unwrap();
        storage.record_usage("profile-b").unwrap();
        storage.record_usage("profile-b").unwrap();

        // Make profile-c a favorite
        storage.toggle_favorite("profile-c").unwrap();

        let all = vec![
            "profile-a".to_string(),
            "profile-b".to_string(),
            "profile-c".to_string(),
            "profile-d".to_string(),
        ];

        let sorted = storage.sorted_profile_names(&all);
        // profile-c first (favorite), then profile-b (most used), then profile-a, then profile-d
        assert_eq!(sorted[0], "profile-c");
        assert_eq!(sorted[1], "profile-b");
        assert_eq!(sorted[2], "profile-a");
    }
}
