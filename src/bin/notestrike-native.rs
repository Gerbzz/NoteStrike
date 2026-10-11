//! Native Windows entry point. Build and action-method tests are verified;
//! an interactive desktop and accessibility walkthrough is still pending.

use std::path::PathBuf;
use std::process::ExitCode;

use eframe::egui;
use notestrike::Notebook;
use notestrike::native_ui::NativeEditor;

fn main() -> ExitCode {
    let Some(root) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("Usage: notestrike-native NOTEBOOK_DIR (run `notestrike init` first)");
        return ExitCode::FAILURE;
    };
    let notebook = match Notebook::open(&root) {
        Ok(notebook) => notebook,
        Err(error) => {
            eprintln!("NoteStrike: {error}");
            return ExitCode::FAILURE;
        }
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("NoteStrike")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([800.0, 520.0]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    match eframe::run_native(
        "NoteStrike",
        options,
        Box::new(move |_cc| Ok(Box::new(NativeEditor::new(root, notebook)))),
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("NoteStrike window: {error}");
            ExitCode::FAILURE
        }
    }
}
