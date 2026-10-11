use std::fs;

use notestrike::Notebook;

#[test]
fn linked_note_edits_search_and_reopen() {
    let root = std::env::temp_dir().join(format!("notestrike-roundtrip-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let mut notebook = Notebook::create(&root).unwrap();
    let guide = notebook.add("Valve Guide", "Pressure: 12 bar.").unwrap();
    let field = notebook
        .add("Field Log", "# Field Log\nObserved blue valves.")
        .unwrap();
    notebook
        .update(
            field.id,
            &format!(
                "# Field Log\nObserved blue valves.\n[[{}|Valve Guide]]",
                guide.id
            ),
        )
        .unwrap();
    assert_eq!(notebook.search("blue").len(), 1);
    assert_eq!(notebook.backlinks(guide.id), vec![field.id]);
    notebook.rename(guide.id, "Valve Handbook").unwrap();
    drop(notebook);

    let reopened = Notebook::open(&root).unwrap();
    assert_eq!(reopened.search("blue")[0].id, field.id);
    assert_eq!(reopened.backlinks(guide.id), vec![field.id]);
    assert_eq!(reopened.get(guide.id).unwrap().title, "Valve Handbook");
    assert!(
        reopened
            .get(field.id)
            .unwrap()
            .body
            .contains(&guide.id.to_string())
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn external_change_blocks_dirty_note_save() {
    let root = std::env::temp_dir().join(format!("notestrike-conflict-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let mut notebook = Notebook::create(&root).unwrap();
    let note = notebook.add("Field Log", "original").unwrap();
    let path = notebook.note_path(note.id).unwrap();
    fs::write(&path, "external edit").unwrap();
    assert!(
        notebook
            .update(note.id, "new local edit")
            .unwrap_err()
            .contains("changed on disk")
    );
    assert_eq!(fs::read_to_string(path).unwrap(), "external edit");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn corrupt_note_is_reported_while_valid_notes_remain_searchable_and_repair_is_isolated() {
    use uuid::Uuid;

    let root = std::env::temp_dir().join(format!("notestrike-repair-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let mut notebook = Notebook::create(&root).unwrap();
    let valid = notebook
        .add("Field Log", "Blue valves stayed open.")
        .unwrap();
    let bad = root.join("notes").join("corrupt.md");
    let original = b"Missing front matter; preserve every byte.";
    fs::write(&bad, original).unwrap();
    let mut reopened = Notebook::open(&root).unwrap();
    assert_eq!(reopened.list().len(), 1);
    assert_eq!(reopened.search("Blue")[0].id, valid.id);
    assert_eq!(reopened.warnings().len(), 1);
    assert_eq!(reopened.warnings()[0].path, bad);
    assert!(reopened.warnings()[0].reason.contains("front matter"));
    assert_eq!(fs::read(&bad).unwrap(), original);

    let replacement = root.join("replacement.md");
    fs::write(&replacement, b"also invalid").unwrap();
    assert!(
        reopened
            .repair_from_file("corrupt.md", &replacement)
            .is_err()
    );
    assert_eq!(fs::read(&bad).unwrap(), original);
    let recovered_id = Uuid::new_v4();
    fs::write(
        &replacement,
        format!("---\nid: {recovered_id}\ntitle: \"Recovered\"\ncreated: 1\nmodified: 1\n---\nRecovered text."),
    )
    .unwrap();
    let backup = reopened
        .repair_from_file("corrupt.md", &replacement)
        .unwrap();
    assert_eq!(fs::read(&backup).unwrap(), original);
    assert!(reopened.warnings().is_empty());
    assert_eq!(
        reopened.get(valid.id).unwrap().body,
        "Blue valves stayed open."
    );
    assert_eq!(reopened.get(recovered_id).unwrap().body, "Recovered text.");
    let again = Notebook::open(&root).unwrap();
    assert_eq!(again.list().len(), 2);
    assert!(again.warnings().is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn persistent_windows_delete_sharing_denial_preserves_original_note() {
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;
    use uuid::Uuid;

    let root = std::env::temp_dir().join(format!("notestrike-locked-{}", Uuid::new_v4()));
    let mut notebook = Notebook::create(&root).unwrap();
    let note = notebook.add("Locked", "Keep original bytes.").unwrap();
    let path = notebook.note_path(note.id).unwrap();
    let before = fs::read(&path).unwrap();
    // Permit reads/writes while withholding FILE_SHARE_DELETE from rename.
    let lock = OpenOptions::new()
        .read(true)
        .share_mode(0x1 | 0x2)
        .open(&path)
        .unwrap();
    let error = notebook.update(note.id, "Replacement").unwrap_err();
    assert!(error.contains("publish staged note"), "{error}");
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(notebook.get(note.id).unwrap().body, "Keep original bytes.");
    let staged: Vec<_> = fs::read_dir(root.join("notes"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "tmp"))
        .collect();
    assert_eq!(
        staged.len(),
        1,
        "failed publish must keep a staged recovery copy"
    );
    assert!(fs::read(&staged[0]).unwrap().ends_with(b"Replacement"));
    drop(lock);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn external_edit_during_windows_retry_is_a_conflict_with_staged_recovery() {
    use std::fs::OpenOptions;
    use std::io::{Seek, SeekFrom, Write};
    use std::os::windows::fs::OpenOptionsExt;
    use std::time::{Duration, Instant};
    use uuid::Uuid;

    let root = std::env::temp_dir().join(format!("notestrike-race-{}", Uuid::new_v4()));
    let mut notebook = Notebook::create(&root).unwrap();
    let note = notebook.add("Original", "body-original").unwrap();
    let path = notebook.note_path(note.id).unwrap();
    let mut held = OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0x1 | 0x2) // Allow reads/writes but deny delete/rename.
        .open(&path)
        .unwrap();
    let writer = std::thread::spawn(move || {
        let result = notebook.rename(note.id, "Renamed");
        (notebook, result)
    });

    let deadline = Instant::now() + Duration::from_secs(3);
    let staged = loop {
        let candidate = fs::read_dir(root.join("notes"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.extension().is_some_and(|ext| ext == "tmp")
                    && fs::read(path).is_ok_and(|bytes| {
                        bytes.ends_with(b"body-original")
                            && bytes.windows(7).any(|window| window == b"Renamed")
                    })
            });
        if let Some(candidate) = candidate {
            break candidate;
        }
        assert!(
            Instant::now() < deadline,
            "save did not reach its staged retry window"
        );
        std::thread::sleep(Duration::from_millis(10));
    };

    let external = String::from_utf8(fs::read(&path).unwrap())
        .unwrap()
        .replace("body-original", "body-external")
        .into_bytes();
    held.seek(SeekFrom::Start(0)).unwrap();
    held.write_all(&external).unwrap();
    held.set_len(external.len() as u64).unwrap();
    held.sync_all().unwrap();
    assert_eq!(fs::read(&path).unwrap(), external);
    drop(held);

    let (notebook, result) = writer.join().unwrap();
    let error = result.expect_err("a changed disk note must not be replaced by the staged rename");
    assert!(error.contains("changed on disk"), "{error}");
    assert!(error.contains("staged"), "{error}");
    assert_eq!(fs::read(&path).unwrap(), external);
    assert_eq!(notebook.get(note.id).unwrap().body, "body-original");
    assert!(fs::read(&staged).unwrap().ends_with(b"body-original"));
    assert!(root.starts_with(std::env::temp_dir()));
    fs::remove_dir_all(root).unwrap();
}
