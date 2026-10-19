//! egui/eframe front end. All editor behaviour lives in [`crate::workspace`];
//! this module draws it: menu bar, tab strip, find/replace bar, notices, the
//! text area, status bar, and modal prompts.

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Color32, FontId, Key, KeyboardShortcut, Layout, RichText, TextBuffer,
    ViewportCommand,
};
use egui::text::{CCursor, CCursorRange, LayoutJob};
use egui::text_edit::TextEditState;

use crate::document::human_size;
use crate::encoding::TextEncoding;
use crate::eol::LineEnding;
use crate::search;
use crate::workspace::{
    Answer, ExternalState, FileDialogs, Prompt, Settings, ThemeChoice, Workspace,
};

pub const APP_NAME: &str = "NoteStrike";
pub const LOGO_PNG: &[u8] = include_bytes!("../assets/branding/NoteStrike.png");
pub const PWDS_LOGO_PNG: &[u8] = include_bytes!("../assets/branding/PWDS-logo.png");

/// Native file dialogs through `rfd`: the common item dialogs on Windows,
/// NSOpenPanel/NSSavePanel on macOS, and GTK 3 (or the XDG portal) on Linux.
pub struct NativeDialogs;

impl FileDialogs for NativeDialogs {
    fn open_files(&mut self) -> Option<Vec<PathBuf>> {
        rfd::FileDialog::new()
            .set_title("Open")
            .add_filter(
                "Text documents",
                &["txt", "md", "log", "ini", "csv", "json", "xml"],
            )
            .add_filter("All files", &["*"])
            .pick_files()
    }

    fn save_file(&mut self, suggested_name: &str, directory: Option<&Path>) -> Option<PathBuf> {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Save As")
            .set_file_name(suggested_name)
            .add_filter("Text documents", &["txt"])
            .add_filter("All files", &["*"]);
        if let Some(dir) = directory {
            dialog = dialog.set_directory(dir);
        }
        dialog.save_file()
    }

    fn pick_folder(&mut self, title: &str) -> Option<PathBuf> {
        rfd::FileDialog::new().set_title(title).pick_folder()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NoticeAction {
    Reload(Option<TextEncoding>),
    KeepMine,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClipboardOp {
    Cut,
    Copy,
    Paste,
}

/// One-shot capture of the rendered window (for documentation screenshots
/// produced by the real app, not mockups).
struct CaptureOnce {
    path: PathBuf,
    outcome: Arc<Mutex<Option<Result<(), String>>>>,
    started: Instant,
    settle_frames: u8,
    requested: bool,
}

pub struct NoteStrikeApp {
    pub ws: Workspace,
    dialogs: Box<dyn FileDialogs>,
    settings_path: Option<PathBuf>,
    saved_settings: Settings,
    applied_theme: Option<ThemeChoice>,
    setup_done: bool,
    logo: Option<egui::TextureHandle>,
    pwds_logo: Option<egui::TextureHandle>,
    goto_input: String,
    clipboard_request: Option<ClipboardOp>,
    focus_editor: bool,
    scroll_to_selection: bool,
    prompt_signature: Option<(usize, std::mem::Discriminant<Prompt>)>,
    capture: Option<CaptureOnce>,
    closing: bool,
}

const BASE_FONT_SIZE: f32 = 14.0;

/// Keyboard shortcuts. `Modifiers::COMMAND` is Ctrl on Windows and Linux and
/// Cmd on macOS. Where macOS reserves or conventionally uses a different key
/// (Cmd+H hides the app, Cmd+G is Find Next, Cmd+Tab switches apps), the Mac
/// binding follows the Mac convention. Menu labels are generated from these
/// definitions, so they always show the key that actually works.
pub mod keys {
    use eframe::egui::{Key, KeyboardShortcut, ModifierNames, Modifiers};

    pub const IS_MAC: bool = cfg!(target_os = "macos");

    const CMD: Modifiers = Modifiers::COMMAND;
    const CMD_SHIFT: Modifiers = Modifiers::COMMAND.plus(Modifiers::SHIFT);
    const fn cmd(key: Key) -> KeyboardShortcut {
        KeyboardShortcut::new(CMD, key)
    }
    const fn pick(mac: KeyboardShortcut, other: KeyboardShortcut) -> KeyboardShortcut {
        if IS_MAC { mac } else { other }
    }

    pub const NEW_TAB: KeyboardShortcut = cmd(Key::N);
    pub const NEW_TAB_ALT: KeyboardShortcut = cmd(Key::T);
    pub const OPEN: KeyboardShortcut = cmd(Key::O);
    pub const SAVE: KeyboardShortcut = cmd(Key::S);
    pub const SAVE_AS: KeyboardShortcut = KeyboardShortcut::new(CMD_SHIFT, Key::S);
    pub const CLOSE_TAB: KeyboardShortcut = cmd(Key::W);
    pub const CLOSE_TAB_ALT: KeyboardShortcut = cmd(Key::F4);
    /// Ctrl+Tab on every platform: Cmd+Tab is the macOS app switcher.
    pub const NEXT_TAB: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL, Key::Tab);
    pub const PREVIOUS_TAB: KeyboardShortcut =
        KeyboardShortcut::new(Modifiers::CTRL.plus(Modifiers::SHIFT), Key::Tab);
    pub const UNDO: KeyboardShortcut = cmd(Key::Z);
    pub const REDO: KeyboardShortcut = pick(KeyboardShortcut::new(CMD_SHIFT, Key::Z), cmd(Key::Y));
    pub const CUT: KeyboardShortcut = cmd(Key::X);
    pub const COPY: KeyboardShortcut = cmd(Key::C);
    pub const PASTE: KeyboardShortcut = cmd(Key::V);
    pub const SELECT_ALL: KeyboardShortcut = cmd(Key::A);
    pub const FIND: KeyboardShortcut = cmd(Key::F);
    pub const FIND_NEXT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F3);
    pub const FIND_PREVIOUS: KeyboardShortcut = KeyboardShortcut::new(Modifiers::SHIFT, Key::F3);
    /// Cmd+G / Cmd+Shift+G on macOS, in addition to F3 / Shift+F3.
    pub const FIND_NEXT_ALT: Option<KeyboardShortcut> =
        if IS_MAC { Some(cmd(Key::G)) } else { None };
    pub const FIND_PREVIOUS_ALT: Option<KeyboardShortcut> = if IS_MAC {
        Some(KeyboardShortcut::new(CMD_SHIFT, Key::G))
    } else {
        None
    };
    /// Cmd+Option+F on macOS (Cmd+H hides the app); Ctrl+H elsewhere.
    pub const REPLACE: KeyboardShortcut = pick(
        KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::ALT), Key::F),
        cmd(Key::H),
    );
    /// Cmd+L on macOS (Cmd+G is Find Next there); Ctrl+G elsewhere.
    pub const GO_TO_LINE: KeyboardShortcut = pick(cmd(Key::L), cmd(Key::G));
    pub const ZOOM_IN: KeyboardShortcut = cmd(Key::Plus);
    pub const ZOOM_IN_ALT: KeyboardShortcut = cmd(Key::Equals);
    pub const ZOOM_OUT: KeyboardShortcut = cmd(Key::Minus);
    pub const ZOOM_RESET: KeyboardShortcut = cmd(Key::Num0);

    /// "Ctrl+S" on Windows and Linux, "Cmd+S" on macOS.
    pub fn label(shortcut: KeyboardShortcut) -> String {
        shortcut.format(&ModifierNames::NAMES, IS_MAC)
    }

    /// How the window is closed from the keyboard on this platform.
    pub fn quit_label() -> &'static str {
        if IS_MAC { "Cmd+Q" } else { "Alt+F4" }
    }

    /// The Help > Keyboard shortcuts text for this platform.
    pub fn help_text() -> String {
        let l = label;
        let mut rows = vec![
            (format!("{} / {}", l(NEW_TAB), l(NEW_TAB_ALT)), "New tab"),
            (l(OPEN), "Open"),
            (l(SAVE), "Save"),
            (l(SAVE_AS), "Save as"),
            (l(CLOSE_TAB), "Close tab"),
            (
                format!("{} / {}", l(NEXT_TAB), l(PREVIOUS_TAB)),
                "Next / previous tab",
            ),
            (format!("{} / {}", l(UNDO), l(REDO)), "Undo / redo"),
            (
                format!("{} / {} / {}", l(CUT), l(COPY), l(PASTE)),
                "Cut / copy / paste",
            ),
            (l(SELECT_ALL), "Select all"),
            (l(FIND), "Find"),
        ];
        let mut next = format!("{} / {}", l(FIND_NEXT), l(FIND_PREVIOUS));
        if let (Some(n), Some(p)) = (FIND_NEXT_ALT, FIND_PREVIOUS_ALT) {
            next = format!("{next} or {} / {}", l(n), l(p));
        }
        rows.push((next, "Find next / previous"));
        rows.push((l(REPLACE), "Replace"));
        rows.push((l(GO_TO_LINE), "Go to line"));
        rows.push((
            format!("{} / {} / {}", l(ZOOM_IN), l(ZOOM_OUT), l(ZOOM_RESET)),
            "Zoom in / out / reset",
        ));
        rows.push(("Esc".into(), "Close the find bar or a dialog"));
        rows.push((quit_label().into(), "Exit"));
        rows.iter()
            .map(|(keys, action)| format!("{keys}\t{action}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Map the persisted theme setting to egui's preference. `System` lets egui
/// follow the operating system's light/dark setting (reported by winit) and
/// falls back to egui's default when the OS does not report one.
pub fn theme_preference(choice: ThemeChoice) -> egui::ThemePreference {
    match choice {
        ThemeChoice::System => egui::ThemePreference::System,
        ThemeChoice::Light => egui::ThemePreference::Light,
        ThemeChoice::Dark => egui::ThemePreference::Dark,
    }
}

/// Colours that NoteStrike sets on top of egui's light and dark visuals.
/// Every text/background pair is checked against WCAG AA in the tests below.
pub mod palette {
    use eframe::egui::{Color32, Theme};

    pub struct Palette {
        pub panel: Color32,
        pub editor: Color32,
        /// Editor text override (`None` keeps egui's text colour).
        pub editor_text: Option<Color32>,
        pub weak_text: Color32,
        pub warn_text: Color32,
        pub error_text: Color32,
        pub notice_bg: Color32,
        pub notice_text: Color32,
        pub warn_icon: Color32,
        /// How strongly the accent tints the text selection.
        pub selection_strength: f32,
    }

    pub const LIGHT: Palette = Palette {
        panel: Color32::from_rgb(243, 243, 243),
        editor: Color32::WHITE,
        editor_text: None,
        weak_text: Color32::from_rgb(0x5F, 0x5F, 0x5F),
        warn_text: Color32::from_rgb(0x9A, 0x34, 0x12),
        error_text: Color32::from_rgb(0xB9, 0x1C, 0x1C),
        notice_bg: Color32::from_rgb(0xFF, 0xF4, 0xE5),
        notice_text: Color32::from_rgb(0x1F, 0x1F, 0x1F),
        warn_icon: Color32::from_rgb(0x92, 0x40, 0x0E),
        selection_strength: 0.3,
    };

    pub const DARK: Palette = Palette {
        panel: Color32::from_rgb(32, 32, 32),
        editor: Color32::from_rgb(24, 24, 24),
        editor_text: Some(Color32::from_rgb(0xD4, 0xD4, 0xD4)),
        weak_text: Color32::from_rgb(0xA0, 0xA0, 0xA0),
        warn_text: Color32::from_rgb(0xFB, 0xBF, 0x24),
        error_text: Color32::from_rgb(0xF8, 0x71, 0x71),
        notice_bg: Color32::from_rgb(0x2B, 0x24, 0x14),
        notice_text: Color32::from_rgb(0xF0, 0xF0, 0xF0),
        warn_icon: Color32::from_rgb(0xFA, 0xCC, 0x15),
        selection_strength: 0.45,
    };

    pub fn for_theme(theme: Theme) -> &'static Palette {
        for_dark(theme == Theme::Dark)
    }

    pub fn for_dark(dark: bool) -> &'static Palette {
        if dark { &DARK } else { &LIGHT }
    }
}

impl NoteStrikeApp {
    pub fn new(
        ws: Workspace,
        dialogs: Box<dyn FileDialogs>,
        settings_path: Option<PathBuf>,
    ) -> Self {
        let saved_settings = ws.settings.clone();
        Self {
            ws,
            dialogs,
            settings_path,
            saved_settings,
            applied_theme: None,
            setup_done: false,
            logo: None,
            pwds_logo: None,
            goto_input: String::new(),
            clipboard_request: None,
            focus_editor: true,
            scroll_to_selection: false,
            prompt_signature: None,
            capture: None,
            closing: false,
        }
    }

    /// Capture one fully painted frame to `path`, then close the window.
    pub fn capture_once(&mut self, path: PathBuf) -> Arc<Mutex<Option<Result<(), String>>>> {
        let outcome = Arc::new(Mutex::new(None));
        self.capture = Some(CaptureOnce {
            path,
            outcome: Arc::clone(&outcome),
            started: Instant::now(),
            settle_frames: 0,
            requested: false,
        });
        outcome
    }

    fn setup(&mut self, ctx: &egui::Context) {
        if self.setup_done {
            return;
        }
        self.setup_done = true;
        install_system_fonts(ctx);
        // winit reports the OS theme on Windows, macOS and some Wayland
        // compositors, but never on X11. Ask the desktop instead, so System
        // does not fall back to dark on a light Linux desktop.
        #[cfg(all(unix, not(target_os = "macos")))]
        if let Some(theme) = desktop_theme::detect() {
            ctx.options_mut(|o| o.fallback_theme = theme);
        }
        // Ctrl +/- zoom the text, not the whole interface.
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            ctx.style_mut_of(theme, |style| {
                style.spacing.item_spacing = egui::vec2(8.0, 6.0);
                style.spacing.button_padding = egui::vec2(8.0, 4.0);
                style.spacing.menu_margin = egui::Margin::same(6);
                style
                    .text_styles
                    .insert(egui::TextStyle::Body, FontId::proportional(14.0));
                style
                    .text_styles
                    .insert(egui::TextStyle::Button, FontId::proportional(14.0));
                style
                    .text_styles
                    .insert(egui::TextStyle::Small, FontId::proportional(12.0));
                style
                    .text_styles
                    .insert(egui::TextStyle::Heading, FontId::proportional(20.0));
                let accent = if theme == egui::Theme::Light {
                    Color32::from_rgb(0, 95, 184)
                } else {
                    Color32::from_rgb(96, 165, 250)
                };
                style.visuals.selection.bg_fill =
                    accent.gamma_multiply(palette::for_theme(theme).selection_strength);
                style.visuals.hyperlink_color = accent;
                let p = palette::for_theme(theme);
                style.visuals.panel_fill = p.panel;
                style.visuals.extreme_bg_color = p.editor;
                // egui's default weak and warning colours miss WCAG AA on
                // these backgrounds; use the checked palette instead.
                style.visuals.weak_text_color = Some(p.weak_text);
                style.visuals.warn_fg_color = p.warn_text;
                style.visuals.error_fg_color = p.error_text;
            });
        }
        self.logo = load_texture(ctx, "notestrike-logo", LOGO_PNG);
        self.pwds_logo = load_texture(ctx, "pwds-logo", PWDS_LOGO_PNG);
    }

    fn apply_theme(&mut self, ctx: &egui::Context) {
        if self.applied_theme == Some(self.ws.settings.theme) {
            return;
        }
        self.applied_theme = Some(self.ws.settings.theme);
        ctx.set_theme(theme_preference(self.ws.settings.theme));
    }

    fn persist_settings(&mut self) {
        if self.ws.settings != self.saved_settings {
            if let Some(path) = &self.settings_path {
                let _ = self.ws.settings.save(path);
            }
            self.saved_settings = self.ws.settings.clone();
        }
    }

    fn zoom(&mut self, delta: i32) {
        let z = &mut self.ws.settings.zoom_percent;
        *z = if delta == 0 {
            100
        } else {
            (*z as i32 + delta).clamp(Settings::ZOOM_MIN as i32, Settings::ZOOM_MAX as i32) as u32
        };
    }

    fn editor_id(tab_id: u64) -> egui::Id {
        egui::Id::new(("notestrike-editor", tab_id))
    }

    fn undo_redo(&mut self, ctx: &egui::Context, redo: bool) {
        let Some(tab) = self.ws.active_tab() else {
            return;
        };
        let (tab_id, id) = (tab.id, Self::editor_id(tab.id));
        let Some(mut state) = TextEditState::load(ctx, id) else {
            return;
        };
        let mut undoer = state.undoer();
        let current = (
            state.cursor.char_range().unwrap_or_default(),
            tab.text.clone(),
        );
        let restored = if redo {
            undoer.redo(&current).cloned()
        } else {
            undoer.undo(&current).cloned()
        };
        if let Some((range, text)) = restored {
            if let Some(tab) = self.ws.active_tab_mut() {
                tab.text = text;
            }
            state.cursor.set_char_range(Some(range));
            self.ws.text_edited(tab_id);
        }
        state.set_undoer(undoer);
        state.store(ctx, id);
        self.focus_editor = true;
    }

    // ----- keyboard ------------------------------------------------------

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if self.ws.current_prompt().is_some() {
            return;
        }
        let pressed = |shortcut: KeyboardShortcut| ctx.input_mut(|i| i.consume_shortcut(&shortcut));
        // egui matches Shift and Alt loosely, so the more specific shortcut
        // must be tested first (Cmd/Ctrl+Shift+S before Cmd/Ctrl+S).
        if pressed(keys::SAVE_AS) {
            self.ws.save_active(true, self.dialogs.as_mut());
        } else if pressed(keys::SAVE) {
            self.ws.save_active(false, self.dialogs.as_mut());
        }
        if pressed(keys::NEW_TAB) || pressed(keys::NEW_TAB_ALT) {
            self.ws.new_tab();
            self.focus_editor = true;
        }
        if pressed(keys::OPEN) {
            self.ws.open_dialog(self.dialogs.as_mut());
        }
        if (pressed(keys::CLOSE_TAB) || pressed(keys::CLOSE_TAB_ALT))
            && let Some(id) = self.ws.active_tab().map(|t| t.id)
        {
            self.ws.request_close_tab(id);
        }
        // Replace (Cmd+Option+F on macOS) before Find (Cmd+F).
        if pressed(keys::REPLACE) {
            self.ws.show_find(true);
        }
        if pressed(keys::FIND) {
            self.ws.show_find(false);
        }
        if pressed(keys::FIND_PREVIOUS) || keys::FIND_PREVIOUS_ALT.is_some_and(pressed) {
            self.find_and_show(false);
        } else if pressed(keys::FIND_NEXT) || keys::FIND_NEXT_ALT.is_some_and(pressed) {
            self.find_and_show(true);
        }
        if pressed(keys::GO_TO_LINE) {
            self.goto_input.clear();
            self.ws.prompts.push_back(Prompt::GoToLine);
        }
        if pressed(keys::PREVIOUS_TAB) {
            self.ws.next_tab(false);
            self.focus_editor = true;
        } else if pressed(keys::NEXT_TAB) {
            self.ws.next_tab(true);
            self.focus_editor = true;
        }
        if pressed(keys::ZOOM_IN) || pressed(keys::ZOOM_IN_ALT) {
            self.zoom(10);
        }
        if pressed(keys::ZOOM_OUT) {
            self.zoom(-10);
        }
        if pressed(keys::ZOOM_RESET) {
            self.zoom(0);
        }
        if self.ws.find.visible && ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.ws.find.visible = false;
            self.focus_editor = true;
        }
    }

    fn find_and_show(&mut self, forward: bool) {
        if self.ws.find_step(forward) {
            self.focus_editor = true;
        }
    }

    // ----- menus ---------------------------------------------------------

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.add(egui::Button::new("New tab").shortcut_text(keys::label(keys::NEW_TAB))).clicked() {
                    self.ws.new_tab();
                    self.focus_editor = true;
                }
                if ui.add(egui::Button::new("Open…").shortcut_text(keys::label(keys::OPEN))).clicked() {
                    ui.close();
                    self.ws.open_dialog(self.dialogs.as_mut());
                }
                ui.separator();
                if ui.add(egui::Button::new("Save").shortcut_text(keys::label(keys::SAVE))).clicked() {
                    ui.close();
                    self.ws.save_active(false, self.dialogs.as_mut());
                }
                if ui.add(egui::Button::new("Save as…").shortcut_text(keys::label(keys::SAVE_AS))).clicked() {
                    ui.close();
                    self.ws.save_active(true, self.dialogs.as_mut());
                }
                if ui.button("Save all").clicked() {
                    ui.close();
                    self.ws.save_all(self.dialogs.as_mut());
                }
                ui.separator();
                let has_path = self.ws.active_tab().is_some_and(|t| t.path.is_some());
                ui.add_enabled_ui(has_path, |ui| {
                    if ui.button("Reload from disk").clicked() {
                        ui.close();
                        if let Some(id) = self.ws.active_tab().map(|t| t.id) {
                            self.ws.request_reload(id, None);
                        }
                    }
                    ui.menu_button("Reopen with encoding", |ui| {
                        for enc in TextEncoding::ALL {
                            if ui.button(enc.label()).clicked() {
                                ui.close();
                                if let Some(id) = self.ws.active_tab().map(|t| t.id) {
                                    self.ws.request_reload(id, Some(enc));
                                }
                            }
                        }
                    });
                });
                ui.separator();
                if ui
                    .button("Import NoteStrike 0.1 notebook…")
                    .on_hover_text("Copy notes from an old NoteStrike notebook folder into plain .md files. The notebook is not changed.")
                    .clicked()
                {
                    ui.close();
                    self.ws.import_notebook(self.dialogs.as_mut());
                }
                ui.separator();
                if ui.add(egui::Button::new("Close tab").shortcut_text(keys::label(keys::CLOSE_TAB))).clicked() {
                    ui.close();
                    if let Some(id) = self.ws.active_tab().map(|t| t.id) {
                        self.ws.request_close_tab(id);
                    }
                }
                if ui.add(egui::Button::new(if keys::IS_MAC { "Quit NoteStrike" } else { "Exit" }).shortcut_text(keys::quit_label())).clicked() {
                    ui.close();
                    self.ws.request_quit();
                }
            });
            ui.menu_button("Edit", |ui| {
                if ui.add(egui::Button::new("Undo").shortcut_text(keys::label(keys::UNDO))).clicked() {
                    ui.close();
                    self.undo_redo(ui.ctx(), false);
                }
                if ui.add(egui::Button::new("Redo").shortcut_text(keys::label(keys::REDO))).clicked() {
                    ui.close();
                    self.undo_redo(ui.ctx(), true);
                }
                ui.separator();
                for (label, shortcut, op) in [
                    ("Cut", keys::CUT, ClipboardOp::Cut),
                    ("Copy", keys::COPY, ClipboardOp::Copy),
                    ("Paste", keys::PASTE, ClipboardOp::Paste),
                ] {
                    if ui.add(egui::Button::new(label).shortcut_text(keys::label(shortcut))).clicked() {
                        ui.close();
                        self.clipboard_request = Some(op);
                    }
                }
                if ui.add(egui::Button::new("Select all").shortcut_text(keys::label(keys::SELECT_ALL))).clicked() {
                    ui.close();
                    if let Some(tab) = self.ws.active_tab() {
                        let end = tab.text.chars().count();
                        self.ws.selection_request = Some((tab.id, 0..end));
                    }
                }
                ui.separator();
                if ui.add(egui::Button::new("Find…").shortcut_text(keys::label(keys::FIND))).clicked() {
                    ui.close();
                    self.ws.show_find(false);
                }
                if ui.add(egui::Button::new("Find next").shortcut_text(keys::label(keys::FIND_NEXT))).clicked() {
                    ui.close();
                    self.find_and_show(true);
                }
                if ui.add(egui::Button::new("Find previous").shortcut_text(keys::label(keys::FIND_PREVIOUS))).clicked() {
                    ui.close();
                    self.find_and_show(false);
                }
                if ui.add(egui::Button::new("Replace…").shortcut_text(keys::label(keys::REPLACE))).clicked() {
                    ui.close();
                    self.ws.show_find(true);
                }
                if ui.add(egui::Button::new("Go to line…").shortcut_text(keys::label(keys::GO_TO_LINE))).clicked() {
                    ui.close();
                    self.goto_input.clear();
                    self.ws.prompts.push_back(Prompt::GoToLine);
                }
            });
            ui.menu_button("Format", |ui| {
                ui.checkbox(&mut self.ws.settings.word_wrap, "Word wrap");
                ui.separator();
                let Some(tab_id) = self.ws.active_tab().map(|t| t.id) else {
                    return;
                };
                let (current_eol, current_enc) = self
                    .ws
                    .active_tab()
                    .map(|t| (t.eol, t.encoding))
                    .unwrap_or((LineEnding::platform_default(), TextEncoding::Utf8));
                ui.menu_button("Line endings", |ui| {
                    for eol in LineEnding::ALL {
                        if ui.radio(current_eol == eol, eol.label()).clicked() {
                            ui.close();
                            self.ws.set_eol(tab_id, eol);
                        }
                    }
                });
                ui.menu_button("Save with encoding", |ui| {
                    for enc in TextEncoding::ALL {
                        if ui.radio(current_enc == enc, enc.label()).clicked() {
                            ui.close();
                            self.ws.set_encoding(tab_id, enc);
                        }
                    }
                });
            });
            ui.menu_button("View", |ui| {
                ui.menu_button("Zoom", |ui| {
                    if ui.add(egui::Button::new("Zoom in").shortcut_text(keys::label(keys::ZOOM_IN))).clicked() {
                        self.zoom(10);
                    }
                    if ui.add(egui::Button::new("Zoom out").shortcut_text(keys::label(keys::ZOOM_OUT))).clicked() {
                        self.zoom(-10);
                    }
                    if ui.add(egui::Button::new("Restore default zoom").shortcut_text(keys::label(keys::ZOOM_RESET))).clicked() {
                        self.zoom(0);
                    }
                });
                ui.checkbox(&mut self.ws.settings.status_bar, "Status bar");
                ui.menu_button("Theme", |ui| {
                    for (choice, label) in [
                        (ThemeChoice::System, SYSTEM_THEME_LABEL),
                        (ThemeChoice::Light, "Light"),
                        (ThemeChoice::Dark, "Dark"),
                    ] {
                        if ui.radio(self.ws.settings.theme == choice, label).clicked() {
                            // Applied on the next frame and saved to settings.json.
                            self.ws.settings.theme = choice;
                            ui.close();
                        }
                    }
                });
            });
            ui.menu_button("Help", |ui| {
                if ui.button("Keyboard shortcuts").clicked() {
                    ui.close();
                    self.ws.prompts.push_back(Prompt::Message {
                        title: "Keyboard shortcuts".into(),
                        body: keys::help_text(),
                    });
                }
                if ui.button("About NoteStrike").clicked() {
                    ui.close();
                    self.ws.prompts.push_back(Prompt::About);
                }
            });
        });
    }

    // ----- tab strip -----------------------------------------------------

    fn tab_strip(&mut self, ui: &mut egui::Ui) {
        let mut activate = None;
        let mut close = None;
        let mut add = false;
        egui::ScrollArea::horizontal()
            .id_salt("notestrike-tabs")
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for (index, tab) in self.ws.tabs.iter().enumerate() {
                        let selected = index == self.ws.active;
                        let title = tab.title();
                        let dirty = tab.is_dirty();
                        let fill = if selected {
                            ui.visuals().extreme_bg_color
                        } else {
                            Color32::TRANSPARENT
                        };
                        let frame = egui::Frame::NONE
                            .fill(fill)
                            .corner_radius(egui::CornerRadius {
                                nw: 6,
                                ne: 6,
                                sw: 0,
                                se: 0,
                            })
                            .inner_margin(egui::Margin::symmetric(8, 4));
                        frame.show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let label = if dirty {
                                    format!("● {title}")
                                } else {
                                    title.clone()
                                };
                                let text = if selected {
                                    RichText::new(label).strong()
                                } else {
                                    RichText::new(label)
                                };
                                let response = ui
                                    .add(
                                        egui::Button::selectable(selected, text)
                                            .frame_when_inactive(false),
                                    )
                                    .on_hover_text(match &tab.path {
                                        Some(p) => p.display().to_string(),
                                        None => "Not saved yet".into(),
                                    });
                                let accessible = format!(
                                    "Tab {} of {}: {title}{}",
                                    index + 1,
                                    self.ws.tabs.len(),
                                    if dirty { ", modified" } else { "" }
                                );
                                response.widget_info(|| {
                                    egui::WidgetInfo::selected(
                                        egui::WidgetType::SelectableLabel,
                                        true,
                                        selected,
                                        &accessible,
                                    )
                                });
                                if response.clicked() {
                                    activate = Some(tab.id);
                                }
                                if response.middle_clicked() {
                                    close = Some(tab.id);
                                }
                                let close_btn = ui
                                    .add(
                                        egui::Button::new(RichText::new("×").size(15.0))
                                            .frame_when_inactive(false)
                                            .small(),
                                    )
                                    .on_hover_text(format!(
                                        "Close tab ({})",
                                        keys::label(keys::CLOSE_TAB)
                                    ));
                                let close_name = format!("Close {title}");
                                close_btn.widget_info(|| {
                                    egui::WidgetInfo::labeled(
                                        egui::WidgetType::Button,
                                        true,
                                        &close_name,
                                    )
                                });
                                if close_btn.clicked() {
                                    close = Some(tab.id);
                                }
                            });
                        });
                    }
                    let plus = ui
                        .add(
                            egui::Button::new(RichText::new("+").size(16.0))
                                .frame_when_inactive(false),
                        )
                        .on_hover_text(format!("New tab ({})", keys::label(keys::NEW_TAB)));
                    plus.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "New tab")
                    });
                    if plus.clicked() {
                        add = true;
                    }
                });
            });
        if let Some(id) = activate {
            self.ws.activate(id);
            self.focus_editor = true;
        }
        if let Some(id) = close {
            self.ws.request_close_tab(id);
        }
        if add {
            self.ws.new_tab();
            self.focus_editor = true;
        }
    }

    // ----- find bar ------------------------------------------------------

    fn find_bar(&mut self, ui: &mut egui::Ui) {
        let narrow = ui.available_width() < 640.0;
        let field_width = if narrow {
            (ui.available_width() - 24.0).max(120.0)
        } else {
            220.0
        };
        ui.horizontal_wrapped(|ui| {
            let find_label = ui.label("Find");
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.ws.find.needle)
                    .desired_width(field_width)
                    .hint_text("Find"),
            );
            let response = response.labelled_by(find_label.id);
            if self.ws.find.focus_request {
                response.request_focus();
                self.ws.find.focus_request = false;
            }
            if response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                let backwards = ui.input(|i| i.modifiers.shift);
                self.ws.find_step(!backwards);
                self.scroll_to_selection = true;
            }
            if ui
                .button("↑")
                .on_hover_text("Find previous (Shift+F3)")
                .clicked()
            {
                self.ws.find_step(false);
                self.scroll_to_selection = true;
            }
            if ui.button("↓").on_hover_text("Find next (F3)").clicked() {
                self.ws.find_step(true);
                self.scroll_to_selection = true;
            }
            ui.checkbox(&mut self.ws.find.options.match_case, "Match case");
            ui.checkbox(&mut self.ws.find.options.whole_word, "Whole word");
            ui.checkbox(&mut self.ws.find.options.wrap_around, "Wrap around");
            if let Some(tab) = self.ws.active_tab()
                && !self.ws.find.needle.is_empty()
            {
                let n =
                    search::count_matches(&tab.text, &self.ws.find.needle, self.ws.find.options);
                ui.label(
                    RichText::new(format!("{n} match{}", if n == 1 { "" } else { "es" })).weak(),
                );
            }
            if ui
                .button(if self.ws.find.replace_visible {
                    "Hide replace"
                } else {
                    "Replace…"
                })
                .clicked()
            {
                self.ws.find.replace_visible = !self.ws.find.replace_visible;
            }
            if ui.button("×").on_hover_text("Close (Esc)").clicked() {
                self.ws.find.visible = false;
                self.focus_editor = true;
            }
        });
        if self.ws.find.replace_visible {
            ui.horizontal_wrapped(|ui| {
                let label = ui.label("Replace");
                ui.add(
                    egui::TextEdit::singleline(&mut self.ws.find.replacement)
                        .desired_width(field_width)
                        .hint_text("Replace with"),
                )
                .labelled_by(label.id);
                if ui.button("Replace").clicked() {
                    self.ws.replace_step();
                    self.scroll_to_selection = true;
                }
                if ui.button("Replace all").clicked() {
                    self.ws.replace_all();
                }
            });
        }
    }

    // ----- notices -------------------------------------------------------

    fn notices(&mut self, ui: &mut egui::Ui) {
        let Some(tab) = self.ws.active_tab() else {
            return;
        };
        let id = tab.id;
        let p = palette::for_dark(ui.visuals().dark_mode);
        let (warn, notice_text) = (p.warn_icon, p.notice_text);
        let mut action: Option<NoticeAction> = None;
        let notice = |ui: &mut egui::Ui, text: String| {
            ui.label(RichText::new("⚠").color(warn));
            ui.label(RichText::new(text).color(notice_text));
        };
        match tab.external {
            ExternalState::ChangedOnDisk => {
                ui.horizontal_wrapped(|ui| {
                    notice(ui, "This file was changed by another program.".into());
                    if ui.button("Reload").clicked() {
                        action = Some(NoticeAction::Reload(None));
                    }
                    if ui.button("Keep my version").clicked() {
                        action = Some(NoticeAction::KeepMine);
                    }
                });
            }
            ExternalState::MissingOnDisk => {
                ui.horizontal_wrapped(|ui| {
                    notice(
                        ui,
                        "This file was deleted, renamed, or moved. Your text is still here.".into(),
                    );
                    if ui.button("Keep editing").clicked() {
                        action = Some(NoticeAction::KeepMine);
                    }
                });
            }
            ExternalState::InSync => {}
        }
        if tab.invalid_sequences > 0 {
            let enc = tab.encoding;
            ui.horizontal_wrapped(|ui| {
                notice(
                    ui,
                    format!(
                        "{} byte sequence(s) could not be decoded as {} and are shown as \u{FFFD}{}. Saving would replace them permanently.",
                        tab.invalid_sequences,
                        enc.label(),
                        tab.first_invalid_offset.map(|o| format!(" (first at byte {o})")).unwrap_or_default()
                    ),
                );
                for alt in [TextEncoding::Windows1252, TextEncoding::Utf8, TextEncoding::Utf16Le] {
                    if alt != enc && ui.button(format!("Reopen as {}", alt.label())).clicked() {
                        action = Some(NoticeAction::Reload(Some(alt)));
                    }
                }
            });
        }
        if tab.contains_nul {
            ui.horizontal_wrapped(|ui| notice(ui, "This file contains NUL bytes and may be binary. Saving it as text can damage it.".into()));
        }
        if tab.read_only {
            ui.horizontal_wrapped(|ui| {
                notice(
                    ui,
                    "This file is read-only. Use Save As to keep your changes in a new file."
                        .into(),
                )
            });
        }
        if let Some(counts) = tab.mixed_eol {
            ui.horizontal_wrapped(|ui| {
                notice(
                    ui,
                    format!(
                        "Mixed line endings ({} CRLF, {} LF, {} CR). Saving will use {}.",
                        counts.crlf,
                        counts.lf,
                        counts.cr,
                        tab.eol.short()
                    ),
                );
            });
        }
        match action {
            Some(NoticeAction::Reload(encoding)) => self.ws.request_reload(id, encoding),
            Some(NoticeAction::KeepMine) => self.ws.keep_my_version(id),
            None => {}
        }
    }

    // ----- editor --------------------------------------------------------

    fn editor(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let font = FontId::monospace(BASE_FONT_SIZE * self.ws.settings.zoom_percent as f32 / 100.0);
        let wrap = self.ws.settings.word_wrap;
        let Some(index) = self.ws.tabs.get(self.ws.active).map(|_| self.ws.active) else {
            return;
        };
        let tab_id = self.ws.tabs[index].id;
        let title = self.ws.tabs[index].title();
        let id = Self::editor_id(tab_id);

        let mut scroll_now = std::mem::take(&mut self.scroll_to_selection);
        if let Some((target, range)) = self.ws.selection_request.take()
            && target == tab_id
        {
            // egui collapses the selection of an unfocused TextEdit, so focus
            // the editor first; the match is then shown selected.
            ctx.memory_mut(|m| m.request_focus(id));
            let mut state = TextEditState::load(&ctx, id).unwrap_or_default();
            state.cursor.set_char_range(Some(CCursorRange::two(
                CCursor::new(range.start),
                CCursor::new(range.end),
            )));
            state.store(&ctx, id);
            self.focus_editor = true;
            scroll_now = true;
        }

        let text_color = palette::for_dark(ui.visuals().dark_mode)
            .editor_text
            .unwrap_or_else(|| ui.visuals().text_color());
        let mut layouter = |ui: &egui::Ui, buf: &dyn TextBuffer, wrap_width: f32| {
            let job = LayoutJob::simple(
                buf.as_str().to_owned(),
                font.clone(),
                text_color,
                if wrap { wrap_width } else { f32::INFINITY },
            );
            ui.fonts_mut(|f| f.layout_job(job))
        };

        let row_height = font.size * 1.25;
        let available = ui.available_size();
        let rows = ((available.y / row_height).floor() as usize).max(1);
        let scroll = if wrap {
            egui::ScrollArea::vertical()
        } else {
            egui::ScrollArea::both()
        };
        let clipboard = self.clipboard_request.take();
        let focus = std::mem::take(&mut self.focus_editor);
        let tab = &mut self.ws.tabs[index];
        let output = scroll
            .id_salt(("notestrike-scroll", tab_id))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let edit = egui::TextEdit::multiline(&mut tab.text)
                    .id(id)
                    .font(font.clone())
                    .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(10, 8)))
                    .lock_focus(true)
                    .desired_rows(rows)
                    .desired_width(if wrap {
                        ui.available_width()
                    } else {
                        f32::INFINITY
                    })
                    .min_size(available)
                    .layouter(&mut layouter);
                let output = edit.show(ui);
                if scroll_now && let Some(range) = output.cursor_range {
                    let rect = output
                        .galley
                        .pos_from_cursor(range.primary)
                        .translate(output.galley_pos.to_vec2())
                        .expand(row_height);
                    ui.scroll_to_rect(rect, Some(Align::Center));
                }
                output
            })
            .inner;
        let response = &output.response.response;
        if focus || clipboard.is_some() {
            response.request_focus();
        }
        if let Some(op) = clipboard {
            ctx.send_viewport_cmd(match op {
                ClipboardOp::Cut => ViewportCommand::RequestCut,
                ClipboardOp::Copy => ViewportCommand::RequestCopy,
                ClipboardOp::Paste => ViewportCommand::RequestPaste,
            });
        }
        ctx.accesskit_node_builder(response.id, |node| {
            node.set_label(format!("Text editor: {title}"));
        });
        if response.changed() {
            self.ws.text_edited(tab_id);
        }
        if let Some(range) = output.cursor_range {
            let (a, b) = (range.primary.index, range.secondary.index);
            self.ws.selection = (a.min(b).into(), a.max(b).into());
        }
    }

    // ----- status bar ----------------------------------------------------

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let Some(tab) = self.ws.active_tab() else {
            return;
        };
        let width = ui.available_width();
        let (line, col) = search::line_col(&tab.text, self.ws.selection.1);
        let chars = tab.text.chars().count();
        let (tab_id, eol, enc) = (tab.id, tab.eol, tab.encoding);
        let status = self.ws.status.clone();
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.menu_button(enc.label(), |ui| {
                ui.label(RichText::new("Save with encoding").small().weak());
                for candidate in TextEncoding::ALL {
                    if ui.radio(candidate == enc, candidate.label()).clicked() {
                        ui.close();
                        self.ws.set_encoding(tab_id, candidate);
                    }
                }
            })
            .response
            .on_hover_text("Encoding used when saving. To decode the file differently, use File > Reopen with encoding.");
            ui.separator();
            ui.menu_button(eol.label(), |ui| {
                for candidate in LineEnding::ALL {
                    if ui.radio(candidate == eol, candidate.label()).clicked() {
                        ui.close();
                        self.ws.set_eol(tab_id, candidate);
                    }
                }
            })
            .response
            .on_hover_text("Line endings used when saving");
            ui.separator();
            if width > 560.0 {
                ui.label(format!("{}%", self.ws.settings.zoom_percent))
                    .on_hover_text(format!("Zoom ({} / {} / {})", keys::label(keys::ZOOM_IN), keys::label(keys::ZOOM_OUT), keys::label(keys::ZOOM_RESET)));
                ui.separator();
            }
            if width > 700.0 {
                ui.label(format!("{chars} characters"));
                ui.separator();
            }
            ui.label(format!("Ln {line}, Col {col}"));
            ui.separator();
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.add(egui::Label::new(RichText::new(status).small()).truncate());
            });
        });
    }

    // ----- prompts -------------------------------------------------------

    fn prompts(&mut self, ctx: &egui::Context) {
        let Some(prompt) = self.ws.current_prompt().cloned() else {
            self.prompt_signature = None;
            return;
        };
        let signature = (self.ws.prompts.len(), std::mem::discriminant(&prompt));
        let first_frame = self.prompt_signature != Some(signature);
        self.prompt_signature = Some(signature);
        let title_of = |ws: &Workspace, id: u64| ws.tab(id).map(|t| t.title()).unwrap_or_default();
        let mut answer: Option<Answer> = None;
        let modal = egui::Modal::new(egui::Id::new("notestrike-prompt")).show(ctx, |ui| {
            ui.set_max_width(460.0);
            let buttons: Vec<(&str, Answer)> = match &prompt {
                Prompt::UnsavedTab { tab } => {
                    ui.heading("Unsaved changes");
                    ui.label(format!("Do you want to save changes to {}?", title_of(&self.ws, *tab)));
                    vec![("Save", Answer::Save), ("Don't save", Answer::DontSave), ("Cancel", Answer::Cancel)]
                }
                Prompt::UnsavedOnQuit { tabs } => {
                    ui.heading("Unsaved changes");
                    ui.label(format!("{} document(s) have unsaved changes:", tabs.len()));
                    for id in tabs {
                        ui.label(format!("  •  {}", title_of(&self.ws, *id)));
                    }
                    vec![("Save all", Answer::SaveAll), ("Don't save", Answer::DiscardAll), ("Cancel", Answer::Cancel)]
                }
                Prompt::ChangedOnDisk { save } => {
                    ui.heading("File changed on disk");
                    ui.label(format!(
                        "{} was changed or removed by another program after you opened it. Overwrite it with your version, save your version under a new name, or cancel.",
                        title_of(&self.ws, save.tab)
                    ));
                    vec![("Overwrite", Answer::Overwrite), ("Save as…", Answer::SaveAs), ("Cancel", Answer::Cancel)]
                }
                Prompt::LossySave { save } => {
                    let tab = self.ws.tab(save.tab);
                    ui.heading("Undecodable bytes");
                    ui.label(format!(
                        "{} contains {} byte sequence(s) that could not be decoded as {}. They are shown as \u{FFFD}, and saving writes \u{FFFD} in their place permanently.\n\nTo keep the original bytes, cancel and use File > Reopen with encoding.",
                        title_of(&self.ws, save.tab),
                        tab.map_or(0, |t| t.invalid_sequences),
                        tab.map_or("UTF-8", |t| t.encoding.label())
                    ));
                    vec![("Save anyway", Answer::SaveAnyway), ("Save as…", Answer::SaveAs), ("Cancel", Answer::Cancel)]
                }
                Prompt::Unrepresentable { details, .. } => {
                    ui.heading("Characters cannot be saved");
                    ui.label(format!("{details}. Nothing was written."));
                    vec![("Save as UTF-8", Answer::UseUtf8), ("Cancel", Answer::Cancel)]
                }
                Prompt::ReadOnly { save } => {
                    ui.heading("Read-only file");
                    ui.label(format!(
                        "{} is read-only. Save your changes under a new name?",
                        title_of(&self.ws, save.tab)
                    ));
                    vec![("Save as…", Answer::SaveAs), ("Cancel", Answer::Cancel)]
                }
                Prompt::LargeFile { path, size } => {
                    ui.heading("Large file");
                    ui.label(format!(
                        "{} is {}. NoteStrike keeps the whole file in memory, so editing may be slow.",
                        path.display(),
                        human_size(*size)
                    ));
                    vec![("Open anyway", Answer::OpenAnyway), ("Cancel", Answer::Cancel)]
                }
                Prompt::ConfirmReload { tab, encoding } => {
                    ui.heading("Discard unsaved changes?");
                    ui.label(format!(
                        "Reloading {}{} discards your unsaved changes.",
                        title_of(&self.ws, *tab),
                        encoding.map(|e| format!(" as {}", e.label())).unwrap_or_default()
                    ));
                    vec![("Reload", Answer::Reload), ("Cancel", Answer::Cancel)]
                }
                Prompt::Recovery { orphans } => {
                    ui.heading("Recover unsaved work");
                    ui.label("NoteStrike did not close normally last time. These documents can be recovered:");
                    egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                        for orphan in orphans {
                            let e = &orphan.entry;
                            let kind = if e.text.is_some() { "unsaved changes" } else { "reopen" };
                            let location = e.path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "not saved yet".into());
                            ui.add(egui::Label::new(format!("  •  {} ({kind})", e.title)));
                            ui.add(egui::Label::new(RichText::new(format!("      {location}")).small().weak()).truncate())
                                .on_hover_text(&location);
                        }
                    });
                    vec![("Restore", Answer::Restore), ("Discard…", Answer::Discard), ("Later", Answer::Later)]
                }
                Prompt::ConfirmDiscardRecovery { orphans } => {
                    ui.heading("Discard recovered text?");
                    ui.label(format!(
                        "This permanently deletes NoteStrike's {} recovery cop{}. Your own files are not touched.",
                        orphans.len(),
                        if orphans.len() == 1 { "y" } else { "ies" }
                    ));
                    vec![("Discard", Answer::Discard), ("Back", Answer::Cancel)]
                }
                Prompt::GoToLine => {
                    ui.heading("Go to line");
                    let total = self.ws.active_tab().map_or(1, |t| search::line_count(&t.text));
                    let label = ui.label(format!("Line number (1–{total}):"));
                    let field = ui.add(egui::TextEdit::singleline(&mut self.goto_input).desired_width(160.0)).labelled_by(label.id);
                    if first_frame {
                        field.request_focus();
                    }
                    if self.ws.status.starts_with("The line number") {
                        ui.label(RichText::new(&self.ws.status).color(ui.visuals().warn_fg_color));
                    }
                    let parsed = self.goto_input.trim().parse::<usize>();
                    if field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) && let Ok(n) = parsed {
                        answer = Some(Answer::GoTo(n));
                    }
                    vec![("Go to", Answer::GoTo(parsed.unwrap_or(0))), ("Cancel", Answer::Cancel)]
                }
                Prompt::Message { title, body } => {
                    ui.heading(title);
                    egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                        ui.label(body);
                    });
                    vec![("OK", Answer::Ok)]
                }
                Prompt::About => {
                    ui.vertical_centered(|ui| {
                        if let Some(logo) = &self.logo {
                            ui.add(egui::Image::new(logo).fit_to_exact_size(egui::vec2(96.0, 96.0)).alt_text("NoteStrike hedgehog logo"));
                        }
                        ui.heading(format!("{APP_NAME} {}", env!("CARGO_PKG_VERSION")));
                        ui.label("A lightweight plain-text editor.");
                        ui.add_space(8.0);
                        ui.label(RichText::new("Part of the PWDS Strike suite by").weak());
                        if let Some(logo) = &self.pwds_logo {
                            // The official PWDS mark is black; in dark mode it sits on a
                            // light plate so it stays visible.
                            let plate = if ui.visuals().dark_mode {
                                Color32::from_rgb(243, 243, 243)
                            } else {
                                Color32::TRANSPARENT
                            };
                            let (rect, response) =
                                ui.allocate_exact_size(egui::vec2(140.0, 60.0), egui::Sense::hover());
                            ui.painter().rect_filled(rect, 6.0, plate);
                            egui::Image::new(logo).paint_at(ui, rect.shrink2(egui::vec2(10.0, 4.0)));
                            response.widget_info(|| {
                                egui::WidgetInfo::labeled(egui::WidgetType::Image, true, "ProWebDevSolutions logo")
                            });
                        }
                        ui.hyperlink_to("ProWebDevSolutions", "https://prowebdevsolutions.com");
                        ui.add_space(8.0);
                        ui.label(RichText::new("Built with Rust, egui/eframe, and rfd (MIT / Apache-2.0).").small().weak());
                    });
                    vec![("OK", Answer::Ok)]
                }
            };
            ui.add_space(12.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // Right-to-left: show buttons in reverse so the first one is leftmost.
                for (index, (label, value)) in buttons.iter().enumerate().rev() {
                    let button = if index == 0 {
                        egui::Button::new(RichText::new(*label).strong())
                    } else {
                        egui::Button::new(*label)
                    };
                    let response = ui.add(button.min_size(egui::vec2(88.0, 28.0)));
                    if index == 0 && first_frame && !matches!(prompt, Prompt::GoToLine) {
                        response.request_focus();
                    }
                    if response.clicked() {
                        answer = Some(*value);
                    }
                }
            });
        });
        if answer.is_none() && modal.should_close() {
            answer = Some(match prompt {
                Prompt::Message { .. } | Prompt::About => Answer::Ok,
                Prompt::Recovery { .. } => Answer::Later,
                _ => Answer::Cancel,
            });
        }
        if let Some(answer) = answer {
            if matches!(answer, Answer::GoTo(_)) {
                self.scroll_to_selection = true;
            }
            self.ws.answer(answer, self.dialogs.as_mut());
            self.focus_editor = self.ws.current_prompt().is_none();
        }
    }

    // ----- capture -------------------------------------------------------

    fn capture_after_frame(&mut self, ctx: &egui::Context) {
        let Some(capture) = self.capture.as_mut() else {
            return;
        };
        if capture.started.elapsed() > Duration::from_secs(15) {
            self.finish_capture(ctx, Err("Timed out waiting for a native screenshot".into()));
            return;
        }
        if capture.requested {
            let image = ctx.input(|input| {
                input.events.iter().find_map(|event| match event {
                    egui::Event::Screenshot { image, .. } => Some(Arc::clone(image)),
                    _ => None,
                })
            });
            if let Some(image) = image {
                let path = capture.path.clone();
                let result = write_capture_png(&path, &image);
                self.finish_capture(ctx, result);
            } else {
                ctx.request_repaint();
            }
            return;
        }
        if capture.settle_frames < 4 {
            capture.settle_frames += 1;
        } else {
            ctx.send_viewport_cmd(ViewportCommand::Screenshot(egui::UserData::default()));
            capture.requested = true;
        }
        ctx.request_repaint();
    }

    fn finish_capture(&mut self, ctx: &egui::Context, result: Result<(), String>) {
        if let Some(capture) = self.capture.take() {
            *capture.outcome.lock().unwrap() = Some(result);
            self.ws.shutdown();
            self.closing = true;
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }

    fn window_title(&self) -> String {
        match self.ws.active_tab() {
            Some(tab) => format!(
                "{}{} - {APP_NAME}",
                if tab.is_dirty() { "*" } else { "" },
                tab.title()
            ),
            None => APP_NAME.into(),
        }
    }
}

impl eframe::App for NoteStrikeApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.setup(&ctx);
        self.apply_theme(&ctx);

        // Files dropped onto the window open in new tabs.
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        if !dropped.is_empty() {
            self.ws.open_paths(&dropped);
        }

        // Window close button / Alt+F4 / Cmd+Q (macos_menu): ask about unsaved tabs first.
        if ctx.input(|i| i.viewport().close_requested()) && !self.closing {
            self.ws.request_quit();
            if !self.ws.quit_approved() {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            }
        }

        self.handle_shortcuts(&ctx);

        egui::Panel::top("notestrike-menu").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::top("notestrike-tabs")
            .frame(
                egui::Frame::NONE
                    .fill(ui.visuals().panel_fill)
                    .inner_margin(egui::Margin {
                        left: 6,
                        right: 6,
                        top: 4,
                        bottom: 0,
                    }),
            )
            .show(ui, |ui| self.tab_strip(ui));
        if self.ws.settings.status_bar {
            egui::Panel::bottom("notestrike-status")
                .frame(
                    egui::Frame::NONE
                        .fill(ui.visuals().panel_fill)
                        .inner_margin(egui::Margin::symmetric(10, 3)),
                )
                .show(ui, |ui| self.status_bar(ui));
        }
        if self.ws.find.visible {
            egui::Panel::top("notestrike-find")
                .frame(
                    egui::Frame::NONE
                        .fill(ui.visuals().extreme_bg_color)
                        .inner_margin(egui::Margin::symmetric(10, 6)),
                )
                .show(ui, |ui| self.find_bar(ui));
        }
        let has_notice = self.ws.active_tab().is_some_and(|t| {
            t.external != ExternalState::InSync
                || t.invalid_sequences > 0
                || t.contains_nul
                || t.read_only
                || t.mixed_eol.is_some()
        });
        if has_notice {
            egui::Panel::top("notestrike-notices")
                .frame(
                    egui::Frame::NONE
                        .fill(palette::for_dark(ui.visuals().dark_mode).notice_bg)
                        .inner_margin(egui::Margin::symmetric(10, 6)),
                )
                .show(ui, |ui| self.notices(ui));
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(ui.visuals().extreme_bg_color))
            .show(ui, |ui| self.editor(ui));

        self.prompts(&ctx);

        ctx.send_viewport_cmd(ViewportCommand::Title(self.window_title()));
        self.ws.tick(Instant::now());
        self.persist_settings();
        ctx.request_repaint_after(Duration::from_millis(500));

        if self.ws.quit_approved() && !self.closing {
            self.closing = true;
            self.ws.shutdown();
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
        self.capture_after_frame(&ctx);
    }
}

/// Light/dark preference of a Linux desktop (GNOME and GTK-based desktops):
/// `GTK_THEME`, then the `color-scheme` and `gtk-theme` GSettings keys.
#[cfg(any(test, all(unix, not(target_os = "macos"))))]
mod desktop_theme {
    use eframe::egui::Theme;

    #[cfg(all(unix, not(target_os = "macos")))]
    pub fn detect() -> Option<Theme> {
        if let Some(name) = std::env::var_os("GTK_THEME") {
            return Some(from_gtk_theme_name(&name.to_string_lossy()));
        }
        let gsettings = |key: &str| {
            std::process::Command::new("gsettings")
                .args(["get", "org.gnome.desktop.interface", key])
                .stderr(std::process::Stdio::null())
                .output()
                .ok()
                .filter(|out| out.status.success())
                .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
        };
        if let Some(theme) = gsettings("color-scheme").and_then(|v| from_color_scheme(&v)) {
            return Some(theme);
        }
        gsettings("gtk-theme").map(|v| from_gtk_theme_name(&v))
    }

    /// `'prefer-dark'` / `'prefer-light'`; `'default'` means "not set".
    pub fn from_color_scheme(value: &str) -> Option<Theme> {
        match value.trim().trim_matches('\'') {
            "prefer-dark" => Some(Theme::Dark),
            "prefer-light" => Some(Theme::Light),
            _ => None,
        }
    }

    /// Theme names such as `Adwaita:dark`, `'Yaru-dark'` or `Breeze`.
    pub fn from_gtk_theme_name(name: &str) -> Theme {
        if name.to_ascii_lowercase().contains("dark") {
            Theme::Dark
        } else {
            Theme::Light
        }
    }
}

/// Label of View > Theme > System for this platform.
const SYSTEM_THEME_LABEL: &str = if cfg!(windows) {
    "System (follow Windows)"
} else if cfg!(target_os = "macos") {
    "System (follow macOS)"
} else {
    "System (follow desktop)"
};

type FontCandidates = [(&'static str, &'static [&'static str], egui::FontFamily); 2];

/// Segoe UI and Consolas (or Cascadia Mono), installed with Windows.
#[cfg(windows)]
const SYSTEM_FONTS: FontCandidates = [
    (
        "system-ui",
        &["C:\\Windows\\Fonts\\segoeui.ttf"],
        egui::FontFamily::Proportional,
    ),
    (
        "system-mono",
        &[
            "C:\\Windows\\Fonts\\consola.ttf",
            "C:\\Windows\\Fonts\\CascadiaMono.ttf",
        ],
        egui::FontFamily::Monospace,
    ),
];

/// DejaVu, installed by most Linux desktops (Debian/Ubuntu and Fedora paths).
#[cfg(all(unix, not(target_os = "macos")))]
const SYSTEM_FONTS: FontCandidates = [
    (
        "system-ui",
        &[
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans.ttf",
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
        ],
        egui::FontFamily::Proportional,
    ),
    (
        "system-mono",
        &[
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
            "/usr/share/fonts/dejavu-sans-mono-fonts/DejaVuSansMono.ttf",
            "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
        ],
        egui::FontFamily::Monospace,
    ),
];

/// macOS system fonts ship as collections and variable fonts; egui's built-in
/// fonts are used there.
#[cfg(not(any(windows, all(unix, not(target_os = "macos")))))]
const SYSTEM_FONTS: FontCandidates = [
    ("system-ui", &[], egui::FontFamily::Proportional),
    ("system-mono", &[], egui::FontFamily::Monospace),
];

/// Prefer the platform's UI and monospace fonts when installed (loaded from
/// the user's own system at runtime, never bundled); egui's built-in fonts
/// stay as fallbacks for missing glyphs.
fn install_system_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let candidates = SYSTEM_FONTS;
    let mut changed = false;
    for (name, paths, family) in candidates {
        if let Some(bytes) = paths.iter().find_map(|p| std::fs::read(p).ok()) {
            fonts
                .font_data
                .insert(name.into(), Arc::new(egui::FontData::from_owned(bytes)));
            fonts
                .families
                .entry(family)
                .or_default()
                .insert(0, name.into());
            changed = true;
        }
    }
    // Consolas and egui's Hack lack some symbols (e.g. U+FFFD, shown for
    // undecodable bytes); fall back to the UI font for those glyphs.
    if fonts.font_data.contains_key("system-ui") {
        fonts
            .families
            .entry(egui::FontFamily::Monospace)
            .or_default()
            .push("system-ui".into());
    }
    if changed {
        ctx.set_fonts(fonts);
    }
}

fn load_texture(ctx: &egui::Context, name: &str, png: &[u8]) -> Option<egui::TextureHandle> {
    let image = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .ok()?
        .to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    let color = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
    Some(ctx.load_texture(name, color, egui::TextureOptions::LINEAR))
}

/// Window icon from the NoteStrike logo.
pub fn window_icon() -> Option<egui::IconData> {
    let image = image::load_from_memory_with_format(LOGO_PNG, image::ImageFormat::Png).ok()?;
    let image = image
        .resize(256, 256, image::imageops::FilterType::Triangle)
        .to_rgba8();
    Some(egui::IconData {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    })
}

fn write_capture_png(path: &Path, screenshot: &egui::ColorImage) -> Result<(), String> {
    let [width, height] = screenshot.size;
    let mut pixels = Vec::with_capacity(width * height * 4);
    for color in &screenshot.pixels {
        // The window is opaque; drop the alpha egui reports for translucent fills.
        let [r, g, b, _] = color.to_array();
        pixels.extend_from_slice(&[r, g, b, 255]);
    }
    let image = image::RgbaImage::from_raw(width as u32, height as u32, pixels)
        .ok_or("Could not construct screenshot image")?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("Create screenshot {}: {error}", path.display()))?;
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut file, image::ImageFormat::Png)
        .map_err(|error| format!("Encode screenshot: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("Sync screenshot: {error}"))
}

#[cfg(test)]
mod tests {
    //! Headless frames of the real UI code: rendering every prompt, typing
    //! into the editor, keyboard shortcuts, and the window close request.

    use std::collections::VecDeque;

    use super::*;
    use crate::encoding::Unrepresentable;
    use crate::workspace::PendingSave;
    use eframe::egui::Modifiers;

    #[derive(Default)]
    struct Scripted {
        save: VecDeque<Option<PathBuf>>,
    }

    impl FileDialogs for Scripted {
        fn open_files(&mut self) -> Option<Vec<PathBuf>> {
            None
        }
        fn save_file(&mut self, _: &str, _: Option<&Path>) -> Option<PathBuf> {
            self.save.pop_front().flatten()
        }
        fn pick_folder(&mut self, _: &str) -> Option<PathBuf> {
            None
        }
    }

    fn frame(
        app: &mut NoteStrikeApp,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        close: bool,
    ) -> egui::FullOutput {
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 600.0),
            )),
            events,
            ..Default::default()
        };
        if close {
            input
                .viewports
                .entry(egui::ViewportId::ROOT)
                .or_default()
                .events
                .push(egui::ViewportEvent::Close);
        }
        let mut eframe_frame = eframe::Frame::_new_kittest();
        let mut output = ctx.run_ui(input, |ui| eframe::App::ui(app, ui, &mut eframe_frame));
        output.textures_delta.clear();
        output
    }

    fn app_with(dialogs: Scripted) -> NoteStrikeApp {
        NoteStrikeApp::new(
            Workspace::new(None, Settings::default()),
            Box::new(dialogs),
            None,
        )
    }

    #[test]
    fn typing_marks_dirty_and_ctrl_s_saves_through_save_as() {
        let dir =
            std::env::temp_dir().join(format!("notestrike-ui-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("typed.txt");
        let mut app = app_with(Scripted {
            save: VecDeque::from([Some(target.clone())]),
        });
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![], false);
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::Text("Sample line".into())],
            false,
        );
        assert_eq!(app.ws.active_tab().unwrap().text, "Sample line");
        assert!(app.ws.active_tab().unwrap().is_dirty());
        assert_eq!(app.window_title(), "*Untitled 1 - NoteStrike");
        let ctrl_s = egui::Event::Key {
            key: Key::S,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::COMMAND,
        };
        frame(&mut app, &ctx, vec![ctrl_s], false);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "Sample line");
        assert!(!app.ws.active_tab().unwrap().is_dirty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn window_close_with_unsaved_text_is_cancelled_and_asks() {
        let mut app = app_with(Scripted::default());
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![], false);
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::Text("unsaved".into())],
            false,
        );
        let output = frame(&mut app, &ctx, vec![], true);
        let commands = &output.viewport_output[&egui::ViewportId::ROOT].commands;
        assert!(commands.contains(&ViewportCommand::CancelClose));
        assert!(matches!(
            app.ws.current_prompt(),
            Some(Prompt::UnsavedOnQuit { .. })
        ));
        assert!(!app.ws.quit_approved());
    }

    #[test]
    fn every_prompt_renders_headlessly() {
        let mut app = app_with(Scripted::default());
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![], false);
        let tab = app.ws.active_tab().unwrap().id;
        let save = PendingSave {
            tab,
            save_as: false,
            chosen_path: None,
            overwrite_external: false,
            accept_lossy: false,
        };
        let prompts = vec![
            Prompt::UnsavedTab { tab },
            Prompt::UnsavedOnQuit { tabs: vec![tab] },
            Prompt::ChangedOnDisk { save: save.clone() },
            Prompt::LossySave { save: save.clone() },
            Prompt::Unrepresentable {
                save: save.clone(),
                details: Unrepresentable {
                    encoding: TextEncoding::Windows1252,
                    count: 1,
                    first: '☃',
                    first_char_index: 0,
                },
            },
            Prompt::ReadOnly { save },
            Prompt::LargeFile {
                path: PathBuf::from("big.log"),
                size: 9 * 1024 * 1024,
            },
            Prompt::ConfirmReload {
                tab,
                encoding: None,
            },
            Prompt::Recovery { orphans: vec![] },
            Prompt::ConfirmDiscardRecovery { orphans: vec![] },
            Prompt::GoToLine,
            Prompt::Message {
                title: "Title".into(),
                body: "Body".into(),
            },
            Prompt::About,
        ];
        for prompt in prompts {
            app.ws.prompts.clear();
            app.ws.prompts.push_back(prompt);
            frame(&mut app, &ctx, vec![], false);
            frame(&mut app, &ctx, vec![], false);
            assert!(
                app.ws.current_prompt().is_some(),
                "prompt stays until answered"
            );
        }
        // Escape dismisses a message.
        app.ws.prompts.clear();
        app.ws.prompts.push_back(Prompt::About);
        let esc = egui::Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        frame(&mut app, &ctx, vec![esc], false);
        assert!(app.ws.current_prompt().is_none());
    }

    #[test]
    fn find_bar_and_narrow_layout_render() {
        let mut app = app_with(Scripted::default());
        app.ws.active_tab_mut().unwrap().text = "alpha beta alpha".into();
        app.ws.show_find(true);
        app.ws.find.needle = "alpha".into();
        app.ws.settings.word_wrap = false;
        let ctx = egui::Context::default();
        for _ in 0..3 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(380.0, 500.0),
                )),
                ..Default::default()
            };
            let mut eframe_frame = eframe::Frame::_new_kittest();
            ctx.run_ui(input, |ui| eframe::App::ui(&mut app, ui, &mut eframe_frame))
                .textures_delta
                .clear();
        }
        assert!(app.ws.find_step(true));
        assert_eq!(app.ws.selection, (0, 5));
    }

    fn themed_frame(app: &mut NoteStrikeApp, ctx: &egui::Context, system: Option<egui::Theme>) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 600.0),
            )),
            system_theme: system,
            ..Default::default()
        };
        let mut eframe_frame = eframe::Frame::_new_kittest();
        ctx.run_ui(input, |ui| eframe::App::ui(app, ui, &mut eframe_frame))
            .textures_delta
            .clear();
    }

    #[test]
    fn system_theme_follows_the_os_and_explicit_choices_override_it() {
        let ctx = egui::Context::default();
        let mut app = app_with(Scripted::default());
        assert_eq!(
            app.ws.settings.theme,
            ThemeChoice::System,
            "default is System"
        );
        themed_frame(&mut app, &ctx, Some(egui::Theme::Light));
        themed_frame(&mut app, &ctx, Some(egui::Theme::Light));
        assert_eq!(ctx.theme(), egui::Theme::Light);
        themed_frame(&mut app, &ctx, Some(egui::Theme::Dark));
        assert_eq!(
            ctx.theme(),
            egui::Theme::Dark,
            "System tracks an OS change live"
        );
        // When the OS reports nothing, egui's fallback (dark) is used.
        themed_frame(&mut app, &ctx, None);
        assert_eq!(ctx.theme(), egui::Theme::Dark);

        app.ws.settings.theme = ThemeChoice::Light;
        themed_frame(&mut app, &ctx, Some(egui::Theme::Dark));
        assert_eq!(
            ctx.theme(),
            egui::Theme::Light,
            "explicit Light ignores a dark OS"
        );
        app.ws.settings.theme = ThemeChoice::Dark;
        themed_frame(&mut app, &ctx, Some(egui::Theme::Light));
        assert_eq!(
            ctx.theme(),
            egui::Theme::Dark,
            "switching applies on the next frame"
        );
        assert_eq!(
            theme_preference(ThemeChoice::System),
            egui::ThemePreference::System
        );
    }

    #[test]
    fn theme_choice_persists_across_restarts() {
        let dir = std::env::temp_dir().join(format!(
            "notestrike-theme-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let settings_path = dir.join("settings.json");
        let ctx = egui::Context::default();
        let mut app = NoteStrikeApp::new(
            Workspace::new(None, Settings::default()),
            Box::new(Scripted::default()),
            Some(settings_path.clone()),
        );
        themed_frame(&mut app, &ctx, Some(egui::Theme::Light));
        app.ws.settings.theme = ThemeChoice::Dark;
        themed_frame(&mut app, &ctx, Some(egui::Theme::Light));
        assert_eq!(Settings::load(&settings_path).theme, ThemeChoice::Dark);

        // "Restart": a new app and context built from the saved settings.
        let ctx = egui::Context::default();
        let mut restarted = NoteStrikeApp::new(
            Workspace::new(None, Settings::load(&settings_path)),
            Box::new(Scripted::default()),
            Some(settings_path.clone()),
        );
        themed_frame(&mut restarted, &ctx, Some(egui::Theme::Light));
        themed_frame(&mut restarted, &ctx, Some(egui::Theme::Light));
        assert_eq!(ctx.theme(), egui::Theme::Dark);

        restarted.ws.settings.theme = ThemeChoice::System;
        themed_frame(&mut restarted, &ctx, Some(egui::Theme::Light));
        assert_eq!(Settings::load(&settings_path).theme, ThemeChoice::System);
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn luminance(c: Color32) -> f64 {
        let lin = |v: u8| {
            let s = f64::from(v) / 255.0;
            if s <= 0.04045 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
    }

    fn contrast(a: Color32, b: Color32) -> f64 {
        let (x, y) = (luminance(a), luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    /// Composite a premultiplied, possibly translucent colour over an opaque one.
    fn over(top: Color32, bottom: Color32) -> Color32 {
        let k = 1.0 - f32::from(top.a()) / 255.0;
        let mix = |t: u8, b: u8| (f32::from(t) + f32::from(b) * k).round().min(255.0) as u8;
        Color32::from_rgb(
            mix(top.r(), bottom.r()),
            mix(top.g(), bottom.g()),
            mix(top.b(), bottom.b()),
        )
    }

    #[test]
    fn both_themes_meet_wcag_aa_contrast() {
        let ctx = egui::Context::default();
        let mut app = app_with(Scripted::default());
        themed_frame(&mut app, &ctx, Some(egui::Theme::Light));
        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            let v = ctx.style_of(theme).visuals.clone();
            let p = palette::for_theme(theme);
            let text = v.text_color();
            let editor_text = p.editor_text.unwrap_or(text);
            let text_pairs = [
                ("body text on panel", text, v.panel_fill),
                ("body text in dialogs", text, v.window_fill),
                ("editor text", editor_text, v.extreme_bg_color),
                (
                    "selected editor text",
                    editor_text,
                    over(v.selection.bg_fill, v.extreme_bg_color),
                ),
                ("weak text on panel", v.weak_text_color(), v.panel_fill),
                ("weak text in dialogs", v.weak_text_color(), v.window_fill),
                ("warning text on panel", v.warn_fg_color, v.panel_fill),
                ("warning text in dialogs", v.warn_fg_color, v.window_fill),
                ("error text in dialogs", v.error_fg_color, v.window_fill),
                ("links in About", v.hyperlink_color, v.window_fill),
                ("banner text", p.notice_text, p.notice_bg),
                (
                    "button text",
                    v.widgets.inactive.fg_stroke.color,
                    v.widgets.inactive.weak_bg_fill,
                ),
                (
                    "hovered button text",
                    v.widgets.hovered.fg_stroke.color,
                    v.widgets.hovered.weak_bg_fill,
                ),
            ];
            for (what, fg, bg) in text_pairs {
                let ratio = contrast(fg, bg);
                assert!(ratio >= 4.5, "{theme:?}: {what} contrast {ratio:.2} < 4.5");
            }
            let ui_pairs = [("banner warning icon", p.warn_icon, p.notice_bg)];
            for (what, fg, bg) in ui_pairs {
                let ratio = contrast(fg, bg);
                assert!(ratio >= 3.0, "{theme:?}: {what} contrast {ratio:.2} < 3");
            }
        }
    }

    #[test]
    fn found_match_stays_selected_in_the_editor() {
        let mut app = app_with(Scripted::default());
        let ctx = egui::Context::default();
        app.ws.active_tab_mut().unwrap().text = "one two three two".into();
        frame(&mut app, &ctx, vec![], false);
        app.ws.find.needle = "two".into();
        assert!(app.ws.find_step(true));
        frame(&mut app, &ctx, vec![], false);
        frame(&mut app, &ctx, vec![], false);
        let id = NoteStrikeApp::editor_id(app.ws.active_tab().unwrap().id);
        let range = TextEditState::load(&ctx, id)
            .unwrap()
            .cursor
            .char_range()
            .unwrap();
        let (a, b) = (range.primary.index, range.secondary.index);
        assert_eq!((usize::from(a.min(b)), usize::from(a.max(b))), (4, 7));
        assert_eq!(app.ws.selection, (4, 7));
    }

    fn key_event(shortcut: KeyboardShortcut) -> egui::Event {
        egui::Event::Key {
            key: shortcut.logical_key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: shortcut.modifiers,
        }
    }

    #[test]
    fn shortcut_labels_follow_the_platform() {
        let save = keys::label(keys::SAVE);
        if cfg!(target_os = "macos") {
            assert_eq!(save, "Cmd+S");
            assert_eq!(keys::label(keys::REDO), "Shift+Cmd+Z");
            assert_ne!(
                keys::REPLACE,
                KeyboardShortcut::new(Modifiers::COMMAND, Key::H)
            );
            assert_eq!(keys::quit_label(), "Cmd+Q");
        } else {
            assert_eq!(save, "Ctrl+S");
            assert_eq!(keys::label(keys::SAVE_AS), "Ctrl+Shift+S");
            assert_eq!(keys::label(keys::REPLACE), "Ctrl+H");
            assert_eq!(keys::quit_label(), "Alt+F4");
        }
        assert_eq!(keys::label(keys::NEXT_TAB), "Ctrl+Tab");
        let help = keys::help_text();
        assert!(help.contains(&format!("{save}	Save")), "{help}");
        assert_eq!(help.contains("Cmd"), cfg!(target_os = "macos"), "{help}");
    }

    #[test]
    fn replace_shortcut_opens_replace_not_plain_find() {
        let mut app = app_with(Scripted::default());
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![key_event(keys::REPLACE)], false);
        assert!(app.ws.find.visible && app.ws.find.replace_visible);
        app.ws.find.visible = false;
        frame(&mut app, &ctx, vec![key_event(keys::FIND)], false);
        assert!(app.ws.find.visible && !app.ws.find.replace_visible);
        frame(&mut app, &ctx, vec![key_event(keys::GO_TO_LINE)], false);
        assert!(matches!(app.ws.current_prompt(), Some(Prompt::GoToLine)));
    }

    #[test]
    fn linux_desktop_theme_values_are_parsed() {
        use egui::Theme;
        assert_eq!(
            desktop_theme::from_color_scheme("'prefer-dark'\n"),
            Some(Theme::Dark)
        );
        assert_eq!(
            desktop_theme::from_color_scheme("'prefer-light'"),
            Some(Theme::Light)
        );
        assert_eq!(desktop_theme::from_color_scheme("'default'"), None);
        assert_eq!(
            desktop_theme::from_gtk_theme_name("Adwaita:dark"),
            Theme::Dark
        );
        assert_eq!(
            desktop_theme::from_gtk_theme_name("'Yaru-dark'\n"),
            Theme::Dark
        );
        assert_eq!(
            desktop_theme::from_gtk_theme_name("'Adwaita'"),
            Theme::Light
        );
    }
}
