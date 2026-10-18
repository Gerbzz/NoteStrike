//! Editor flows driven through the GUI-free workspace with scripted dialogs:
//! cancellation of every dialog and prompt, unsaved-change protection on tab
//! close and quit, external changes, read-only files, undecodable bytes,
//! repeated save / save as, and reopening.

mod common;

use std::fs;

use common::{Script, TempDir, set_readonly};
use notestrike::document::SOFT_SIZE_LIMIT;
use notestrike::encoding::TextEncoding;
use notestrike::eol::LineEnding;
use notestrike::workspace::{Answer, ExternalState, Prompt, Settings, Workspace};

fn ws() -> Workspace {
    Workspace::new(None, Settings::default())
}

fn type_text(ws: &mut Workspace, text: &str) {
    let id = ws.active_tab().unwrap().id;
    ws.active_tab_mut().unwrap().text.push_str(text);
    ws.text_edited(id);
}

fn prompt_name(ws: &Workspace) -> Option<&'static str> {
    ws.current_prompt().map(|p| match p {
        Prompt::UnsavedTab { .. } => "UnsavedTab",
        Prompt::UnsavedOnQuit { .. } => "UnsavedOnQuit",
        Prompt::ChangedOnDisk { .. } => "ChangedOnDisk",
        Prompt::LossySave { .. } => "LossySave",
        Prompt::Unrepresentable { .. } => "Unrepresentable",
        Prompt::ReadOnly { .. } => "ReadOnly",
        Prompt::LargeFile { .. } => "LargeFile",
        Prompt::ConfirmReload { .. } => "ConfirmReload",
        Prompt::Recovery { .. } => "Recovery",
        Prompt::ConfirmDiscardRecovery { .. } => "ConfirmDiscardRecovery",
        Prompt::GoToLine => "GoToLine",
        Prompt::Message { .. } => "Message",
        Prompt::About => "About",
    })
}

#[test]
fn cancelling_open_and_save_as_dialogs_changes_nothing() {
    let dir = TempDir::new("cancel-dialogs");
    let mut ws = ws();
    let mut script = Script::default();
    script.open.push_back(None);
    ws.open_dialog(&mut script);
    assert_eq!(ws.tabs.len(), 1);
    assert_eq!(ws.status, "Open cancelled.");

    type_text(&mut ws, "draft");
    script.save.push_back(None); // Save on an untitled tab opens Save As.
    ws.save_active(false, &mut script);
    assert!(ws.active_tab().unwrap().is_dirty());
    assert_eq!(ws.status, "Save cancelled.");
    assert!(fs::read_dir(dir.path()).unwrap().next().is_none());

    script.save.push_back(None);
    ws.save_active(true, &mut script);
    assert!(ws.active_tab().unwrap().is_dirty());
    assert_eq!(
        script.calls,
        vec!["open", "save:Untitled 1.txt", "save:Untitled 1.txt"]
    );
}

#[test]
fn closing_a_modified_tab_asks_and_every_answer_behaves() {
    let dir = TempDir::new("close-tab");
    let mut ws = ws();
    let mut script = Script::default();
    type_text(&mut ws, "keep me");
    let first = ws.active_tab().unwrap().id;

    // Cancel keeps the tab.
    ws.request_close_tab(first);
    assert_eq!(prompt_name(&ws), Some("UnsavedTab"));
    ws.answer(Answer::Cancel, &mut script);
    assert_eq!(ws.tabs.len(), 1);
    assert!(ws.tab(first).unwrap().is_dirty());

    // Save -> Save As cancelled keeps the tab too.
    ws.request_close_tab(first);
    script.save.push_back(None);
    ws.answer(Answer::Save, &mut script);
    assert!(ws.tab(first).is_some());

    // Save -> Save As chosen saves and closes.
    let path = dir.join("kept.txt");
    ws.request_close_tab(first);
    script.save.push_back(Some(path.clone()));
    ws.answer(Answer::Save, &mut script);
    assert!(ws.tab(first).is_none());
    assert_eq!(fs::read_to_string(&path).unwrap(), "keep me");
    assert_eq!(ws.tabs.len(), 1, "a fresh empty tab replaces the last one");

    // Don't save discards.
    type_text(&mut ws, "throw away");
    let second = ws.active_tab().unwrap().id;
    ws.request_close_tab(second);
    ws.answer(Answer::DontSave, &mut script);
    assert!(ws.tab(second).is_none());
}

#[test]
fn quit_with_unsaved_tabs_requires_a_decision() {
    let dir = TempDir::new("quit");
    let mut script = Script::default();

    // Clean workspace quits immediately.
    let mut clean = ws();
    clean.request_quit();
    assert!(clean.quit_approved());

    let mut ws = ws();
    type_text(&mut ws, "one");
    ws.new_tab();
    type_text(&mut ws, "two");

    ws.request_quit();
    assert_eq!(prompt_name(&ws), Some("UnsavedOnQuit"));
    ws.answer(Answer::Cancel, &mut script);
    assert!(!ws.quit_approved());
    assert_eq!(ws.tabs.len(), 2);

    // Save all, but cancel the second Save As: quitting stops, nothing lost.
    ws.request_quit();
    script.save.push_back(Some(dir.join("one.txt")));
    script.save.push_back(None);
    ws.answer(Answer::SaveAll, &mut script);
    assert!(!ws.quit_approved());
    assert!(ws.status.starts_with("Exit cancelled"));
    assert_eq!(fs::read_to_string(dir.join("one.txt")).unwrap(), "one");
    assert!(ws.tabs.iter().any(|t| t.is_dirty() && t.text == "two"));

    // Save all again: only the remaining dirty tab is asked about.
    ws.request_quit();
    script.save.push_back(Some(dir.join("two.txt")));
    ws.answer(Answer::SaveAll, &mut script);
    assert!(ws.quit_approved());
    assert_eq!(fs::read_to_string(dir.join("two.txt")).unwrap(), "two");

    // Don't save quits without writing.
    let mut discard = Workspace::new(None, Settings::default());
    type_text(&mut discard, "gone");
    discard.request_quit();
    discard.answer(Answer::DiscardAll, &mut script);
    assert!(discard.quit_approved());
}

#[test]
fn external_change_is_detected_and_save_asks_before_overwriting() {
    let dir = TempDir::new("ws-external");
    let path = dir.join("shared.txt");
    fs::write(&path, "line one\r\n").unwrap();
    let mut ws = ws();
    let mut script = Script::default();
    ws.open_paths(std::slice::from_ref(&path));
    type_text(&mut ws, "my edit");
    fs::write(&path, "another program wrote this, longer").unwrap();

    ws.check_disk();
    assert_eq!(
        ws.active_tab().unwrap().external,
        ExternalState::ChangedOnDisk
    );

    ws.save_active(false, &mut script);
    assert_eq!(prompt_name(&ws), Some("ChangedOnDisk"));
    ws.answer(Answer::Cancel, &mut script);
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "another program wrote this, longer"
    );

    // Save As keeps both versions.
    let copy = dir.join("mine.txt");
    ws.save_active(false, &mut script);
    script.save.push_back(Some(copy.clone()));
    ws.answer(Answer::SaveAs, &mut script);
    assert_eq!(fs::read_to_string(&copy).unwrap(), "line one\r\nmy edit");
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "another program wrote this, longer"
    );
    assert_eq!(
        ws.active_tab().unwrap().path.as_deref(),
        Some(copy.as_path())
    );
}

#[test]
fn external_change_overwrite_and_reload_paths() {
    let dir = TempDir::new("ws-overwrite");
    let path = dir.join("a.txt");
    fs::write(&path, "v1").unwrap();
    let mut ws = ws();
    let mut script = Script::default();
    ws.open_paths(std::slice::from_ref(&path));
    let id = ws.active_tab().unwrap().id;
    type_text(&mut ws, " mine");
    fs::write(&path, "v2 external").unwrap();

    // Reload of a modified tab asks; cancel keeps edits.
    ws.request_reload(id, None);
    assert_eq!(prompt_name(&ws), Some("ConfirmReload"));
    ws.answer(Answer::Cancel, &mut script);
    assert_eq!(ws.tab(id).unwrap().text, "v1 mine");

    ws.save_active(false, &mut script);
    ws.answer(Answer::Overwrite, &mut script);
    assert_eq!(fs::read_to_string(&path).unwrap(), "v1 mine");
    assert!(!ws.tab(id).unwrap().is_dirty());

    fs::write(&path, "v3 external").unwrap();
    ws.check_disk();
    ws.request_reload(id, None);
    assert!(
        ws.current_prompt().is_none(),
        "clean tab reloads without asking"
    );
    assert_eq!(ws.tab(id).unwrap().text, "v3 external");
    assert_eq!(ws.tab(id).unwrap().external, ExternalState::InSync);
}

#[test]
fn read_only_file_offers_save_as_and_cancel_keeps_edits() {
    let dir = TempDir::new("ws-readonly");
    let path = dir.join("ro.txt");
    fs::write(&path, "locked").unwrap();
    set_readonly(&path, true);
    let mut ws = ws();
    let mut script = Script::default();
    ws.open_paths(std::slice::from_ref(&path));
    assert!(ws.active_tab().unwrap().read_only);
    type_text(&mut ws, " edit");

    ws.save_active(false, &mut script);
    assert_eq!(prompt_name(&ws), Some("ReadOnly"));
    ws.answer(Answer::Cancel, &mut script);
    assert!(ws.active_tab().unwrap().is_dirty());

    ws.save_active(false, &mut script);
    script.save.push_back(None);
    ws.answer(Answer::SaveAs, &mut script);
    assert!(ws.active_tab().unwrap().is_dirty());

    ws.save_active(false, &mut script);
    let copy = dir.join("copy.txt");
    script.save.push_back(Some(copy.clone()));
    ws.answer(Answer::SaveAs, &mut script);
    assert_eq!(fs::read_to_string(&copy).unwrap(), "locked edit");
    assert_eq!(fs::read_to_string(&path).unwrap(), "locked");
    set_readonly(&path, false);
}

#[test]
fn undecodable_bytes_require_confirmation_before_being_replaced() {
    let dir = TempDir::new("ws-lossy");
    let path = dir.join("damaged.txt");
    let original = b"caf\xC3\xA9 \xFF end".to_vec();
    fs::write(&path, &original).unwrap();
    let mut ws = ws();
    let mut script = Script::default();
    ws.open_paths(std::slice::from_ref(&path));
    let id = ws.active_tab().unwrap().id;
    assert_eq!(ws.tab(id).unwrap().invalid_sequences, 1);
    type_text(&mut ws, "!");

    ws.save_active(false, &mut script);
    assert_eq!(prompt_name(&ws), Some("LossySave"));
    ws.answer(Answer::Cancel, &mut script);
    assert_eq!(
        fs::read(&path).unwrap(),
        original,
        "cancel leaves bytes intact"
    );

    // Reopen as Windows-1252 (asks because the tab is modified) keeps every byte.
    ws.request_reload(id, Some(TextEncoding::Windows1252));
    ws.answer(Answer::Reload, &mut script);
    let tab = ws.tab(id).unwrap();
    assert_eq!(tab.encoding, TextEncoding::Windows1252);
    assert_eq!(tab.invalid_sequences, 0);
    type_text(&mut ws, "!");
    ws.save_active(false, &mut script);
    assert!(ws.current_prompt().is_none());
    let mut expected = original.clone();
    expected.push(b'!');
    assert_eq!(fs::read(&path).unwrap(), expected);

    // Explicit "save anyway" on a damaged UTF-8 file writes U+FFFD.
    let other = dir.join("damaged2.txt");
    fs::write(&other, &original).unwrap();
    ws.open_paths(std::slice::from_ref(&other));
    ws.save_active(false, &mut script);
    ws.answer(Answer::SaveAnyway, &mut script);
    assert_eq!(fs::read_to_string(&other).unwrap(), "café \u{FFFD} end");
}

#[test]
fn unrepresentable_characters_prompt_and_cancel() {
    let dir = TempDir::new("ws-unrep");
    let path = dir.join("legacy.txt");
    fs::write(&path, b"caf\xE9").unwrap();
    let mut ws = ws();
    let mut script = Script::default();
    ws.open_paths(std::slice::from_ref(&path));
    assert_eq!(ws.active_tab().unwrap().encoding, TextEncoding::Windows1252);
    type_text(&mut ws, " ☃");
    ws.save_active(false, &mut script);
    assert_eq!(prompt_name(&ws), Some("Unrepresentable"));
    ws.answer(Answer::Cancel, &mut script);
    assert_eq!(fs::read(&path).unwrap(), b"caf\xE9");

    ws.save_active(false, &mut script);
    ws.answer(Answer::UseUtf8, &mut script);
    assert_eq!(fs::read_to_string(&path).unwrap(), "café ☃");
    assert_eq!(ws.active_tab().unwrap().encoding, TextEncoding::Utf8);
}

#[test]
fn large_file_prompt_can_be_cancelled_or_accepted() {
    let dir = TempDir::new("ws-large");
    let path = dir.join("big.log");
    fs::File::create(&path)
        .unwrap()
        .set_len(SOFT_SIZE_LIMIT + 10)
        .unwrap();
    let mut ws = ws();
    let mut script = Script::default();
    ws.open_paths(std::slice::from_ref(&path));
    assert_eq!(prompt_name(&ws), Some("LargeFile"));
    ws.answer(Answer::Cancel, &mut script);
    assert!(ws.active_tab().unwrap().path.is_none());
    ws.open_paths(std::slice::from_ref(&path));
    ws.answer(Answer::OpenAnyway, &mut script);
    assert_eq!(
        ws.active_tab().unwrap().path.as_deref(),
        Some(path.as_path())
    );
}

#[test]
fn go_to_line_prompt_validates_and_cancels() {
    let mut ws = ws();
    let mut script = Script::default();
    type_text(&mut ws, "a\nb\nc");
    ws.prompts.push_back(Prompt::GoToLine);
    ws.answer(Answer::GoTo(9), &mut script);
    assert_eq!(
        prompt_name(&ws),
        Some("GoToLine"),
        "out of range keeps the dialog open"
    );
    ws.answer(Answer::GoTo(3), &mut script);
    assert!(ws.current_prompt().is_none());
    assert_eq!(ws.selection, (4, 4));
    ws.prompts.push_back(Prompt::GoToLine);
    ws.answer(Answer::Cancel, &mut script);
    assert!(ws.current_prompt().is_none());
}

#[test]
fn repeated_save_and_save_as_then_reopen() {
    let dir = TempDir::new("ws-repeat");
    let mut ws = ws();
    let mut script = Script::default();
    let first = dir.join("first.txt");
    type_text(&mut ws, "v1");
    script.save.push_back(Some(first.clone()));
    ws.save_active(false, &mut script);
    for n in 2..=10 {
        type_text(&mut ws, &format!("\nv{n}"));
        ws.save_active(false, &mut script);
        assert!(!ws.active_tab().unwrap().is_dirty());
    }
    assert_eq!(
        script.calls.len(),
        1,
        "Save on a named file does not show a dialog"
    );
    let second = dir.join("second.txt");
    script.save.push_back(Some(second.clone()));
    ws.save_active(true, &mut script);
    let third = dir.join("third.txt");
    script.save.push_back(Some(third.clone()));
    ws.save_active(true, &mut script);
    let expected = (1..=10)
        .map(|n| format!("v{n}"))
        .collect::<Vec<_>>()
        .join("\r\n");
    let expected = if cfg!(windows) {
        expected
    } else {
        expected.replace("\r\n", "\n")
    };
    for path in [&first, &second, &third] {
        assert_eq!(fs::read_to_string(path).unwrap(), expected);
    }
    assert_eq!(
        ws.active_tab().unwrap().path.as_deref(),
        Some(third.as_path())
    );

    // Opening an already-open file activates its tab instead of duplicating it.
    ws.new_tab();
    ws.open_paths(std::slice::from_ref(&third));
    assert_eq!(
        ws.tabs
            .iter()
            .filter(|t| t.path.as_deref() == Some(third.as_path()))
            .count(),
        1
    );
    assert_eq!(
        ws.active_tab().unwrap().path.as_deref(),
        Some(third.as_path())
    );

    // Save As onto a file open in another tab is refused.
    ws.open_paths(std::slice::from_ref(&first));
    script.save.push_back(Some(third.clone()));
    ws.save_active(true, &mut script);
    assert_eq!(prompt_name(&ws), Some("Message"));

    // A fresh workspace reopens the same content.
    let mut reopened = Workspace::new(None, Settings::default());
    reopened.open_paths(std::slice::from_ref(&third));
    assert_eq!(
        reopened.active_tab().unwrap().text,
        (1..=10)
            .map(|n| format!("v{n}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn line_ending_and_encoding_changes_mark_dirty_and_are_written() {
    let dir = TempDir::new("ws-format");
    let path = dir.join("f.txt");
    fs::write(&path, "a\nb\n").unwrap();
    let mut ws = ws();
    let mut script = Script::default();
    ws.open_paths(std::slice::from_ref(&path));
    let id = ws.active_tab().unwrap().id;
    assert_eq!(ws.tab(id).unwrap().eol, LineEnding::Lf);
    ws.set_eol(id, LineEnding::CrLf);
    ws.set_encoding(id, TextEncoding::Utf16Le);
    assert!(ws.tab(id).unwrap().is_dirty());
    ws.save_tab(id, &mut script);
    let expected: Vec<u8> = [0xFF, 0xFE]
        .into_iter()
        .chain("a\r\nb\r\n".encode_utf16().flat_map(u16::to_le_bytes))
        .collect();
    assert_eq!(fs::read(&path).unwrap(), expected);
}

#[test]
fn find_and_replace() {
    let mut ws = ws();
    type_text(&mut ws, "one two one TWO one");
    ws.find.needle = "one".into();
    assert!(ws.find_step(true));
    assert_eq!(ws.selection, (0, 3));
    assert!(ws.find_step(true));
    assert_eq!(ws.selection, (8, 11));
    assert!(ws.find_step(false));
    assert_eq!(ws.selection, (0, 3));
    ws.find.replacement = "1".into();
    ws.replace_step();
    assert_eq!(ws.active_tab().unwrap().text, "1 two one TWO one");
    assert_eq!(ws.selection, (6, 9), "moved to the next match");
    ws.find.needle = "two".into();
    ws.find.replacement = "2".into();
    assert_eq!(ws.replace_all(), 2);
    assert_eq!(ws.active_tab().unwrap().text, "1 2 one 2 one");
    ws.find.options.match_case = true;
    ws.find.needle = "missing".into();
    assert!(!ws.find_step(true));
    assert!(ws.status.contains("Cannot find"));
    assert!(ws.active_tab().unwrap().is_dirty());
}

#[test]
fn import_dialog_cancellation() {
    let mut ws = ws();
    let mut script = Script::default();
    script.folder.push_back(None);
    ws.import_notebook(&mut script);
    assert_eq!(ws.status, "Import cancelled.");
    let notebook = TempDir::new("ws-import-cancel");
    common::write_legacy_note(notebook.path(), "a.md", "id-1", "A", 1, "body");
    script.folder.push_back(Some(notebook.path().to_path_buf()));
    script.folder.push_back(None);
    ws.import_notebook(&mut script);
    assert_eq!(ws.status, "Import cancelled.");
}

#[test]
fn settings_round_trip_and_fall_back_to_defaults() {
    use notestrike::workspace::ThemeChoice;
    let dir = TempDir::new("settings");
    let path = dir.join("settings.json");
    assert_eq!(Settings::load(&path), Settings::default(), "missing file");
    assert_eq!(Settings::default().theme, ThemeChoice::System);
    for theme in [ThemeChoice::Light, ThemeChoice::Dark, ThemeChoice::System] {
        let settings = Settings {
            theme,
            word_wrap: false,
            zoom_percent: 150,
            status_bar: false,
        };
        settings.save(&path).unwrap();
        assert_eq!(Settings::load(&path), settings);
    }
    fs::write(&path, b"{not json").unwrap();
    assert_eq!(Settings::load(&path), Settings::default(), "corrupt file");
    fs::write(&path, br#"{"theme":"purple","zoom_percent":9000}"#).unwrap();
    let odd = Settings::load(&path);
    assert_eq!(odd.theme, ThemeChoice::System);
    assert_eq!(odd.zoom_percent, Settings::ZOOM_MAX);
}

#[test]
fn new_tabs_use_the_platform_line_ending_and_files_keep_theirs() {
    let expected = if cfg!(windows) {
        LineEnding::CrLf
    } else {
        LineEnding::Lf
    };
    assert_eq!(LineEnding::platform_default(), expected);
    let dir = TempDir::new("ws-platform-eol");
    let mut ws = ws();
    let mut script = Script::default();
    ws.new_tab();
    let id = ws.active_tab().unwrap().id;
    assert_eq!(ws.tab(id).unwrap().eol, expected);

    // An existing file keeps its own convention on every OS.
    for (name, bytes, eol) in [
        ("crlf.txt", &b"a\r\nb\r\n"[..], LineEnding::CrLf),
        ("lf.txt", &b"a\nb\n"[..], LineEnding::Lf),
    ] {
        let path = dir.join(name);
        fs::write(&path, bytes).unwrap();
        ws.open_paths(std::slice::from_ref(&path));
        let id = ws.active_tab().unwrap().id;
        assert_eq!(ws.tab(id).unwrap().eol, eol, "{name}");
        ws.active_tab_mut().unwrap().text.push_str("c\n");
        ws.text_edited(id);
        ws.save_tab(id, &mut script);
        let saved = fs::read(&path).unwrap();
        let mut want = bytes.to_vec();
        want.extend_from_slice(if eol == LineEnding::CrLf {
            b"c\r\n"
        } else {
            b"c\n"
        });
        assert_eq!(saved, want, "{name}");
    }
}
