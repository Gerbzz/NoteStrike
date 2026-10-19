//! Per-user locations for `settings.json` and the recovery journal.
//!
//! | OS      | settings.json                                   | recovery journal                              |
//! |---------|-------------------------------------------------|-----------------------------------------------|
//! | Windows | `%APPDATA%\PWDS\NoteStrike`                     | `%LOCALAPPDATA%\PWDS\NoteStrike\recovery`     |
//! | macOS   | `~/Library/Application Support/NoteStrike`      | `~/Library/Application Support/NoteStrike/recovery` |
//! | Linux   | `$XDG_CONFIG_HOME/notestrike` (`~/.config/...`) | `$XDG_DATA_HOME/notestrike/recovery` (`~/.local/share/...`) |
//!
//! `NOTESTRIKE_DATA_DIR` overrides both (tests and portable setups): settings
//! live in that folder and the journal in its `recovery` subfolder.
//!
//! Earlier builds kept everything in one data folder (`%LOCALAPPDATA%\PWDS\
//! NoteStrike` on Windows, `~/.local/share/notestrike` everywhere else).
//! [`AppDirs::migrate`] copies what it finds there into the new locations the
//! first time they are needed. It never deletes or changes the old files.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::document::{self, FaultPlan};
use crate::recovery;

/// Which platform's conventions to apply. Normally [`Platform::current`];
/// tests pick one explicitly so every OS layout is checked on every OS.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Windows,
    MacOs,
    /// Linux and the BSDs (XDG base directories).
    Unix,
}

impl Platform {
    pub fn current() -> Self {
        if cfg!(windows) {
            Platform::Windows
        } else if cfg!(target_os = "macos") {
            Platform::MacOs
        } else {
            Platform::Unix
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppDirs {
    pub settings_file: PathBuf,
    pub recovery_dir: PathBuf,
    /// Where earlier builds kept `settings.json`, when that differs.
    pub legacy_settings_file: Option<PathBuf>,
    /// Where earlier builds kept the recovery journal, when that differs.
    pub legacy_recovery_dir: Option<PathBuf>,
}

/// Marker written into the recovery folder after a legacy journal was copied,
/// so it is copied only once (the old files are left where they were).
pub const LEGACY_IMPORT_MARKER: &str = "imported-legacy-journal.txt";

impl AppDirs {
    /// Locations for this user on this OS, or `None` when the OS reports no
    /// home/profile folder (NoteStrike then runs without settings or recovery).
    pub fn current() -> Option<Self> {
        Self::resolve(Platform::current(), |name| std::env::var_os(name))
    }

    pub fn resolve(platform: Platform, env: impl Fn(&str) -> Option<OsString>) -> Option<Self> {
        let var = |name: &str| {
            env(name)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        };
        if let Some(dir) = var("NOTESTRIKE_DATA_DIR") {
            return Some(Self {
                settings_file: dir.join("settings.json"),
                recovery_dir: dir.join("recovery"),
                legacy_settings_file: None,
                legacy_recovery_dir: None,
            });
        }
        let dirs = match platform {
            Platform::Windows => {
                let local = var("LOCALAPPDATA")?.join("PWDS").join("NoteStrike");
                // Settings roam with the profile; the journal stays local.
                let settings_dir = var("APPDATA")
                    .map(|roaming| roaming.join("PWDS").join("NoteStrike"))
                    .unwrap_or_else(|| local.clone());
                Self {
                    settings_file: settings_dir.join("settings.json"),
                    recovery_dir: local.join("recovery"),
                    legacy_settings_file: Some(local.join("settings.json")),
                    legacy_recovery_dir: None,
                }
            }
            Platform::MacOs => {
                let home = var("HOME")?;
                let support = home
                    .join("Library")
                    .join("Application Support")
                    .join("NoteStrike");
                let legacy = xdg_data(&var, &home);
                Self {
                    settings_file: support.join("settings.json"),
                    recovery_dir: support.join("recovery"),
                    legacy_settings_file: Some(legacy.join("settings.json")),
                    legacy_recovery_dir: Some(legacy.join("recovery")),
                }
            }
            Platform::Unix => {
                let home = var("HOME");
                let config = match var("XDG_CONFIG_HOME") {
                    Some(base) => base.join("notestrike"),
                    None => home.clone()?.join(".config").join("notestrike"),
                };
                let data = match var("XDG_DATA_HOME") {
                    Some(base) => base.join("notestrike"),
                    None => home?.join(".local").join("share").join("notestrike"),
                };
                Self {
                    settings_file: config.join("settings.json"),
                    recovery_dir: data.join("recovery"),
                    legacy_settings_file: Some(data.join("settings.json")),
                    legacy_recovery_dir: None,
                }
            }
        };
        Some(dirs.without_identical_legacy())
    }

    fn without_identical_legacy(mut self) -> Self {
        if self.legacy_settings_file.as_ref() == Some(&self.settings_file) {
            self.legacy_settings_file = None;
        }
        if self.legacy_recovery_dir.as_ref() == Some(&self.recovery_dir) {
            self.legacy_recovery_dir = None;
        }
        self
    }

    /// Copy settings and an unrecovered journal from the legacy locations.
    /// Existing files in the new locations always win; nothing is deleted.
    pub fn migrate(&self) -> io::Result<()> {
        if let Some(legacy) = &self.legacy_settings_file
            && !self.settings_file.exists()
            && legacy.is_file()
        {
            let bytes = fs::read(legacy)?;
            if let Some(parent) = self.settings_file.parent() {
                fs::create_dir_all(parent)?;
            }
            document::write_atomic(&self.settings_file, &bytes, FaultPlan::default())?;
        }
        if let Some(legacy) = &self.legacy_recovery_dir {
            copy_legacy_journal(legacy, &self.recovery_dir)?;
        }
        Ok(())
    }
}

fn xdg_data(var: &impl Fn(&str) -> Option<PathBuf>, home: &Path) -> PathBuf {
    match var("XDG_DATA_HOME") {
        Some(base) => base.join("notestrike"),
        None => home.join(".local").join("share").join("notestrike"),
    }
}

/// Copy journal entries (`*.json`) from an old recovery folder, once. Lock
/// files are not copied, so the entries show up as orphans and are offered
/// for recovery on this start, exactly as they would have been before.
fn copy_legacy_journal(from: &Path, to: &Path) -> io::Result<()> {
    let marker = to.join(LEGACY_IMPORT_MARKER);
    if marker.exists() || !from.is_dir() {
        return Ok(());
    }
    let mut copied = 0usize;
    for entry in fs::read_dir(from)?.flatten() {
        let source = entry.path();
        if source.extension().is_none_or(|e| e != "json") || !source.is_file() {
            continue;
        }
        let Some(name) = source.file_name() else {
            continue;
        };
        // Leave entries of an older NoteStrike that is still running alone.
        let session = source
            .file_stem()
            .and_then(|stem| stem.to_str())
            .and_then(|stem| stem.split_once("__"))
            .map(|(session, _)| session.to_owned());
        if session.is_some_and(|session| recovery::session_is_alive(from, &session)) {
            continue;
        }
        fs::create_dir_all(to)?;
        let target = to.join(name);
        if !target.exists() {
            document::write_atomic(&target, &fs::read(&source)?, FaultPlan::default())?;
            copied += 1;
        }
    }
    if copied > 0 {
        let note = format!(
            "Copied {copied} recovery journal file(s) from {}.\nThe originals were left in place.\n",
            from.display()
        );
        document::write_atomic(&marker, note.as_bytes(), FaultPlan::default())?;
    }
    Ok(())
}
