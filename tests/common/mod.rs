#![allow(dead_code)]

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};

use notestrike::workspace::FileDialogs;

/// A fresh, unique temporary directory removed on drop. Tests never touch
/// real user documents.
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "notestrike-test-{label}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        clear_readonly(&self.0);
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn clear_readonly(dir: &Path) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                clear_readonly(&path);
            } else if let Ok(meta) = fs::metadata(&path) {
                let mut perms = meta.permissions();
                if perms.readonly() {
                    #[allow(clippy::permissions_set_readonly_false)]
                    perms.set_readonly(false);
                    let _ = fs::set_permissions(&path, perms);
                }
            }
        }
    }
}

pub fn set_readonly(path: &Path, readonly: bool) {
    let mut perms = fs::metadata(path).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(readonly);
    fs::set_permissions(path, perms).unwrap();
}

/// Scripted file dialogs: each call pops the next queued answer (`None` =
/// the user pressed Cancel). Panics if a dialog opens unexpectedly.
#[derive(Default)]
pub struct Script {
    pub open: VecDeque<Option<Vec<PathBuf>>>,
    pub save: VecDeque<Option<PathBuf>>,
    pub folder: VecDeque<Option<PathBuf>>,
    pub calls: Vec<String>,
}

impl FileDialogs for Script {
    fn open_files(&mut self) -> Option<Vec<PathBuf>> {
        self.calls.push("open".into());
        self.open.pop_front().expect("unexpected Open dialog")
    }

    fn save_file(&mut self, suggested: &str, _dir: Option<&Path>) -> Option<PathBuf> {
        self.calls.push(format!("save:{suggested}"));
        self.save.pop_front().expect("unexpected Save As dialog")
    }

    fn pick_folder(&mut self, title: &str) -> Option<PathBuf> {
        self.calls.push(format!("folder:{title}"));
        self.folder.pop_front().expect("unexpected folder dialog")
    }
}

/// Write a NoteStrike 0.1 notebook note exactly as the 0.1 core encoded it.
pub fn write_legacy_note(
    root: &Path,
    file: &str,
    id: &str,
    title: &str,
    modified: u64,
    body: &str,
) -> PathBuf {
    let notes = root.join("notes");
    fs::create_dir_all(&notes).unwrap();
    let path = notes.join(file);
    let title_json = serde_json::to_string(title).unwrap();
    fs::write(
        &path,
        format!("---\nid: {id}\ntitle: {title_json}\ncreated: 1700000000\nmodified: {modified}\n---\n{body}"),
    )
    .unwrap();
    path
}
