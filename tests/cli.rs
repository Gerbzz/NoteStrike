//! The `notestrike-cli` helper run as a separate process.

mod common;

use std::fs;
use std::process::Command;

use common::{TempDir, write_legacy_note};

fn cli(args: &[&str]) -> (bool, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_notestrike-cli"))
        .args(args)
        .output()
        .unwrap();
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn inspect_reports_encoding_line_endings_and_damage() {
    let dir = TempDir::new("cli-inspect");
    let utf16 = dir.join("u16.txt");
    let mut bytes = vec![0xFF, 0xFE];
    bytes.extend("a\r\nb\r\n".encode_utf16().flat_map(u16::to_le_bytes));
    fs::write(&utf16, &bytes).unwrap();
    let (ok, out, err) = cli(&["inspect", utf16.to_str().unwrap()]);
    assert!(ok, "{err}");
    assert!(out.contains("encoding\tutf-16le"));
    assert!(out.contains("bom\ttrue"));
    assert!(out.contains("eol\tCRLF"));
    assert!(out.contains("invalid_sequences\t0"));

    let damaged = dir.join("bad.txt");
    fs::write(&damaged, b"ok \xC3\xA9 \xFF\n").unwrap();
    let (ok, out, _) = cli(&["inspect", damaged.to_str().unwrap()]);
    assert!(ok);
    assert!(out.contains("encoding\tutf-8"));
    assert!(out.contains("invalid_sequences\t1"));
    assert!(out.contains("first_invalid_offset\t6"));
    assert!(out.contains("eol\tLF"));

    let (ok, _, err) = cli(&["inspect", dir.join("missing.txt").to_str().unwrap()]);
    assert!(!ok);
    assert!(err.contains("does not exist"));
}

#[test]
fn list_and_import_notebook_without_modifying_it() {
    let notebook = TempDir::new("cli-notebook");
    let dest = TempDir::new("cli-dest");
    let note = write_legacy_note(
        notebook.path(),
        "a-1.md",
        "id-a",
        "Alpha",
        1_700_000_000,
        "alpha body",
    );
    let original = fs::read(&note).unwrap();

    let (ok, out, err) = cli(&["list-notebook", notebook.path().to_str().unwrap()]);
    assert!(ok, "{err}");
    assert!(out.contains("id-a\tAlpha"));

    let (ok, out, err) = cli(&[
        "import-notebook",
        notebook.path().to_str().unwrap(),
        dest.path().to_str().unwrap(),
    ]);
    assert!(ok, "{err}");
    assert!(out.contains("Imported 1 note(s); the notebook was not modified."));
    assert_eq!(fs::read(dest.join("Alpha.md")).unwrap(), b"alpha body");
    assert_eq!(fs::read(&note).unwrap(), original);

    let (ok, _, _) = cli(&["bogus"]);
    assert!(!ok);
}
