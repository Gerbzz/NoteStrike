//! Crash/session recovery journal.
//!
//! While the editor runs it keeps one small JSON file per open tab in the
//! recovery folder. Unsaved tabs include their full text; saved tabs only
//! record their path so the session can be reopened. Each running editor
//! holds a session lock file; when the editor exits normally it deletes its
//! journal. Journal files whose lock is no longer held belong to an editor
//! that crashed or was killed, and are offered for recovery on next start.
//!
//! The journal only ever contains NoteStrike's own recovery copies. It never
//! touches the user's documents.

use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use uuid::Uuid;

use crate::document::{self, FaultPlan};
use crate::encoding::TextEncoding;
use crate::eol::LineEnding;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalEntry {
    pub tab_id: u64,
    /// Order of the tab in the window.
    pub position: usize,
    pub title: String,
    pub path: Option<PathBuf>,
    /// Unsaved text (with `\n` endings), or `None` for a saved tab.
    pub text: Option<String>,
    pub encoding: TextEncoding,
    pub utf16_bom: bool,
    pub eol: LineEnding,
    /// Decoder loss must survive a crash so saving cannot silently replace bytes.
    pub invalid_sequences: usize,
    pub first_invalid_offset: Option<usize>,
    /// SHA-256 (hex) of the file on disk the unsaved text was based on.
    pub base_hash: Option<String>,
    pub base_len: Option<u64>,
    pub saved_unix: u64,
}

impl JournalEntry {
    fn to_json(&self) -> Value {
        json!({
            "format": "notestrike-recovery-1",
            "tab_id": self.tab_id,
            "position": self.position,
            "title": self.title,
            "path": self.path.as_ref().map(|p| p.to_string_lossy().into_owned()),
            "text": self.text,
            "encoding": self.encoding.id(),
            "utf16_bom": self.utf16_bom,
            "eol": self.eol.short(),
            "invalid_sequences": self.invalid_sequences,
            "first_invalid_offset": self.first_invalid_offset,
            "base_hash": self.base_hash,
            "base_len": self.base_len,
            "saved_unix": self.saved_unix,
        })
    }

    fn from_json(value: &Value) -> Option<Self> {
        if value.get("format")?.as_str()? != "notestrike-recovery-1" {
            return None;
        }
        // Older journals did not record decoder loss. Be conservative for
        // recovered edits to files: their original bytes may be undecodable.
        let legacy_may_be_lossy = value.get("path").and_then(Value::as_str).is_some()
            && value.get("text").and_then(Value::as_str).is_some();
        Some(Self {
            tab_id: value.get("tab_id")?.as_u64()?,
            position: value.get("position").and_then(Value::as_u64).unwrap_or(0) as usize,
            title: value.get("title")?.as_str()?.to_owned(),
            path: value.get("path").and_then(Value::as_str).map(PathBuf::from),
            text: value.get("text").and_then(Value::as_str).map(str::to_owned),
            encoding: TextEncoding::from_id(value.get("encoding")?.as_str()?)?,
            utf16_bom: value
                .get("utf16_bom")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            eol: LineEnding::from_short(value.get("eol")?.as_str()?)?,
            invalid_sequences: value
                .get("invalid_sequences")
                .and_then(Value::as_u64)
                .map(|count| usize::try_from(count).unwrap_or(usize::MAX))
                .unwrap_or(usize::from(legacy_may_be_lossy)),
            first_invalid_offset: value
                .get("first_invalid_offset")
                .and_then(Value::as_u64)
                .and_then(|offset| usize::try_from(offset).ok()),
            base_hash: value
                .get("base_hash")
                .and_then(Value::as_str)
                .map(str::to_owned),
            base_len: value.get("base_len").and_then(Value::as_u64),
            saved_unix: value.get("saved_unix").and_then(Value::as_u64).unwrap_or(0),
        })
    }
}

/// A journal entry left behind by a session that did not exit normally.
#[derive(Clone, Debug)]
pub struct OrphanEntry {
    pub file: PathBuf,
    pub entry: JournalEntry,
}

pub struct RecoveryStore {
    dir: PathBuf,
    session: String,
    lock_path: PathBuf,
    _lock: fs::File,
}

impl RecoveryStore {
    /// Open (creating) the recovery folder and start a new session.
    pub fn open(dir: &Path) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let session = Uuid::new_v4().simple().to_string();
        let lock_path = dir.join(format!("{session}.lock"));
        let lock = open_exclusive_lock(&lock_path)?;
        Ok(Self {
            dir: dir.to_path_buf(),
            session,
            lock_path,
            _lock: lock,
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn entry_path(&self, tab_id: u64) -> PathBuf {
        self.dir.join(format!("{}__{tab_id}.json", self.session))
    }

    /// Atomically write (or replace) one tab's journal entry.
    pub fn write(&self, entry: &JournalEntry) -> io::Result<()> {
        let bytes = serde_json::to_vec(&entry.to_json()).map_err(io::Error::other)?;
        document::write_atomic(&self.entry_path(entry.tab_id), &bytes, FaultPlan::default())
    }

    pub fn remove(&self, tab_id: u64) {
        let _ = fs::remove_file(self.entry_path(tab_id));
    }

    /// Delete every journal entry of this session (the lock stays held).
    pub fn clear_session(&self) {
        for (path, session, _) in self.journal_files() {
            if session == self.session {
                let _ = fs::remove_file(path);
            }
        }
    }

    /// Normal exit: delete this session's journal, release and remove the lock.
    pub fn shutdown(self) {
        self.clear_session();
        let lock_path = self.lock_path.clone();
        drop(self);
        let _ = fs::remove_file(lock_path);
    }

    /// Journal entries from sessions whose editor is no longer running,
    /// ordered by session and tab position.
    pub fn orphans(&self) -> Vec<OrphanEntry> {
        let mut out = Vec::new();
        for (path, session, _) in self.journal_files() {
            if session == self.session || session_is_alive(&self.dir, &session) {
                continue;
            }
            let entry = fs::read(&path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .and_then(|value| JournalEntry::from_json(&value));
            if let Some(entry) = entry {
                out.push(OrphanEntry { file: path, entry });
            }
        }
        out.sort_by(|a, b| {
            a.entry
                .saved_unix
                .cmp(&b.entry.saved_unix)
                .then(a.entry.position.cmp(&b.entry.position))
        });
        out
    }

    /// Delete orphan journal files after they were restored or explicitly
    /// discarded by the user, plus any stale lock files.
    pub fn delete_orphans(&self, orphans: &[OrphanEntry]) {
        for orphan in orphans {
            let _ = fs::remove_file(&orphan.file);
        }
        if let Ok(entries) = fs::read_dir(&self.dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path == self.lock_path || path.extension().is_none_or(|e| e != "lock") {
                    continue;
                }
                let session = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let has_entries = self.journal_files().iter().any(|(_, s, _)| *s == session);
                if !has_entries && !session_is_alive(&self.dir, &session) {
                    let _ = fs::remove_file(path);
                }
            }
        }
    }

    fn journal_files(&self) -> Vec<(PathBuf, String, u64)> {
        let mut out = Vec::new();
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return out;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let Some(stem) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
                continue;
            };
            let Some((session, tab)) = stem.split_once("__") else {
                continue;
            };
            if let Ok(tab) = tab.parse() {
                out.push((path.clone(), session.to_owned(), tab));
            }
        }
        out
    }
}

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Hold an exclusive OS lock on the session's lock file for the lifetime of
/// the editor. The OS releases it when the process exits for any reason,
/// including a crash or being killed.
fn open_exclusive_lock(path: &Path) -> io::Result<fs::File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    file.try_lock().map_err(|error| match error {
        fs::TryLockError::WouldBlock => io::Error::other("session lock already held"),
        fs::TryLockError::Error(error) => error,
    })?;
    Ok(file)
}

/// A session is alive while some process still holds its lock.
pub(crate) fn session_is_alive(dir: &Path, session: &str) -> bool {
    let lock = dir.join(format!("{session}.lock"));
    let Ok(file) = OpenOptions::new().read(true).write(true).open(&lock) else {
        return false;
    };
    match file.try_lock() {
        Ok(()) => {
            let _ = file.unlock();
            false
        }
        Err(fs::TryLockError::WouldBlock) => true,
        Err(fs::TryLockError::Error(_)) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::JournalEntry;
    use serde_json::json;

    #[test]
    fn old_file_journal_requires_lossy_confirmation_when_damage_is_unknown() {
        let old = json!({
            "format": "notestrike-recovery-1",
            "tab_id": 7,
            "title": "damaged.txt",
            "path": "damaged.txt",
            "text": "edited text",
            "encoding": "utf-8",
            "eol": "LF"
        });
        let entry = JournalEntry::from_json(&old).unwrap();
        assert_eq!(entry.invalid_sequences, 1);
        assert_eq!(entry.first_invalid_offset, None);
        let round_trip = JournalEntry::from_json(&entry.to_json()).unwrap();
        assert_eq!(round_trip.invalid_sequences, 1);
    }
}
