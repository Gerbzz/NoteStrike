# Current native editor visual

The [wide capture](../assets/screenshots/current-editor-wide.png) and [compact capture](../assets/screenshots/current-editor-compact.png) are unaltered app-window PNGs from `notestrike-native` at source commit `dc4a2fc29b8e694975c58605ea8aeba38bc34cc2` in [hosted run 38989364672](https://github.com/Gerbzz/NoteStrike/actions/runs/38989364672). The Linux runner used Xvfb and Mesa software OpenGL after the hosted Windows runner lacked OpenGL 2.0. No native executable was downloaded or run on the Office PC.

The workflow created a disposable notebook with a selected **A Project Hub** note and 12 real backlinks. It captured both 1280×800 and 800×520 layouts. The compact screenshot shows the populated Details section constrained above the editor.

- Wide PNG SHA-256: `19255CE8A37CB0C1BEBC010BA27F4A12E27B598908DFB683632FA2D4B2E55CF1`
- Compact PNG SHA-256: `747E60ADA6E503CEA1D859624E1812C9D0F6B1656CFCA2B2D166D366C20556F1`

These captures establish rendered layout and visible content. Pointer, keyboard, focus, and screen-reader walkthroughs remain open before merge.
