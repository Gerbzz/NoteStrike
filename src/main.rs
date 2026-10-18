//! NoteStrike desktop editor.
//!
//! Usage: `notestrike [FILE ...]` opens each file in a tab.
//! `notestrike --capture OUT.png [WIDTH HEIGHT] [FILE ...]` renders the real
//! window once, writes it to OUT.png, and exits (documentation screenshots).
//! In capture mode `--find TEXT REPLACEMENT` or `--about` (after the size)
//! opens the find/replace bar or the About dialog first.

// No console window for the release build on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::process::ExitCode;

use eframe::egui;
use notestrike::native_ui::{self, NativeDialogs, NoteStrikeApp};
use notestrike::paths::AppDirs;
use notestrike::recovery::RecoveryStore;
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
    // Capture-only scene setup, so documentation screenshots can show the
    // find/replace bar or the About dialog of the real window:
    // `--find TEXT REPLACEMENT` or `--about`, right after the size.
    let mut find: Option<(String, String)> = None;
    let mut about = false;
    if capture.is_some() {
        if args.first().is_some_and(|a| a == "--find") && args.len() >= 3 {
            find = Some((args[1].clone(), args[2].clone()));
            args.drain(..3);
        } else if args.first().is_some_and(|a| a == "--about") {
            about = true;
            args.drain(..1);
        }
    }
    let files: Vec<PathBuf> = args.iter().map(PathBuf::from).collect();

    let dirs = AppDirs::current();
    if let Some(dirs) = &dirs
        && let Err(error) = dirs.migrate()
    {
        eprintln!("NoteStrike: could not copy settings from an earlier version: {error}");
    }
    let recovery = dirs
        .as_ref()
        .and_then(|dirs| RecoveryStore::open(&dirs.recovery_dir).ok());
    let settings_path = dirs.map(|dirs| dirs.settings_file);
    let settings = settings_path
        .as_deref()
        .map(Settings::load)
        .unwrap_or_default();
    let mut ws = Workspace::new(recovery, settings);
    ws.open_paths(&files);
    if let Some((needle, replacement)) = find {
        ws.find.needle = needle;
        ws.find.replacement = replacement;
        ws.show_find(true);
        ws.find.focus_request = false;
        ws.find_step(true);
    }
    if about {
        ws.prompts.push_back(notestrike::workspace::Prompt::About);
    }

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
