#[path = "build_support/windows_icon.rs"]
mod windows_icon;

fn main() {
    windows_icon::embed("assets/branding/NoteStrike-geometric.ico", "notestrike");
}
