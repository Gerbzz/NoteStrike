//! Native Windows entry point. Build and action-method tests are verified;
//! an interactive desktop and accessibility walkthrough is still pending.

use std::path::PathBuf;
use std::process::ExitCode;

use eframe::egui;
use notestrike::Notebook;
use notestrike::native_ui::NativeEditor;

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let Some(root) = args.next().map(PathBuf::from) else {
        eprintln!(
            "Usage: notestrike-native NOTEBOOK_DIR [--capture FILE.png [WIDTH HEIGHT]] (run `notestrike init` first)"
        );
        return ExitCode::FAILURE;
    };
    let capture_args: Vec<_> = args.collect();
    let (capture_path, window_size) = match capture_args.as_slice() {
        [] => (None, [1280.0, 800.0]),
        [flag, output] if flag == "--capture" => {
            (Some(PathBuf::from(output)), [1280.0, 800.0])
        }
        [flag, output, width, height] if flag == "--capture" => {
            let dimensions = (width.to_string_lossy().parse::<u32>(), height.to_string_lossy().parse::<u32>());
            match dimensions {
                (Ok(width), Ok(height)) if (800..=2000).contains(&width) && (520..=1400).contains(&height) => {
                    (Some(PathBuf::from(output)), [width as f32, height as f32])
                }
                _ => {
                    eprintln!("Capture window size must be WIDTH 800-2000 and HEIGHT 520-1400");
                    return ExitCode::FAILURE;
                }
            }
        }
        _ => {
            eprintln!("Usage: notestrike-native NOTEBOOK_DIR [--capture FILE.png [WIDTH HEIGHT]]");
            return ExitCode::FAILURE;
        }
    };
    if capture_path.as_ref().is_some_and(|output| output.extension().is_none_or(|ext| ext != "png")) {
        eprintln!("Capture path must end in .png");
        return ExitCode::FAILURE;
    }
    let notebook = match Notebook::open(&root) {
        Ok(notebook) => notebook,
        Err(error) => {
            eprintln!("NoteStrike: {error}");
            return ExitCode::FAILURE;
        }
    };
    let mut editor = NativeEditor::new(root, notebook);
    let capture_result = capture_path.map(|path| editor.capture_once(path));
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("NoteStrike")
            .with_inner_size(window_size)
            .with_min_inner_size([800.0, 520.0]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    match eframe::run_native(
        "NoteStrike",
        options,
        Box::new(move |_cc| Ok(Box::new(editor))),
    ) {
        Ok(()) => {
            if let Some(result) = capture_result {
                match result.lock().unwrap().take() {
                    Some(Ok(())) => ExitCode::SUCCESS,
                    Some(Err(error)) => {
                        eprintln!("NoteStrike capture: {error}");
                        ExitCode::FAILURE
                    }
                    None => {
                        eprintln!("NoteStrike capture: window closed before screenshot completed");
                        ExitCode::FAILURE
                    }
                }
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(error) => {
            eprintln!("NoteStrike window: {error}");
            ExitCode::FAILURE
        }
    }
}
