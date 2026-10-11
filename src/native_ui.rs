//! Native editor adapter for the local notebook core.

use std::path::PathBuf;

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
    status: String,
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
            status: if warning_count == 0 {
                "Ready".into()
            } else {
                format!("{warning_count} note file(s) need repair; valid notes remain available.")
            },
        }
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
        self.notebook
            .backlinks(id)
            .into_iter()
            .filter_map(|source| {
                self.notebook
                    .get(source)
                    .map(|note| (source, note.title.clone()))
            })
            .collect()
    }

    fn note_list(&mut self, ui: &mut egui::Ui) {
        ui.heading("Notebook");
        ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Search titles and text"));
        if ui.button("New note").clicked() {
            self.create_note();
        }
        ui.separator();
        let notes = self.matching_notes();
        egui::ScrollArea::vertical().show(ui, |ui| {
            if notes.is_empty() {
                ui.label("No matching notes.");
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
        ui.heading("Context");
        if !self.notebook.warnings().is_empty() {
            ui.colored_label(egui::Color32::YELLOW, "Unreadable note files");
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
        ui.label("Rename note");
        ui.text_edit_singleline(&mut self.rename_draft);
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

    fn editor(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.selected else {
            ui.centered_and_justified(|ui| {
                ui.heading("Create a note to begin.");
            });
            return;
        };
        let title = self
            .notebook
            .get(id)
            .map(|note| note.title.clone())
            .unwrap_or_default();
        ui.horizontal(|ui| {
            ui.heading(title);
            if self.dirty {
                ui.strong("Unsaved");
            }
        });
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
                egui::Color32::YELLOW,
                "The disk copy changed while this note was open.",
            );
            if ui.button("Save editor text as a separate note").clicked() {
                self.create_copy();
            }
        }
        ui.separator();
        if self.preview {
            egui::ScrollArea::vertical().show(ui, |ui| markdown_preview(ui, &self.body_draft));
        } else {
            let size = ui.available_size();
            let editor = egui::TextEdit::multiline(&mut self.body_draft)
                .desired_width(f32::INFINITY)
                .code_editor();
            if ui.add_sized(size, editor).changed() {
                self.dirty = self
                    .notebook
                    .get(id)
                    .is_some_and(|note| self.body_draft != note.body);
            }
        }
    }
}

impl eframe::App for NativeEditor {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if ui.input(|input| input.modifiers.command && input.key_pressed(egui::Key::S))
            && self.dirty
        {
            self.save();
        }
        egui::Panel::top("notestrike_toolbar")
            .exact_size(44.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.strong("NoteStrike");
                    ui.separator();
                    ui.label(self.root.display().to_string());
                    ui.separator();
                    ui.label(&self.status);
                });
            });
        egui::Panel::left("notestrike_notes")
            .exact_size(270.0)
            .show(ui, |ui| self.note_list(ui));
        egui::Panel::right("notestrike_context")
            .exact_size(240.0)
            .show(ui, |ui| self.context_rail(ui));
        egui::CentralPanel::default().show(ui, |ui| self.editor(ui));
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
}
