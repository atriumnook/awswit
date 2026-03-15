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

impl HistoryEntry {
    /// Calculate a frecency score based on recency and frequency.
    /// More recent and more frequently used profiles score higher.
    pub fn frecency_score(&self, now: DateTime<Utc>) -> f64 {
        let hours = (now - self.last_used).num_seconds().max(0) as f64 / 3600.0;
        let weight = if hours < 1.0 {
            4.0
        } else if hours < 24.0 {
            2.0
        } else if hours < 168.0 {
            1.0
        } else {
            0.5
        };
        self.use_count as f64 * weight
    }
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
            .map_err(|e| AwswitError::ConfigFileError {
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
                return Err(AwswitError::ConfigFileError {
                    message: format!("Failed to read history: {}", e),
                });
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

    /// Save history to file atomically.
    ///
    /// Writes to a temporary file with restricted permissions, then renames
    /// it into place. This avoids a race window where the file exists with
    /// default permissions before `set_permissions` is called.
    pub fn save(&self) -> Result<(), AwswitError> {
        let path = Self::history_path()?;

        // Ensure directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let content = serde_json::to_string_pretty(self)?;

        // Write to a PID-unique temp file in the same directory, then atomically rename.
        let tmp_path = path.with_extension(format!("json.{}.tmp", std::process::id()));

        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&tmp_path)?;
            file.write_all(content.as_bytes())?;
            file.sync_all()?;
        }

        #[cfg(not(unix))]
        {
            use std::io::Write;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp_path)?;
            file.write_all(content.as_bytes())?;
            file.sync_all()?;
        }

        if let Err(e) = fs::rename(&tmp_path, &path) {
            let _ = fs::remove_file(&tmp_path);
            return Err(e.into());
        }

        Ok(())
    }

    /// Record profile usage
    pub fn record_use(&mut self, profile_name: &str) {
        let now = Utc::now();

        if let Some(entry) = self.entries.get_mut(profile_name) {
            entry.last_used = now;
            entry.use_count = entry.use_count.saturating_add(1);
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

    /// Compare two profiles by frecency order: favorites first, then frecency descending, then name ascending.
    pub fn compare_by_frecency(&self, a: &str, b: &str, now: DateTime<Utc>) -> std::cmp::Ordering {
        let a_fav = self.is_favorite(a);
        let b_fav = self.is_favorite(b);
        b_fav
            .cmp(&a_fav)
            .then_with(|| {
                let a_score = self.get(a).map(|e| e.frecency_score(now)).unwrap_or(0.0);
                let b_score = self.get(b).map(|e| e.frecency_score(now)).unwrap_or(0.0);
                b_score
                    .partial_cmp(&a_score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.cmp(b))
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
    fn test_frecency_within_one_hour() {
        let now = Utc::now();
        let entry = HistoryEntry {
            name: "test".to_string(),
            last_used: now - chrono::Duration::minutes(30),
            use_count: 5,
            is_favorite: false,
        };
        let score = entry.frecency_score(now);
        assert!((score - 20.0).abs() < f64::EPSILON); // 5 * 4.0
    }

    #[test]
    fn test_frecency_within_one_day() {
        let now = Utc::now();
        let entry = HistoryEntry {
            name: "test".to_string(),
            last_used: now - chrono::Duration::hours(12),
            use_count: 3,
            is_favorite: false,
        };
        let score = entry.frecency_score(now);
        assert!((score - 6.0).abs() < f64::EPSILON); // 3 * 2.0
    }

    #[test]
    fn test_frecency_within_one_week() {
        let now = Utc::now();
        let entry = HistoryEntry {
            name: "test".to_string(),
            last_used: now - chrono::Duration::days(3),
            use_count: 10,
            is_favorite: false,
        };
        let score = entry.frecency_score(now);
        assert!((score - 10.0).abs() < f64::EPSILON); // 10 * 1.0
    }

    #[test]
    fn test_frecency_older_than_one_week() {
        let now = Utc::now();
        let entry = HistoryEntry {
            name: "test".to_string(),
            last_used: now - chrono::Duration::days(30),
            use_count: 8,
            is_favorite: false,
        };
        let score = entry.frecency_score(now);
        assert!((score - 4.0).abs() < f64::EPSILON); // 8 * 0.5
    }

    #[test]
    fn test_frecency_at_zero_hours_boundary() {
        let now = Utc::now();
        let entry = HistoryEntry {
            name: "test".to_string(),
            last_used: now,
            use_count: 2,
            is_favorite: false,
        };
        assert!((entry.frecency_score(now) - 8.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_frecency_at_one_hour_boundary() {
        let now = Utc::now();
        let entry = HistoryEntry {
            name: "test".to_string(),
            last_used: now - chrono::Duration::hours(1),
            use_count: 2,
            is_favorite: false,
        };
        assert!((entry.frecency_score(now) - 4.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_frecency_at_twenty_four_hour_boundary() {
        let now = Utc::now();
        let entry = HistoryEntry {
            name: "test".to_string(),
            last_used: now - chrono::Duration::hours(24),
            use_count: 2,
            is_favorite: false,
        };
        assert!((entry.frecency_score(now) - 2.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_frecency_at_one_week_boundary() {
        let now = Utc::now();
        let entry = HistoryEntry {
            name: "test".to_string(),
            last_used: now - chrono::Duration::hours(168),
            use_count: 2,
            is_favorite: false,
        };
        assert!((entry.frecency_score(now) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_frecency_far_past_boundary() {
        let now = Utc::now();
        let entry = HistoryEntry {
            name: "test".to_string(),
            last_used: now - chrono::Duration::hours(1000),
            use_count: 2,
            is_favorite: false,
        };
        assert!((entry.frecency_score(now) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_compare_by_frecency_favorites_first() {
        let mut history = ProfileHistory::default();
        history.set_favorite("fav", true);
        history.record_use("recent");
        let now = Utc::now();
        assert_eq!(
            history.compare_by_frecency("fav", "recent", now),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn test_compare_by_frecency_higher_score_first() {
        let mut history = ProfileHistory::default();
        // Use "heavy" many times, "light" once
        for _ in 0..10 {
            history.record_use("heavy");
        }
        history.record_use("light");
        let now = Utc::now();
        assert_eq!(
            history.compare_by_frecency("heavy", "light", now),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn test_compare_by_frecency_unknown_profiles_alphabetical() {
        let history = ProfileHistory::default();
        let now = Utc::now();
        assert_eq!(
            history.compare_by_frecency("aaa", "zzz", now),
            std::cmp::Ordering::Less
        );
        assert_eq!(
            history.compare_by_frecency("zzz", "aaa", now),
            std::cmp::Ordering::Greater
        );
        assert_eq!(
            history.compare_by_frecency("same", "same", now),
            std::cmp::Ordering::Equal
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_save_creates_file_with_restricted_permissions() {
        use std::os::unix::fs::MetadataExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.json");

        // Temporarily override the history path by saving directly
        let history = ProfileHistory::default();
        let content = serde_json::to_string_pretty(&history).unwrap();

        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let tmp_path = path.with_extension(format!("json.{}.tmp", std::process::id()));
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&tmp_path)
                .unwrap();
            file.write_all(content.as_bytes()).unwrap();
            file.sync_all().unwrap();
            fs::rename(&tmp_path, &path).unwrap();
        }

        let metadata = fs::metadata(&path).unwrap();
        assert_eq!(metadata.mode() & 0o777, 0o600);
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.json");

        let mut history = ProfileHistory::default();
        history.record_use("test-profile");
        history.set_favorite("test-profile", true);

        let content = serde_json::to_string_pretty(&history).unwrap();
        fs::write(&path, &content).unwrap();

        let loaded: ProfileHistory = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(loaded.get("test-profile").unwrap().use_count, 1);
        assert!(loaded.is_favorite("test-profile"));
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
