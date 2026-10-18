<p align="center">
  <img src="assets/branding/NoteStrike.png" alt="NoteStrike hedgehog logo" width="128" height="128">
</p>

<h1 align="center">NoteStrike</h1>

<p align="center"><strong>A lightweight plain-text editor for Windows, Linux and macOS, written in Rust.</strong><br>
Tabs, safe saves, encodings and line endings handled honestly, and your unsaved work recovered after a crash.</p>

<p align="center">
  Status: <strong>alpha</strong> · Hands-on tested: Windows 11 · Linux and macOS: see <a href="#platforms">Platforms</a> · Part of the PWDS Strike suite
</p>

![NoteStrike editing a sample checklist in the light theme, with two tabs and the status bar showing line, column, line endings and encoding](assets/screenshots/notestrike-editor-light.png)

*Real NoteStrike window in the **Light** theme, captured from the current build with sample text. Dark-theme captures are under [Screenshots](#screenshots).*

## Why NoteStrike

NoteStrike replaces Notepad for everyday text files: configs, logs, notes, READMEs. It stays small and familiar on purpose. It is **not** an IDE and **not** a notebook or wiki: no projects, no plugins, no Markdown rendering, no links between notes.

What it does care about is not losing or corrupting your text:

- **Familiar by design.** Menu bar, tabs, status bar, and the Notepad shortcuts you already know.
- **Safe saves.** Every save writes a temporary file, flushes it to disk, and then replaces the original in one step. If the save is interrupted, the original file is still intact.
- **Honest encodings.** UTF-8 (with or without BOM), UTF-16 LE/BE, and Windows-1252 are detected and written back the way they were found. Bytes that cannot be decoded are reported, never silently replaced.
- **Crash recovery.** Unsaved text is journaled in the background and offered back after a crash or forced shutdown.

## Features

Everything in this list is implemented and covered by automated tests (see [Verification](#verification)). The list of what has *not* yet been checked by hand on a real desktop is under [Implemented vs planned](#implemented-vs-planned).

### Editing
- Plain-text editing in tabs, with new, open, save, save as, save all, and close tab.
- Find and replace: match case, whole word, wrap around, find next/previous, replace, replace all, and a live match count.
- Go to line, undo/redo, cut/copy/paste, select all.
- Word wrap and text zoom (50–400%).
- **Themes:** View > Theme > System / Light / Dark. The default is System, which follows Windows or macOS (reported by winit). On Linux, where winit cannot report the theme under X11, System asks the desktop instead (`GTK_THEME`, then the GNOME `color-scheme` and `gtk-theme` settings). If nothing reports a preference, System uses the dark theme (egui's fallback). A change applies immediately and is remembered across restarts (in `settings.json`, see [Where NoteStrike keeps its data](#where-notestrike-keeps-its-data)). Both themes are checked against WCAG AA contrast in the tests: text, dialogs, notices, selection, links, and buttons.
- Open files from the command line (`notestrike a.txt b.log`) or by dragging them onto the window.

### Encodings and line endings
- Detects UTF-8, UTF-8 with BOM, UTF-16 LE/BE (with or without BOM, sniffed from NUL patterns), and falls back to Windows-1252 for legacy single-byte files.
- Damaged UTF-8 or UTF-16 is decoded as far as possible. A notice shows how many byte sequences could not be decoded and where the first one is, and offers to reopen the file in another encoding. Saving such a file asks first, because the U+FFFD replacement characters would become permanent.
- Saving in Windows-1252 refuses characters it cannot represent and offers UTF-8 instead. It never writes `?`.
- New documents use the platform's convention: CRLF on Windows, LF on Linux and macOS. Existing files keep their own on every OS: CRLF, LF, and CR line endings are detected and kept on save. Mixed line endings are reported, and the dominant one is used.
- The status bar shows, and lets you change, the encoding and line endings used for the next save.

### Not losing work
- **Atomic saves:** temp file in the same folder, `fsync`, then rename. A failed or interrupted save leaves the original untouched.
- **External changes:** open files are checked every 2 seconds. If another program changed or deleted a file, a notice offers Reload or Keep my version. Saving over an external change asks you to overwrite, save as a new file, or cancel.
- **Read-only files** are detected and offered Save As. On Linux and macOS a file you cannot write (for example one owned by root) counts as read-only too, and saving keeps the original file's permission bits.
- **Unsaved-change prompts** on tab close, window close (including Alt+F4, and Cmd+Q on macOS), and File > Exit: Save / Don't save / Cancel. Cancelling any Save As dialog stops the close.
- **Crash/session recovery:** each tab is journaled to the recovery folder (see [Where NoteStrike keeps its data](#where-notestrike-keeps-its-data)) about a second after you stop typing. After a crash, NoteStrike offers Restore / Discard / Later for unsaved text and reopens the files that were open. A normal exit deletes the journal.
- **Large files:** files over 8 MiB need confirmation; files over 64 MiB are refused (NoteStrike keeps the whole text in memory).

### Accessibility
- AccessKit is enabled. The editor, tabs ("Tab 2 of 3: notes.txt, modified"), and icon-only buttons have accessible names, and the find and replace fields are labelled.
- Every command has a keyboard shortcut. The first button of each dialog is focused, and Esc cancels.

## Screenshots

Every image is a real frame of the current build in the theme named under it. All text is sample content.

| Light theme | Dark theme |
|---|---|
| ![Editor in the light theme with two tabs and the status bar](assets/screenshots/notestrike-editor-light.png)<br>**Editor, Light** | ![Editor in the dark theme with two tabs and the status bar](assets/screenshots/notestrike-editor-dark.png)<br>**Editor, Dark** |
| ![Find and replace bar in the light theme with the match "booking" selected in the text](assets/screenshots/notestrike-find-replace-light.png)<br>**Find and replace, Light** | ![Find and replace bar in the dark theme with the match "booking" selected in the text](assets/screenshots/notestrike-find-replace-dark.png)<br>**Find and replace, Dark** |
| ![About dialog in the light theme with the NoteStrike and PWDS logos](assets/screenshots/notestrike-about-light.png)<br>**About, Light** | ![About dialog in the dark theme; the PWDS logo sits on a light plate](assets/screenshots/notestrike-about-dark.png)<br>**About, Dark** |
| ![Notices for an undecodable byte and mixed line endings in the light theme](assets/screenshots/notestrike-damaged-utf8-light.png)<br>**Damaged UTF-8 notices, Light** | ![Notices for an undecodable byte and mixed line endings in the dark theme](assets/screenshots/notestrike-damaged-utf8-dark.png)<br>**Damaged UTF-8 notices, Dark** |
| ![NoteStrike at 420 pixels wide with word-wrapped sample text](assets/screenshots/notestrike-compact-light.png)<br>**Narrow window (420 px), Light** | ![Recover unsaved work dialog listing an untitled document](assets/screenshots/notestrike-crash-recovery-light.png)<br>**Recovery after a real crash, Light** |

The app's own `--capture` mode rendered and saved each frame at the window's size. For the find/replace and About shots it was run with `--find` or `--about`, which open that bar or dialog before the frame is taken. These are not mockups. [docs/screenshots.md](docs/screenshots.md) records the commands and hashes.

## Keyboard shortcuts

| Action | Windows and Linux | macOS |
|---|---|---|
| New tab / Open | Ctrl+N (or Ctrl+T) / Ctrl+O | Cmd+N (or Cmd+T) / Cmd+O |
| Save / Save as | Ctrl+S / Ctrl+Shift+S | Cmd+S / Shift+Cmd+S |
| Close tab | Ctrl+W | Cmd+W |
| Next / previous tab | Ctrl+Tab / Ctrl+Shift+Tab | Ctrl+Tab / Ctrl+Shift+Tab (Cmd+Tab is the app switcher) |
| Undo / Redo | Ctrl+Z / Ctrl+Y | Cmd+Z / Shift+Cmd+Z |
| Find / Replace | Ctrl+F / Ctrl+H | Cmd+F / Option+Cmd+F (Cmd+H hides the app) |
| Find next / previous | F3 / Shift+F3 | Cmd+G / Shift+Cmd+G, or F3 / Shift+F3 |
| Go to line | Ctrl+G | Cmd+L |
| Zoom in / out / reset | Ctrl+Plus / Ctrl+Minus / Ctrl+0 | Cmd+Plus / Cmd+Minus / Cmd+0 |
| Quit (asks about unsaved tabs) | Alt+F4 | Cmd+Q |
| Close the find bar or a dialog | Esc | Esc |

The menus and Help > Keyboard shortcuts show the keys for the platform you are on; they are generated from the same table the editor uses (`src/native_ui.rs`, `mod keys`). On macOS NoteStrike replaces winit's default app menu, whose Quit would end the process without asking about unsaved tabs, with its own menu (About, Services, Hide, Quit, Window); its Quit closes the window through the normal close request.

## Coming from NoteStrike 0.1 (Markdown notebook)

NoteStrike 0.1 was a Markdown notebook prototype. It kept notes in a folder you chose (`notestrike init DIR`) as `DIR/notes/*.md` files with a small front-matter header. It never used a fixed AppData or Documents location.

**NoteStrike 0.2 never deletes, moves, or rewrites those notes.** You have two ways to keep using them:

- **Import as plain files:** File > *Import NoteStrike 0.1 notebook…*, or `notestrike-cli import-notebook NOTEBOOK_DIR DEST_DIR`. Each note's body is copied to `DEST_DIR/<title>.md` without the front matter, and the copy keeps the note's modified time. Notes that cannot be parsed are copied byte for byte. Existing files in the destination are never overwritten (`Title (2).md` is used instead), and a destination inside the notebook's `notes` folder is refused. The notebook is only read.
- **Open directly:** a 0.1 note is a plain `.md` file, so File > Open works on it. Opening does not change it; only saving your own edits does.

`notestrike-cli list-notebook NOTEBOOK_DIR` lists a notebook without touching it. Backup files the 0.1 repair command left behind (`*.md.<id>.bak`) and the empty `assets` folder are not imported, but they are left in place.

## Build and run

Requires a recent stable Rust toolchain from [rustup](https://rustup.rs) (tested locally with 1.96.1; CI uses the current stable).

### Prerequisites per OS

| OS | Build prerequisites | At run time |
|---|---|---|
| Windows 10/11 | Rust (MSVC toolchain) and the Visual Studio Build Tools "Desktop development with C++" workload, which rustup offers to install. | Nothing extra. OpenGL 2+ from the graphics driver. |
| macOS 11+ | Rust and the Xcode Command Line Tools (`xcode-select --install`). | Nothing extra. |
| Linux (Debian/Ubuntu) | `sudo apt install build-essential pkg-config libxkbcommon-dev libwayland-dev libx11-dev libxcursor-dev libxrandr-dev libxi-dev libgl1-mesa-dev` | An X11 or Wayland session with OpenGL (Mesa is fine), `libxkbcommon-x11-0`, and `xdg-desktop-portal` with a backend (`xdg-desktop-portal-gtk`, `-gnome` or `-kde`) for the Open/Save dialogs. |
| Linux (Fedora) | `sudo dnf install gcc pkg-config libxkbcommon-devel wayland-devel libX11-devel libXcursor-devel libXrandr-devel libXi-devel mesa-libGL-devel` | As above (`xdg-desktop-portal-gtk` or `-kde`). Fedora package names are the equivalents of the Ubuntu list; CI does not build on Fedora. |

On Linux the file dialogs come from `rfd`'s default XDG desktop portal backend, so no GTK development headers (`libgtk-3-dev`) are needed to build. eframe loads the X11, Wayland, xkbcommon and OpenGL libraries at run time; the `-dev` packages above are the set CI installs and builds with on Ubuntu.

### Build

```sh
cargo build --release --locked
./target/release/notestrike                 # the editor (notestrike.exe on Windows)
./target/release/notestrike notes.txt       # open files
./target/release/notestrike-cli inspect notes.txt
```

`cargo build --no-default-features` builds only the library and `notestrike-cli`, with no GUI dependencies. Set `NOTESTRIKE_DATA_DIR` to keep settings and the recovery journal somewhere else (for example, for a portable copy).

### Where NoteStrike keeps its data

| OS | `settings.json` | Recovery journal |
|---|---|---|
| Windows | `%APPDATA%\PWDS\NoteStrike` | `%LOCALAPPDATA%\PWDS\NoteStrike\recovery` |
| macOS | `~/Library/Application Support/NoteStrike` | `~/Library/Application Support/NoteStrike/recovery` |
| Linux | `$XDG_CONFIG_HOME/notestrike` (default `~/.config/notestrike`) | `$XDG_DATA_HOME/notestrike/recovery` (default `~/.local/share/notestrike/recovery`) |

With `NOTESTRIKE_DATA_DIR` set, both live in that folder (`settings.json` and `recovery/`). Earlier builds kept `settings.json` next to the journal (`%LOCALAPPDATA%\PWDS\NoteStrike` on Windows, `~/.local/share/notestrike` on Linux and macOS). On first start NoteStrike copies it to the new location, and on macOS also copies an unrecovered journal, so it is offered for recovery. The old files are never changed or deleted, and a file already in the new location is never overwritten.

## Verification

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-features --all-targets -- -D warnings
cargo clippy --locked --no-default-features --all-targets -- -D warnings
cargo test --locked --all-features
```

The matrix version of `.github/workflows/native-editor.yml` runs exactly these on Windows, Ubuntu and macOS, then builds release binaries and captures the real window with `scripts/ci_capture.py`. See [Platforms](#platforms) for what has actually run so far.

The tests (70 on Windows and 71 on Linux and macOS, where one more checks Unix permission bits; plus one helper that a test runs as a child process) create their own files in the system temp folder and never touch user documents. They cover:

- Round trips of every encoding and line ending, byte for byte.
- Damaged UTF-8 and UTF-16, binary files, and Windows-1252 fallback.
- Interrupted saves (temp file cleaned up, or left behind as a killed process would leave it), read-only files, external changes and deleted files, touch-without-change, repeated saves, size limits.
- Cancelling every dialog and prompt; save/close/quit flows; reopening.
- Recovery after a simulated crash and after a real `abort()` of a child process (checked to be SIGABRT on Unix and the `__fastfail` status on Windows).
- Settings and recovery locations for every OS, and copying data from older locations without touching the originals.
- Platform line-ending defaults for new tabs; platform keyboard shortcuts and their menu labels.
- The 0.1 notebook import, with byte and timestamp checks proving the notebook is unchanged.
- Headless frames of the real UI: typing, Ctrl+S, the window close request, every prompt, and find selection.
- Themes: System follows the OS and explicit choices override it, a change applies on the next frame, the choice persists across a restart, and every colour pair passes WCAG AA in both themes.

## Platforms

A platform is listed as **supported** only after CI has built and tested it there, with a link to that run. Hands-on testing (a person using the editor on a real desktop) is reported separately.

| Platform | Status | Evidence |
|---|---|---|
| Windows (x64) | **Supported.** Built and tested in CI; hands-on tested on Windows 11 | [CI run 39163714508](https://github.com/Gerbzz/NoteStrike/actions/runs/39163714508) on `windows-latest`, rustc 1.99.1: fmt check, `cargo test` with the GUI feature (70 passed, 0 failed), debug build of both binaries |
| Linux (Ubuntu, x64) | **Supported (CI only).** Built and tested in CI, not hands-on tested | [CI run 39163714508](https://github.com/Gerbzz/NoteStrike/actions/runs/39163714508) on `ubuntu-latest`, rustc 1.99.1: `cargo test` with the GUI feature (71 passed, 0 failed), debug build, and a 960x640 capture of the real window under xvfb |
| macOS (Apple silicon) | **Not verified yet.** Not built or tested anywhere so far | The macOS CI job is in the matrix workflow, which has not been pushed yet (see below) |

That run used the older two-job workflow, which has no clippy step, no release build and no macOS job. The matrix workflow in `.github/workflows/native-editor.yml` (windows-latest, ubuntu-latest and macos-latest, with fmt, clippy `-D warnings`, `cargo test --locked --all-features`, release builds, window captures, and artifacts) is ready. It is not yet on GitHub because the push needs a token with the `workflow` scope. This table gets updated when it runs.

## Limitations

- **Hands-on tested on Windows 11 only.** Linux and macOS are built and tested in CI (see [Platforms](#platforms)), but nobody has used the editor on those desktops yet: native file dialogs, clipboard, IME, the macOS app menu and screen readers there are unverified.
- **Not a large-file editor.** The whole file stays in memory and is laid out by egui; a few MB is comfortable, and files over 64 MiB are refused.
- **Mixed line endings** are unified to the dominant style on save (you are told when the file opens).
- **Saving replaces the file** through a rename. This keeps the content safe, but a hard link to the old file keeps the old content, and Windows ACLs or alternate data streams on the original file are not copied.
- **Polling** for external changes compares size and modified time; a same-size edit within the file system's timestamp resolution is only caught at save time, when the file is re-hashed.
- **No printing**, no right-to-left or complex-script shaping beyond what egui provides, no spell check, no session restore after a *normal* exit (only after a crash).
- **No IME composition testing** has been done.
- **No installer** and no file associations yet.

## Implemented vs planned

| Area | Status |
|---|---|
| Tabs, open/save/save as/save all, find/replace, go to line, zoom, wrap, themes | Implemented |
| Encoding detection and handling, line-ending preservation | Implemented |
| Atomic saves, external-change and read-only handling, unsaved-change prompts | Implemented |
| Crash recovery journal | Implemented |
| 0.1 notebook import | Implemented |
| Interactive desktop walkthrough with mouse and keyboard (OS dialogs, clipboard menu items, screen reader) | **Not yet verified.** See [docs/screenshots.md](docs/screenshots.md) |
| Installer, file associations, "Open with" | Planned |
| Session restore after normal exit, recent files | Planned |
| Printing | Planned |
| Linux and macOS: CI build, tests and window capture | Implemented, see [Platforms](#platforms) |
| Linux and macOS: hands-on desktop verification | Planned |

## Credits and licenses

- Built with [egui / eframe](https://github.com/emilk/egui) (MIT OR Apache-2.0), [rfd](https://github.com/PolyMeilex/rfd) (MIT) for native file dialogs, [image](https://github.com/image-rs/image) (MIT OR Apache-2.0), [sha2](https://github.com/RustCrypto/hashes) (MIT OR Apache-2.0), [uuid](https://github.com/uuid-rs/uuid) (MIT OR Apache-2.0), [serde_json](https://github.com/serde-rs/json) (MIT OR Apache-2.0), and [same-file](https://github.com/BurntSushi/same-file) (MIT OR Unlicense). The full dependency graph is in `Cargo.lock`.
- On Windows, NoteStrike uses the Segoe UI and Consolas fonts already installed with the system, and on Linux DejaVu Sans and DejaVu Sans Mono when installed; it does not bundle them. egui's built-in fonts are the fallback (and are used on macOS).
- The README layout takes inspiration from [PhotoCraft](https://github.com/storytold/photocraft) and [ArtCraft](https://getartcraft.com/): a short pitch, highlights, real screenshots with captions, and a frank status section. No code, text, or assets were copied from either project.
- The NoteStrike hedgehog logo is original Strike suite artwork ([provenance](assets/branding/provenance.json)). The [PWDS logo](assets/branding/PWDS-logo.png) appears only in Help > About.

NoteStrike is a [ProWebDevSolutions](https://prowebdevsolutions.com) project. This checkout makes no first-party license grant. See [SOURCE_SCOPE.md](SOURCE_SCOPE.md).
