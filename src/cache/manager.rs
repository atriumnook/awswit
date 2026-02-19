use std::path::PathBuf;

use sha2::{Digest, Sha256};

use crate::aws::credentials::Credentials;
use crate::config::AwswitConfig;
use crate::error::{AwswitError, Result};

/// Manages credential caching
pub struct CacheManager {
    cache_dir: PathBuf,
}

impl CacheManager {
    /// Create a new cache manager
    pub fn new() -> Result<Self> {
        let cache_dir = AwswitConfig::config_dir().join("cache");
        if !cache_dir.exists() {
            std::fs::create_dir_all(&cache_dir)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&cache_dir, std::fs::Permissions::from_mode(0o700))?;
            }
        }
        Ok(Self { cache_dir })
    }

    /// Generate a cache key for a profile name
    fn cache_key(&self, profile_name: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(profile_name.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Get the cache file path for a profile
    fn cache_path(&self, profile_name: &str) -> PathBuf {
        let key = self.cache_key(profile_name);
        self.cache_dir.join(format!("{}.json", key))
    }

    /// Get cached credentials for a profile
    pub fn get(&self, profile_name: &str) -> Result<Option<Credentials>> {
        let path = self.cache_path(profile_name);
        if !path.exists() {
            return Ok(None);
        }

        let content = std::fs::read_to_string(&path).map_err(|e| {
            AwswitError::cache_error(format!("Failed to read cache file: {}", e))
        })?;

        let creds: Credentials = serde_json::from_str(&content).map_err(|e| {
            AwswitError::cache_error(format!("Failed to parse cache file: {}", e))
        })?;

        if creds.is_expired() {
            // Remove expired cache
            let _ = std::fs::remove_file(&path);
            return Ok(None);
        }

        Ok(Some(creds))
    }

    /// Store credentials in the cache
    pub fn put(&self, profile_name: &str, credentials: &Credentials) -> Result<()> {
        let path = self.cache_path(profile_name);
        let content = serde_json::to_string_pretty(credentials)?;

        std::fs::write(&path, content).map_err(|e| {
            AwswitError::cache_error(format!("Failed to write cache file: {}", e))
        })?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).map_err(
                |e| {
                    AwswitError::cache_error(format!(
                        "Failed to set cache file permissions: {}",
                        e
                    ))
                },
            )?;
        }

        Ok(())
    }

    /// Remove cached credentials for a profile
    pub fn remove(&self, profile_name: &str) -> Result<()> {
        let path = self.cache_path(profile_name);
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| {
                AwswitError::cache_error(format!("Failed to remove cache file: {}", e))
            })?;
        }
        Ok(())
    }

    /// Clear all cached credentials
    pub fn clear_all(&self) -> Result<()> {
        if self.cache_dir.exists() {
            for entry in std::fs::read_dir(&self.cache_dir)? {
                let entry = entry?;
                if entry.path().extension().and_then(|e| e.to_str()) == Some("json") {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_manager(dir: &TempDir) -> CacheManager {
        let cache_dir = dir.path().join("cache");
        std::fs::create_dir_all(&cache_dir).unwrap();
        CacheManager { cache_dir }
    }

    #[test]
    fn test_cache_put_get() {
        let dir = TempDir::new().unwrap();
        let manager = test_manager(&dir);

        let creds = Credentials {
            access_key_id: "AKID".to_string(),
            secret_access_key: "SECRET".to_string(),
            session_token: Some("TOKEN".to_string()),
            expiration: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
            region: Some("us-east-1".to_string()),
        };

        manager.put("test-profile", &creds).unwrap();

        let cached = manager.get("test-profile").unwrap();
        assert!(cached.is_some());

        let cached = cached.unwrap();
        assert_eq!(cached.access_key_id, "AKID");
        assert_eq!(cached.secret_access_key, "SECRET");
    }

    #[test]
    fn test_cache_miss() {
        let dir = TempDir::new().unwrap();
        let manager = test_manager(&dir);

        let cached = manager.get("nonexistent").unwrap();
        assert!(cached.is_none());
    }

    #[test]
    fn test_cache_expired() {
        let dir = TempDir::new().unwrap();
        let manager = test_manager(&dir);

        let creds = Credentials {
            access_key_id: "AKID".to_string(),
            secret_access_key: "SECRET".to_string(),
            session_token: None,
            expiration: Some(chrono::Utc::now() - chrono::Duration::hours(1)),
            region: None,
        };

        manager.put("test-profile", &creds).unwrap();
        let cached = manager.get("test-profile").unwrap();
        assert!(cached.is_none());
    }

    #[test]
    fn test_cache_remove() {
        let dir = TempDir::new().unwrap();
        let manager = test_manager(&dir);

        let creds = Credentials {
            access_key_id: "AKID".to_string(),
            secret_access_key: "SECRET".to_string(),
            session_token: None,
            expiration: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
            region: None,
        };

        manager.put("test-profile", &creds).unwrap();
        manager.remove("test-profile").unwrap();

        let cached = manager.get("test-profile").unwrap();
        assert!(cached.is_none());
    }

    #[test]
    fn test_cache_clear_all() {
        let dir = TempDir::new().unwrap();
        let manager = test_manager(&dir);

        let creds = Credentials {
            access_key_id: "AKID".to_string(),
            secret_access_key: "SECRET".to_string(),
            session_token: None,
            expiration: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
            region: None,
        };

        manager.put("profile1", &creds).unwrap();
        manager.put("profile2", &creds).unwrap();
        manager.clear_all().unwrap();

        assert!(manager.get("profile1").unwrap().is_none());
        assert!(manager.get("profile2").unwrap().is_none());
    }
}
