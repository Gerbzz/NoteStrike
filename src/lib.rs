//! Local Markdown notebook storage and linked-note search.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use same_file::Handle;
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[cfg(feature = "native-ui")]
pub mod native_ui;

const MAX_NOTE_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Note {
    pub id: Uuid,
    pub title: String,
    pub body: String,
    pub created: u64,
    pub modified: u64,
}

struct StoredNote {
    note: Note,
    path: PathBuf,
    disk_hash: [u8; 32],
}

#[derive(Clone, Debug)]
pub struct NoteWarning {
    pub path: PathBuf,
    pub reason: String,
    disk_hash: Option<[u8; 32]>,
}

pub struct Notebook {
    root: PathBuf,
    notes: BTreeMap<Uuid, StoredNote>,
    warnings: Vec<NoteWarning>,
}

impl Notebook {
    pub fn create(root: &Path) -> Result<Self, String> {
        fs::create_dir_all(root.join("notes")).map_err(|e| e.to_string())?;
        fs::create_dir_all(root.join("assets")).map_err(|e| e.to_string())?;
        Self::open(root)
    }

    pub fn open(root: &Path) -> Result<Self, String> {
        let directory = root.join("notes");
        if !directory.is_dir() {
            return Err("not a NoteStrike notebook: notes directory is missing".into());
        }
        let mut notes = BTreeMap::new();
        let mut warnings = Vec::new();
        let mut entries = Vec::new();
        for result in fs::read_dir(&directory).map_err(|e| e.to_string())? {
            match result {
                Ok(entry) => entries.push(entry),
                Err(error) => warnings.push(NoteWarning {
                    path: directory.clone(),
                    reason: format!("could not list a note: {error}"),
                    disk_hash: None,
                }),
            }
        }
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "md") {
                continue;
            }
            let kind = match entry.file_type() {
                Ok(kind) => kind,
                Err(error) => {
                    warnings.push(NoteWarning {
                        path,
                        reason: format!("could not inspect note: {error}"),
                        disk_hash: None,
                    });
                    continue;
                }
            };
            if kind.is_symlink() {
                warnings.push(NoteWarning {
                    path,
                    reason: "note symlinks are not supported".into(),
                    disk_hash: None,
                });
                continue;
            }
            let bytes = match fs::read(&path) {
                Ok(bytes) => bytes,
                Err(error) => {
                    warnings.push(NoteWarning {
                        path,
                        reason: error.to_string(),
                        disk_hash: None,
                    });
                    continue;
                }
            };
            let disk_hash = digest(&bytes);
            if bytes.len() > MAX_NOTE_BYTES {
                warnings.push(NoteWarning {
                    path,
                    reason: "note exceeds 2 MiB".into(),
                    disk_hash: Some(disk_hash),
                });
                continue;
            }
            let note = match parse(&bytes) {
                Ok(note) => note,
                Err(reason) => {
                    warnings.push(NoteWarning {
                        path,
                        reason,
                        disk_hash: Some(disk_hash),
                    });
                    continue;
                }
            };
            if notes.contains_key(&note.id) {
                warnings.push(NoteWarning {
                    path,
                    reason: "duplicate note ID in notebook".into(),
                    disk_hash: Some(disk_hash),
                });
                continue;
            }
            notes.insert(
                note.id,
                StoredNote {
                    note,
                    path,
                    disk_hash,
                },
            );
        }
        Ok(Self {
            root: root.to_path_buf(),
            notes,
            warnings,
        })
    }

    pub fn warnings(&self) -> &[NoteWarning] {
        &self.warnings
    }

    /// Replace one explicitly named unreadable note with validated Markdown, keeping
    /// a byte-identical adjacent backup. A changed disk file or duplicate ID is refused.
    pub fn repair_from_file(
        &mut self,
        bad_name: &str,
        replacement_path: &Path,
    ) -> Result<PathBuf, String> {
        let mut components = Path::new(bad_name).components();
        if !matches!(components.next(), Some(Component::Normal(_)))
            || components.next().is_some()
            || !bad_name.ends_with(".md")
        {
            return Err("repair target must be one warned .md filename".into());
        }
        let warning = self
            .warnings
            .iter()
            .find(|warning| {
                warning
                    .path
                    .file_name()
                    .is_some_and(|name| name == std::ffi::OsStr::new(bad_name))
            })
            .cloned()
            .ok_or("no unreadable note has that filename")?;
        let expected_hash = warning
            .disk_hash
            .ok_or("unreadable note cannot be safely repaired by this command")?;
        if replacement_path == warning.path {
            return Err("replacement must be a separate file".into());
        }
        let replacement = fs::read(replacement_path).map_err(|e| e.to_string())?;
        if replacement.len() > MAX_NOTE_BYTES {
            return Err("replacement note exceeds 2 MiB".into());
        }
        let note = parse(&replacement)?;
        if self.notes.contains_key(&note.id) {
            return Err("replacement note ID already belongs to a valid note".into());
        }
        if fs::symlink_metadata(&warning.path)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_symlink()
        {
            return Err("repair target became a symlink".into());
        }
        let identity = Handle::from_path(&warning.path).map_err(|e| e.to_string())?;
        let replacement_identity =
            Handle::from_path(replacement_path).map_err(|e| e.to_string())?;
        if identity == replacement_identity {
            return Err("replacement must be a separate file, not an alias of the target".into());
        }
        let current = fs::read(&warning.path).map_err(|e| e.to_string())?;
        if digest(&current) != expected_hash {
            return Err("unreadable note changed on disk; reopen before repairing".into());
        }
        let backup = warning
            .path
            .with_extension(format!("md.{}.bak", Uuid::new_v4()));
        fs::hard_link(&warning.path, &backup)
            .map_err(|e| format!("could not preserve original note: {e}"))?;
        if digest(&fs::read(&backup).map_err(|e| e.to_string())?) != expected_hash
            || digest(&fs::read(&warning.path).map_err(|e| e.to_string())?) != expected_hash
        {
            return Err(
                "unreadable note changed while preserving its backup; source was not replaced"
                    .into(),
            );
        }
        stage(
            &warning.path,
            &replacement,
            Some((&identity, expected_hash)),
        )?;
        self.notes.insert(
            note.id,
            StoredNote {
                note,
                path: warning.path.clone(),
                disk_hash: digest(&replacement),
            },
        );
        self.warnings.retain(|item| item.path != warning.path);
        Ok(backup)
    }

    pub fn add(&mut self, title: &str, body: &str) -> Result<Note, String> {
        validate(title, body)?;
        let now = timestamp()?;
        let note = Note {
            id: Uuid::new_v4(),
            title: title.into(),
            body: body.into(),
            created: now,
            modified: now,
        };
        let path = self.root.join("notes").join(format!(
            "{}-{}.md",
            slug(title),
            &note.id.simple().to_string()[..8]
        ));
        let bytes = encode(&note)?;
        stage(&path, &bytes, None)?;
        self.notes.insert(
            note.id,
            StoredNote {
                note: note.clone(),
                path,
                disk_hash: digest(&bytes),
            },
        );
        Ok(note)
    }

    pub fn update(&mut self, id: Uuid, body: &str) -> Result<(), String> {
        let old = self.notes.get(&id).ok_or("note does not exist")?;
        validate(&old.note.title, body)?;
        let mut note = old.note.clone();
        note.body = body.into();
        note.modified = timestamp()?;
        self.persist(id, note)
    }

    pub fn rename(&mut self, id: Uuid, title: &str) -> Result<(), String> {
        let old = self.notes.get(&id).ok_or("note does not exist")?;
        validate(title, &old.note.body)?;
        let mut note = old.note.clone();
        note.title = title.into();
        note.modified = timestamp()?;
        self.persist(id, note)
    }

    fn persist(&mut self, id: Uuid, note: Note) -> Result<(), String> {
        let stored = self.notes.get_mut(&id).ok_or("note does not exist")?;
        let identity = Handle::from_path(&stored.path)
            .map_err(|e| format!("inspect note before save ({}): {e}", stored.path.display()))?;
        let current = fs::read(&stored.path)
            .map_err(|e| format!("read note before save ({}): {e}", stored.path.display()))?;
        if digest(&current) != stored.disk_hash {
            return Err("note changed on disk; choose which version to keep".into());
        }
        let bytes = encode(&note)?;
        stage(&stored.path, &bytes, Some((&identity, stored.disk_hash)))?;
        stored.note = note;
        stored.disk_hash = digest(&bytes);
        Ok(())
    }

    pub fn get(&self, id: Uuid) -> Option<&Note> {
        self.notes.get(&id).map(|n| &n.note)
    }

    pub fn note_path(&self, id: Uuid) -> Option<PathBuf> {
        self.notes.get(&id).map(|n| n.path.clone())
    }

    pub fn list(&self) -> Vec<&Note> {
        let mut notes: Vec<_> = self.notes.values().map(|n| &n.note).collect();
        notes.sort_by(|a, b| {
            a.title
                .to_lowercase()
                .cmp(&b.title.to_lowercase())
                .then(a.id.cmp(&b.id))
        });
        notes
    }

    pub fn search(&self, query: &str) -> Vec<&Note> {
        let needle = query.trim().to_lowercase();
        if needle.is_empty() {
            return Vec::new();
        }
        self.list()
            .into_iter()
            .filter(|note| {
                note.title.to_lowercase().contains(&needle)
                    || searchable_body(&note.body).to_lowercase().contains(&needle)
            })
            .collect()
    }

    pub fn backlinks(&self, target: Uuid) -> Vec<Uuid> {
        self.notes
            .values()
            .filter(|item| links(&item.note.body).contains(&target))
            .map(|item| item.note.id)
            .collect()
    }
}

fn validate(title: &str, body: &str) -> Result<(), String> {
    if title.trim().is_empty() || title.len() > 200 || title.contains(['\r', '\n', '\0']) {
        return Err("title must be 1 to 200 bytes on one line".into());
    }
    if body.len() > MAX_NOTE_BYTES || body.contains('\0') {
        return Err("note body exceeds 2 MiB or contains a NUL byte".into());
    }
    Ok(())
}

fn slug(title: &str) -> String {
    let mut out = String::new();
    for c in title.chars() {
        if out.len() >= 40 {
            break;
        }
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "note".into()
    } else {
        trimmed.into()
    }
}

fn timestamp() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|e| e.to_string())
}

fn encode(note: &Note) -> Result<Vec<u8>, String> {
    let title = serde_json::to_string(&note.title).map_err(|e| e.to_string())?;
    Ok(format!(
        "---\nid: {}\ntitle: {title}\ncreated: {}\nmodified: {}\n---\n{}",
        note.id, note.created, note.modified, note.body
    )
    .into_bytes())
}

fn parse(bytes: &[u8]) -> Result<Note, String> {
    let source = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
    let rest = source.strip_prefix("---\n").ok_or("missing front matter")?;
    let (header, body) = rest
        .split_once("\n---\n")
        .ok_or("unterminated front matter")?;
    let mut values = BTreeMap::new();
    for line in header.lines() {
        let (key, value) = line.split_once(": ").ok_or("invalid front matter line")?;
        if !["id", "title", "created", "modified"].contains(&key)
            || values.insert(key, value).is_some()
        {
            return Err("unexpected or repeated front matter key".into());
        }
    }
    let get = |key| values.get(key).copied().ok_or(format!("missing {key}"));
    let id = Uuid::parse_str(get("id")?).map_err(|e| e.to_string())?;
    let title: String = serde_json::from_str(get("title")?).map_err(|e| e.to_string())?;
    let created = get("created")?
        .parse()
        .map_err(|_| "invalid created time")?;
    let modified = get("modified")?
        .parse()
        .map_err(|_| "invalid modified time")?;
    validate(&title, body)?;
    Ok(Note {
        id,
        title,
        body: body.into(),
        created,
        modified,
    })
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn stage(path: &Path, bytes: &[u8], expected: Option<(&Handle, [u8; 32])>) -> Result<(), String> {
    let temp = path.with_extension(format!("md.{}.tmp", Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|e| format!("create staged note ({}): {e}", temp.display()))?;
    if let Err(e) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        let _ = fs::remove_file(&temp);
        return Err(format!("write staged note ({}): {e}", temp.display()));
    }
    drop(file);
    let result = if let Some((identity, hash)) = expected {
        replace_with_bounded_retry(&temp, path, identity, hash)
    } else {
        fs::hard_link(&temp, path).map_err(|e| e.to_string())
    };
    if let Err(e) = result {
        // The complete, synced local edit is the recovery copy when publication
        // fails or another process changes the destination during a retry.
        return Err(format!(
            "publish staged note ({}): {e}; staged copy at {}",
            path.display(),
            temp.display()
        ));
    }
    if expected.is_none() {
        let _ = fs::remove_file(&temp);
    }
    Ok(())
}

fn open_publish_guard(path: &Path) -> io::Result<fs::File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Block in-place writers while checking and replacing the old file,
        // but permit our rename to delete the old directory entry.
        options.share_mode(0x1 | 0x4); // FILE_SHARE_READ | FILE_SHARE_DELETE
    }
    options.open(path)
}

fn transient_windows_sharing_error(error: &io::Error, attempt: usize) -> bool {
    cfg!(windows) && attempt < 40 && matches!(error.raw_os_error(), Some(5 | 32 | 33))
}

fn replace_with_bounded_retry(
    temp: &Path,
    path: &Path,
    expected_identity: &Handle,
    expected_hash: [u8; 32],
) -> Result<(), String> {
    // Each attempt reopens and rehashes the destination. On Windows the read
    // handle excludes writers until the rename completes. A sharing denial is
    // retried; a changed body or file identity is a conflict, never a retry.
    for attempt in 0..=40 {
        let kind = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if !kind.is_file() || kind.file_type().is_symlink() {
            return Err("note changed on disk: destination is no longer a regular file".into());
        }
        let mut guard = match open_publish_guard(path) {
            Ok(guard) => guard,
            Err(error) if transient_windows_sharing_error(&error, attempt) => {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
            Err(error) => return Err(error.to_string()),
        };
        let guard_identity = Handle::from_file(guard.try_clone().map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        if &guard_identity != expected_identity {
            return Err("note changed on disk: destination file was replaced".into());
        }
        let mut hasher = Sha256::new();
        let mut buffer = [0u8; 8192];
        loop {
            let count = guard.read(&mut buffer).map_err(|e| e.to_string())?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
        }
        let current_hash: [u8; 32] = hasher.finalize().into();
        if current_hash != expected_hash {
            return Err("note changed on disk; choose which version to keep".into());
        }
        // A separate process can replace the path while our handle remains
        // open. Check the path identity again immediately before publication.
        let path_identity = Handle::from_path(path).map_err(|e| e.to_string())?;
        if path_identity != guard_identity {
            return Err("note changed on disk: destination path was replaced".into());
        }
        match fs::rename(temp, path) {
            Ok(()) => return Ok(()),
            Err(error) if transient_windows_sharing_error(&error, attempt) => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    unreachable!("the retry loop returns success or the final error")
}

fn searchable_body(body: &str) -> String {
    let mut in_code = false;
    body.lines()
        .filter_map(|line| {
            if line.trim_start().starts_with("```") {
                in_code = !in_code;
                return None;
            }
            (!in_code).then_some(line)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn links(body: &str) -> Vec<Uuid> {
    body.split("[[")
        .skip(1)
        .filter_map(|chunk| {
            let inside = chunk.split_once("]]")?.0;
            let id = inside.split_once('|').map_or(inside, |(id, _)| id);
            Uuid::parse_str(id).ok()
        })
        .collect()
}
