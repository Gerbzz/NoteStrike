//! NoteStrike desktop editor.
//!
//! Usage: `notestrike [FILE ...]` opens each file in a tab.
//! `notestrike --capture OUT.png [WIDTH HEIGHT] [FILE ...]` renders the real
//! window once, writes it to OUT.png, and exits (documentation screenshots).

// No console window for the release build on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::process::ExitCode;

use eframe::egui;
use notestrike::native_ui::{self, NativeDialogs, NoteStrikeApp};
use notestrike::recovery::{self, RecoveryStore};
use notestrike::workspace::{Settings, Workspace};

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut capture: Option<PathBuf> = None;
    let mut size = [960.0_f32, 640.0];
    if args.first().is_some_and(|a| a == "--capture") {
        if args.len() < 2 || !args[1].ends_with(".png") {
            eprintln!("Usage: notestrike --capture OUT.png [WIDTH HEIGHT] [FILE ...]");
            return ExitCode::FAILURE;
        }
        capture = Some(PathBuf::from(&args[1]));
        args.drain(..2);
        if args.len() >= 2
            && let (Ok(w), Ok(h)) = (args[0].parse::<u32>(), args[1].parse::<u32>())
        {
            if !(360..=2400).contains(&w) || !(300..=1600).contains(&h) {
                eprintln!("Capture size must be WIDTH 360-2400 and HEIGHT 300-1600");
                return ExitCode::FAILURE;
            }
            size = [w as f32, h as f32];
            args.drain(..2);
        }
    }
    let files: Vec<PathBuf> = args.iter().map(PathBuf::from).collect();

    let data_dir = recovery::data_dir();
    let recovery = data_dir
        .as_ref()
        .and_then(|dir| RecoveryStore::open(&dir.join("recovery")).ok());
    let settings_path = data_dir.as_ref().map(|dir| dir.join("settings.json"));
    let settings = settings_path
        .as_deref()
        .map(Settings::load)
        .unwrap_or_default();
    let mut ws = Workspace::new(recovery, settings);
    ws.open_paths(&files);

    let mut app = NoteStrikeApp::new(ws, Box::new(NativeDialogs), settings_path);
    let capture_result = capture.map(|path| app.capture_once(path));

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("NoteStrike")
        .with_app_id("notestrike")
        .with_inner_size(size)
        .with_min_inner_size([360.0, 300.0])
        .with_drag_and_drop(true);
    if let Some(icon) = native_ui::window_icon() {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    match eframe::run_native(
        "NoteStrike",
        options,
        Box::new(move |_cc| Ok(Box::new(app))),
    ) {
        Ok(()) => match capture_result {
            None => ExitCode::SUCCESS,
            Some(result) => match result.lock().unwrap().take() {
                Some(Ok(())) => ExitCode::SUCCESS,
                Some(Err(error)) => {
                    eprintln!("NoteStrike capture: {error}");
                    ExitCode::FAILURE
                }
                None => {
                    eprintln!("NoteStrike capture: window closed before the screenshot");
                    ExitCode::FAILURE
                }
            },
        },
        Err(error) => {
            eprintln!("NoteStrike window: {error}");
            ExitCode::FAILURE
        }
    }
}
