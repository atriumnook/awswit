//! Integration tests for CredentialStore trait lifecycle.
//! Tests set/get/remove, expiry handling, and corrupt JSON recovery.

use std::fs;
use std::path::PathBuf;

use awswit::aws::Credentials;
use awswit::cache::CredentialStore;
use chrono::{Duration, Utc};

#[derive(serde::Serialize, serde::Deserialize)]
struct CacheEntry {
    cache_key: String,
    #[serde(flatten)]
    credentials: Credentials,
}

struct TestStore {
    cache_dir: PathBuf,
}

impl TestStore {
    fn new(tmp: &tempfile::TempDir) -> Self {
        let cache_dir = tmp.path().join("cache");
        fs::create_dir_all(&cache_dir).unwrap();
        Self { cache_dir }
    }

    fn cache_file_path(&self, key: &str) -> PathBuf {
        let hex: String = key
            .as_bytes()
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect();
        self.cache_dir.join(format!("{}.json", hex))
    }
}

impl CredentialStore for TestStore {
    fn get(&self, key: &str) -> Result<Option<Credentials>, awswit::error::AwswitError> {
        let path = self.cache_file_path(key);
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(awswit::error::AwswitError::CacheError {
                    message: e.to_string(),
                });
            }
        };
        let entry: CacheEntry = match serde_json::from_str(&content) {
            Ok(e) => e,
            Err(_) => {
                let _ = fs::remove_file(&path);
                return Ok(None);
            }
        };
        if entry.cache_key != key || entry.credentials.is_expired() {
            let _ = fs::remove_file(&path);
            return Ok(None);
        }
        Ok(Some(entry.credentials))
    }

    fn set(&self, key: &str, credentials: &Credentials) -> Result<(), awswit::error::AwswitError> {
        let path = self.cache_file_path(key);
        let entry = CacheEntry {
            cache_key: key.to_string(),
            credentials: credentials.clone(),
        };
        fs::write(&path, serde_json::to_string_pretty(&entry).unwrap()).map_err(|e| {
            awswit::error::AwswitError::CacheError {
                message: e.to_string(),
            }
        })
    }

    fn remove(&self, key: &str) -> Result<(), awswit::error::AwswitError> {
        let path = self.cache_file_path(key);
        match fs::remove_file(&path) {
            Ok(()) | Err(_) => Ok(()),
        }
    }
}

#[test]
fn full_lifecycle_set_get_remove() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = TestStore::new(&tmp);

    let creds = Credentials {
        access_key_id: "AKIA_LIFECYCLE".to_string(),
        secret_access_key: "secret".to_string(),
        session_token: Some("tok".to_string()),
        expiration: Some(Utc::now() + Duration::hours(1)),
        region: Some("us-east-1".to_string()),
    };

    store.set("key1", &creds).unwrap();
    let got = store.get("key1").unwrap().expect("should exist");
    assert_eq!(got.access_key_id, "AKIA_LIFECYCLE");
    assert_eq!(got.region, Some("us-east-1".to_string()));

    store.remove("key1").unwrap();
    assert!(store.get("key1").unwrap().is_none());
}

#[test]
fn expired_returns_none() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = TestStore::new(&tmp);

    let creds = Credentials {
        access_key_id: "AKIA_EXPIRED".to_string(),
        secret_access_key: "secret".to_string(),
        session_token: None,
        expiration: Some(Utc::now() - Duration::hours(1)),
        region: None,
    };

    store.set("expired", &creds).unwrap();
    assert!(store.get("expired").unwrap().is_none());
}

#[test]
fn corrupt_json_graceful_miss() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = TestStore::new(&tmp);

    let path = store.cache_file_path("corrupt");
    fs::write(&path, "NOT VALID JSON {{{{").unwrap();

    assert!(store.get("corrupt").unwrap().is_none());
    assert!(!path.exists(), "corrupt file should be cleaned up");
}

#[test]
fn remove_nonexistent_is_ok() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = TestStore::new(&tmp);
    store.remove("never-set").unwrap();
}
