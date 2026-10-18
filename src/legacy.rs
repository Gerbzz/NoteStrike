//! Read-only access to notebooks created by NoteStrike 0.1 (the Markdown
//! notebook prototype), and import of their notes as plain files.
//!
//! A 0.1 notebook is any folder the user chose, containing `notes/*.md` files
//! that start with a small front-matter block (`id`, `title`, `created`,
//! `modified`). NoteStrike 0.1 never used a fixed AppData/Documents location;
//! the notebook folder was always passed on the command line.
//!
//! Nothing in this module writes inside the notebook. Import copies note
//! bodies into a separate destination folder using create-new semantics, so
//! existing files there are never overwritten either.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacyNote {
    pub path: PathBuf,
    pub id: String,
    pub title: String,
    pub body: String,
    pub modified: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LegacyItem {
    Note(LegacyNote),
    /// A `.md` file that could not be parsed as a 0.1 note (kept verbatim).
    Unparsed {
        path: PathBuf,
        reason: String,
    },
}

/// True if `root` looks like a NoteStrike 0.1 notebook (`root/notes` exists).
pub fn is_legacy_notebook(root: &Path) -> bool {
    root.join("notes").is_dir()
}

/// List the notebook's notes without modifying anything.
pub fn scan(root: &Path) -> Result<Vec<LegacyItem>, String> {
    let notes_dir = root.join("notes");
    if !notes_dir.is_dir() {
        return Err(format!(
            "{} is not a NoteStrike 0.1 notebook (no notes folder)",
            root.display()
        ));
    }
    let mut paths: Vec<PathBuf> = fs::read_dir(&notes_dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .collect();
    paths.sort();
    let mut items = Vec::new();
    for path in paths {
        let meta = match fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(error) => {
                items.push(LegacyItem::Unparsed {
                    path,
                    reason: error.to_string(),
                });
                continue;
            }
        };
        if !meta.is_file() {
            continue;
        }
        match fs::read(&path) {
            Ok(bytes) => match parse(&bytes) {
                Ok((id, title, body, modified)) => items.push(LegacyItem::Note(LegacyNote {
                    path,
                    id,
                    title,
                    body,
                    modified,
                })),
                Err(reason) => items.push(LegacyItem::Unparsed { path, reason }),
            },
            Err(error) => items.push(LegacyItem::Unparsed {
                path,
                reason: error.to_string(),
            }),
        }
    }
    Ok(items)
}

/// Parser for the 0.1 note format (kept from the 0.1 notebook core).
fn parse(bytes: &[u8]) -> Result<(String, String, String, u64), String> {
    let source = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
    let rest = source.strip_prefix("---\n").ok_or("missing front matter")?;
    let (header, body) = rest
        .split_once("\n---\n")
        .ok_or("unterminated front matter")?;
    let mut values = BTreeMap::new();
    for line in header.lines() {
        let (key, value) = line.split_once(": ").ok_or("invalid front matter line")?;
        if values.insert(key, value).is_some() {
            return Err("repeated front matter key".into());
        }
    }
    let id = values.get("id").ok_or("missing id")?.to_string();
    let title: String = serde_json::from_str(values.get("title").ok_or("missing title")?)
        .map_err(|e| e.to_string())?;
    let modified = values
        .get("modified")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    Ok((id, title, body.to_owned(), modified))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// (source note, new plain file) for parsed notes: front matter removed.
    pub converted: Vec<(PathBuf, PathBuf)>,
    /// (source, copy) for files that could not be parsed: bytes copied as-is.
    pub copied_verbatim: Vec<(PathBuf, PathBuf, String)>,
}

impl ImportReport {
    pub fn total(&self) -> usize {
        self.converted.len() + self.copied_verbatim.len()
    }
}

/// Copy every note of the notebook at `root` into `dest` as a plain `.md`
/// file named after its title. The notebook itself is only read.
pub fn import(root: &Path, dest: &Path) -> Result<ImportReport, String> {
    let items = scan(root)?;
    fs::create_dir_all(dest).map_err(|e| format!("create {}: {e}", dest.display()))?;
    let notes_dir = fs::canonicalize(root.join("notes")).map_err(|e| e.to_string())?;
    let dest_real = fs::canonicalize(dest).map_err(|e| e.to_string())?;
    if dest_real.starts_with(&notes_dir) {
        return Err("choose a destination outside the notebook's notes folder".into());
    }
    let mut report = ImportReport::default();
    for item in items {
        match item {
            LegacyItem::Note(note) => {
                let target = write_new(
                    dest,
                    &file_stem_for(&note.title),
                    "md",
                    note.body.as_bytes(),
                )
                .map_err(|e| format!("write copy of {}: {e}", note.path.display()))?;
                if note.modified > 0 {
                    // Keep the note's own modified time on the copy (best effort).
                    if let Ok(file) = OpenOptions::new().write(true).open(&target) {
                        let _ = file.set_modified(UNIX_EPOCH + Duration::from_secs(note.modified));
                    }
                }
                report.converted.push((note.path, target));
            }
            LegacyItem::Unparsed { path, reason } => {
                let bytes = fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
                let stem = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "note".into());
                let target = write_new(dest, &stem, "md", &bytes)
                    .map_err(|e| format!("write copy of {}: {e}", path.display()))?;
                report.copied_verbatim.push((path, target, reason));
            }
        }
    }
    Ok(report)
}

/// A safe file name from a note title (keeps Unicode letters and spaces).
fn file_stem_for(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .take(80)
        .collect();
    let trimmed = cleaned.trim().trim_end_matches('.').trim();
    let reserved = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "LPT1", "LPT2", "LPT3",
    ];
    if trimmed.is_empty() || reserved.contains(&trimmed.to_ascii_uppercase().as_str()) {
        "Untitled note".into()
    } else {
        trimmed.into()
    }
}

/// Create `dest/stem.ext`, or `stem (2).ext`, ... never replacing a file.
fn write_new(dest: &Path, stem: &str, ext: &str, bytes: &[u8]) -> io::Result<PathBuf> {
    for n in 1..10_000 {
        let name = if n == 1 {
            format!("{stem}.{ext}")
        } else {
            format!("{stem} ({n}).{ext}")
        };
        let path = dest.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                file.write_all(bytes)?;
                file.sync_all()?;
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::other("too many files with the same name"))
}
