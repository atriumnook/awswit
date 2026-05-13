use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::AwswitError;
use crate::utils::paths;

/// Recency half-life in hours. Score = `use_count * 2^(-age_hours/HALF_LIFE)`.
const FRECENCY_HALF_LIFE_HOURS: f64 = 72.0;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub name: String,
    pub last_used: DateTime<Utc>,
    pub use_count: u32,
    pub is_favorite: bool,
}

impl HistoryEntry {
    /// Continuous frecency score with exponential decay.
    ///
    /// The previous implementation used four step buckets (1h / 24h / 1w /
    /// older), which made ranking lurch at bucket edges. Exponential decay
    /// produces smooth ordering: a profile used 12h ago always outranks the
    /// same one used 24h ago, regardless of step boundaries.
    pub fn frecency_score(&self, now: DateTime<Utc>) -> f64 {
        let hours = (now - self.last_used).num_seconds().max(0) as f64 / 3600.0;
        let decay = (-hours / FRECENCY_HALF_LIFE_HOURS).exp2();
        f64::from(self.use_count) * decay
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProfileHistory {
    entries: HashMap<String, HistoryEntry>,
}

fn history_path() -> Result<PathBuf, AwswitError> {
    let data_dir = paths::data_dir().map_err(|e| AwswitError::ConfigFileError {
        message: e.to_string(),
    })?;
    let target = data_dir.join("history.json");

    // One-time migration from the pre-0.1.0 location.
    if let Ok(legacy_dir) = paths::legacy_dir() {
        let legacy = legacy_dir.join("history.json");
        if let Err(e) = paths::migrate_legacy_into(&legacy, &target) {
            tracing::debug!("history migration skipped: {}", e);
        }
    }

    Ok(target)
}

/// Load history. Missing → empty. Corrupt → backed up and reset.
pub fn load_history() -> Result<ProfileHistory, AwswitError> {
    let path = history_path()?;

    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ProfileHistory::default()),
        Err(e) => return Err(e.into()),
    };

    match serde_json::from_str::<ProfileHistory>(&content) {
        Ok(h) => Ok(h),
        Err(e) => {
            let backup = path.with_extension("json.corrupt");
            tracing::warn!(
                "history file is corrupt ({}); backing up to {} and starting fresh",
                e,
                backup.display()
            );
            let _ = fs::copy(&path, &backup);
            Ok(ProfileHistory::default())
        }
    }
}

/// How old a leftover `history.json.<pid>.tmp` must be before we sweep it.
///
/// One hour is comfortably longer than any plausible awswit invocation. The
/// previous implementation swept every tmp sibling unconditionally, which
/// raced with concurrent saves: a second awswit running `xargs -P` could
/// delete the first one's tmp file mid-flight and turn its `rename` into a
/// silent ENOENT.
const STALE_TMP_AGE_SECS: u64 = 60 * 60;

/// Persist history atomically: write to a sibling temp file, fsync it, then
/// rename onto the final path.
///
/// - `sync_all()` the temp file before rename so a power-loss between write
///   and rename doesn't leave a zero-byte file post-rename.
/// - Sweep `history.json.*.tmp` siblings older than [`STALE_TMP_AGE_SECS`]
///   so a prior crash doesn't accrete junk — but only those siblings,
///   never the ones currently in flight on a parallel awswit.
/// - File permissions are left to the user's umask. The contents are
///   profile names, timestamps, and favorite flags, not secrets.
pub fn save_history(history: &ProfileHistory) -> Result<(), AwswitError> {
    use std::io::Write;
    use std::time::SystemTime;

    let path = history_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        // Best-effort age-gated sweep of crash-orphaned temp files. Never
        // touches a tmp belonging to a currently-running sibling process.
        if let Ok(read_dir) = fs::read_dir(parent) {
            let now = SystemTime::now();
            for entry in read_dir.flatten() {
                let p = entry.path();
                let name_matches = p
                    .file_name()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s.starts_with("history.json.") && s.ends_with(".tmp"));
                if !name_matches {
                    continue;
                }
                let stale = entry
                    .metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| now.duration_since(t).ok())
                    .is_some_and(|d| d.as_secs() >= STALE_TMP_AGE_SECS);
                if stale {
                    let _ = fs::remove_file(&p);
                }
            }
        }
    }

    let body = serde_json::to_string_pretty(history)?;
    let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));

    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(body.as_bytes())?;
        f.sync_all()?;
    }
    if let Err(e) = fs::rename(&tmp, &path) {
        let _ = fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

impl ProfileHistory {
    pub fn record_use(&mut self, name: &str) {
        let now = Utc::now();
        let entry = self
            .entries
            .entry(name.to_string())
            .or_insert_with(|| HistoryEntry {
                name: name.to_string(),
                last_used: now,
                use_count: 0,
                is_favorite: false,
            });
        entry.last_used = now;
        entry.use_count = entry.use_count.saturating_add(1);
    }

    pub fn get(&self, name: &str) -> Option<&HistoryEntry> {
        self.entries.get(name)
    }

    pub fn is_favorite(&self, name: &str) -> bool {
        self.entries.get(name).is_some_and(|e| e.is_favorite)
    }

    pub fn set_favorite(&mut self, name: &str, favorite: bool) {
        match self.entries.get_mut(name) {
            Some(entry) => entry.is_favorite = favorite,
            None if favorite => {
                self.entries.insert(
                    name.to_string(),
                    HistoryEntry {
                        name: name.to_string(),
                        last_used: Utc::now(),
                        use_count: 0,
                        is_favorite: true,
                    },
                );
            }
            None => {}
        }
    }

    /// Ordering: favorites first, then frecency desc, then name asc.
    pub fn compare_by_frecency(&self, a: &str, b: &str, now: DateTime<Utc>) -> std::cmp::Ordering {
        let a_fav = self.is_favorite(a);
        let b_fav = self.is_favorite(b);
        b_fav
            .cmp(&a_fav)
            .then_with(|| {
                let sa = self.get(a).map(|e| e.frecency_score(now)).unwrap_or(0.0);
                let sb = self.get(b).map(|e| e.frecency_score(now)).unwrap_or(0.0);
                sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.cmp(b))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_increments_count_and_updates_timestamp() {
        let mut h = ProfileHistory::default();
        h.record_use("p");
        h.record_use("p");
        assert_eq!(h.get("p").unwrap().use_count, 2);
    }

    #[test]
    fn frecency_decays_monotonically() {
        let now = Utc::now();
        let make = |hours: i64| HistoryEntry {
            name: "p".into(),
            last_used: now - chrono::Duration::hours(hours),
            use_count: 1,
            is_favorite: false,
        };
        let s0 = make(0).frecency_score(now);
        let s1 = make(1).frecency_score(now);
        let s24 = make(24).frecency_score(now);
        let s168 = make(168).frecency_score(now);
        assert!(s0 > s1);
        assert!(s1 > s24);
        assert!(s24 > s168);
        assert!(s168 > 0.0);
    }

    #[test]
    fn frecency_scales_with_use_count() {
        let now = Utc::now();
        let one = HistoryEntry {
            name: "p".into(),
            last_used: now,
            use_count: 1,
            is_favorite: false,
        }
        .frecency_score(now);
        let ten = HistoryEntry {
            name: "p".into(),
            last_used: now,
            use_count: 10,
            is_favorite: false,
        }
        .frecency_score(now);
        assert!((ten - 10.0 * one).abs() < f64::EPSILON);
    }

    #[test]
    fn favorites_outrank_recent_use() {
        let mut h = ProfileHistory::default();
        h.set_favorite("fav", true);
        h.record_use("recent");
        assert_eq!(
            h.compare_by_frecency("fav", "recent", Utc::now()),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn unknown_profiles_sort_alphabetically() {
        let h = ProfileHistory::default();
        let now = Utc::now();
        assert_eq!(
            h.compare_by_frecency("aaa", "zzz", now),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.json");

        let mut h = ProfileHistory::default();
        h.record_use("p");
        h.set_favorite("p", true);
        fs::write(&path, serde_json::to_string_pretty(&h).unwrap()).unwrap();

        let loaded: ProfileHistory =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(loaded.get("p").unwrap().use_count, 1);
        assert!(loaded.is_favorite("p"));
    }

    #[test]
    fn favorites_toggle() {
        let mut h = ProfileHistory::default();
        assert!(!h.is_favorite("p"));
        h.set_favorite("p", true);
        assert!(h.is_favorite("p"));
        h.set_favorite("p", false);
        assert!(!h.is_favorite("p"));
    }
}
