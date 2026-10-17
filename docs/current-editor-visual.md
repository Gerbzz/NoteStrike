# Current native editor visual

The [wide capture](../assets/screenshots/current-editor-wide.png) and [compact capture](../assets/screenshots/current-editor-compact.png) are unaltered app-window PNGs from `notestrike-native` at source commit `fe0643a531ac4bc34848d1e13b43979ddabce452` in [hosted run 38991325340](https://github.com/Gerbzz/NoteStrike/actions/runs/38991325340). The Linux runner used Xvfb and Mesa software OpenGL after the hosted Windows runner lacked OpenGL 2.0. Windows formatting, ten tests, and native build passed in the same run. No native executable was downloaded or run on the Office PC.

The workflow created a disposable notebook with selected **A Project Hub**, 60 review lines, and 12 real backlinks. The 1280×800 image shows the start of the long note and the sorted backlink list. The 800×520 image keeps Details open and shows review items 58–60 plus **FINAL ACCESSIBLE LINE** at the end of the source editor. The headless regression checks that long source content exceeds the compact viewport without losing its final text.

- Wide PNG SHA-256: `E30E462BF9F8B2A2F74A56A645E6DF9D953B365E3DB96746219C650DF54E491B`
- Compact PNG SHA-256: `B990CCC02355072B1FA570B8F5A97FC67B81AAAFC61E7AAF51BD394F64E23901`

These captures establish rendered layout and visible content. Interactive pointer, keyboard, focus, and screen-reader walkthroughs remain open.
