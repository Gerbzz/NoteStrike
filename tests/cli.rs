use std::fs;
use std::process::Command;

#[test]
fn independent_processes_edit_search_and_reopen_notes() {
    let root = std::env::temp_dir().join(format!("notestrike-cli-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let bin = env!("CARGO_BIN_EXE_notestrike");
    let dir = root.to_str().unwrap();
    let run = |args: Vec<&str>| -> String {
        let output = Command::new(bin).args(args).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    };
    run(vec!["init", dir]);
    let guide = run(vec!["new", dir, "Valve Guide", "Pressure: 12 bar."])
        .trim()
        .to_string();
    let field = run(vec!["new", dir, "Field Log", "Observed blue valves."])
        .trim()
        .to_string();
    run(vec![
        "edit",
        dir,
        &field,
        &format!("Observed blue valves. [[{guide}|Valve Guide]]"),
    ]);
    assert!(run(vec!["search", dir, "blue"]).contains(&field));
    assert!(run(vec!["backlinks", dir, &guide]).contains(&field));
    run(vec!["rename", dir, &guide, "Valve Handbook"]);
    assert!(run(vec!["show", dir, &guide]).contains("Valve Handbook"));
    assert!(run(vec!["show", dir, &field]).contains(&guide));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_lists_valid_notes_and_reports_malformed_note_before_explicit_repair() {
    use uuid::Uuid;

    let root = std::env::temp_dir().join(format!("notestrike-cli-repair-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let dir = root.to_str().unwrap();
    let bin = env!("CARGO_BIN_EXE_notestrike");
    assert!(
        Command::new(bin)
            .args(["init", dir])
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(
        Command::new(bin)
            .args(["new", dir, "Healthy", "Search me"])
            .output()
            .unwrap()
            .status
            .success()
    );
    let bad = root.join("notes").join("corrupt.md");
    fs::write(&bad, b"unparseable user bytes").unwrap();
    let list = Command::new(bin).args(["list", dir]).output().unwrap();
    assert!(list.status.success());
    assert!(String::from_utf8_lossy(&list.stdout).contains("Healthy"));
    assert!(String::from_utf8_lossy(&list.stderr).contains("corrupt.md"));
    let warnings = Command::new(bin).args(["warnings", dir]).output().unwrap();
    assert!(String::from_utf8_lossy(&warnings.stdout).contains("missing front matter"));
    let replacement = root.join("replacement.md");
    let id = Uuid::new_v4();
    fs::write(
        &replacement,
        format!("---\nid: {id}\ntitle: \"Recovered\"\ncreated: 1\nmodified: 1\n---\nRestored."),
    )
    .unwrap();
    let repaired = Command::new(bin)
        .args(["repair", dir, "corrupt.md", replacement.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        repaired.status.success(),
        "{}",
        String::from_utf8_lossy(&repaired.stderr)
    );
    assert!(String::from_utf8_lossy(&repaired.stdout).contains("original bytes preserved"));
    assert!(
        String::from_utf8_lossy(
            &Command::new(bin)
                .args(["list", dir])
                .output()
                .unwrap()
                .stdout
        )
        .contains("Recovered")
    );
    fs::remove_dir_all(root).unwrap();
}
