//! Editor state without any GUI code: tabs, dirty tracking, open/save flows,
//! prompts (unsaved changes, external changes, encoding problems, recovery),
//! find/replace, and the recovery journal. The egui front end in
//! `native_ui.rs` renders this state; tests drive it directly with scripted
//! file dialogs.

use std::collections::VecDeque;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::document::{
    self, DiskSnapshot, DiskState, LoadError, LoadOptions, SaveError, SaveRequest,
};
use crate::encoding::{TextEncoding, Unrepresentable};
use crate::eol::{EolCounts, LineEnding};
use crate::legacy;
use crate::recovery::{self, JournalEntry, OrphanEntry, RecoveryStore};
use crate::search::{self, FindOptions};

/// Native (or scripted) file pickers. Returning `None` means the user cancelled.
pub trait FileDialogs {
    fn open_files(&mut self) -> Option<Vec<PathBuf>>;
    fn save_file(&mut self, suggested_name: &str, directory: Option<&Path>) -> Option<PathBuf>;
    fn pick_folder(&mut self, title: &str) -> Option<PathBuf>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExternalState {
    InSync,
    ChangedOnDisk,
    MissingOnDisk,
}

#[derive(Clone, Debug)]
pub struct Tab {
    pub id: u64,
    pub path: Option<PathBuf>,
    untitled_number: usize,
    pub text: String,
    saved_text: String,
    pub encoding: TextEncoding,
    pub utf16_bom: bool,
    pub eol: LineEnding,
    saved_format: (TextEncoding, bool, LineEnding),
    pub snapshot: Option<DiskSnapshot>,
    pub read_only: bool,
    /// Undecodable byte sequences found when the file was opened. While
    /// non-zero, saving asks first because U+FFFD would replace the bytes.
    pub invalid_sequences: usize,
    pub first_invalid_offset: Option<usize>,
    pub contains_nul: bool,
    /// Set when the file had mixed line endings (they are unified on save).
    pub mixed_eol: Option<EolCounts>,
    pub external: ExternalState,
    /// Recovered unsaved text is dirty until saved, whatever the disk holds.
    forced_dirty: bool,
    dirty: bool,
    journal_pending: bool,
    last_edit: Option<Instant>,
    /// Incremented whenever the text is replaced programmatically, so the UI
    /// can reset per-tab editor state (cursor, undo) where needed.
    pub revision: u64,
}

impl Tab {
    pub fn title(&self) -> String {
        match &self.path {
            Some(path) => path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string()),
            None => format!("Untitled {}", self.untitled_number),
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    fn is_pristine_untitled(&self) -> bool {
        self.path.is_none() && self.text.is_empty() && !self.dirty
    }

    fn recompute_dirty(&mut self) {
        self.dirty = self.forced_dirty
            || self.text != self.saved_text
            || (self.encoding, self.utf16_bom, self.eol) != self.saved_format;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingSave {
    pub tab: u64,
    /// Show the Save As dialog (or the tab has no path yet).
    pub save_as: bool,
    /// Target already chosen in a dialog earlier in this flow.
    pub chosen_path: Option<PathBuf>,
    pub overwrite_external: bool,
    pub accept_lossy: bool,
}

impl PendingSave {
    fn new(tab: u64, save_as: bool) -> Self {
        Self {
            tab,
            save_as,
            chosen_path: None,
            overwrite_external: false,
            accept_lossy: false,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Prompt {
    /// Save / DontSave / Cancel.
    UnsavedTab { tab: u64 },
    /// SaveAll / DiscardAll / Cancel.
    UnsavedOnQuit { tabs: Vec<u64> },
    /// Overwrite / SaveAs / Cancel.
    ChangedOnDisk { save: PendingSave },
    /// SaveAnyway / Cancel.
    LossySave { save: PendingSave },
    /// UseUtf8 / Cancel.
    Unrepresentable {
        save: PendingSave,
        details: Unrepresentable,
    },
    /// SaveAs / Cancel.
    ReadOnly { save: PendingSave },
    /// OpenAnyway / Cancel.
    LargeFile { path: PathBuf, size: u64 },
    /// Reload / Cancel. Discards unsaved edits of the tab.
    ConfirmReload {
        tab: u64,
        encoding: Option<TextEncoding>,
    },
    /// Restore / Discard / Later.
    Recovery { orphans: Vec<OrphanEntry> },
    /// Discard / Cancel.
    ConfirmDiscardRecovery { orphans: Vec<OrphanEntry> },
    /// GoTo(line) / Cancel.
    GoToLine,
    /// Ok.
    Message { title: String, body: String },
    /// Ok.
    About,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Save,
    DontSave,
    SaveAll,
    DiscardAll,
    Overwrite,
    SaveAs,
    SaveAnyway,
    UseUtf8,
    OpenAnyway,
    Reload,
    Restore,
    Discard,
    Later,
    GoTo(usize),
    Ok,
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AfterSave {
    CloseTab(u64),
    Quit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flow {
    Done,
    Waiting,
    Aborted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeChoice {
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub theme: ThemeChoice,
    pub word_wrap: bool,
    pub zoom_percent: u32,
    pub status_bar: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::System,
            word_wrap: true,
            zoom_percent: 100,
            status_bar: true,
        }
    }
}

impl Settings {
    pub const ZOOM_MIN: u32 = 50;
    pub const ZOOM_MAX: u32 = 400;

    pub fn load(path: &Path) -> Self {
        let Some(value) = std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        else {
            return Self::default();
        };
        let defaults = Self::default();
        Self {
            theme: match value.get("theme").and_then(|v| v.as_str()) {
                Some("light") => ThemeChoice::Light,
                Some("dark") => ThemeChoice::Dark,
                _ => ThemeChoice::System,
            },
            word_wrap: value
                .get("word_wrap")
                .and_then(|v| v.as_bool())
                .unwrap_or(defaults.word_wrap),
            zoom_percent: value
                .get("zoom_percent")
                .and_then(|v| v.as_u64())
                .map(|z| (z as u32).clamp(Self::ZOOM_MIN, Self::ZOOM_MAX))
                .unwrap_or(100),
            status_bar: value
                .get("status_bar")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let theme = match self.theme {
            ThemeChoice::System => "system",
            ThemeChoice::Light => "light",
            ThemeChoice::Dark => "dark",
        };
        let value = serde_json::json!({
            "theme": theme,
            "word_wrap": self.word_wrap,
            "zoom_percent": self.zoom_percent,
            "status_bar": self.status_bar,
        });
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        document::write_atomic(
            path,
            &serde_json::to_vec_pretty(&value).map_err(std::io::Error::other)?,
            document::FaultPlan::default(),
        )
    }
}

#[derive(Clone, Debug, Default)]
pub struct FindState {
    pub visible: bool,
    pub replace_visible: bool,
    pub needle: String,
    pub replacement: String,
    pub options: FindOptions,
    /// Request the UI to focus the search field.
    pub focus_request: bool,
}

pub struct Workspace {
    pub tabs: Vec<Tab>,
    pub active: usize,
    next_id: u64,
    next_untitled: usize,
    pub prompts: VecDeque<Prompt>,
    pub status: String,
    recovery: Option<RecoveryStore>,
    quit_approved: bool,
    quit_queue: Option<VecDeque<u64>>,
    after_save: Option<AfterSave>,
    pub find: FindState,
    /// Current selection of the active tab in chars: (start, end), start <= end.
    pub selection: (usize, usize),
    /// Selection the UI should apply to the editor (tab id, char range).
    pub selection_request: Option<(u64, Range<usize>)>,
    pub settings: Settings,
    last_disk_check: Option<Instant>,
    journal_warning_shown: bool,
}

/// Delay after the last keystroke before an unsaved tab is journaled.
pub const JOURNAL_DELAY: Duration = Duration::from_millis(1000);
const DISK_CHECK_INTERVAL: Duration = Duration::from_secs(2);

impl Workspace {
    /// Start a workspace with one empty tab. Orphaned recovery entries from
    /// a crashed session are offered through a [`Prompt::Recovery`].
    pub fn new(recovery: Option<RecoveryStore>, settings: Settings) -> Self {
        let mut ws = Self {
            tabs: Vec::new(),
            active: 0,
            next_id: 1,
            next_untitled: 1,
            prompts: VecDeque::new(),
            status: "Ready".into(),
            recovery,
            quit_approved: false,
            quit_queue: None,
            after_save: None,
            find: FindState {
                options: FindOptions {
                    wrap_around: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            selection: (0, 0),
            selection_request: None,
            settings,
            last_disk_check: None,
            journal_warning_shown: false,
        };
        if ws.recovery.is_none() {
            ws.status = "Recovery journal unavailable: unsaved text is not backed up.".into();
        }
        ws.new_tab();
        if let Some(store) = &ws.recovery {
            let orphans = store.orphans();
            if !orphans.is_empty() {
                ws.prompts.push_back(Prompt::Recovery { orphans });
            }
        }
        ws
    }

    pub fn recovery_dir(&self) -> Option<&Path> {
        self.recovery.as_ref().map(|r| r.dir())
    }

    // ----- tabs ---------------------------------------------------------

    pub fn active_tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active)
    }

    pub fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        self.tabs.get_mut(self.active)
    }

    pub fn tab(&self, id: u64) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == id)
    }

    fn tab_mut(&mut self, id: u64) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    fn index_of(&self, id: u64) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    pub fn activate(&mut self, id: u64) {
        if let Some(index) = self.index_of(id) {
            self.active = index;
            self.selection = (0, 0);
        }
    }

    pub fn next_tab(&mut self, forward: bool) {
        if self.tabs.is_empty() {
            return;
        }
        let n = self.tabs.len();
        self.active = if forward {
            (self.active + 1) % n
        } else {
            (self.active + n - 1) % n
        };
        self.selection = (0, 0);
    }

    fn blank_tab(&mut self) -> Tab {
        let id = self.next_id;
        self.next_id += 1;
        let eol = LineEnding::platform_default();
        Tab {
            id,
            path: None,
            untitled_number: 0,
            text: String::new(),
            saved_text: String::new(),
            encoding: TextEncoding::Utf8,
            utf16_bom: true,
            eol,
            saved_format: (TextEncoding::Utf8, true, eol),
            snapshot: None,
            read_only: false,
            invalid_sequences: 0,
            first_invalid_offset: None,
            contains_nul: false,
            mixed_eol: None,
            external: ExternalState::InSync,
            forced_dirty: false,
            dirty: false,
            journal_pending: false,
            last_edit: None,
            revision: 0,
        }
    }

    pub fn new_tab(&mut self) -> u64 {
        let mut tab = self.blank_tab();
        tab.untitled_number = self.next_untitled;
        self.next_untitled += 1;
        let id = tab.id;
        self.tabs.push(tab);
        self.active = self.tabs.len() - 1;
        self.selection = (0, 0);
        id
    }

    /// Call after the UI changed `text` of a tab.
    pub fn text_edited(&mut self, id: u64) {
        if let Some(tab) = self.tab_mut(id) {
            tab.recompute_dirty();
            tab.journal_pending = true;
            tab.last_edit = Some(Instant::now());
        }
    }

    pub fn set_encoding(&mut self, id: u64, encoding: TextEncoding) {
        if let Some(tab) = self.tab_mut(id) {
            tab.encoding = encoding;
            if matches!(encoding, TextEncoding::Utf16Le | TextEncoding::Utf16Be)
                && tab.saved_format.0 != encoding
            {
                tab.utf16_bom = true;
            }
            tab.recompute_dirty();
            tab.journal_pending = true;
            tab.last_edit = Some(Instant::now());
        }
        self.status = format!("Will save as {}.", encoding.label());
    }

    pub fn set_eol(&mut self, id: u64, eol: LineEnding) {
        if let Some(tab) = self.tab_mut(id) {
            tab.eol = eol;
            tab.recompute_dirty();
            tab.journal_pending = true;
            tab.last_edit = Some(Instant::now());
        }
        self.status = format!("Line endings will be saved as {}.", eol.label());
    }

    pub fn request_close_tab(&mut self, id: u64) {
        match self.tab(id) {
            Some(tab) if tab.is_dirty() => {
                self.activate(id);
                self.prompts.push_back(Prompt::UnsavedTab { tab: id });
            }
            Some(_) => self.close_tab_now(id),
            None => {}
        }
    }

    fn close_tab_now(&mut self, id: u64) {
        let Some(index) = self.index_of(id) else {
            return;
        };
        self.tabs.remove(index);
        if let Some(store) = &self.recovery {
            store.remove(id);
        }
        if self.tabs.is_empty() {
            self.new_tab();
        } else if self.active >= self.tabs.len() || self.active > index {
            self.active = self.active.saturating_sub(1).min(self.tabs.len() - 1);
        }
        self.selection = (0, 0);
    }

    // ----- open / reload -------------------------------------------------

    pub fn open_dialog(&mut self, dialogs: &mut dyn FileDialogs) {
        match dialogs.open_files() {
            Some(paths) => self.open_paths(&paths),
            None => self.status = "Open cancelled.".into(),
        }
    }

    pub fn open_paths(&mut self, paths: &[PathBuf]) {
        for path in paths {
            self.open_path(path, LoadOptions::default());
        }
    }

    fn find_open(&self, path: &Path) -> Option<u64> {
        self.tabs.iter().find_map(|tab| {
            let open = tab.path.as_deref()?;
            let same = open == path || same_file::is_same_file(open, path).unwrap_or(false);
            same.then_some(tab.id)
        })
    }

    pub fn open_path(&mut self, path: &Path, options: LoadOptions) {
        if let Some(id) = self.find_open(path) {
            self.activate(id);
            self.status = format!("{} is already open.", path.display());
            return;
        }
        match document::load(path, options) {
            Ok(loaded) => {
                let replace = self.tabs.len() == 1 && self.tabs[0].is_pristine_untitled();
                let mut tab = self.blank_tab();
                apply_loaded(&mut tab, loaded);
                let id = tab.id;
                let notice = load_notice(&tab);
                if replace {
                    self.tabs[0] = tab;
                    self.active = 0;
                } else {
                    self.tabs.push(tab);
                    self.active = self.tabs.len() - 1;
                }
                // Start at the top of the file, like Notepad.
                self.selection = (0, 0);
                self.selection_request = Some((id, 0..0));
                self.journal_now(id);
                self.status = notice;
            }
            Err(LoadError::TooLarge {
                size,
                hard_limit: false,
            }) => self.prompts.push_back(Prompt::LargeFile {
                path: path.to_path_buf(),
                size,
            }),
            Err(error) => self.prompts.push_back(Prompt::Message {
                title: "Could not open file".into(),
                body: format!("{}\n\n{error}.", path.display()),
            }),
        }
    }

    /// "Reopen with encoding" / "Reload from disk".
    pub fn request_reload(&mut self, id: u64, encoding: Option<TextEncoding>) {
        match self.tab(id) {
            Some(tab) if tab.path.is_none() => {
                self.status = "This document has not been saved to a file yet.".into();
            }
            Some(tab) if tab.is_dirty() => self
                .prompts
                .push_back(Prompt::ConfirmReload { tab: id, encoding }),
            Some(_) => self.reload_now(id, encoding),
            None => {}
        }
    }

    fn reload_now(&mut self, id: u64, encoding: Option<TextEncoding>) {
        let Some(path) = self.tab(id).and_then(|t| t.path.clone()) else {
            return;
        };
        let options = LoadOptions {
            allow_large: true,
            forced_encoding: encoding,
        };
        match document::load(&path, options) {
            Ok(loaded) => {
                if let Some(tab) = self.tab_mut(id) {
                    apply_loaded(tab, loaded);
                    tab.forced_dirty = false;
                    tab.recompute_dirty();
                    tab.revision += 1;
                    let notice = load_notice(tab);
                    self.status = format!("Reloaded. {notice}");
                }
                self.journal_now(id);
            }
            Err(error) => self.prompts.push_back(Prompt::Message {
                title: "Could not reload file".into(),
                body: format!("{}\n\n{error}.", path.display()),
            }),
        }
    }

    /// Keep the editor text after an external change; the next save will ask
    /// before overwriting the other program's version.
    pub fn keep_my_version(&mut self, id: u64) {
        if let Some(tab) = self.tab_mut(id) {
            tab.external = ExternalState::InSync;
            tab.forced_dirty = true;
            tab.recompute_dirty();
        }
        self.status =
            "Keeping your version. Saving will ask before replacing the file on disk.".into();
    }

    // ----- save flows ----------------------------------------------------

    pub fn save_active(&mut self, save_as: bool, dialogs: &mut dyn FileDialogs) {
        if let Some(id) = self.active_tab().map(|t| t.id) {
            self.after_save = None;
            self.run_save(PendingSave::new(id, save_as), dialogs);
        }
    }

    pub fn save_tab(&mut self, id: u64, dialogs: &mut dyn FileDialogs) {
        self.after_save = None;
        self.run_save(PendingSave::new(id, false), dialogs);
    }

    /// Save every modified tab (stops at the first cancel or error).
    pub fn save_all(&mut self, dialogs: &mut dyn FileDialogs) {
        let dirty: Vec<u64> = self
            .tabs
            .iter()
            .filter(|t| t.is_dirty())
            .map(|t| t.id)
            .collect();
        for id in dirty {
            self.after_save = None;
            self.activate(id);
            if self.run_save(PendingSave::new(id, false), dialogs) != Flow::Done {
                break;
            }
        }
    }

    fn run_save(&mut self, mut pending: PendingSave, dialogs: &mut dyn FileDialogs) -> Flow {
        let flow = self.try_save(&mut pending, dialogs);
        match flow {
            Flow::Done => self.on_save_done(dialogs),
            Flow::Aborted => self.on_save_aborted(),
            Flow::Waiting => {}
        }
        flow
    }

    fn try_save(&mut self, pending: &mut PendingSave, dialogs: &mut dyn FileDialogs) -> Flow {
        let Some(tab) = self.tab(pending.tab).cloned() else {
            return Flow::Aborted;
        };
        // 1. Choose the target.
        let mut new_target = false;
        let path = match (&pending.chosen_path, pending.save_as || tab.path.is_none()) {
            (Some(chosen), _) => {
                new_target = Some(chosen) != tab.path.as_ref();
                chosen.clone()
            }
            (None, true) => {
                let suggested = match &tab.path {
                    Some(_) => tab.title(),
                    None => format!("{}.txt", tab.title()),
                };
                let dir = tab
                    .path
                    .as_ref()
                    .and_then(|p| p.parent())
                    .map(Path::to_path_buf);
                match dialogs.save_file(&suggested, dir.as_deref()) {
                    Some(chosen) => {
                        if let Some(other) =
                            self.find_open(&chosen).filter(|&other| other != tab.id)
                        {
                            let name = self.tab(other).map(Tab::title).unwrap_or_default();
                            self.prompts.push_back(Prompt::Message {
                                title: "File is open in another tab".into(),
                                body: format!(
                                    "{} is already open in the tab \"{name}\". Close that tab first or choose another name.",
                                    chosen.display()
                                ),
                            });
                            return Flow::Aborted;
                        }
                        // The OS dialog already confirmed replacing an existing file.
                        new_target = Some(&chosen) != tab.path.as_ref();
                        if !new_target {
                            pending.overwrite_external = true;
                        }
                        pending.chosen_path = Some(chosen.clone());
                        chosen
                    }
                    None => {
                        self.status = "Save cancelled.".into();
                        return Flow::Aborted;
                    }
                }
            }
            (None, false) => tab.path.clone().expect("tab has a path"),
        };
        // 2. Warn before making undecodable bytes permanent.
        if tab.invalid_sequences > 0 && !pending.accept_lossy {
            pending.chosen_path = Some(path);
            self.prompts.push_back(Prompt::LossySave {
                save: pending.clone(),
            });
            return Flow::Waiting;
        }
        // 3. Write.
        let expected = if new_target {
            None
        } else {
            tab.snapshot.as_ref()
        };
        let request = SaveRequest {
            path: &path,
            text: &tab.text,
            encoding: tab.encoding,
            utf16_bom: tab.utf16_bom,
            eol: tab.eol,
            expected,
            overwrite_external_changes: pending.overwrite_external,
        };
        match document::save(&request) {
            Ok(snapshot) => {
                let id = tab.id;
                if let Some(tab) = self.tab_mut(id) {
                    tab.path = Some(path.clone());
                    tab.saved_text = tab.text.clone();
                    tab.saved_format = (tab.encoding, tab.utf16_bom, tab.eol);
                    tab.snapshot = Some(snapshot);
                    tab.external = ExternalState::InSync;
                    tab.invalid_sequences = 0;
                    tab.first_invalid_offset = None;
                    tab.mixed_eol = None;
                    tab.read_only = false;
                    tab.forced_dirty = false;
                    tab.recompute_dirty();
                }
                self.journal_now(id);
                self.status = format!("Saved {}", path.display());
                Flow::Done
            }
            Err(SaveError::ChangedOnDisk) => {
                pending.chosen_path = Some(path);
                self.prompts.push_back(Prompt::ChangedOnDisk {
                    save: pending.clone(),
                });
                Flow::Waiting
            }
            Err(SaveError::ReadOnly) => {
                pending.chosen_path = Some(path);
                self.prompts.push_back(Prompt::ReadOnly {
                    save: pending.clone(),
                });
                Flow::Waiting
            }
            Err(SaveError::Unrepresentable(details)) => {
                pending.chosen_path = Some(path);
                self.prompts.push_back(Prompt::Unrepresentable {
                    save: pending.clone(),
                    details,
                });
                Flow::Waiting
            }
            Err(error @ (SaveError::PermissionDenied(_) | SaveError::Io(_))) => {
                self.prompts.push_back(Prompt::Message {
                    title: "Could not save".into(),
                    body: format!(
                        "{}\n\n{error}.\n\nThe original file was not changed. Your text is still open{}; try Save As to choose another location.",
                        path.display(),
                        if self.recovery.is_some() {
                            " and backed up in the recovery journal"
                        } else {
                            ""
                        }
                    ),
                });
                self.status = "Save failed; nothing was overwritten.".into();
                Flow::Aborted
            }
        }
    }

    fn on_save_done(&mut self, dialogs: &mut dyn FileDialogs) {
        match self.after_save.take() {
            Some(AfterSave::CloseTab(id)) => self.close_tab_now(id),
            Some(AfterSave::Quit) => self.continue_quit(dialogs),
            None => {}
        }
    }

    fn on_save_aborted(&mut self) {
        let was_quitting = self.quit_queue.take().is_some();
        self.after_save = None;
        if was_quitting {
            self.status = "Exit cancelled; unsaved documents are still open.".into();
        }
    }

    // ----- quit ----------------------------------------------------------

    /// Ask to quit (menu Exit or window close button).
    pub fn request_quit(&mut self) {
        if self.quit_approved {
            return;
        }
        if self
            .prompts
            .iter()
            .any(|p| matches!(p, Prompt::UnsavedOnQuit { .. }))
        {
            return;
        }
        let dirty: Vec<u64> = self
            .tabs
            .iter()
            .filter(|t| t.is_dirty())
            .map(|t| t.id)
            .collect();
        if dirty.is_empty() {
            self.quit_approved = true;
        } else {
            self.prompts
                .push_back(Prompt::UnsavedOnQuit { tabs: dirty });
        }
    }

    pub fn quit_approved(&self) -> bool {
        self.quit_approved
    }

    fn continue_quit(&mut self, dialogs: &mut dyn FileDialogs) {
        loop {
            let next = self.quit_queue.as_mut().and_then(VecDeque::pop_front);
            let Some(id) = next else {
                if self.quit_queue.take().is_some() {
                    self.quit_approved = true;
                }
                return;
            };
            if !self.tab(id).is_some_and(Tab::is_dirty) {
                continue;
            }
            self.activate(id);
            self.after_save = Some(AfterSave::Quit);
            // Done recurses into continue_quit via on_save_done.
            self.run_save(PendingSave::new(id, false), dialogs);
            return;
        }
    }

    /// Release the recovery journal on a normal exit.
    pub fn shutdown(&mut self) {
        if let Some(store) = self.recovery.take() {
            store.shutdown();
        }
    }

    // ----- prompts -------------------------------------------------------

    pub fn current_prompt(&self) -> Option<&Prompt> {
        self.prompts.front()
    }

    /// Answer the front prompt.
    pub fn answer(&mut self, answer: Answer, dialogs: &mut dyn FileDialogs) {
        let Some(prompt) = self.prompts.pop_front() else {
            return;
        };
        match (prompt, answer) {
            (Prompt::UnsavedTab { tab }, Answer::Save) => {
                self.after_save = Some(AfterSave::CloseTab(tab));
                self.run_save(PendingSave::new(tab, false), dialogs);
            }
            (Prompt::UnsavedTab { tab }, Answer::DontSave) => self.close_tab_now(tab),
            (Prompt::UnsavedOnQuit { tabs }, Answer::SaveAll) => {
                self.quit_queue = Some(tabs.into_iter().collect());
                self.continue_quit(dialogs);
            }
            (Prompt::UnsavedOnQuit { .. }, Answer::DiscardAll) => {
                if let Some(store) = &self.recovery {
                    store.clear_session();
                }
                self.quit_approved = true;
            }
            (Prompt::ChangedOnDisk { mut save }, Answer::Overwrite) => {
                save.overwrite_external = true;
                self.run_save(save, dialogs);
            }
            (
                Prompt::ChangedOnDisk { save }
                | Prompt::ReadOnly { save }
                | Prompt::LossySave { save },
                Answer::SaveAs,
            ) => {
                let mut fresh = PendingSave::new(save.tab, true);
                fresh.accept_lossy = save.accept_lossy;
                self.run_save(fresh, dialogs);
            }
            (Prompt::LossySave { mut save }, Answer::SaveAnyway) => {
                save.accept_lossy = true;
                self.run_save(save, dialogs);
            }
            (Prompt::Unrepresentable { save, .. }, Answer::UseUtf8) => {
                if let Some(tab) = self.tab_mut(save.tab) {
                    tab.encoding = TextEncoding::Utf8;
                    tab.recompute_dirty();
                }
                self.run_save(save, dialogs);
            }
            (Prompt::LargeFile { path, .. }, Answer::OpenAnyway) => self.open_path(
                &path,
                LoadOptions {
                    allow_large: true,
                    forced_encoding: None,
                },
            ),
            (Prompt::ConfirmReload { tab, encoding }, Answer::Reload) => {
                self.reload_now(tab, encoding)
            }
            (Prompt::Recovery { orphans }, Answer::Restore) => self.restore(orphans),
            (Prompt::Recovery { orphans }, Answer::Discard) => {
                self.prompts
                    .push_front(Prompt::ConfirmDiscardRecovery { orphans });
            }
            (Prompt::Recovery { .. }, Answer::Later) => {
                self.status = "Recovered text was kept; it will be offered again next time.".into();
            }
            (Prompt::ConfirmDiscardRecovery { orphans }, Answer::Discard) => {
                if let Some(store) = &self.recovery {
                    store.delete_orphans(&orphans);
                }
                self.status = "Recovered text discarded.".into();
            }
            (Prompt::ConfirmDiscardRecovery { orphans }, Answer::Cancel) => {
                // Back to the recovery choice rather than losing it.
                self.prompts.push_front(Prompt::Recovery { orphans });
            }
            (Prompt::GoToLine, Answer::GoTo(line)) => {
                if let Err(message) = self.go_to_line(line) {
                    self.prompts.push_front(Prompt::GoToLine);
                    self.status = message;
                }
            }
            (Prompt::Recovery { orphans }, Answer::Cancel) => {
                // Closing the recovery dialog is the same as "Later".
                let _ = orphans;
                self.status = "Recovered text was kept; it will be offered again next time.".into();
            }
            (
                Prompt::UnsavedTab { .. }
                | Prompt::UnsavedOnQuit { .. }
                | Prompt::ChangedOnDisk { .. }
                | Prompt::LossySave { .. }
                | Prompt::Unrepresentable { .. }
                | Prompt::ReadOnly { .. },
                Answer::Cancel,
            ) => {
                let was_quitting = self.quit_queue.is_some();
                self.on_save_aborted();
                if !was_quitting {
                    self.status = "Cancelled.".into();
                }
            }
            (
                Prompt::LargeFile { .. } | Prompt::ConfirmReload { .. } | Prompt::GoToLine,
                Answer::Cancel,
            ) => {
                self.status = "Cancelled.".into();
            }
            (Prompt::Message { .. } | Prompt::About, _) => {}
            (prompt, answer) => {
                // An answer that does not belong to this prompt: keep the prompt.
                debug_assert!(false, "unexpected answer {answer:?} for {prompt:?}");
                self.prompts.push_front(prompt);
            }
        }
    }

    // ----- recovery --------------------------------------------------------

    fn restore(&mut self, orphans: Vec<OrphanEntry>) {
        let mut restored = 0;
        let mut reopened = 0;
        let mut entries: Vec<&OrphanEntry> = orphans.iter().collect();
        entries.sort_by_key(|o| o.entry.position);
        for orphan in entries {
            let entry = &orphan.entry;
            match (&entry.text, &entry.path) {
                (None, Some(path)) => {
                    if path.exists() {
                        self.open_path(
                            path,
                            LoadOptions {
                                allow_large: true,
                                forced_encoding: None,
                            },
                        );
                        reopened += 1;
                    }
                }
                (Some(text), _) => {
                    let mut tab = self.blank_tab();
                    tab.path = entry.path.clone();
                    if tab.path.is_none() {
                        tab.untitled_number = self.next_untitled;
                        self.next_untitled += 1;
                    }
                    tab.text = text.clone();
                    tab.encoding = entry.encoding;
                    tab.utf16_bom = entry.utf16_bom;
                    tab.eol = entry.eol;
                    tab.invalid_sequences = entry.invalid_sequences;
                    tab.first_invalid_offset = entry.first_invalid_offset;
                    tab.saved_format = (entry.encoding, entry.utf16_bom, entry.eol);
                    if let Some(hash) = entry
                        .base_hash
                        .as_deref()
                        .and_then(DiskSnapshot::hash_from_hex)
                    {
                        tab.snapshot = Some(DiskSnapshot {
                            len: entry.base_len.unwrap_or(0),
                            modified: None,
                            hash,
                        });
                    }
                    if let (Some(path), Some(snapshot)) = (&tab.path, &tab.snapshot) {
                        tab.external = match document::disk_state(path, snapshot) {
                            Ok((DiskState::Unchanged, _)) => ExternalState::InSync,
                            Ok((DiskState::Changed, _)) => ExternalState::ChangedOnDisk,
                            Ok((DiskState::Missing, _)) => ExternalState::MissingOnDisk,
                            Err(_) => ExternalState::ChangedOnDisk,
                        };
                    }
                    tab.forced_dirty = true;
                    tab.recompute_dirty();
                    tab.journal_pending = true;
                    let id = tab.id;
                    if self.tabs.len() == 1 && self.tabs[0].is_pristine_untitled() {
                        self.tabs[0] = tab;
                        self.active = 0;
                    } else {
                        self.tabs.push(tab);
                        self.active = self.tabs.len() - 1;
                    }
                    self.journal_now(id);
                    restored += 1;
                }
                (None, None) => {}
            }
        }
        if let Some(store) = &self.recovery {
            store.delete_orphans(&orphans);
        }
        self.status =
            format!("Recovered {restored} unsaved document(s) and reopened {reopened} file(s).");
    }

    fn journal_entry(&self, index: usize) -> Option<JournalEntry> {
        let tab = self.tabs.get(index)?;
        if tab.path.is_none() && !tab.is_dirty() {
            return None;
        }
        Some(JournalEntry {
            tab_id: tab.id,
            position: index,
            title: tab.title(),
            path: tab.path.clone(),
            text: tab.is_dirty().then(|| tab.text.clone()),
            encoding: tab.encoding,
            utf16_bom: tab.utf16_bom,
            eol: tab.eol,
            invalid_sequences: tab.invalid_sequences,
            first_invalid_offset: tab.first_invalid_offset,
            base_hash: tab.snapshot.as_ref().map(DiskSnapshot::hash_hex),
            base_len: tab.snapshot.as_ref().map(|s| s.len),
            saved_unix: recovery::now_unix(),
        })
    }

    fn journal_now(&mut self, id: u64) {
        let Some(index) = self.index_of(id) else {
            return;
        };
        let entry = self.journal_entry(index);
        if let Some(tab) = self.tabs.get_mut(index) {
            tab.journal_pending = false;
        }
        let Some(store) = &self.recovery else { return };
        let result = match entry {
            Some(entry) => store.write(&entry),
            None => {
                store.remove(id);
                Ok(())
            }
        };
        if let Err(error) = result
            && !self.journal_warning_shown
        {
            self.journal_warning_shown = true;
            self.status = format!("Could not write the recovery journal: {error}");
        }
    }

    /// Write every pending journal entry immediately (used on focus loss and in tests).
    pub fn flush_journal(&mut self) {
        let ids: Vec<u64> = self
            .tabs
            .iter()
            .filter(|t| t.journal_pending)
            .map(|t| t.id)
            .collect();
        for id in ids {
            self.journal_now(id);
        }
    }

    /// Periodic work: journal tabs idle for [`JOURNAL_DELAY`] and check open
    /// files for external changes.
    pub fn tick(&mut self, now: Instant) {
        let due: Vec<u64> = self
            .tabs
            .iter()
            .filter(|t| {
                t.journal_pending
                    && t.last_edit
                        .is_none_or(|at| now.duration_since(at) >= JOURNAL_DELAY)
            })
            .map(|t| t.id)
            .collect();
        for id in due {
            self.journal_now(id);
        }
        if self
            .last_disk_check
            .is_none_or(|at| now.duration_since(at) >= DISK_CHECK_INTERVAL)
        {
            self.last_disk_check = Some(now);
            self.check_disk();
        }
    }

    /// Compare every open file with its snapshot and flag external changes.
    pub fn check_disk(&mut self) {
        for tab in &mut self.tabs {
            let (Some(path), Some(snapshot)) = (&tab.path, &tab.snapshot) else {
                continue;
            };
            if tab.external != ExternalState::InSync {
                continue;
            }
            match document::disk_state(path, snapshot) {
                Ok((DiskState::Unchanged, Some(fresh))) => tab.snapshot = Some(fresh),
                Ok((DiskState::Unchanged, None)) => {}
                Ok((DiskState::Changed, _)) => tab.external = ExternalState::ChangedOnDisk,
                Ok((DiskState::Missing, _)) => tab.external = ExternalState::MissingOnDisk,
                Err(_) => {}
            }
        }
    }

    // ----- find / replace / go to -----------------------------------------

    pub fn show_find(&mut self, replace: bool) {
        self.find.visible = true;
        self.find.replace_visible = replace;
        self.find.focus_request = true;
        // Seed the search with a single-line selection, like Notepad.
        if let Some(tab) = self.active_tab() {
            let (start, end) = self.selection;
            if start != end {
                let a = search::char_to_byte(&tab.text, start);
                let b = search::char_to_byte(&tab.text, end);
                let selected = &tab.text[a..b];
                if !selected.contains('\n') {
                    self.find.needle = selected.to_owned();
                }
            }
        }
    }

    /// Find the next/previous match from the current selection.
    pub fn find_step(&mut self, forward: bool) -> bool {
        let Some(tab) = self.active_tab() else {
            return false;
        };
        if self.find.needle.is_empty() {
            self.find.visible = true;
            self.find.focus_request = true;
            return false;
        }
        let (start, end) = self.selection;
        let text = &tab.text;
        let found = if forward {
            search::find_next(
                text,
                &self.find.needle,
                search::char_to_byte(text, end),
                self.find.options,
            )
        } else {
            search::find_prev(
                text,
                &self.find.needle,
                search::char_to_byte(text, start),
                self.find.options,
            )
        };
        let found = found.map(|range| {
            (
                tab.id,
                search::byte_to_char(text, range.start)..search::byte_to_char(text, range.end),
            )
        });
        match found {
            Some((id, chars)) => {
                self.selection = (chars.start, chars.end);
                self.selection_request = Some((id, chars));
                self.status = String::new();
                true
            }
            None => {
                self.status = format!("Cannot find \"{}\"", self.find.needle);
                false
            }
        }
    }

    /// Replace the current selection if it is a match, then find the next one.
    pub fn replace_step(&mut self) {
        let Some(index) = self.tabs.get(self.active).map(|_| self.active) else {
            return;
        };
        if self.find.needle.is_empty() {
            return;
        }
        let (start, end) = self.selection;
        let tab = &self.tabs[index];
        let a = search::char_to_byte(&tab.text, start);
        let b = search::char_to_byte(&tab.text, end);
        let opts = FindOptions {
            wrap_around: false,
            ..self.find.options
        };
        let is_match =
            a != b && search::find_next(&tab.text, &self.find.needle, a, opts) == Some(a..b);
        if is_match {
            let id = tab.id;
            let replacement = self.find.replacement.clone();
            let tab = &mut self.tabs[index];
            tab.text.replace_range(a..b, &replacement);
            tab.revision += 1;
            let new_end = start + replacement.chars().count();
            self.selection = (new_end, new_end);
            self.text_edited(id);
        }
        self.find_step(true);
    }

    pub fn replace_all(&mut self) -> usize {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return 0;
        };
        let (text, count) = search::replace_all(
            &tab.text,
            &self.find.needle,
            &self.find.replacement,
            self.find.options,
        );
        if count > 0 {
            tab.text = text;
            tab.revision += 1;
            let id = tab.id;
            self.selection = (0, 0);
            self.text_edited(id);
        }
        self.status = format!("Replaced {count} occurrence(s).");
        count
    }

    pub fn go_to_line(&mut self, line: usize) -> Result<(), String> {
        let Some(tab) = self.active_tab() else {
            return Err("No document".into());
        };
        let total = search::line_count(&tab.text);
        let id = tab.id;
        match search::line_start_char(&tab.text, line) {
            Some(start) => {
                self.selection = (start, start);
                self.selection_request = Some((id, start..start));
                Ok(())
            }
            None => Err(format!(
                "The line number is beyond the total number of lines ({total})."
            )),
        }
    }

    // ----- legacy notebook import ------------------------------------------

    /// Import a NoteStrike 0.1 notebook as plain files. The notebook is only read.
    pub fn import_notebook(&mut self, dialogs: &mut dyn FileDialogs) {
        let Some(root) =
            dialogs.pick_folder("Choose a NoteStrike 0.1 notebook folder (contains \"notes\")")
        else {
            self.status = "Import cancelled.".into();
            return;
        };
        if !legacy::is_legacy_notebook(&root) {
            self.prompts.push_back(Prompt::Message {
                title: "Not a NoteStrike notebook".into(),
                body: format!(
                    "{} has no \"notes\" folder. Choose the folder you used with NoteStrike 0.1 (the one passed to `notestrike init`).",
                    root.display()
                ),
            });
            return;
        }
        let Some(dest) = dialogs.pick_folder("Choose an empty folder for the plain-text copies")
        else {
            self.status = "Import cancelled.".into();
            return;
        };
        match legacy::import(&root, &dest) {
            Ok(report) => {
                let mut body = format!(
                    "Copied {} note(s) to {} as plain .md files.\nThe original notebook was not changed.",
                    report.total(),
                    dest.display()
                );
                if !report.copied_verbatim.is_empty() {
                    body.push_str(&format!(
                        "\n\n{} file(s) could not be parsed and were copied byte-for-byte.",
                        report.copied_verbatim.len()
                    ));
                }
                body.push_str("\n\nUse File > Open to edit the copies.");
                self.prompts.push_back(Prompt::Message {
                    title: "Notebook imported".into(),
                    body,
                });
                self.status = format!("Imported {} note(s).", report.total());
            }
            Err(error) => self.prompts.push_back(Prompt::Message {
                title: "Import failed".into(),
                body: format!("{error}\n\nThe original notebook was not changed."),
            }),
        }
    }
}

fn apply_loaded(tab: &mut Tab, loaded: document::LoadedFile) {
    tab.path = Some(loaded.path);
    tab.text = loaded.text.clone();
    tab.saved_text = loaded.text;
    tab.encoding = loaded.encoding;
    tab.utf16_bom = loaded.utf16_bom;
    tab.eol = loaded.eol;
    tab.saved_format = (loaded.encoding, loaded.utf16_bom, loaded.eol);
    tab.snapshot = Some(loaded.snapshot);
    tab.read_only = loaded.read_only;
    tab.invalid_sequences = loaded.invalid_sequences;
    tab.first_invalid_offset = loaded.first_invalid_offset;
    tab.contains_nul = loaded.contains_nul;
    tab.mixed_eol = loaded.eol_counts.is_mixed().then_some(loaded.eol_counts);
    tab.external = ExternalState::InSync;
    tab.recompute_dirty();
}

fn load_notice(tab: &Tab) -> String {
    let mut notes = Vec::new();
    if tab.invalid_sequences > 0 {
        notes.push(format!(
            "{} undecodable byte sequence(s) shown as \u{FFFD}",
            tab.invalid_sequences
        ));
    }
    if tab.contains_nul {
        notes.push("contains NUL bytes (binary file?)".into());
    }
    if tab.mixed_eol.is_some() {
        notes.push(format!(
            "mixed line endings; will save as {}",
            tab.eol.short()
        ));
    }
    if tab.read_only {
        notes.push("read-only".into());
    }
    if notes.is_empty() {
        format!(
            "Opened {} ({}, {})",
            tab.title(),
            tab.encoding.label(),
            tab.eol.short()
        )
    } else {
        format!("Opened {}: {}", tab.title(), notes.join("; "))
    }
}
