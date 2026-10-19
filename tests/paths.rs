//! Per-OS settings and recovery locations, and copying data from the
//! locations earlier builds used. Every platform's layout is checked on every
//! platform; migration runs against temporary folders only.

mod common;

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use common::TempDir;
use notestrike::paths::{AppDirs, LEGACY_IMPORT_MARKER, Platform};
use notestrike::recovery::RecoveryStore;
use notestrike::workspace::{Settings, ThemeChoice};

fn env(pairs: &[(&str, &Path)]) -> impl Fn(&str) -> Option<OsString> {
    let map: HashMap<String, OsString> = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.as_os_str().to_owned()))
        .collect();
    move |name| map.get(name).cloned()
}

fn p(base: &Path, parts: &[&str]) -> PathBuf {
    parts
        .iter()
        .fold(base.to_path_buf(), |path, part| path.join(part))
}

#[test]
fn windows_settings_roam_and_the_journal_stays_local() {
    let local = Path::new("L");
    let roaming = Path::new("R");
    let dirs = AppDirs::resolve(
        Platform::Windows,
        env(&[("LOCALAPPDATA", local), ("APPDATA", roaming)]),
    )
    .unwrap();
    assert_eq!(
        dirs.settings_file,
        p(roaming, &["PWDS", "NoteStrike", "settings.json"])
    );
    // Unchanged from earlier builds, so an unrecovered journal is still found.
    assert_eq!(
        dirs.recovery_dir,
        p(local, &["PWDS", "NoteStrike", "recovery"])
    );
    assert_eq!(
        dirs.legacy_settings_file,
        Some(p(local, &["PWDS", "NoteStrike", "settings.json"]))
    );
    assert_eq!(dirs.legacy_recovery_dir, None);
}

#[test]
fn macos_uses_application_support() {
    let home = Path::new("H");
    let dirs = AppDirs::resolve(Platform::MacOs, env(&[("HOME", home)])).unwrap();
    let support = p(home, &["Library", "Application Support", "NoteStrike"]);
    assert_eq!(dirs.settings_file, support.join("settings.json"));
    assert_eq!(dirs.recovery_dir, support.join("recovery"));
    let legacy = p(home, &[".local", "share", "notestrike"]);
    assert_eq!(
        dirs.legacy_settings_file,
        Some(legacy.join("settings.json"))
    );
    assert_eq!(dirs.legacy_recovery_dir, Some(legacy.join("recovery")));
}

#[test]
fn linux_follows_the_xdg_base_directories() {
    let home = Path::new("H");
    let dirs = AppDirs::resolve(Platform::Unix, env(&[("HOME", home)])).unwrap();
    assert_eq!(
        dirs.settings_file,
        p(home, &[".config", "notestrike", "settings.json"])
    );
    assert_eq!(
        dirs.recovery_dir,
        p(home, &[".local", "share", "notestrike", "recovery"])
    );

    let (config, data) = (Path::new("C"), Path::new("D"));
    let dirs = AppDirs::resolve(
        Platform::Unix,
        env(&[
            ("HOME", home),
            ("XDG_CONFIG_HOME", config),
            ("XDG_DATA_HOME", data),
        ]),
    )
    .unwrap();
    assert_eq!(
        dirs.settings_file,
        p(config, &["notestrike", "settings.json"])
    );
    assert_eq!(dirs.recovery_dir, p(data, &["notestrike", "recovery"]));
    assert_eq!(
        dirs.legacy_settings_file,
        Some(p(data, &["notestrike", "settings.json"]))
    );
}

#[test]
fn override_and_missing_home() {
    let portable = Path::new("P");
    for platform in [Platform::Windows, Platform::MacOs, Platform::Unix] {
        let dirs = AppDirs::resolve(platform, env(&[("NOTESTRIKE_DATA_DIR", portable)])).unwrap();
        assert_eq!(dirs.settings_file, portable.join("settings.json"));
        assert_eq!(dirs.recovery_dir, portable.join("recovery"));
        assert_eq!(dirs.legacy_settings_file, None);
        assert_eq!(AppDirs::resolve(platform, env(&[])), None);
    }
    // An empty variable counts as unset.
    assert_eq!(
        AppDirs::resolve(Platform::Unix, env(&[("HOME", Path::new(""))])),
        None
    );
}

#[test]
fn migration_copies_settings_once_and_never_deletes() {
    let root = TempDir::new("migrate-settings");
    let local = root.join("Local");
    let roaming = root.join("Roaming");
    let dirs = AppDirs::resolve(
        Platform::Windows,
        env(&[("LOCALAPPDATA", &local), ("APPDATA", &roaming)]),
    )
    .unwrap();
    let legacy = dirs.legacy_settings_file.clone().unwrap();
    fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    let old = Settings {
        theme: ThemeChoice::Dark,
        zoom_percent: 150,
        ..Settings::default()
    };
    old.save(&legacy).unwrap();
    let legacy_bytes = fs::read(&legacy).unwrap();

    dirs.migrate().unwrap();
    assert_eq!(Settings::load(&dirs.settings_file), old);
    assert_eq!(
        fs::read(&legacy).unwrap(),
        legacy_bytes,
        "old file untouched"
    );

    // Settings changed after the migration are not overwritten again.
    let newer = Settings {
        theme: ThemeChoice::Light,
        ..old.clone()
    };
    newer.save(&dirs.settings_file).unwrap();
    dirs.migrate().unwrap();
    assert_eq!(Settings::load(&dirs.settings_file), newer);
    assert!(legacy.exists());
}

#[test]
fn migration_brings_an_unrecovered_journal_along_once() {
    let root = TempDir::new("migrate-journal");
    let home = root.path();
    let dirs = AppDirs::resolve(Platform::MacOs, env(&[("HOME", home)])).unwrap();
    let legacy = dirs.legacy_recovery_dir.clone().unwrap();

    // A session of an earlier build that crashed: a journal entry and a lock
    // file that nobody holds any more.
    fs::create_dir_all(&legacy).unwrap();
    fs::write(legacy.join("deadbeef__1.json"), b"{\"crashed\":true}").unwrap();
    fs::write(legacy.join("deadbeef.lock"), b"").unwrap();
    // A session of an earlier build that is still running is left alone.
    let live = RecoveryStore::open(&legacy).unwrap();
    let live_session = fs::read_dir(&legacy)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .find(|n| n.ends_with(".lock") && n != "deadbeef.lock")
        .unwrap()
        .trim_end_matches(".lock")
        .to_owned();
    fs::write(legacy.join(format!("{live_session}__1.json")), b"{}").unwrap();

    dirs.migrate().unwrap();
    let copied = dirs.recovery_dir.join("deadbeef__1.json");
    assert_eq!(fs::read(&copied).unwrap(), b"{\"crashed\":true}");
    assert!(!dirs.recovery_dir.join("deadbeef.lock").exists());
    assert!(
        !dirs
            .recovery_dir
            .join(format!("{live_session}__1.json"))
            .exists()
    );
    assert!(dirs.recovery_dir.join(LEGACY_IMPORT_MARKER).exists());
    assert!(legacy.join("deadbeef__1.json").exists(), "original kept");

    // Once recovered (deleted from the new folder) it is not copied again.
    fs::remove_file(&copied).unwrap();
    dirs.migrate().unwrap();
    assert!(!copied.exists());
    drop(live);
}
