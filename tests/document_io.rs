//! Atomic saves, interrupted saves, read-only files, external changes,
//! size limits, and repeated save / reopen.

mod common;

use std::fs;
use std::time::{Duration, SystemTime};

use common::{TempDir, set_readonly};
use notestrike::document::{
    self, DiskState, FaultPlan, HARD_SIZE_LIMIT, LoadError, LoadOptions, SOFT_SIZE_LIMIT,
    SaveError, SaveRequest, TEMP_MARKER,
};
use notestrike::encoding::TextEncoding;
use notestrike::eol::LineEnding;

fn request<'a>(
    path: &'a std::path::Path,
    text: &'a str,
    expected: Option<&'a document::DiskSnapshot>,
) -> SaveRequest<'a> {
    SaveRequest {
        path,
        text,
        encoding: TextEncoding::Utf8,
        utf16_bom: false,
        eol: LineEnding::CrLf,
        expected,
        overwrite_external_changes: false,
    }
}

fn temp_files(dir: &std::path::Path) -> Vec<String> {
    fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(TEMP_MARKER))
        .collect()
}

#[test]
fn save_writes_complete_file_and_leaves_no_temp_files() {
    let dir = TempDir::new("atomic");
    let path = dir.join("note.txt");
    let snap = document::save(&request(&path, "first\nsecond", None)).unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"first\r\nsecond");
    assert_eq!(snap.len, 13);
    assert!(temp_files(dir.path()).is_empty());
}

#[test]
fn interrupted_save_keeps_the_original_file_intact() {
    let dir = TempDir::new("interrupted");
    let path = dir.join("important.txt");
    fs::write(&path, b"original contents\r\n").unwrap();
    let loaded = document::load(&path, LoadOptions::default()).unwrap();

    // Interruption after the temp file is written, before rename; cleaned up.
    let err = document::save_with_faults(
        &request(&path, "new contents", Some(&loaded.snapshot)),
        FaultPlan {
            fail_before_rename: true,
            leave_temp_file: false,
        },
    )
    .unwrap_err();
    assert!(matches!(err, SaveError::Io(_)));
    assert_eq!(fs::read(&path).unwrap(), b"original contents\r\n");
    assert!(temp_files(dir.path()).is_empty());

    // Process killed at the same point: a stray temp file remains, but the
    // original is untouched and the next save still succeeds.
    document::save_with_faults(
        &request(&path, "new contents", Some(&loaded.snapshot)),
        FaultPlan {
            fail_before_rename: true,
            leave_temp_file: true,
        },
    )
    .unwrap_err();
    assert_eq!(fs::read(&path).unwrap(), b"original contents\r\n");
    let leftovers = temp_files(dir.path());
    assert_eq!(leftovers.len(), 1);
    let stray = dir.join(&leftovers[0]);
    assert_eq!(
        fs::read(&stray).unwrap(),
        b"new contents",
        "the temp file held the complete new text"
    );

    document::save(&request(&path, "new contents", Some(&loaded.snapshot))).unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"new contents");
}

#[test]
fn read_only_file_is_refused_and_unchanged() {
    let dir = TempDir::new("readonly");
    let path = dir.join("locked.txt");
    fs::write(&path, b"do not change").unwrap();
    set_readonly(&path, true);
    let loaded = document::load(&path, LoadOptions::default()).unwrap();
    assert!(loaded.read_only);
    let err = document::save(&request(&path, "changed", Some(&loaded.snapshot))).unwrap_err();
    assert_eq!(err, SaveError::ReadOnly);
    assert_eq!(fs::read(&path).unwrap(), b"do not change");
    assert!(temp_files(dir.path()).is_empty());
    set_readonly(&path, false);
}

#[test]
fn external_change_blocks_save_until_overwrite_is_chosen() {
    let dir = TempDir::new("external");
    let path = dir.join("shared.txt");
    fs::write(&path, b"v1").unwrap();
    let loaded = document::load(&path, LoadOptions::default()).unwrap();
    fs::write(&path, b"changed by another program").unwrap();

    let (state, _) = document::disk_state(&path, &loaded.snapshot).unwrap();
    assert_eq!(state, DiskState::Changed);
    let err = document::save(&request(&path, "mine", Some(&loaded.snapshot))).unwrap_err();
    assert_eq!(err, SaveError::ChangedOnDisk);
    assert_eq!(fs::read(&path).unwrap(), b"changed by another program");

    let mut overwrite = request(&path, "mine", Some(&loaded.snapshot));
    overwrite.overwrite_external_changes = true;
    document::save(&overwrite).unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"mine");
}

#[test]
fn deleted_file_is_reported_and_recreated_only_on_request() {
    let dir = TempDir::new("deleted");
    let path = dir.join("gone.txt");
    fs::write(&path, b"v1").unwrap();
    let loaded = document::load(&path, LoadOptions::default()).unwrap();
    fs::remove_file(&path).unwrap();
    assert_eq!(
        document::disk_state(&path, &loaded.snapshot).unwrap().0,
        DiskState::Missing
    );
    assert_eq!(
        document::save(&request(&path, "again", Some(&loaded.snapshot))).unwrap_err(),
        SaveError::ChangedOnDisk
    );
    assert!(!path.exists());
    let mut recreate = request(&path, "again", Some(&loaded.snapshot));
    recreate.overwrite_external_changes = true;
    document::save(&recreate).unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"again");
}

#[test]
fn touching_a_file_without_changing_bytes_is_not_a_conflict() {
    let dir = TempDir::new("touch");
    let path = dir.join("t.txt");
    fs::write(&path, b"same").unwrap();
    let loaded = document::load(&path, LoadOptions::default()).unwrap();
    let file = fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.set_modified(SystemTime::now() + Duration::from_secs(120))
        .unwrap();
    drop(file);
    assert_eq!(
        document::disk_state(&path, &loaded.snapshot).unwrap().0,
        DiskState::Unchanged
    );
    document::save(&request(&path, "updated", Some(&loaded.snapshot))).unwrap();
}

#[test]
fn repeated_saves_and_reopen_stay_consistent() {
    let dir = TempDir::new("repeat");
    let path = dir.join("r.txt");
    let mut snapshot = document::save(&request(&path, "0", None)).unwrap();
    for round in 1..=25 {
        let text = format!("round {round}\nline");
        snapshot = document::save(&request(&path, &text, Some(&snapshot))).unwrap();
        let reopened = document::load(&path, LoadOptions::default()).unwrap();
        assert_eq!(reopened.text, text);
        assert_eq!(reopened.snapshot.hash, snapshot.hash);
    }
    assert!(temp_files(dir.path()).is_empty());
}

#[test]
fn size_limits_require_confirmation_or_refuse() {
    let dir = TempDir::new("large");
    let soft = dir.join("soft.log");
    fs::File::create(&soft)
        .unwrap()
        .set_len(SOFT_SIZE_LIMIT + 1)
        .unwrap();
    assert_eq!(
        document::load(&soft, LoadOptions::default()).unwrap_err(),
        LoadError::TooLarge {
            size: SOFT_SIZE_LIMIT + 1,
            hard_limit: false
        }
    );
    let loaded = document::load(
        &soft,
        LoadOptions {
            allow_large: true,
            forced_encoding: None,
        },
    )
    .unwrap();
    assert_eq!(loaded.snapshot.len, SOFT_SIZE_LIMIT + 1);

    let hard = dir.join("hard.log");
    fs::File::create(&hard)
        .unwrap()
        .set_len(HARD_SIZE_LIMIT + 1)
        .unwrap();
    let err = document::load(
        &hard,
        LoadOptions {
            allow_large: true,
            forced_encoding: None,
        },
    )
    .unwrap_err();
    assert!(matches!(
        err,
        LoadError::TooLarge {
            hard_limit: true,
            ..
        }
    ));
}

#[test]
fn missing_and_directory_paths_are_load_errors() {
    let dir = TempDir::new("missing");
    assert_eq!(
        document::load(&dir.join("nope.txt"), LoadOptions::default()).unwrap_err(),
        LoadError::NotFound
    );
    assert_eq!(
        document::load(dir.path(), LoadOptions::default()).unwrap_err(),
        LoadError::NotAFile
    );
}

#[test]
fn save_into_missing_folder_fails_without_side_effects() {
    let dir = TempDir::new("nofolder");
    let path = dir.join("missing-folder").join("x.txt");
    assert!(matches!(
        document::save(&request(&path, "x", None)).unwrap_err(),
        SaveError::Io(_) | SaveError::PermissionDenied(_)
    ));
    assert!(!dir.join("missing-folder").exists());
}
