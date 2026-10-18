# Screenshot notes

Every image in `assets/screenshots/` is a real frame of the NoteStrike build at commit `abb8c75`. Each was produced on Windows 11 by the app's own `--capture OUT.png WIDTH HEIGHT [--find TEXT REPLACEMENT | --about] [FILES]` mode. That mode opens the real window and, if asked, opens the find/replace bar (and finds the first match) or the About dialog. It then waits a few painted frames, asks eframe for a screenshot of the window, writes it as an opaque PNG, and exits cleanly. The images are cropped to the app window by construction. No mockups or edits were made.

All visible text is sample content written for these captures. The files lived in `%TEMP%\NoteStrike-samples\docs`. The theme was set the same way a user's choice is stored: through `settings.json` in the run's `NOTESTRIKE_DATA_DIR`, containing `"theme":"light"` or `"theme":"dark"`.

| File | Theme | Command (abridged) | SHA-256 |
|---|---|---|---|
| `notestrike-editor-light.png` | Light | `--capture … 960 600 sample-legacy-1252.txt sample-checklist.txt` | `c52294e0e4ddf07aca0a3704a691e7b23a3e9152c39edbc223a7f9e4ab396314` |
| `notestrike-editor-dark.png` | Dark | same | `4222a1ad25716d2f2cc32c59c94e8603816d3e041ab561bb612d56e8648b64fb` |
| `notestrike-find-replace-light.png` | Light | `--capture … 960 600 --find booking reservation sample-checklist.txt` | `c3921d1b4ed8933344e53d6d1bc0a11723c8dcd32c6e57e58661219af5a097ec` |
| `notestrike-find-replace-dark.png` | Dark | same | `26e678d550ddd32ab751fb76686f8d4308db969840149aaaa6ac6ab87cc0601b` |
| `notestrike-about-light.png` | Light | `--capture … 960 600 --about sample-checklist.txt` | `b389de1ae7ad26f37e45c6484dc1ac347cbf277f06000ad156aee6f7b9b53c0a` |
| `notestrike-about-dark.png` | Dark | same | `e8ac5c58fddb46e90b6e7bf776dd94eea5b7d6a6ac0eac0787e2a832245b87ca` |
| `notestrike-damaged-utf8-light.png` | Light | `--capture … 960 600 sample-checklist.txt sample-damaged.txt` (stray 0xFF byte, mixed CRLF/LF) | `513ac94a95d493ae7e793adbc2622e863e336be671007f2f5b58ce25b4e3bd4f` |
| `notestrike-damaged-utf8-dark.png` | Dark | same | `aa128590fc221b32069581efada91bf3235d059e0369230677d57a5e0257ae52` |
| `notestrike-compact-light.png` | Light | `--capture … 420 600 sample-checklist.txt` | `ca6471e5380d9bdada75aaee974cd2de5e12471ba8d0277a99bdc49b264cafe7` |
| `notestrike-crash-recovery-light.png` | Light | The `crash_child` test helper (`tests/recovery.rs`) journaled typed text and called `std::process::abort()`. Then `--capture … 960 600` ran against that data folder. | `3628bd8bc6a09dfd460267c1d892746c34a5bfd23a93d0c1f874d41e4c36eed7` |

## What was not verified interactively

The plan was to also drive the running build with mouse and keyboard through desktop automation (computer use): typing, menus (including View > Theme), the Windows file dialogs, cut/copy/paste from the Edit menu, and closing with unsaved changes. When that access was requested for the NoteStrike window, it was **denied**, so no interactive desktop session took place.

These behaviours are covered only by headless tests that run the real UI code on synthetic input, not by a person or an automated session using the real window:

- typing and Ctrl+S
- the window close request with unsaved text
- rendering of every prompt
- find selection
- theme switching, persistence, and System resolution

These behaviours are still unverified on a real desktop:

- the native Open, Save As, and folder pickers (rfd)
- the Edit menu Cut/Copy/Paste items
- drag-and-drop
- live OS theme changes while the app is running
- screen-reader output through AccessKit
