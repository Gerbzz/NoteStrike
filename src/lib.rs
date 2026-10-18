//! NoteStrike: a lightweight plain-text editor (Notepad replacement).
//!
//! The library holds everything that is not GUI code so it can be tested
//! without a window:
//!
//! * [`encoding`]: detect/decode/encode UTF-8 (± BOM), UTF-16 LE/BE, Windows-1252.
//! * [`eol`]: detect, normalise, and restore CRLF / LF / CR line endings.
//! * [`document`]: size-checked loads and atomic, conflict-checked saves.
//! * [`search`]: find / replace and cursor helpers.
//! * [`recovery`]: crash/session recovery journal.
//! * [`paths`]: per-OS settings and recovery locations, and their migration.
//! * [`workspace`]: tabs, dirty tracking, prompts, and save/close/quit flows.
//! * [`legacy`]: read-only access to NoteStrike 0.1 notebooks and import of
//!   their notes as plain files.

pub mod document;
pub mod encoding;
pub mod eol;
pub mod legacy;
pub mod paths;
pub mod recovery;
pub mod search;
pub mod workspace;

#[cfg(feature = "native-ui")]
pub mod native_ui;
