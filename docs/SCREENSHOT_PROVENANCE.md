# NoteStrike native editor screenshot provenance

`assets/screenshots/note-editor.png` is a genuine 1280 x 800 screenshot returned by NoteStrike's egui/eframe Glow renderer on 14 October 2026 UTC. A local, self-authored `demo-notebook` contains the sample “Field Log” and “Project Plan” notes; their Markdown bodies total 512 bytes. The fixture remains local and contains no user notes, personal path, client data, or downloaded media.

The first-party native editor adds a one-shot `--capture FILE.png` option. It waits for painted frames, requests `egui::ViewportCommand::Screenshot`, saves the resulting `egui::Event::Screenshot` as PNG, and closes the window. The normal `notestrike-native NOTEBOOK_DIR` path remains available. The image visibly shows the notebook list, selected note, Markdown editor, and context rail. The app exited with status 0.

| Item | SHA-256 |
| --- | --- |
| Local capture executable (not published) | `81B20467984F00B7C3BB01AE7BF9A385A4DC1CF6FC1ABBDC758663B12B775CAA` |
| Published screenshot PNG | `AD6CA85E296220C6A49959C3FF4D21B6E70FDCF2EA211032B422D60BC22F165D` |

The executable was built from the workspace NoteStrike package with `cargo build --locked --offline -j 1 -p notestrike --features native-ui --bin notestrike-native`, using cached dependencies. Cargo reported a successful dev build in 7.34 seconds. The standalone repository presents the same package source and dependency versions, with its own lockfile and without a first-party license declaration. No dependency download, clean, installer, or release was made.

The screenshot verifies one rendered editor state. It does not verify interactive editing, save and conflict handling, screen-reader behavior, attachments, or a release build. The previously documented integration tests were not rerun after this capture-only change. See `SOURCE_SCOPE.md` for ownership and dependency attribution.
