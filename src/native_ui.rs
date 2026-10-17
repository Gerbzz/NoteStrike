//! Native editor adapter for the local notebook core.

use std::fs::OpenOptions;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;
use uuid::Uuid;

use crate::Notebook;

pub struct NativeEditor {
    root: PathBuf,
    notebook: Notebook,
    selected: Option<Uuid>,
    pending_selection: Option<Uuid>,
    search: String,
    body_draft: String,
    rename_draft: String,
    dirty: bool,
    preview: bool,
    styled: bool,
    status: String,
    capture: Option<CaptureOnce>,
}

struct CaptureOnce {
    path: PathBuf,
    outcome: Arc<Mutex<Option<Result<(), String>>>>,
    started: Instant,
    settle_frames: u8,
    requested: bool,
    scroll_to_end: bool,
}

impl NativeEditor {
    pub fn new(root: PathBuf, notebook: Notebook) -> Self {
        let warning_count = notebook.warnings().len();
        let selected = notebook.list().first().map(|note| note.id);
        let (body_draft, rename_draft) = selected
            .and_then(|id| notebook.get(id))
            .map(|note| (note.body.clone(), note.title.clone()))
            .unwrap_or_default();
        Self {
            root,
            notebook,
            selected,
            pending_selection: None,
            search: String::new(),
            body_draft,
            rename_draft,
            dirty: false,
            preview: false,
            styled: false,
            status: if warning_count == 0 {
                "Ready".into()
            } else {
                format!("{warning_count} note file(s) need repair; valid notes remain available.")
            },
            capture: None,
        }
    }

    /// Capture one fully painted editor frame, then close the capture window.
    pub fn capture_once(&mut self, path: PathBuf) -> Arc<Mutex<Option<Result<(), String>>>> {
        self.capture_with_scroll(path, false)
    }

    /// Capture a fully painted frame with the source editor scrolled to its end.
    pub fn capture_once_at_end(&mut self, path: PathBuf) -> Arc<Mutex<Option<Result<(), String>>>> {
        self.capture_with_scroll(path, true)
    }

    fn capture_with_scroll(
        &mut self,
        path: PathBuf,
        scroll_to_end: bool,
    ) -> Arc<Mutex<Option<Result<(), String>>>> {
        let outcome = Arc::new(Mutex::new(None));
        self.capture = Some(CaptureOnce {
            path,
            outcome: Arc::clone(&outcome),
            started: Instant::now(),
            settle_frames: 0,
            requested: false,
            scroll_to_end,
        });
        outcome
    }

    fn write_capture_png(
        path: &std::path::Path,
        screenshot: &egui::ColorImage,
    ) -> Result<(), String> {
        let [width, height] = screenshot.size;
        let bytes = width
            .checked_mul(height)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or("Screenshot dimensions overflow")?;
        let mut pixels = Vec::with_capacity(bytes);
        for color in &screenshot.pixels {
            pixels.extend_from_slice(&color.to_array());
        }
        if pixels.len() != bytes {
            return Err("Screenshot pixel count does not match its dimensions".into());
        }
        let width = u32::try_from(width).map_err(|_| "Screenshot width exceeds PNG limits")?;
        let height = u32::try_from(height).map_err(|_| "Screenshot height exceeds PNG limits")?;
        let image = image::RgbaImage::from_raw(width, height, pixels)
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
            .map_err(|error| format!("Sync screenshot: {error}"))?;
        Ok(())
    }

    fn finish_capture(&mut self, ctx: &egui::Context, result: Result<(), String>) {
        if let Some(capture) = self.capture.take() {
            *capture.outcome.lock().unwrap() = Some(result);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn capture_after_frame(&mut self, ctx: &egui::Context) {
        let Some(capture) = self.capture.as_ref() else {
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
                let result = Self::write_capture_png(&capture.path, &image);
                self.finish_capture(ctx, result);
            } else {
                ctx.request_repaint();
            }
            return;
        }
        let capture = self.capture.as_mut().unwrap();
        if capture.settle_frames < 2 {
            capture.settle_frames += 1;
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            capture.requested = true;
        }
        ctx.request_repaint();
    }

    fn select(&mut self, id: Uuid) {
        if self.selected == Some(id) {
            return;
        }
        if self.dirty {
            self.pending_selection = Some(id);
            self.status = "Save or discard your current edit before switching notes.".into();
            return;
        }
        self.selected = Some(id);
        if let Some(note) = self.notebook.get(id) {
            self.body_draft = note.body.clone();
            self.rename_draft = note.title.clone();
            self.status = "Ready".into();
        }
    }

    fn save(&mut self) {
        let Some(id) = self.selected else { return };
        match self.notebook.update(id, &self.body_draft) {
            Ok(()) => {
                self.dirty = false;
                self.status = "Saved to the notebook.".into();
                if let Some(next) = self.pending_selection.take() {
                    self.select(next);
                }
            }
            Err(error) => self.status = error,
        }
    }

    fn discard(&mut self) {
        if let Some(id) = self.selected
            && let Some(note) = self.notebook.get(id)
        {
            self.body_draft = note.body.clone();
            self.rename_draft = note.title.clone();
        }
        self.dirty = false;
        self.status = "Unsaved edit discarded.".into();
        if let Some(next) = self.pending_selection.take() {
            self.select(next);
        }
    }

    fn reload_disk(&mut self) {
        match Notebook::open(&self.root) {
            Ok(notebook) => {
                let selected = self.selected.filter(|id| notebook.get(*id).is_some());
                self.notebook = notebook;
                self.selected = selected;
                self.dirty = false;
                self.pending_selection = None;
                if let Some(id) = selected
                    && let Some(note) = self.notebook.get(id)
                {
                    self.body_draft = note.body.clone();
                    self.rename_draft = note.title.clone();
                }
                self.status = if self.notebook.warnings().is_empty() {
                    "Reloaded from disk.".into()
                } else {
                    format!(
                        "Reloaded; {} note file(s) need repair.",
                        self.notebook.warnings().len()
                    )
                };
            }
            Err(error) => self.status = error,
        }
    }

    fn create_copy(&mut self) {
        let title = self
            .selected
            .and_then(|id| self.notebook.get(id))
            .map(|note| format!("{} (local copy)", note.title))
            .unwrap_or_else(|| "Recovered note".into());
        match self.notebook.add(&title, &self.body_draft) {
            Ok(note) => {
                self.dirty = false;
                self.selected = Some(note.id);
                self.rename_draft = note.title;
                self.pending_selection = None;
                self.status = "Saved a separate copy; the disk version was kept.".into();
            }
            Err(error) => self.status = error,
        }
    }

    fn create_note(&mut self) {
        if self.dirty {
            self.status = "Save or discard your current edit before creating a note.".into();
            return;
        }
        match self.notebook.add("Untitled", "") {
            Ok(note) => self.select(note.id),
            Err(error) => self.status = error,
        }
    }

    fn rename(&mut self) {
        if self.dirty {
            self.status = "Save the body before renaming this note.".into();
            return;
        }
        let Some(id) = self.selected else { return };
        match self.notebook.rename(id, &self.rename_draft) {
            Ok(()) => self.status = "Title saved; ID links still point here.".into(),
            Err(error) => self.status = error,
        }
    }

    fn matching_notes(&self) -> Vec<(Uuid, String)> {
        let notes = if self.search.trim().is_empty() {
            self.notebook.list()
        } else {
            self.notebook.search(&self.search)
        };
        notes
            .into_iter()
            .map(|note| (note.id, note.title.clone()))
            .collect()
    }

    fn backlink_notes(&self, id: Uuid) -> Vec<(Uuid, String)> {
        let mut links: Vec<_> = self
            .notebook
            .backlinks(id)
            .into_iter()
            .filter_map(|source| {
                self.notebook
                    .get(source)
                    .map(|note| (source, note.title.clone()))
            })
            .collect();
        links.sort_by(|a, b| {
            a.1.to_lowercase()
                .cmp(&b.1.to_lowercase())
                .then(a.0.cmp(&b.0))
        });
        links
    }

    fn note_list(&mut self, ui: &mut egui::Ui) {
        ui.heading("Notes");
        ui.label(
            egui::RichText::new(format!("{} in this notebook", self.notebook.list().len()))
                .small()
                .weak(),
        );
        ui.add_space(10.0);
        ui.add_sized(
            [ui.available_width(), 36.0],
            egui::TextEdit::singleline(&mut self.search).hint_text("Search notes"),
        );
        ui.add_space(4.0);
        if ui
            .add_sized([ui.available_width(), 36.0], egui::Button::new("New note"))
            .clicked()
        {
            self.create_note();
        }
        ui.add_space(12.0);
        ui.separator();
        let notes = self.matching_notes();
        egui::ScrollArea::vertical().show(ui, |ui| {
            if notes.is_empty() {
                ui.add_space(12.0);
                ui.label(if self.search.trim().is_empty() {
                    "No notes yet. Create one to start your local notebook."
                } else {
                    "No matches. Try a shorter search or clear the field."
                });
            }
            for (id, title) in notes {
                if ui
                    .selectable_label(self.selected == Some(id), title)
                    .clicked()
                {
                    self.select(id);
                }
            }
        });
    }

    fn context_rail(&mut self, ui: &mut egui::Ui) {
        ui.heading("Details");
        if !self.notebook.warnings().is_empty() {
            ui.colored_label(
                egui::Color32::from_rgb(139, 77, 22),
                "Unreadable note files",
            );
            for warning in self.notebook.warnings().iter().take(5) {
                let name = warning
                    .path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "?".into());
                ui.label(format!("{}: {}", name, warning.reason));
            }
            ui.label(egui::RichText::new("Repair from a validated Markdown copy with `notestrike repair`; the original is backed up.").small());
            ui.separator();
        }
        let Some(id) = self.selected else {
            ui.label("Select a note to see its links.");
            return;
        };
        ui.label("Title");
        ui.add_sized(
            [ui.available_width(), 34.0],
            egui::TextEdit::singleline(&mut self.rename_draft),
        );
        if ui.button("Save title").clicked() {
            self.rename();
        }
        ui.separator();
        ui.strong("Backlinks");
        let links = self.backlink_notes(id);
        if links.is_empty() {
            ui.label("No notes link here yet.");
        }
        for (source, title) in links {
            if ui.button(title).clicked() {
                self.select(source);
            }
        }
        ui.separator();
        ui.weak(format!("ID: {id}"));
    }

    fn source_editor(
        &mut self,
        ui: &mut egui::Ui,
        id: Uuid,
        scroll_to_end: bool,
    ) -> egui::containers::scroll_area::ScrollAreaOutput<()> {
        let visible_rows = ((ui.available_height().max(80.0) / 18.0).ceil() as usize).max(4);
        let area = egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .id_salt(("notestrike-source", id));
        area.show(ui, |ui| {
            let editor = egui::TextEdit::multiline(&mut self.body_draft)
                .desired_width(f32::INFINITY)
                .desired_rows(visible_rows)
                .code_editor();
            let response = ui.add(editor);
            if scroll_to_end {
                ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
            }
            if response.changed() {
                self.dirty = self
                    .notebook
                    .get(id)
                    .is_some_and(|note| self.body_draft != note.body);
            }
        })
    }

    fn editor(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.selected else {
            ui.vertical_centered(|ui| {
                ui.add_space(56.0);
                ui.heading("Start with a note");
                ui.label("Your notes are saved as local Markdown files in this notebook.");
                ui.add_space(14.0);
                if ui.button("Create a note").clicked() {
                    self.create_note();
                }
            });
            return;
        };
        let title = self
            .notebook
            .get(id)
            .map(|note| note.title.clone())
            .unwrap_or_default();
        ui.heading(title);
        ui.label(if self.dirty {
            "Unsaved changes"
        } else {
            "No unsaved edits"
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(self.dirty, egui::Button::new("Save · Ctrl+S"))
                .clicked()
            {
                self.save();
            }
            if self.dirty && ui.button("Discard").clicked() {
                self.discard();
            }
            if ui
                .button(if self.preview {
                    "Edit source"
                } else {
                    "Preview"
                })
                .clicked()
            {
                self.preview = !self.preview;
            }
            if ui.button("Reload disk").clicked() {
                if self.dirty {
                    self.status =
                        "Discard the unsaved edit or save a copy before reloading.".into();
                } else {
                    self.reload_disk();
                }
            }
        });
        if self.status.contains("changed on disk") && self.dirty {
            ui.colored_label(
                egui::Color32::from_rgb(139, 77, 22),
                "The disk copy changed while this note was open.",
            );
            if ui.button("Save editor text as a separate note").clicked() {
                self.create_copy();
            }
        }
        ui.separator();
        if self.preview {
            egui::ScrollArea::vertical().show(ui, |ui| {
                if self.body_draft.trim().is_empty() {
                    ui.label("Nothing to preview. Switch to Edit source to write this note.");
                } else {
                    markdown_preview(ui, &self.body_draft);
                }
            });
        } else {
            let scroll_to_end = self
                .capture
                .as_ref()
                .is_some_and(|capture| capture.scroll_to_end);
            self.source_editor(ui, id, scroll_to_end);
        }
    }
}

impl eframe::App for NativeEditor {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if ui.input(|input| input.modifiers.command && input.key_pressed(egui::Key::S))
            && self.dirty
        {
            self.save();
        }
        if !self.styled {
            ctx.set_theme(egui::Theme::Light);
            let mut style = (*ctx.style_of(egui::Theme::Light)).clone();
            style.visuals = egui::Visuals::light();
            style.visuals.panel_fill = egui::Color32::from_rgb(246, 247, 245);
            style.visuals.extreme_bg_color = egui::Color32::WHITE;
            style.visuals.override_text_color = Some(egui::Color32::from_rgb(30, 43, 42));
            style.visuals.selection.bg_fill = egui::Color32::from_rgb(177, 222, 209);
            style.visuals.widgets.inactive.bg_fill = egui::Color32::WHITE;
            style.visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(226, 240, 235);
            style.visuals.widgets.active.bg_fill = egui::Color32::from_rgb(191, 226, 216);
            style
                .text_styles
                .insert(egui::TextStyle::Body, egui::FontId::proportional(15.0));
            style
                .text_styles
                .insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
            style
                .text_styles
                .insert(egui::TextStyle::Heading, egui::FontId::proportional(23.0));
            style
                .text_styles
                .insert(egui::TextStyle::Monospace, egui::FontId::monospace(15.0));
            style.spacing.item_spacing = egui::vec2(8.0, 8.0);
            style.spacing.button_padding = egui::vec2(10.0, 6.0);
            ctx.set_style_of(egui::Theme::Light, style);
            self.styled = true;
        }
        let compact = ui.available_width() < 1050.0;
        egui::Panel::top("notestrike_toolbar")
            .exact_size(54.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("NoteStrike");
                    ui.separator();
                    let notebook_name = self
                        .root
                        .file_name()
                        .map(|name| name.to_string_lossy())
                        .unwrap_or_default();
                    ui.label(notebook_name.to_string())
                        .on_hover_text(self.root.display().to_string());
                });
            });
        egui::Panel::bottom("notestrike_status")
            .exact_size(30.0)
            .show(ui, |ui| {
                ui.label(egui::RichText::new(&self.status).small());
            });
        egui::Panel::left("notestrike_notes")
            .exact_size(if compact { 230.0 } else { 270.0 })
            .show(ui, |ui| self.note_list(ui));
        if !compact {
            egui::Panel::right("notestrike_context")
                .exact_size(240.0)
                .show(ui, |ui| self.context_rail(ui));
        }
        egui::CentralPanel::default().show(ui, |ui| {
            if compact {
                let backlink_count = self
                    .selected
                    .map(|id| self.notebook.backlinks(id).len())
                    .unwrap_or(0);
                egui::CollapsingHeader::new(format!("Details and backlinks ({backlink_count})"))
                    .default_open(true)
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .max_height(160.0)
                            .show(ui, |ui| self.context_rail(ui));
                    });
                ui.separator();
            }
            self.editor(ui);
        });
        self.capture_after_frame(&ctx);
    }
}

fn markdown_preview(ui: &mut egui::Ui, source: &str) {
    let mut code = false;
    for line in source.lines() {
        if line.trim_start().starts_with("```") {
            code = !code;
        } else if code {
            ui.monospace(line);
        } else if let Some(heading) = line.strip_prefix("# ") {
            ui.heading(heading);
        } else if let Some(heading) = line.strip_prefix("## ") {
            ui.strong(heading);
        } else if let Some(item) = line.strip_prefix("- ") {
            ui.label(format!("• {item}"));
        } else if line.is_empty() {
            ui.add_space(8.0);
        } else {
            // Egui labels display raw HTML literally; they never execute it or load URLs.
            ui.label(line);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn native_editor_surfaces_malformed_note_without_hiding_valid_notes() {
        let root =
            std::env::temp_dir().join(format!("notestrike-native-warning-{}", Uuid::new_v4()));
        let mut notebook = Notebook::create(&root).unwrap();
        let valid = notebook.add("Good note", "Visible text").unwrap();
        fs::write(root.join("notes").join("broken.md"), b"bad front matter").unwrap();
        let editor = NativeEditor::new(root.clone(), Notebook::open(&root).unwrap());
        assert!(editor.status.contains("need repair"));
        assert_eq!(
            editor.matching_notes(),
            vec![(valid.id, "Good note".into())]
        );
        assert_eq!(editor.notebook.warnings().len(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn editor_actions_save_reopen_search_backlinks_and_preserve_conflicts() {
        let root = std::env::temp_dir().join(format!("notestrike-native-{}", Uuid::new_v4()));
        let mut notebook = Notebook::create(&root).unwrap();
        let guide = notebook.add("Valve Guide", "Pressure: 12 bar.").unwrap();
        let field = notebook.add("Field Log", "Initial field text.").unwrap();

        let mut editor = NativeEditor::new(root.clone(), notebook);
        editor.select(field.id);
        editor.body_draft = format!("Observed blue valves. [[{}|Valve Guide]]", guide.id);
        editor.dirty = true;
        editor.save();
        assert!(!editor.dirty, "{}", editor.status);
        editor.search = "blue".into();
        assert_eq!(
            editor.matching_notes(),
            vec![(field.id, "Field Log".into())]
        );
        assert_eq!(
            editor.backlink_notes(guide.id),
            vec![(field.id, "Field Log".into())]
        );
        drop(editor);

        let reopened = Notebook::open(&root).unwrap();
        assert!(
            reopened
                .get(field.id)
                .unwrap()
                .body
                .contains(&guide.id.to_string())
        );
        let mut editor = NativeEditor::new(root.clone(), reopened);
        editor.select(guide.id);
        assert_eq!(editor.selected, Some(guide.id), "{}", editor.status);
        assert!(!editor.dirty, "{}", editor.status);
        editor.rename_draft = "Valve Handbook".into();
        editor.rename();
        assert_eq!(
            editor.status, "Title saved; ID links still point here.",
            "rename failed: selected={:?}, draft={}",
            editor.selected, editor.rename_draft
        );
        assert_eq!(
            editor.notebook.get(guide.id).unwrap().title,
            "Valve Handbook",
            "status: {}; selected: {:?}; draft: {}",
            editor.status,
            editor.selected,
            editor.rename_draft
        );
        assert_eq!(editor.backlink_notes(guide.id).len(), 1);

        editor.select(field.id);
        editor.body_draft = "Local unsaved field text.".into();
        editor.dirty = true;
        let mut external = Notebook::open(&root).unwrap();
        external.update(field.id, "Disk update wins.").unwrap();
        editor.save();
        assert!(editor.status.contains("changed on disk"));
        assert!(editor.dirty);
        editor.create_copy();
        assert!(editor.status.contains("separate copy"), "{}", editor.status);

        let final_book = Notebook::open(&root).unwrap();
        assert_eq!(final_book.get(field.id).unwrap().body, "Disk update wins.");
        assert_eq!(final_book.get(guide.id).unwrap().title, "Valve Handbook");
        assert_eq!(
            final_book.get(editor.selected.unwrap()).unwrap().body,
            "Local unsaved field text."
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn long_source_scrolls_to_final_line_in_compact_viewport() {
        let root =
            std::env::temp_dir().join(format!("notestrike-native-scroll-{}", Uuid::new_v4()));
        let mut notebook = Notebook::create(&root).unwrap();
        let mut body = String::from("# Project Hub\n\n");
        for line in 1..=60 {
            body.push_str(&format!(
                "- Review item {line:02}: capture the next decision.\n"
            ));
        }
        body.push_str("FINAL ACCESSIBLE LINE: save and close the review.\n");
        let note = notebook.add("A Project Hub", &body).unwrap();
        let mut editor = NativeEditor::new(root.clone(), notebook);
        let context = egui::Context::default();
        let mut measurement = None;
        for _ in 0..2 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 520.0),
                )),
                ..Default::default()
            };
            let mut output = context.run_ui(input, |ui| {
                let output = editor.source_editor(ui, note.id, true);
                measurement = Some((
                    output.content_size.y,
                    output.inner_rect.height(),
                    output.state.offset.y,
                ));
            });
            output.textures_delta.clear();
        }
        let (content_height, viewport_height, scroll_offset) = measurement.unwrap();
        assert!(content_height > viewport_height + 100.0);
        assert!(
            scroll_offset > 100.0,
            "source editor did not scroll to the final line"
        );
        assert!(
            editor
                .body_draft
                .ends_with("FINAL ACCESSIBLE LINE: save and close the review.\n")
        );
        fs::remove_dir_all(root).unwrap();
    }
}
