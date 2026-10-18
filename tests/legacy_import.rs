//! Migration from NoteStrike 0.1 notebooks: importing copies notes to plain
//! files and never deletes, moves, or rewrites anything in the notebook.

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use common::{Script, TempDir, write_legacy_note};
use notestrike::legacy::{self, LegacyItem};
use notestrike::workspace::{Prompt, Settings, Workspace};

/// Every file under `root` with its bytes and modified time.
fn fingerprint(root: &Path) -> BTreeMap<PathBuf, (Vec<u8>, SystemTime)> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let modified = fs::metadata(&path).unwrap().modified().unwrap();
                out.insert(path.clone(), (fs::read(&path).unwrap(), modified));
            }
        }
    }
    out
}

fn sample_notebook(root: &Path) {
    write_legacy_note(
        root,
        "valve-guide-1a2b3c4d.md",
        "7f1d2c3b-0000-4000-8000-000000000001",
        "Valve Guide",
        1_700_000_100,
        "# Valve Guide\n\nPressure: 12 bar.\n",
    );
    write_legacy_note(
        root,
        "field-log-5e6f7a8b.md",
        "7f1d2c3b-0000-4000-8000-000000000002",
        "Field Log: day 1/2",
        1_700_000_200,
        "Observed blue valves. [[7f1d2c3b-0000-4000-8000-000000000001|Valve Guide]]",
    );
    // Same title as another note: must not overwrite the first copy.
    write_legacy_note(
        root,
        "valve-guide-99999999.md",
        "7f1d2c3b-0000-4000-8000-000000000003",
        "Valve Guide",
        1_700_000_300,
        "Second note with the same title.",
    );
    fs::write(
        root.join("notes").join("corrupt.md"),
        b"no front matter \xFF\xFE bytes",
    )
    .unwrap();
    // Files the 0.1 core left behind and that are not notes.
    fs::write(root.join("notes").join("x.md.1234.bak"), b"backup bytes").unwrap();
    fs::create_dir_all(root.join("assets")).unwrap();
}

#[test]
fn import_copies_notes_as_plain_files_and_leaves_notebook_untouched() {
    let notebook = TempDir::new("legacy-src");
    let dest = TempDir::new("legacy-dest");
    sample_notebook(notebook.path());
    let before = fingerprint(notebook.path());

    let items = legacy::scan(notebook.path()).unwrap();
    assert_eq!(items.len(), 4);
    assert_eq!(
        items
            .iter()
            .filter(|i| matches!(i, LegacyItem::Unparsed { .. }))
            .count(),
        1
    );

    let report = legacy::import(notebook.path(), dest.path()).unwrap();
    assert_eq!(report.converted.len(), 3);
    assert_eq!(report.copied_verbatim.len(), 1);

    assert_eq!(
        fingerprint(notebook.path()),
        before,
        "notebook bytes and times unchanged"
    );

    let read = |name: &str| fs::read(dest.join(name)).unwrap();
    assert_eq!(
        read("Valve Guide.md"),
        b"# Valve Guide\n\nPressure: 12 bar.\n"
    );
    assert_eq!(
        read("Valve Guide (2).md"),
        b"Second note with the same title."
    );
    assert_eq!(
        read("Field Log_ day 1_2.md"),
        b"Observed blue valves. [[7f1d2c3b-0000-4000-8000-000000000001|Valve Guide]]"
    );
    assert_eq!(read("corrupt.md"), b"no front matter \xFF\xFE bytes");
    let modified = fs::metadata(dest.join("Valve Guide.md"))
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(
        modified
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        1_700_000_100,
        "copy keeps the note's modified time"
    );

    // Importing again never overwrites earlier copies.
    let again = legacy::import(notebook.path(), dest.path()).unwrap();
    assert_eq!(again.total(), 4);
    assert!(dest.join("Valve Guide (3).md").exists());
    assert_eq!(
        read("Valve Guide.md"),
        b"# Valve Guide\n\nPressure: 12 bar.\n"
    );
    assert_eq!(fingerprint(notebook.path()), before);
}

#[test]
fn import_refuses_destination_inside_notes_folder_and_non_notebooks() {
    let notebook = TempDir::new("legacy-inside");
    sample_notebook(notebook.path());
    let before = fingerprint(notebook.path());
    let inside = notebook.path().join("notes").join("export");
    assert!(legacy::import(notebook.path(), &inside).is_err());
    // The refused attempt created only the empty destination folder.
    fs::remove_dir(&inside).unwrap();
    assert_eq!(fingerprint(notebook.path()), before);

    let plain = TempDir::new("legacy-plain");
    assert!(legacy::scan(plain.path()).is_err());
    assert!(!legacy::is_legacy_notebook(plain.path()));
}

#[test]
fn imported_copy_opens_and_saves_without_touching_the_original() {
    let notebook = TempDir::new("legacy-gui-src");
    let dest = TempDir::new("legacy-gui-dest");
    sample_notebook(notebook.path());
    let before = fingerprint(notebook.path());

    let mut ws = Workspace::new(None, Settings::default());
    let mut script = Script::default();
    script.folder.push_back(Some(notebook.path().to_path_buf()));
    script.folder.push_back(Some(dest.path().to_path_buf()));
    ws.import_notebook(&mut script);
    match ws.current_prompt() {
        Some(Prompt::Message { title, body }) => {
            assert_eq!(title, "Notebook imported");
            assert!(body.contains("Copied 4 note(s)"));
            assert!(body.contains("not changed"));
        }
        other => panic!("expected import report, got {other:?}"),
    }
    ws.prompts.clear();

    let copy = dest.join("Valve Guide.md");
    ws.open_paths(std::slice::from_ref(&copy));
    let id = ws.active_tab().unwrap().id;
    ws.active_tab_mut()
        .unwrap()
        .text
        .push_str("Edited in NoteStrike 0.2.\n");
    ws.text_edited(id);
    ws.save_active(false, &mut script);
    assert!(
        fs::read_to_string(&copy)
            .unwrap()
            .ends_with("Edited in NoteStrike 0.2.\n")
    );
    assert_eq!(fingerprint(notebook.path()), before);

    // Opening an original note directly is also possible (it is a plain .md
    // file); merely opening it does not modify it.
    let original = notebook.path().join("notes").join("field-log-5e6f7a8b.md");
    ws.open_paths(std::slice::from_ref(&original));
    assert!(ws.active_tab().unwrap().text.starts_with("---\nid: "));
    assert!(!ws.active_tab().unwrap().is_dirty());
    ws.shutdown();
    assert_eq!(fingerprint(notebook.path()), before);
}

#[test]
fn import_into_a_non_notebook_folder_reports_instead_of_writing() {
    let not_notebook = TempDir::new("legacy-wrong");
    let mut ws = Workspace::new(None, Settings::default());
    let mut script = Script::default();
    script
        .folder
        .push_back(Some(not_notebook.path().to_path_buf()));
    ws.import_notebook(&mut script);
    assert!(
        matches!(ws.current_prompt(), Some(Prompt::Message { title, .. }) if title == "Not a NoteStrike notebook")
    );
    assert!(fs::read_dir(not_notebook.path()).unwrap().next().is_none());
}
