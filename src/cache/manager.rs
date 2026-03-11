use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::aws::Credentials;
use crate::error::AwswitError;

/// Cache entry wrapping credentials with the original key for collision detection
#[derive(Serialize, Deserialize)]
struct CacheEntry {
    cache_key: String,
    #[serde(flatten)]
    credentials: Credentials,
}

/// Manages credential caching in ~/.awswit/cache/
pub struct CacheManager {
    cache_dir: PathBuf,
}

impl CacheManager {
    /// Create a new cache manager
    pub fn new() -> Result<Self, AwswitError> {
        let cache_dir = dirs::home_dir()
            .ok_or_else(|| AwswitError::CacheError {
                message: "Cannot determine home directory".to_string(),
            })?
            .join(".awswit")
            .join("cache");

        // Ensure cache directory exists with restrictive permissions
        #[cfg(unix)]
        {
            use std::fs::DirBuilder;
            use std::os::unix::fs::DirBuilderExt;
            DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&cache_dir)
                .map_err(|e| AwswitError::CacheError {
                    message: format!("Failed to create cache dir: {}", e),
                })?;
        }

        #[cfg(not(unix))]
        {
            fs::create_dir_all(&cache_dir)?;
        }

        Ok(Self { cache_dir })
    }

    /// Get cached credentials by key
    pub fn get(&self, key: &str) -> Result<Option<Credentials>, AwswitError> {
        let path = self.cache_file_path(key);

        // Read directly — handle NotFound as cache miss (no TOCTOU)
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(AwswitError::CacheError {
                    message: format!("Failed to read cache: {}", e),
                })
            }
        };

        // Corrupt JSON → cache miss, not hard error
        let entry: CacheEntry = match serde_json::from_str(&content) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("Corrupt cache entry '{}': {}, removing", key, e);
                if let Err(e) = self.remove(key) {
                    tracing::warn!("Failed to remove corrupt cache entry '{}': {}", key, e);
                }
                return Ok(None);
            }
        };

        // Verify the stored key matches to detect sanitization collisions
        if entry.cache_key != key {
            tracing::warn!(
                "Cache key mismatch: requested '{}' but file contains '{}', treating as miss",
                key,
                entry.cache_key
            );
            return Ok(None);
        }

        // Check if expired
        if entry.credentials.is_expired() {
            tracing::debug!("Cache entry '{}' is expired, removing", key);
            self.remove(key)?;
            return Ok(None);
        }

        Ok(Some(entry.credentials))
    }

    /// Set cached credentials
    pub fn set(&self, key: &str, credentials: &Credentials) -> Result<(), AwswitError> {
        let path = self.cache_file_path(key);
        let entry = CacheEntry {
            cache_key: key.to_string(),
            credentials: credentials.clone(),
        };
        let content = serde_json::to_string_pretty(&entry)?;

        crate::utils::fs::atomic_write_restricted(&path, content.as_bytes())?;

        tracing::debug!("Cached credentials with key: {}", key);
        Ok(())
    }

    /// Remove cached credentials
    pub fn remove(&self, key: &str) -> Result<(), AwswitError> {
        let path = self.cache_file_path(key);

        match fs::remove_file(&path) {
            Ok(()) => tracing::debug!("Removed cache entry: {}", key),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }

        Ok(())
    }

    /// Get the cache file path for a key.
    /// Uses hex encoding to avoid collisions from character sanitization
    /// (e.g., "role/dev" and "role_dev" would collide with simple replacement).
    fn cache_file_path(&self, key: &str) -> PathBuf {
        let hex_key: String = key.as_bytes().iter().map(|b| format!("{:02x}", b)).collect();
        self.cache_dir.join(format!("{}.json", hex_key))
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

    #[test]
    fn test_corrupt_json_returns_cache_miss() {
        let (manager, _temp) = create_test_manager();

        // Write corrupt JSON to cache file
        let path = manager.cache_file_path("corrupt-key");
        fs::write(&path, "not valid json{{{").unwrap();

        // Should return None, not error
        let result = manager.get("corrupt-key").unwrap();
        assert!(result.is_none());

        // Corrupt file should be cleaned up
        assert!(!path.exists());
    }

    #[test]
    fn test_no_key_collision_with_similar_names() {
        let (manager, _temp) = create_test_manager();

        // These keys would collide with simple character replacement
        let creds1 = Credentials {
            access_key_id: "KEY_ONE".to_string(),
            secret_access_key: "secret1".to_string(),
            session_token: None,
            expiration: Some(Utc::now() + Duration::hours(1)),
            region: None,
        };
        let creds2 = Credentials {
            access_key_id: "KEY_TWO".to_string(),
            secret_access_key: "secret2".to_string(),
            session_token: None,
            expiration: Some(Utc::now() + Duration::hours(1)),
            region: None,
        };

        manager.set("role/dev", &creds1).unwrap();
        manager.set("role_dev", &creds2).unwrap();

        // Both should retrieve correctly without collision
        let retrieved1 = manager.get("role/dev").unwrap().unwrap();
        let retrieved2 = manager.get("role_dev").unwrap().unwrap();
        assert_eq!(retrieved1.access_key_id, "KEY_ONE");
        assert_eq!(retrieved2.access_key_id, "KEY_TWO");
    }

    #[test]
    fn test_hex_encoding_produces_different_paths() {
        let (manager, _temp) = create_test_manager();

        let path1 = manager.cache_file_path("role/dev");
        let path2 = manager.cache_file_path("role_dev");
        assert_ne!(path1, path2);
    }

    #[test]
    fn test_cache_key_mismatch_returns_miss() {
        let (manager, _temp) = create_test_manager();

        // Manually write a cache entry with a mismatched key
        let path = manager.cache_file_path("test-key");
        let entry = CacheEntry {
            cache_key: "different-key".to_string(),
            credentials: Credentials {
                access_key_id: "AKIATEST".to_string(),
                secret_access_key: "secret".to_string(),
                session_token: None,
                expiration: Some(Utc::now() + Duration::hours(1)),
                region: None,
            },
        };
        let content = serde_json::to_string(&entry).unwrap();
        fs::write(&path, content).unwrap();

        // Should return None due to key mismatch
        let result = manager.get("test-key").unwrap();
        assert!(result.is_none());
    }

}
