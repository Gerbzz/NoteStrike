# NoteStrike source scope

## Direction

As of 0.2, NoteStrike is a lightweight plain-text editor (a Notepad replacement). This direction supersedes the 0.1 Markdown notebook with stable-ID backlinks. Out of scope on purpose: IDE features (projects, syntax highlighting, plugins, terminals) and notebook or wiki features (note databases, links, Markdown preview).

## Contents

- `src/` is the first-party Rust source:
  - the GUI-free core: `encoding`, `eol`, `document`, `search`, `recovery`, `workspace`, `legacy`
  - the egui front end: `native_ui.rs`, `main.rs`
  - the CLI helper: `src/bin/notestrike-cli.rs`
- `tests/` holds integration tests. They create disposable files under the system temp folder only.
- `assets/branding/` contains the original NoteStrike logo (see `provenance.json`) and the official PWDS logo, which is used only in Help > About.
- `assets/screenshots/` holds real frames of the current build, rendered by `notestrike --capture` from sample text. Commands and hashes are in `docs/screenshots.md`.

## Reused from 0.1

- The eframe/egui shell and the one-shot `--capture` screenshot code.
- The Windows sharing-violation retry around the final rename, now used by `document::write_atomic`.
- The 0.1 note front-matter parser, now read-only in `legacy.rs` and used only to import notes.
- The logo assets.

## Replaced, and why

- The `Notebook` store (`src/lib.rs`), the notebook CLI (`src/main.rs`), and the notebook editor (`src/native_ui.rs`, `src/bin/notestrike-native.rs`) were replaced. They modelled notes with IDs, titles, and backlinks inside a notebook folder. A plain-text editor needs arbitrary files, encodings, line endings, tabs, and recovery instead.
- The 0.1 screenshots and their provenance documents were removed because they showed the old interface. The hosted capture script was removed for the same reason.

## User data

NoteStrike 0.1 stored notes only in user-chosen notebook folders (`DIR/notes/*.md`). 0.2 never writes inside such a folder: import copies notes elsewhere with create-new semantics, and tests verify that the notebook's bytes and timestamps do not change. NoteStrike 0.2 writes only:

- the files the user saves;
- its own settings and recovery journal in `%LOCALAPPDATA%\PWDS\NoteStrike` (or `NOTESTRIKE_DATA_DIR`).

## Dependencies

All dependencies are third-party crates from crates.io, pinned by `Cargo.lock`: eframe/egui, rfd, image, sha2, uuid, serde_json, and same-file. They are MIT, Apache-2.0, or Unlicense licensed (see the README credits). System fonts (Segoe UI, Consolas) are loaded from the user's Windows installation at runtime and are not redistributed.

## License status

The source and tests are first-party NoteStrike work. This checkout makes no first-party license grant. A binary distribution needs a review of the exact dependency and asset notices. Windows 11 is the only tested platform.
