use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::thread;
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The on-disk schema written by this version of awswit.
pub(crate) const CURRENT_SCHEMA_VERSION: u32 = 1;
const LEGACY_SCHEMA_VERSION: u64 = 0;

/// History is deliberately bounded: it is a convenience index, not an audit log.
pub(crate) const MAX_HISTORY_ENTRIES: usize = 512;

const MAX_HISTORY_FILE_BYTES: u64 = 1024 * 1024;
const UNIQUE_FILE_ATTEMPTS: usize = 128;
const LOCK_WAIT_BUDGET: Duration = Duration::from_secs(2);
const LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(5);
static UNIQUE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A typed storage failure. Messages intentionally never include history contents.
#[derive(Debug, Error)]
pub(crate) enum PreferenceError {
    #[error("could not prepare preference storage: {source}")]
    Prepare {
        #[source]
        source: io::Error,
    },

    #[error("could not lock preference storage: {source}")]
    Lock {
        #[source]
        source: io::Error,
    },

    #[error("could not read preference history: {source}")]
    Read {
        #[source]
        source: io::Error,
    },

    #[error("preference history uses unsupported schema version {found}")]
    UnsupportedSchema { found: u64 },

    #[error("could not preserve corrupt preference history: {source}")]
    PreserveCorrupt {
        #[source]
        source: io::Error,
    },

    #[error("could not encode preference history: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },

    #[error("could not write preference history: {source}")]
    Write {
        #[source]
        source: io::Error,
    },

    #[error("could not atomically replace preference history: {source}")]
    Replace {
        #[source]
        source: io::Error,
    },

    #[error("could not sync preference storage: {source}")]
    Sync {
        #[source]
        source: io::Error,
    },
}

/// A single profile's durable preference metadata.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreferenceEntry {
    favorite: bool,
    use_count: u64,
    last_used_sequence: u64,
    last_used_unix_ms: Option<i64>,
}

impl PreferenceEntry {
    pub fn is_favorite(&self) -> bool {
        self.favorite
    }

    pub fn use_count(&self) -> u64 {
        self.use_count
    }

    /// A store-local logical sequence used for stable MRU ordering.
    pub fn last_used_sequence(&self) -> u64 {
        self.last_used_sequence
    }

    #[cfg(test)]
    pub fn last_used_unix_ms(&self) -> Option<i64> {
        self.last_used_unix_ms
    }
}

/// An immutable snapshot used by the picker and list views.
#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub(crate) struct PreferenceHistory {
    entries: BTreeMap<String, PreferenceEntry>,
    next_sequence: u64,
}

impl PreferenceHistory {
    pub fn get(&self, profile_name: &str) -> Option<&PreferenceEntry> {
        self.entries.get(profile_name)
    }

    pub fn is_favorite(&self, profile_name: &str) -> bool {
        self.get(profile_name)
            .is_some_and(PreferenceEntry::is_favorite)
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Favorites first, then most-recently-used, then usage count, then name.
    ///
    /// The final name comparison makes the order total and reproducible. Wall-clock
    /// timestamps are deliberately excluded, so clock jumps cannot reorder profiles.
    pub fn compare_rank(&self, a: &str, b: &str) -> Ordering {
        let a_entry = self.get(a);
        let b_entry = self.get(b);

        let a_favorite = a_entry.is_some_and(PreferenceEntry::is_favorite);
        let b_favorite = b_entry.is_some_and(PreferenceEntry::is_favorite);
        let a_sequence = a_entry.map_or(0, PreferenceEntry::last_used_sequence);
        let b_sequence = b_entry.map_or(0, PreferenceEntry::last_used_sequence);
        let a_count = a_entry.map_or(0, PreferenceEntry::use_count);
        let b_count = b_entry.map_or(0, PreferenceEntry::use_count);

        b_favorite
            .cmp(&a_favorite)
            .then_with(|| b_sequence.cmp(&a_sequence))
            .then_with(|| b_count.cmp(&a_count))
            .then_with(|| a.cmp(b))
    }

    fn apply(&mut self, delta: &PreferenceDelta) {
        for (name, favorite) in &delta.favorite_changes {
            match self.entries.get_mut(name) {
                Some(entry) => entry.favorite = *favorite,
                None if *favorite => {
                    self.entries.insert(
                        name.clone(),
                        PreferenceEntry {
                            favorite: true,
                            use_count: 0,
                            last_used_sequence: 0,
                            last_used_unix_ms: None,
                        },
                    );
                }
                None => {}
            }
        }

        if let Some(selected) = &delta.selected {
            let sequence = self.allocate_sequence();
            let entry =
                self.entries
                    .entry(selected.profile_name.clone())
                    .or_insert(PreferenceEntry {
                        favorite: false,
                        use_count: 0,
                        last_used_sequence: 0,
                        last_used_unix_ms: None,
                    });
            entry.use_count = entry.use_count.saturating_add(1);
            entry.last_used_sequence = sequence;
            entry.last_used_unix_ms = Some(selected.at_unix_ms);
        }
    }

    fn allocate_sequence(&mut self) -> u64 {
        if self.next_sequence == u64::MAX {
            self.renumber_sequences();
        }
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.next_sequence
    }

    fn renumber_sequences(&mut self) {
        let mut names: Vec<_> = self.entries.keys().cloned().collect();
        names.sort_by(|a, b| {
            let a_sequence = self
                .entries
                .get(a)
                .map_or(0, PreferenceEntry::last_used_sequence);
            let b_sequence = self
                .entries
                .get(b)
                .map_or(0, PreferenceEntry::last_used_sequence);
            a_sequence.cmp(&b_sequence).then_with(|| a.cmp(b))
        });

        for (index, name) in names.into_iter().enumerate() {
            if let Some(entry) = self.entries.get_mut(&name) {
                entry.last_used_sequence = (index as u64).saturating_add(1);
            }
        }
        self.next_sequence = self.entries.len() as u64;
    }

    fn prune(&mut self) {
        self.entries
            .retain(|_, entry| entry.favorite || entry.use_count > 0);

        if self.entries.len() <= MAX_HISTORY_ENTRIES {
            return;
        }

        let mut ranked: Vec<_> = self.entries.keys().cloned().collect();
        ranked.sort_by(|a, b| self.compare_rank(a, b));
        ranked.truncate(MAX_HISTORY_ENTRIES);
        let retained: BTreeSet<_> = ranked.into_iter().collect();
        self.entries.retain(|name, _| retained.contains(name));
    }

    fn normalize_after_decode(&mut self) {
        self.next_sequence = self.next_sequence.max(
            self.entries
                .values()
                .map(|entry| entry.last_used_sequence)
                .max()
                .unwrap_or(0),
        );

        self.prune();
    }
}

/// A selection use the caller has decided to record. The caller supplies the
/// informational usage time; the store never obtains or infers that value.
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct SelectionUse {
    pub profile_name: String,
    pub at_unix_ms: i64,
}

/// The picker's intended changes. It contains no stale history snapshot.
#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub(crate) struct PreferenceDelta {
    favorite_changes: BTreeMap<String, bool>,
    selected: Option<SelectionUse>,
}

impl PreferenceDelta {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_favorite(&mut self, profile_name: impl Into<String>, favorite: bool) {
        self.favorite_changes.insert(profile_name.into(), favorite);
    }

    pub fn record_selection(&mut self, profile_name: impl Into<String>, at_unix_ms: i64) {
        self.selected = Some(SelectionUse {
            profile_name: profile_name.into(),
            at_unix_ms,
        });
    }
}

/// Durable preference storage at one explicit path.
#[derive(Debug, Clone)]
pub(crate) struct PreferenceStore {
    path: PathBuf,
}

impl PreferenceStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Loads a consistent snapshot under the same lock used by commits.
    ///
    /// Invalid JSON or an invalid known schema is preserved as a uniquely named
    /// `.corrupt-*` sibling and, when preservation succeeds, treated as empty
    /// history. A newer schema is left untouched and returned as an error so an
    /// older binary cannot destroy it.
    pub fn load(&self) -> Result<PreferenceHistory, PreferenceError> {
        let _lock = self.lock_exclusive()?;
        self.read_locked()
    }

    /// Applies a picker delta to the latest on-disk state in one transaction.
    ///
    /// The lock covers re-read, merge, prune, synced temp write, and atomic replace,
    /// preventing lost updates between cooperating threads and processes. A parent
    /// directory sync is additionally attempted on Unix; other platforms stop after
    /// syncing the file and atomically replacing it. Entries from every catalog
    /// are retained and compete under the same deterministic global size bound.
    pub fn commit(&self, delta: &PreferenceDelta) -> Result<PreferenceHistory, PreferenceError> {
        let _lock = self.lock_exclusive()?;
        let mut history = self.read_locked()?;
        history.apply(delta);
        history.prune();
        self.write_locked(&history)?;
        Ok(history)
    }

    fn parent(&self) -> &Path {
        self.path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
    }

    fn lock_exclusive(&self) -> Result<File, PreferenceError> {
        fs::create_dir_all(self.parent()).map_err(|source| PreferenceError::Prepare { source })?;

        let lock_path = sibling_path(&self.path, ".lock");
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        set_private_create_mode(&mut options);
        set_internal_lock_flags(&mut options);
        let lock = options
            .open(&lock_path)
            .map_err(|source| PreferenceError::Prepare { source })?;
        if !metadata_is_regular(
            &lock
                .metadata()
                .map_err(|source| PreferenceError::Prepare { source })?,
        ) {
            return Err(PreferenceError::Prepare {
                source: io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "preference lock is not a regular file",
                ),
            });
        }
        enforce_private_file_permissions(&lock)
            .map_err(|source| PreferenceError::Prepare { source })?;
        let deadline = Instant::now() + LOCK_WAIT_BUDGET;
        loop {
            match lock.try_lock() {
                Ok(()) => break,
                Err(fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                    thread::sleep(LOCK_RETRY_INTERVAL);
                }
                Err(error) => {
                    return Err(PreferenceError::Lock {
                        source: io::Error::other(error),
                    });
                }
            }
        }
        Ok(lock)
    }

    fn read_locked(&self) -> Result<PreferenceHistory, PreferenceError> {
        let mut file = match open_regular_for_read(&self.path) {
            Ok(file) => file,
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                return Ok(PreferenceHistory::default());
            }
            Err(source) => return Err(PreferenceError::Read { source }),
        };

        enforce_private_file_permissions(&file)
            .map_err(|source| PreferenceError::Read { source })?;

        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(MAX_HISTORY_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|source| PreferenceError::Read { source })?;

        let decoded = if bytes.len() as u64 > MAX_HISTORY_FILE_BYTES {
            Err(DecodeFailure::Corrupt)
        } else {
            decode_history(&bytes)
        };

        match decoded {
            Ok(mut history) => {
                history.normalize_after_decode();
                Ok(history)
            }
            Err(DecodeFailure::UnsupportedSchema(found)) => {
                Err(PreferenceError::UnsupportedSchema { found })
            }
            Err(DecodeFailure::Corrupt) => {
                self.preserve_corrupt_locked()?;
                Ok(PreferenceHistory::default())
            }
        }
    }

    fn preserve_corrupt_locked(&self) -> Result<(), PreferenceError> {
        // Hard-link + unlink gives the backup an exclusive destination name without
        // loading arbitrary-sized corrupt input into memory. Fall back to an exclusive
        // streamed copy for filesystems that do not support hard links.
        for _ in 0..UNIQUE_FILE_ATTEMPTS {
            let backup_path = unique_sibling_path(&self.path, "corrupt");
            match fs::hard_link(&self.path, &backup_path) {
                Ok(()) => {
                    let private_result = open_regular_for_read(&backup_path).and_then(|backup| {
                        enforce_private_file_permissions(&backup)?;
                        sync_preserved_file(&backup)
                    });
                    if let Err(source) = private_result {
                        let _ = fs::remove_file(&backup_path);
                        return Err(PreferenceError::PreserveCorrupt { source });
                    }
                    fs::remove_file(&self.path)
                        .map_err(|source| PreferenceError::PreserveCorrupt { source })?;
                    self.sync_parent()?;
                    return Ok(());
                }
                Err(source) if source.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(_) => return self.copy_corrupt_exclusively(),
            }
        }

        Err(PreferenceError::PreserveCorrupt {
            source: unique_name_exhausted(),
        })
    }

    fn copy_corrupt_exclusively(&self) -> Result<(), PreferenceError> {
        let (backup_path, mut backup) = open_unique_sibling(&self.path, "corrupt")
            .map_err(|source| PreferenceError::PreserveCorrupt { source })?;
        let mut cleanup = RemoveOnDrop::new(backup_path.clone());
        let mut source_file = open_regular_for_read(&self.path)
            .map_err(|source| PreferenceError::PreserveCorrupt { source })?;

        copy_bounded_corrupt(&mut source_file, &mut backup)
            .map_err(|source| PreferenceError::PreserveCorrupt { source })?;
        backup
            .flush()
            .map_err(|source| PreferenceError::PreserveCorrupt { source })?;
        enforce_private_file_permissions(&backup)
            .map_err(|source| PreferenceError::PreserveCorrupt { source })?;
        backup
            .sync_all()
            .map_err(|source| PreferenceError::PreserveCorrupt { source })?;
        drop(backup);
        fs::remove_file(&self.path)
            .map_err(|source| PreferenceError::PreserveCorrupt { source })?;
        cleanup.keep();
        self.sync_parent()?;
        Ok(())
    }

    fn write_locked(&self, history: &PreferenceHistory) -> Result<(), PreferenceError> {
        let stored = StoredHistory::from(history);
        let mut bytes = serde_json::to_vec_pretty(&stored)
            .map_err(|source| PreferenceError::Encode { source })?;
        bytes.push(b'\n');

        let (temp_path, mut temp) = open_unique_sibling(&self.path, "tmp")
            .map_err(|source| PreferenceError::Write { source })?;
        let mut cleanup = RemoveOnDrop::new(temp_path.clone());
        enforce_private_file_permissions(&temp)
            .map_err(|source| PreferenceError::Write { source })?;
        temp.write_all(&bytes)
            .and_then(|()| temp.flush())
            .and_then(|()| temp.sync_all())
            .map_err(|source| PreferenceError::Write { source })?;
        drop(temp);

        fs::rename(&temp_path, &self.path).map_err(|source| PreferenceError::Replace { source })?;
        cleanup.keep();
        self.sync_parent()
    }

    fn sync_parent(&self) -> Result<(), PreferenceError> {
        sync_directory(self.parent()).map_err(|source| PreferenceError::Sync { source })
    }
}

fn copy_bounded_corrupt(source: &mut impl Read, destination: &mut impl Write) -> io::Result<()> {
    let copied = io::copy(
        &mut source.take(MAX_HISTORY_FILE_BYTES.saturating_add(1)),
        destination,
    )?;
    if copied > MAX_HISTORY_FILE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "corrupt preference history exceeds the preservation limit",
        ));
    }
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredHistory {
    schema_version: u32,
    next_sequence: u64,
    entries: BTreeMap<String, PreferenceEntry>,
}

impl From<&PreferenceHistory> for StoredHistory {
    fn from(history: &PreferenceHistory) -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            next_sequence: history.next_sequence,
            entries: history.entries.clone(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyHistory {
    entries: BTreeMap<String, LegacyEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyEntry {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    last_used: Option<serde_json::Value>,
    #[serde(default)]
    use_count: u64,
    #[serde(default)]
    is_favorite: bool,
}

enum DecodeFailure {
    Corrupt,
    UnsupportedSchema(u64),
}

fn decode_history(bytes: &[u8]) -> Result<PreferenceHistory, DecodeFailure> {
    let mut value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| DecodeFailure::Corrupt)?;

    let schema_version = value.get("schema_version");
    match schema_version {
        Some(version) => {
            let version = version.as_u64().ok_or(DecodeFailure::Corrupt)?;
            if version == LEGACY_SCHEMA_VERSION {
                value
                    .as_object_mut()
                    .ok_or(DecodeFailure::Corrupt)?
                    .remove("schema_version");
                migrate_legacy(value)
            } else if version == u64::from(CURRENT_SCHEMA_VERSION) {
                let stored: StoredHistory =
                    serde_json::from_value(value).map_err(|_| DecodeFailure::Corrupt)?;
                Ok(PreferenceHistory {
                    entries: stored.entries,
                    next_sequence: stored.next_sequence,
                })
            } else {
                Err(DecodeFailure::UnsupportedSchema(version))
            }
        }
        None => migrate_legacy(value),
    }
}

fn migrate_legacy(value: serde_json::Value) -> Result<PreferenceHistory, DecodeFailure> {
    let legacy: LegacyHistory =
        serde_json::from_value(value).map_err(|_| DecodeFailure::Corrupt)?;

    // The legacy writer emitted UTC RFC3339 strings. Their lexical order provides a
    // deterministic best-effort migration of the old recency signal; unusual/missing
    // timestamp forms are still ordered deterministically by profile name.
    let mut legacy_entries: Vec<_> = legacy.entries.into_iter().collect();
    legacy_entries.sort_by(|(a_name, a), (b_name, b)| {
        legacy_timestamp(a)
            .cmp(legacy_timestamp(b))
            .then_with(|| a_name.cmp(b_name))
    });

    let mut entries = BTreeMap::new();
    let mut next_sequence = 0_u64;
    for (name, legacy) in legacy_entries {
        let _legacy_name = legacy.name;
        if legacy.use_count > 0 {
            next_sequence = next_sequence.saturating_add(1);
        }
        if legacy.use_count > 0 || legacy.is_favorite {
            entries.insert(
                name,
                PreferenceEntry {
                    favorite: legacy.is_favorite,
                    use_count: legacy.use_count,
                    last_used_sequence: if legacy.use_count > 0 {
                        next_sequence
                    } else {
                        0
                    },
                    last_used_unix_ms: None,
                },
            );
        }
    }

    Ok(PreferenceHistory {
        entries,
        next_sequence,
    })
}

fn legacy_timestamp(entry: &LegacyEntry) -> &str {
    entry
        .last_used
        .as_ref()
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
}

fn sibling_path(path: &Path, suffix: &str) -> PathBuf {
    let mut file_name = path
        .file_name()
        .map_or_else(|| OsString::from("history.json"), OsString::from);
    file_name.push(suffix);
    path.with_file_name(file_name)
}

fn unique_sibling_path(path: &Path, kind: &str) -> PathBuf {
    let counter = UNIQUE_COUNTER.fetch_add(1, AtomicOrdering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let suffix = format!(".{kind}-{}-{nanos}-{counter}", std::process::id());
    sibling_path(path, &suffix)
}

fn open_unique_sibling(path: &Path, kind: &str) -> io::Result<(PathBuf, File)> {
    for _ in 0..UNIQUE_FILE_ATTEMPTS {
        let candidate = unique_sibling_path(path, kind);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        set_private_create_mode(&mut options);
        match options.open(&candidate) {
            Ok(file) => return Ok((candidate, file)),
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(source) => return Err(source),
        }
    }
    Err(unique_name_exhausted())
}

fn unique_name_exhausted() -> io::Error {
    io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a unique preference storage file",
    )
}

fn open_regular_for_read(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    set_nonblocking_read_flags(&mut options);
    let file = options.open(path)?;
    if !metadata_is_regular(&file.metadata()?) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "preference storage path is not a regular file",
        ));
    }
    Ok(file)
}

#[cfg(unix)]
fn set_private_create_mode(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(0o600);
}

#[cfg(not(unix))]
fn set_private_create_mode(_options: &mut OpenOptions) {}

#[cfg(unix)]
fn set_nonblocking_read_flags(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;

    options.custom_flags(nix::libc::O_NONBLOCK | nix::libc::O_CLOEXEC | nix::libc::O_NOFOLLOW);
}

#[cfg(windows)]
fn set_nonblocking_read_flags(options: &mut OpenOptions) {
    use std::os::windows::fs::OpenOptionsExt;

    // Open the reparse point itself so a concurrent path replacement cannot
    // redirect history reads to an unrelated target before the metadata check.
    options.custom_flags(WINDOWS_OPEN_REPARSE_POINT);
}

#[cfg(not(any(unix, windows)))]
fn set_nonblocking_read_flags(_options: &mut OpenOptions) {}

#[cfg(unix)]
fn set_internal_lock_flags(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;

    options.custom_flags(nix::libc::O_NONBLOCK | nix::libc::O_CLOEXEC | nix::libc::O_NOFOLLOW);
}

#[cfg(windows)]
fn set_internal_lock_flags(options: &mut OpenOptions) {
    use std::os::windows::fs::OpenOptionsExt;

    options.custom_flags(WINDOWS_OPEN_REPARSE_POINT);
}

#[cfg(not(any(unix, windows)))]
fn set_internal_lock_flags(_options: &mut OpenOptions) {}

#[cfg(windows)]
const WINDOWS_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

#[cfg(windows)]
fn metadata_is_regular(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    metadata.is_file() && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
}

#[cfg(not(windows))]
fn metadata_is_regular(metadata: &fs::Metadata) -> bool {
    metadata.is_file()
}

#[cfg(unix)]
fn enforce_private_file_permissions(file: &File) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn enforce_private_file_permissions(_file: &File) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    // Opening directories as files is not portable. The file itself was synced
    // before replacement; platforms without a portable directory fsync stop there.
    Ok(())
}

#[cfg(windows)]
fn sync_preserved_file(_file: &File) -> io::Result<()> {
    // Windows rejects FlushFileBuffers for the read-only verification handle.
    // The hard link references already-durable contents; no new bytes were written.
    Ok(())
}

#[cfg(not(windows))]
fn sync_preserved_file(file: &File) -> io::Result<()> {
    file.sync_all()
}

struct RemoveOnDrop {
    path: PathBuf,
    remove: bool,
}

impl RemoveOnDrop {
    fn new(path: PathBuf) -> Self {
        Self { path, remove: true }
    }

    fn keep(&mut self) {
        self.remove = false;
    }
}

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        if self.remove {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};
    use std::sync::{Arc, Barrier};
    use std::thread;

    #[test]
    fn favorite_and_use_are_committed_in_one_delta() {
        let temp = tempfile::tempdir().unwrap();
        let store = PreferenceStore::new(temp.path().join("history.json"));
        let mut delta = PreferenceDelta::new();
        delta.set_favorite("production", true);
        delta.record_selection("production", 1_700_000_000_123);

        let committed = store.commit(&delta).unwrap();
        let entry = committed.get("production").unwrap();
        assert!(entry.is_favorite());
        assert_eq!(entry.use_count(), 1);
        assert_eq!(entry.last_used_sequence(), 1);
        assert_eq!(entry.last_used_unix_ms(), Some(1_700_000_000_123));
        assert_eq!(store.load().unwrap(), committed);
    }

    #[test]
    fn concurrent_thread_commits_do_not_lose_updates() {
        const THREADS: usize = 24;
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(PreferenceStore::new(temp.path().join("history.json")));
        let barrier = Arc::new(Barrier::new(THREADS));
        let mut workers = Vec::new();

        for index in 0..THREADS {
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            workers.push(thread::spawn(move || {
                let mut delta = PreferenceDelta::new();
                delta.record_selection("shared", index as i64);
                barrier.wait();
                store.commit(&delta).unwrap();
            }));
        }
        for worker in workers {
            worker.join().unwrap();
        }

        let history = store.load().unwrap();
        assert_eq!(history.get("shared").unwrap().use_count(), THREADS as u64);
    }

    #[test]
    fn a_contended_history_lock_is_bounded() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("history.json");
        let store = PreferenceStore::new(&path);
        fs::create_dir_all(temp.path()).unwrap();
        let lock_path = sibling_path(&path, ".lock");
        let held = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)
            .unwrap();
        held.lock().unwrap();

        let started = Instant::now();
        let error = store.load().unwrap_err();
        assert!(matches!(error, PreferenceError::Lock { .. }));
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn corrupt_copy_fallback_never_exceeds_the_history_input_budget() {
        let source = vec![b'x'; usize::try_from(MAX_HISTORY_FILE_BYTES).unwrap() + 1];
        let mut reader = io::Cursor::new(source);
        let mut destination = Vec::new();

        let error = copy_bounded_corrupt(&mut reader, &mut destination).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            destination.len(),
            usize::try_from(MAX_HISTORY_FILE_BYTES).unwrap() + 1
        );
    }

    #[test]
    fn corrupt_copy_fallback_preserves_an_input_at_the_budget() {
        let source = vec![b'x'; usize::try_from(MAX_HISTORY_FILE_BYTES).unwrap()];
        let mut reader = io::Cursor::new(&source);
        let mut destination = Vec::new();

        copy_bounded_corrupt(&mut reader, &mut destination).unwrap();

        assert_eq!(destination, source);
    }

    #[test]
    fn concurrent_commits_from_different_catalogs_preserve_both_sets() {
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(PreferenceStore::new(temp.path().join("history.json")));
        let barrier = Arc::new(Barrier::new(2));
        let mut workers = Vec::new();

        for (profile_name, at_unix_ms) in [("work", 1), ("personal", 2)] {
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            workers.push(thread::spawn(move || {
                let mut delta = PreferenceDelta::new();
                delta.record_selection(profile_name, at_unix_ms);
                barrier.wait();
                store.commit(&delta).unwrap();
            }));
        }
        for worker in workers {
            worker.join().unwrap();
        }

        let history = store.load().unwrap();
        assert_eq!(history.get("work").unwrap().use_count(), 1);
        assert_eq!(history.get("personal").unwrap().use_count(), 1);
    }

    const PROCESS_WORKER_PATH: &str = "AWSWIT_HISTORY_TEST_PATH";

    #[test]
    fn process_commit_worker() {
        let Some(path) = std::env::var_os(PROCESS_WORKER_PATH) else {
            return;
        };
        let store = PreferenceStore::new(path);
        let mut delta = PreferenceDelta::new();
        delta.record_selection("shared-process", 42);
        store.commit(&delta).unwrap();
    }

    #[test]
    fn concurrent_process_commits_do_not_lose_updates() {
        const PROCESSES: usize = 8;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("history.json");
        let test_binary = std::env::current_exe().unwrap();
        let mut children = Vec::new();

        for _ in 0..PROCESSES {
            children.push(
                Command::new(&test_binary)
                    .arg("process_commit_worker")
                    .env(PROCESS_WORKER_PATH, &path)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap(),
            );
        }

        for child in children {
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "worker failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let history = PreferenceStore::new(path).load().unwrap();
        assert_eq!(
            history.get("shared-process").unwrap().use_count(),
            PROCESSES as u64
        );
    }

    #[test]
    fn corrupt_history_is_preserved_under_unique_names() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("history.json");
        let store = PreferenceStore::new(&path);

        fs::write(&path, b"{not-json: SECRET_SENTINEL").unwrap();
        assert!(store.load().unwrap().is_empty());
        assert!(!path.exists());

        fs::write(&path, b"another invalid history").unwrap();
        assert!(store.load().unwrap().is_empty());
        assert!(!path.exists());

        let mut backups: Vec<_> = fs::read_dir(temp.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.to_string_lossy().contains(".corrupt-"))
            .collect();
        backups.sort();
        assert_eq!(backups.len(), 2);
        assert_ne!(backups[0], backups[1]);
        assert!(
            backups
                .iter()
                .any(|backup| { fs::read(backup).unwrap() == b"{not-json: SECRET_SENTINEL" })
        );
    }

    #[cfg(unix)]
    #[test]
    fn history_lock_and_corrupt_backup_are_private() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("history.json");
        let store = PreferenceStore::new(&path);
        let mut delta = PreferenceDelta::new();
        delta.record_selection("dev", 1);
        store.commit(&delta).unwrap();

        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(sibling_path(&path, ".lock"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );

        fs::write(&path, b"invalid").unwrap();
        store.load().unwrap();
        let backup = fs::read_dir(temp.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.to_string_lossy().contains(".corrupt-"))
            .unwrap();
        assert_eq!(
            fs::metadata(backup).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[cfg(unix)]
    #[test]
    fn history_symlink_is_rejected_without_touching_its_target() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("external.json");
        let path = temp.path().join("history.json");
        fs::write(&target, b"external sentinel").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).unwrap();
        symlink(&target, &path).unwrap();

        let error = PreferenceStore::new(&path).load().unwrap_err();
        assert!(matches!(error, PreferenceError::Read { .. }));
        assert_eq!(fs::read(&target).unwrap(), b"external sentinel");
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o644
        );
        assert!(
            fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[cfg(windows)]
    #[test]
    fn history_and_lock_reparse_points_never_redirect_storage() {
        use std::os::windows::fs::symlink_file;

        let temp = tempfile::tempdir().unwrap();
        let history = temp.path().join("history.json");
        let history_target = temp.path().join("history-target.json");
        let contents = br#"{"schema_version":1,"next_sequence":0,"entries":{}}"#;
        fs::write(&history_target, contents).unwrap();
        symlink_file(&history_target, &history).unwrap();

        let error = PreferenceStore::new(&history).load().unwrap_err();
        assert!(matches!(error, PreferenceError::Read { .. }));
        assert_eq!(fs::read(&history_target).unwrap(), contents);

        fs::remove_file(&history).unwrap();
        fs::remove_file(sibling_path(&history, ".lock")).unwrap();
        let lock_target = temp.path().join("lock-target");
        fs::write(&lock_target, b"LOCK-TARGET").unwrap();
        symlink_file(&lock_target, sibling_path(&history, ".lock")).unwrap();

        let error = PreferenceStore::new(&history).load().unwrap_err();
        assert!(matches!(error, PreferenceError::Prepare { .. }));
        assert_eq!(fs::read(&lock_target).unwrap(), b"LOCK-TARGET");
    }

    #[test]
    fn prune_is_bounded_and_deterministic() {
        let temp = tempfile::tempdir().unwrap();
        let store = PreferenceStore::new(temp.path().join("history.json"));
        let names: BTreeSet<_> = (0..(MAX_HISTORY_ENTRIES + 17))
            .map(|index| format!("profile-{index:04}"))
            .collect();
        let mut delta = PreferenceDelta::new();
        for name in &names {
            delta.set_favorite(name, true);
        }

        let history = store.commit(&delta).unwrap();
        assert_eq!(history.len(), MAX_HISTORY_ENTRIES);
        assert!(history.get("profile-0000").is_some());
        assert!(history.get("profile-0511").is_some());
        assert!(history.get("profile-0512").is_none());
    }

    #[test]
    fn different_catalogs_preserve_favorites_and_mru_entries() {
        let temp = tempfile::tempdir().unwrap();
        let store = PreferenceStore::new(temp.path().join("history.json"));
        let mut work = PreferenceDelta::new();
        work.set_favorite("work-favorite", true);
        work.record_selection("work-recent", 1);
        store.commit(&work).unwrap();

        let mut personal = PreferenceDelta::new();
        personal.set_favorite("personal-favorite", true);
        personal.record_selection("personal-recent", 2);
        let history = store.commit(&personal).unwrap();

        assert!(history.is_favorite("work-favorite"));
        assert_eq!(history.get("work-recent").unwrap().use_count(), 1);
        assert!(history.is_favorite("personal-favorite"));
        assert_eq!(history.get("personal-recent").unwrap().use_count(), 1);
    }

    #[test]
    fn entries_without_a_favorite_or_use_are_pruned() {
        let temp = tempfile::tempdir().unwrap();
        let store = PreferenceStore::new(temp.path().join("history.json"));
        let mut create = PreferenceDelta::new();
        create.set_favorite("unseen-favorite", true);
        create.set_favorite("empty", true);
        store.commit(&create).unwrap();

        let mut update = PreferenceDelta::new();
        update.set_favorite("empty", false);
        let history = store.commit(&update).unwrap();
        assert!(history.is_favorite("unseen-favorite"));
        assert!(history.get("empty").is_none());
    }

    #[test]
    fn ranking_is_favorite_then_mru_then_count_then_name() {
        let temp = tempfile::tempdir().unwrap();
        let store = PreferenceStore::new(temp.path().join("history.json"));
        let mut first = PreferenceDelta::new();
        first.record_selection("alpha", 9_999_999);
        store.commit(&first).unwrap();
        let mut second = PreferenceDelta::new();
        second.record_selection("beta", -9_999_999);
        store.commit(&second).unwrap();
        let mut favorite = PreferenceDelta::new();
        favorite.set_favorite("favorite", true);
        let history = store.commit(&favorite).unwrap();

        let mut ranked = vec!["alpha", "favorite", "beta", "unknown"];
        ranked.sort_by(|a, b| history.compare_rank(a, b));
        assert_eq!(ranked, ["favorite", "beta", "alpha", "unknown"]);
    }

    #[test]
    fn exhausted_sequence_is_renumbered_without_panicking() {
        let mut history = PreferenceHistory {
            entries: BTreeMap::from([(
                "older".to_owned(),
                PreferenceEntry {
                    favorite: false,
                    use_count: 1,
                    last_used_sequence: u64::MAX,
                    last_used_unix_ms: Some(1),
                },
            )]),
            next_sequence: u64::MAX,
        };
        let mut delta = PreferenceDelta::new();
        delta.record_selection("newer", 2);

        history.apply(&delta);

        assert_eq!(history.get("older").unwrap().last_used_sequence(), 1);
        assert_eq!(history.get("newer").unwrap().last_used_sequence(), 2);
        assert_eq!(history.next_sequence, 2);
    }

    #[test]
    fn unversioned_legacy_history_is_migrated_in_memory() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("history.json");
        fs::write(
            &path,
            br#"{
              "entries": {
                "older": {
                  "name": "older",
                  "last_used": "2024-01-01T00:00:00Z",
                  "use_count": 4,
                  "is_favorite": true
                },
                "newer": {
                  "name": "newer",
                  "last_used": "2025-01-01T00:00:00Z",
                  "use_count": 1,
                  "is_favorite": false
                }
              }
            }"#,
        )
        .unwrap();

        let history = PreferenceStore::new(path).load().unwrap();
        assert_eq!(history.get("older").unwrap().use_count(), 4);
        assert!(history.get("older").unwrap().is_favorite());
        assert_eq!(history.compare_rank("newer", "unknown"), Ordering::Less);
    }

    #[test]
    fn newer_schema_is_not_moved_or_overwritten() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("history.json");
        let bytes = br#"{"schema_version":999,"future":"data"}"#;
        fs::write(&path, bytes).unwrap();

        let error = PreferenceStore::new(&path).load().unwrap_err();
        assert!(matches!(
            error,
            PreferenceError::UnsupportedSchema { found: 999 }
        ));
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
}
