//! Crash/session recovery: journal entries survive a simulated crash and a
//! real aborted process, are offered on the next start, and can be restored,
//! postponed, or explicitly discarded.

mod common;

use std::fs;
use std::process::{Command, Stdio};

use common::{Script, TempDir};
use notestrike::recovery::RecoveryStore;
use notestrike::workspace::{Answer, Prompt, Settings, Workspace};

fn type_text(ws: &mut Workspace, text: &str) {
    let id = ws.active_tab().unwrap().id;
    ws.active_tab_mut().unwrap().text.push_str(text);
    ws.text_edited(id);
}

fn recovery_prompt_len(ws: &Workspace) -> Option<usize> {
    match ws.current_prompt() {
        Some(Prompt::Recovery { orphans }) => Some(orphans.len()),
        _ => None,
    }
}

#[test]
fn simulated_crash_offers_and_restores_unsaved_text() {
    let data = TempDir::new("recovery-sim");
    let docs = TempDir::new("recovery-docs");
    let saved = docs.join("saved.txt");
    fs::write(&saved, "on disk\r\n").unwrap();
    let mut script = Script::default();
    {
        let store = RecoveryStore::open(data.path()).unwrap();
        let mut ws = Workspace::new(Some(store), Settings::default());
        type_text(&mut ws, "untitled draft, never saved");
        ws.open_paths(std::slice::from_ref(&saved));
        type_text(&mut ws, "edited but not saved");
        ws.new_tab();
        ws.flush_journal();
        // Dropped without shutdown(): what a crash leaves behind.
    }
    let store = RecoveryStore::open(data.path()).unwrap();
    let mut ws = Workspace::new(Some(store), Settings::default());
    assert_eq!(recovery_prompt_len(&ws), Some(2));
    ws.answer(Answer::Restore, &mut script);
    assert_eq!(ws.tabs.len(), 2);
    let draft = ws.tabs.iter().find(|t| t.path.is_none()).unwrap();
    assert_eq!(draft.text, "untitled draft, never saved");
    assert!(draft.is_dirty());
    let edited = ws
        .tabs
        .iter()
        .find(|t| t.path.as_deref() == Some(saved.as_path()))
        .unwrap();
    assert_eq!(edited.text, "on disk\nedited but not saved");
    assert!(edited.is_dirty());
    assert_eq!(
        fs::read_to_string(&saved).unwrap(),
        "on disk\r\n",
        "recovery never writes user files"
    );

    // Saving the recovered tab works (disk unchanged since the crash).
    let id = edited.id;
    ws.save_tab(id, &mut script);
    assert!(ws.current_prompt().is_none());
    assert_eq!(
        fs::read_to_string(&saved).unwrap(),
        "on disk\r\nedited but not saved"
    );

    // Normal exit clears the journal: nothing is offered next time.
    ws.request_quit();
    ws.answer(Answer::DiscardAll, &mut script);
    ws.shutdown();
    let ws = Workspace::new(
        Some(RecoveryStore::open(data.path()).unwrap()),
        Settings::default(),
    );
    assert!(ws.current_prompt().is_none());
}

#[test]
fn recovered_undecodable_file_still_requires_lossy_save_confirmation() {
    let data = TempDir::new("recovery-lossy");
    let docs = TempDir::new("recovery-lossy-docs");
    let path = docs.join("damaged.txt");
    let original = b"caf\xC3\xA9 \xFF end".to_vec();
    fs::write(&path, &original).unwrap();
    {
        let mut ws = Workspace::new(
            Some(RecoveryStore::open(data.path()).unwrap()),
            Settings::default(),
        );
        ws.open_paths(std::slice::from_ref(&path));
        assert_eq!(ws.active_tab().unwrap().invalid_sequences, 1);
        type_text(&mut ws, "draft");
        ws.flush_journal();
    }
    let mut script = Script::default();
    let mut ws = Workspace::new(
        Some(RecoveryStore::open(data.path()).unwrap()),
        Settings::default(),
    );
    ws.answer(Answer::Restore, &mut script);
    let tab = ws.active_tab().unwrap();
    assert_eq!(tab.invalid_sequences, 1);
    assert_eq!(tab.first_invalid_offset, Some(6));
    ws.save_active(false, &mut script);
    assert!(matches!(
        ws.current_prompt(),
        Some(Prompt::LossySave { .. })
    ));
    assert_eq!(fs::read(&path).unwrap(), original);
}

#[test]
fn recovered_file_changed_on_disk_since_crash_is_flagged() {
    let data = TempDir::new("recovery-changed");
    let docs = TempDir::new("recovery-changed-docs");
    let path = docs.join("doc.txt");
    fs::write(&path, "base").unwrap();
    let mut script = Script::default();
    {
        let mut ws = Workspace::new(
            Some(RecoveryStore::open(data.path()).unwrap()),
            Settings::default(),
        );
        ws.open_paths(std::slice::from_ref(&path));
        type_text(&mut ws, " + local");
        ws.flush_journal();
    }
    fs::write(&path, "someone else changed it").unwrap();
    let mut ws = Workspace::new(
        Some(RecoveryStore::open(data.path()).unwrap()),
        Settings::default(),
    );
    ws.answer(Answer::Restore, &mut script);
    let tab = ws.active_tab().unwrap();
    assert_eq!(tab.text, "base + local");
    assert_eq!(
        tab.external,
        notestrike::workspace::ExternalState::ChangedOnDisk
    );
    ws.save_active(false, &mut script);
    assert!(matches!(
        ws.current_prompt(),
        Some(Prompt::ChangedOnDisk { .. })
    ));
    ws.answer(Answer::Cancel, &mut script);
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "someone else changed it"
    );
}

#[test]
fn recovery_later_and_discard_cancellation_keep_the_journal() {
    let data = TempDir::new("recovery-later");
    let mut script = Script::default();
    {
        let mut ws = Workspace::new(
            Some(RecoveryStore::open(data.path()).unwrap()),
            Settings::default(),
        );
        type_text(&mut ws, "precious");
        ws.flush_journal();
    }
    // Later: kept for next time.
    {
        let mut ws = Workspace::new(
            Some(RecoveryStore::open(data.path()).unwrap()),
            Settings::default(),
        );
        assert_eq!(recovery_prompt_len(&ws), Some(1));
        ws.answer(Answer::Later, &mut script);
        ws.shutdown();
    }
    // Discard -> Back returns to the choice; dismissing (Esc) also means later.
    {
        let mut ws = Workspace::new(
            Some(RecoveryStore::open(data.path()).unwrap()),
            Settings::default(),
        );
        ws.answer(Answer::Discard, &mut script);
        assert!(matches!(
            ws.current_prompt(),
            Some(Prompt::ConfirmDiscardRecovery { .. })
        ));
        ws.answer(Answer::Cancel, &mut script);
        assert_eq!(recovery_prompt_len(&ws), Some(1));
        ws.answer(Answer::Cancel, &mut script);
        ws.shutdown();
    }
    // Discard confirmed: gone.
    {
        let mut ws = Workspace::new(
            Some(RecoveryStore::open(data.path()).unwrap()),
            Settings::default(),
        );
        ws.answer(Answer::Discard, &mut script);
        ws.answer(Answer::Discard, &mut script);
        ws.shutdown();
    }
    let ws = Workspace::new(
        Some(RecoveryStore::open(data.path()).unwrap()),
        Settings::default(),
    );
    assert!(ws.current_prompt().is_none());
}

#[test]
fn a_running_session_is_not_mistaken_for_a_crash() {
    let data = TempDir::new("recovery-live");
    let mut first = Workspace::new(
        Some(RecoveryStore::open(data.path()).unwrap()),
        Settings::default(),
    );
    type_text(&mut first, "still open in another window");
    first.flush_journal();
    let second = Workspace::new(
        Some(RecoveryStore::open(data.path()).unwrap()),
        Settings::default(),
    );
    assert!(
        second.current_prompt().is_none(),
        "live session must not be offered for recovery"
    );
    drop(first);
}

const CHILD_ENV: &str = "NOTESTRIKE_CRASH_CHILD_DIR";

/// `std::process::abort` ends the process with SIGABRT on Unix and with
/// `__fastfail` (STATUS_STACK_BUFFER_OVERRUN, 0xC0000409) on Windows. A
/// normal exit or a panic (exit code 101) means the crash never happened.
fn aborted(status: &std::process::ExitStatus) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        status.signal() == Some(6)
    }
    #[cfg(windows)]
    {
        status.code() == Some(0xC000_0409_u32 as i32)
    }
    #[cfg(not(any(unix, windows)))]
    {
        !status.success()
    }
}

/// Helper run in a child process by `recovery_after_real_process_abort`.
#[test]
#[ignore = "helper process for recovery_after_real_process_abort"]
fn crash_child() {
    let Some(dir) = std::env::var_os(CHILD_ENV) else {
        return;
    };
    let mut ws = Workspace::new(
        Some(RecoveryStore::open(std::path::Path::new(&dir)).unwrap()),
        Settings::default(),
    );
    type_text(&mut ws, "typed before the crash");
    ws.flush_journal();
    std::process::abort();
}

#[test]
fn recovery_after_real_process_abort() {
    let data = TempDir::new("recovery-abort");
    // The test binary re-runs itself with only the ignored helper selected.
    // `current_exe` is the test binary on every OS (target/debug/deps/...).
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "crash_child",
            "--exact",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_ENV, data.path())
        .env_remove("RUST_TEST_THREADS")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let status = output.status;
    let details = format!(
        "child status {status:?}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(aborted(&status), "child must have aborted; {details}");
    let mut script = Script::default();
    let mut ws = Workspace::new(
        Some(RecoveryStore::open(data.path()).unwrap()),
        Settings::default(),
    );
    assert_eq!(recovery_prompt_len(&ws), Some(1));
    ws.answer(Answer::Restore, &mut script);
    assert_eq!(ws.active_tab().unwrap().text, "typed before the crash");
    ws.shutdown();
}
