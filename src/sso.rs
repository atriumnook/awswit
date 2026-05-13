//! Read the AWS CLI's SSO token cache to check session expiry.
//!
//! The AWS CLI writes one JSON file per SSO session into
//! `~/.aws/sso/cache/<sha1(startUrl)>.json`. Each file contains
//! `{ "startUrl", "region", "accessToken", "expiresAt", ... }`.
//!
//! We only need `startUrl`, `region`, and `expiresAt` — never the token
//! itself.

use std::fs;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct SsoSession {
    pub start_url: String,
    pub region: String,
    pub expires_at: DateTime<Utc>,
}

impl SsoSession {
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.expires_at <= now
    }
}

#[derive(Debug, Deserialize)]
struct RawCacheEntry {
    #[serde(rename = "startUrl")]
    start_url: Option<String>,
    region: Option<String>,
    #[serde(rename = "expiresAt")]
    expires_at: Option<String>,
}

/// Path to the AWS SSO cache directory.
///
/// Resolution order:
///   1. `$AWSWIT_SSO_CACHE_DIR` — explicit override for containers / CI
///      where the cache lives outside `~/.aws/`.
///   2. `${AWS_CONFIG_FILE%/config}/sso/cache` when `$AWS_CONFIG_FILE` is
///      set, so a self-contained `aws-config` directory works as expected.
///   3. `~/.aws/sso/cache`.
fn sso_cache_dir() -> Option<PathBuf> {
    if let Some(v) = std::env::var_os("AWSWIT_SSO_CACHE_DIR").filter(|s| !s.is_empty()) {
        return Some(PathBuf::from(v));
    }
    if let Ok(cfg) = std::env::var("AWS_CONFIG_FILE") {
        // Expand `~` so a user-style `AWS_CONFIG_FILE=~/.aws/config` works
        // on every platform — `Path::parent()` would otherwise return
        // `~/.aws` which `is_dir()` rejects.
        let expanded = shellexpand::tilde(&cfg).to_string();
        if let Some(parent) = std::path::Path::new(&expanded).parent() {
            let candidate = parent.join("sso").join("cache");
            if candidate.is_dir() {
                return Some(candidate);
            }
        }
    }
    dirs::home_dir().map(|h| h.join(".aws").join("sso").join("cache"))
}

/// Walk the SSO cache and return every readable session. Best-effort: a
/// malformed or unreadable file is skipped.
pub fn load_sessions() -> Vec<SsoSession> {
    let Some(dir) = sso_cache_dir() else {
        return Vec::new();
    };
    let Ok(read_dir) = fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let Ok(body) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(raw) = serde_json::from_str::<RawCacheEntry>(&body) else {
            continue;
        };

        let (Some(start_url), Some(region), Some(expires)) =
            (raw.start_url, raw.region, raw.expires_at)
        else {
            continue;
        };
        let Ok(expires_at) = DateTime::parse_from_rfc3339(&expires) else {
            continue;
        };
        out.push(SsoSession {
            start_url,
            region,
            expires_at: expires_at.with_timezone(&Utc),
        });
    }
    out
}

/// Find the SSO session matching `start_url` and (optionally) `region`.
pub fn find_session<'a>(
    sessions: &'a [SsoSession],
    start_url: &str,
    region: Option<&str>,
) -> Option<&'a SsoSession> {
    sessions
        .iter()
        .find(|s| s.start_url == start_url && region.is_none_or(|r| r.is_empty() || r == s.region))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn expired_in_past() {
        let s = SsoSession {
            start_url: "x".into(),
            region: "us-east-1".into(),
            expires_at: Utc::now() - Duration::hours(1),
        };
        assert!(s.is_expired(Utc::now()));
    }

    #[test]
    fn not_expired_in_future() {
        let s = SsoSession {
            start_url: "x".into(),
            region: "us-east-1".into(),
            expires_at: Utc::now() + Duration::hours(1),
        };
        assert!(!s.is_expired(Utc::now()));
    }

    #[test]
    fn find_session_matches_url() {
        let sessions = vec![SsoSession {
            start_url: "https://a/start".into(),
            region: "us-east-1".into(),
            expires_at: Utc::now(),
        }];
        assert!(find_session(&sessions, "https://a/start", None).is_some());
        assert!(find_session(&sessions, "https://b/start", None).is_none());
        assert!(find_session(&sessions, "https://a/start", Some("us-east-1")).is_some());
        assert!(find_session(&sessions, "https://a/start", Some("eu-west-1")).is_none());
    }
}
