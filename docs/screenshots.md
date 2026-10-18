# Screenshot notes

Every image in `assets/screenshots/` is a real frame of the NoteStrike build at commit `ab6deae`. Each was produced on Windows 11 by the app's own `--capture OUT.png WIDTH HEIGHT [FILES]` mode. That mode opens the real window, waits a few painted frames, asks eframe for a screenshot of the window, writes it as a PNG, and exits cleanly. The images are cropped to the app window by construction. No mockups or edits were made.

All visible text is sample content written for these captures. The files lived in `%TEMP%\NoteStrike-samples\docs`, and each run used its own `NOTESTRIKE_DATA_DIR`.

| File | Command (abridged) | SHA-256 |
|---|---|---|
| `notestrike-editor-light.png` | `notestrike --capture … 960 600 sample-legacy-1252.txt sample-checklist.txt` | `c52294e0e4ddf07aca0a3704a691e7b23a3e9152c39edbc223a7f9e4ab396314` |
| `notestrike-damaged-utf8-dark.png` | settings `{"theme":"dark","zoom_percent":110}`; `notestrike --capture … 960 600 sample-checklist.txt sample-damaged.txt` (contains a stray 0xFF byte and mixed CRLF/LF) | `958568c0cfde7f4b559adacb954bee25dcfd116af358787e39e40c62a1dda64e` |
| `notestrike-compact.png` | `notestrike --capture … 420 600 sample-checklist.txt` | `b58c211afbd763baf7b22c65da92cd21bdf160c65a1d029d421a49e65a8c53a9` |
| `notestrike-crash-recovery.png` | The `crash_child` test helper (`tests/recovery.rs`) typed text into a workspace, journaled it, and called `std::process::abort()`. Then `notestrike --capture … 960 600` was started against that data folder. | `6b35048a4ea1755612398db859c270a661b05d04d1b4ff494e009920f04e5c30` |

## What was not verified interactively

The plan was to also drive the running build with mouse and keyboard through desktop automation (computer use): typing, menus, the Windows file dialogs, cut/copy/paste from the Edit menu, and closing with unsaved changes. When that access was requested for the NoteStrike window, it was **denied**, so no interactive desktop session took place.

These behaviours are covered only by headless tests that run the real UI code on synthetic input, not by a person or an automated session using the real window:

- typing and Ctrl+S
- the window close request with unsaved text
- rendering of every prompt
- the narrow-layout find bar

These behaviours are still unverified on a real desktop:

- the native Open, Save As, and folder pickers (rfd)
- the Edit menu Cut/Copy/Paste items, which use egui viewport clipboard commands
- drag-and-drop of files onto the window
- screen-reader output through AccessKit
