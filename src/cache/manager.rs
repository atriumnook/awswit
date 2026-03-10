use std::fs;
use std::path::PathBuf;

use crate::aws::Credentials;
use crate::error::AwswitError;

/// Manages credential caching in ~/.awswit/cache/
pub struct CacheManager {
    cache_dir: PathBuf,
}

impl CacheManager {
    /// Create a new cache manager
    pub fn new() -> Result<Self, AwswitError> {
        let cache_dir = dirs::home_dir()
            .ok_or_else(|| AwswitError::CacheError("Cannot determine home directory".to_string()))?
            .join(".awswit")
            .join("cache");

        // Ensure cache directory exists
        fs::create_dir_all(&cache_dir)?;

        // Set permissions on the cache directory (unix only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let permissions = fs::Permissions::from_mode(0o700);
            fs::set_permissions(&cache_dir, permissions)?;
        }

        Ok(Self { cache_dir })
    }

    /// Get cached credentials by key
    pub fn get(&self, key: &str) -> Result<Option<Credentials>, AwswitError> {
        let path = self.cache_file_path(key);
        
        if !path.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(&path)
            .map_err(|e| AwswitError::CacheError(format!("Failed to read cache: {}", e)))?;

        let credentials: Credentials = serde_json::from_str(&content)?;

        // Check if expired
        if credentials.is_expired() {
            tracing::debug!("Cache entry '{}' is expired, removing", key);
            self.remove(key)?;
            return Ok(None);
        }

        Ok(Some(credentials))
    }

    /// Set cached credentials
    pub fn set(&self, key: &str, credentials: &Credentials) -> Result<(), AwswitError> {
        let path = self.cache_file_path(key);
        let content = serde_json::to_string_pretty(credentials)?;

        // Write with restrictive permissions atomically to avoid TOCTOU
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
            fs::write(&path, &content)?;
        }

        tracing::debug!("Cached credentials with key: {}", key);
        Ok(())
    }

    /// Remove cached credentials
    pub fn remove(&self, key: &str) -> Result<(), AwswitError> {
        let path = self.cache_file_path(key);
        
        if path.exists() {
            fs::remove_file(&path)?;
            tracing::debug!("Removed cache entry: {}", key);
        }

        Ok(())
    }

    /// Clear all cached credentials
    pub fn clear_all(&self) -> Result<(), AwswitError> {
        for entry in fs::read_dir(&self.cache_dir)? {
            let entry = entry?;
            if entry.path().is_file() {
                fs::remove_file(entry.path())?;
            }
        }
        tracing::info!("Cleared all cached credentials");
        Ok(())
    }

    /// Clear expired cache entries
    pub fn clear_expired(&self) -> Result<usize, AwswitError> {
        let mut removed = 0;

        for entry in fs::read_dir(&self.cache_dir)? {
            let entry = entry?;
            let path = entry.path();
            
            if path.is_file() {
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(creds) = serde_json::from_str::<Credentials>(&content) {
                        if creds.is_expired() {
                            fs::remove_file(&path)?;
                            removed += 1;
                        }
                    }
                }
            }
        }

        tracing::info!("Cleared {} expired cache entries", removed);
        Ok(removed)
    }

    /// List all cache keys
    pub fn list_keys(&self) -> Result<Vec<String>, AwswitError> {
        let mut keys = Vec::new();

        for entry in fs::read_dir(&self.cache_dir)? {
            let entry = entry?;
            if entry.path().is_file() {
                if let Some(name) = entry.file_name().to_str() {
                    if name.ends_with(".json") {
                        keys.push(name.trim_end_matches(".json").to_string());
                    }
                }
            }
        }

        Ok(keys)
    }

    /// Get the cache file path for a key
    fn cache_file_path(&self, key: &str) -> PathBuf {
        // Sanitize key for filesystem
        let safe_key = key.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
        self.cache_dir.join(format!("{}.json", safe_key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use tempfile::TempDir;

    fn create_test_manager() -> (CacheManager, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let cache_dir = temp_dir.path().join(".awswit").join("cache");
        fs::create_dir_all(&cache_dir).unwrap();
        
        let manager = CacheManager { cache_dir };
        (manager, temp_dir)
    }

    #[test]
    fn test_set_and_get() {
        let (manager, _temp) = create_test_manager();
        
        let creds = Credentials {
            access_key_id: "AKIATEST".to_string(),
            secret_access_key: "secret".to_string(),
            session_token: Some("token".to_string()),
            expiration: Some(Utc::now() + Duration::hours(1)),
            region: Some("us-east-1".to_string()),
        };

        manager.set("test-key", &creds).unwrap();
        
        let retrieved = manager.get("test-key").unwrap().unwrap();
        assert_eq!(retrieved.access_key_id, "AKIATEST");
    }

    #[test]
    fn test_expired_credentials() {
        let (manager, _temp) = create_test_manager();
        
        let creds = Credentials {
            access_key_id: "AKIATEST".to_string(),
            secret_access_key: "secret".to_string(),
            session_token: None,
            expiration: Some(Utc::now() - Duration::hours(1)), // Expired
            region: None,
        };

        manager.set("expired-key", &creds).unwrap();
        
        // Should return None for expired credentials
        let retrieved = manager.get("expired-key").unwrap();
        assert!(retrieved.is_none());
    }

    #[test]
    fn test_remove() {
        let (manager, _temp) = create_test_manager();
        
        let creds = Credentials::default();
        manager.set("remove-key", &creds).unwrap();
        
        manager.remove("remove-key").unwrap();
        
        let retrieved = manager.get("remove-key").unwrap();
        assert!(retrieved.is_none());
    }
}
