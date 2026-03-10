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
    
    /// Favorite profiles (for quick access)
    favorites: Vec<String>,
}

impl ProfileHistory {
    /// Get the history file path
    fn history_path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_default()
            .join(".awswit")
            .join("history.json")
    }

    /// Load history from file
    pub fn load() -> Result<Self, AwswitError> {
        let path = Self::history_path();
        
        if !path.exists() {
            return Ok(Self::default());
        }

        let content = fs::read_to_string(&path)
            .map_err(|e| AwswitError::CacheError(format!("Failed to read history: {}", e)))?;

        let history: Self = serde_json::from_str(&content)
            .map_err(|e| AwswitError::CacheError(format!("Failed to parse history: {}", e)))?;

        Ok(history)
    }

    /// Save history to file
    pub fn save(&self) -> Result<(), AwswitError> {
        let path = Self::history_path();
        
        // Ensure directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let content = serde_json::to_string_pretty(self)?;
        #[cfg(unix)]
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
        #[cfg(not(unix))]
        {
            fs::write(&path, content)?;
        }

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
        self.favorites.contains(&profile_name.to_string())
    }

    /// Set favorite status
    pub fn set_favorite(&mut self, profile_name: &str, is_favorite: bool) {
        let name = profile_name.to_string();
        
        if is_favorite {
            if !self.favorites.contains(&name) {
                self.favorites.push(name.clone());
            }
            if let Some(entry) = self.entries.get_mut(&name) {
                entry.is_favorite = true;
            }
        } else {
            self.favorites.retain(|n| n != &name);
            if let Some(entry) = self.entries.get_mut(&name) {
                entry.is_favorite = false;
            }
        }
    }

    /// Toggle favorite status
    pub fn toggle_favorite(&mut self, profile_name: &str) -> bool {
        let new_status = !self.is_favorite(profile_name);
        self.set_favorite(profile_name, new_status);
        new_status
    }

    /// Get recently used profiles
    pub fn recent_profiles(&self, limit: usize) -> Vec<&HistoryEntry> {
        let mut entries: Vec<_> = self.entries.values().collect();
        entries.sort_by(|a, b| b.last_used.cmp(&a.last_used));
        entries.into_iter().take(limit).collect()
    }

    /// Get favorite profiles
    pub fn favorite_profiles(&self) -> Vec<&str> {
        self.favorites.iter().map(|s| s.as_str()).collect()
    }

    /// Get most used profiles
    pub fn most_used(&self, limit: usize) -> Vec<&HistoryEntry> {
        let mut entries: Vec<_> = self.entries.values().collect();
        entries.sort_by(|a, b| b.use_count.cmp(&a.use_count));
        entries.into_iter().take(limit).collect()
    }

    /// Clean up old entries (older than 90 days)
    pub fn cleanup_old(&mut self, days: i64) {
        let cutoff = Utc::now() - chrono::Duration::days(days);
        let favorites = self.favorites.clone();
        self.entries.retain(|name, entry| {
            entry.last_used > cutoff || favorites.contains(name)
        });
    }

    /// Get total number of entries
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if history is empty
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
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

    #[test]
    fn test_toggle_favorite() {
        let mut history = ProfileHistory::default();
        
        assert!(!history.is_favorite("test"));
        
        let status = history.toggle_favorite("test");
        assert!(status);
        assert!(history.is_favorite("test"));
        
        let status = history.toggle_favorite("test");
        assert!(!status);
        assert!(!history.is_favorite("test"));
    }

    #[test]
    fn test_recent_profiles() {
        let mut history = ProfileHistory::default();
        
        history.record_use("profile-a");
        std::thread::sleep(std::time::Duration::from_millis(10));
        history.record_use("profile-b");
        std::thread::sleep(std::time::Duration::from_millis(10));
        history.record_use("profile-c");
        
        let recent = history.recent_profiles(2);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].name, "profile-c");
        assert_eq!(recent[1].name, "profile-b");
    }
}
