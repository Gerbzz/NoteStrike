//! Loading and saving plain-text files safely.
//!
//! * Loads are size-checked, encoding-detected, and line-ending-normalised.
//! * Saves are atomic: the bytes go to a temporary file in the same folder,
//!   are flushed with `fsync`, and only then renamed over the target. An
//!   interrupted save leaves the original file untouched.
//!   Platform notes: `rename` replaces an existing file on every OS (on
//!   Windows std uses `MoveFileExW(MOVEFILE_REPLACE_EXISTING)`), but Windows
//!   can briefly refuse while a scanner holds the file, so that case is
//!   retried. On Unix the folder is `fsync`ed after the rename so the new
//!   name survives a power cut, and the original file's permission bits are
//!   copied to the replacement (an `0600` file stays private).
//! * A file is read-only when its read-only attribute / write bits say so
//!   or, on Unix, when this user cannot open it for writing (for example a
//!   file owned by root in a folder the user can write to: replacing it by
//!   rename would otherwise succeed).
//! * Every loaded or saved file keeps a [`DiskSnapshot`] (length, modified
//!   time, SHA-256) so a save refuses to overwrite a file that another program
//!   changed, unless the user explicitly chooses to overwrite it.

use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::encoding::{self, TextEncoding, Unrepresentable};
use crate::eol::{self, EolCounts, LineEnding};

/// Files above this size need an explicit "open anyway".
pub const SOFT_SIZE_LIMIT: u64 = 8 * 1024 * 1024;
/// Files above this size are refused: the editor keeps the whole text in
/// memory and lays it out every frame, so it is not a large-file viewer.
pub const HARD_SIZE_LIMIT: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiskSnapshot {
    pub len: u64,
    pub modified: Option<SystemTime>,
    pub hash: [u8; 32],
}

impl DiskSnapshot {
    pub fn of_bytes(bytes: &[u8], modified: Option<SystemTime>) -> Self {
        Self {
            len: bytes.len() as u64,
            modified,
            hash: Sha256::digest(bytes).into(),
        }
    }

    pub fn hash_hex(&self) -> String {
        self.hash.iter().map(|b| format!("{b:02x}")).collect()
    }

    pub fn hash_from_hex(hex: &str) -> Option<[u8; 32]> {
        if hex.len() != 64 {
            return None;
        }
        let mut out = [0u8; 32];
        for (index, byte) in out.iter_mut().enumerate() {
            *byte = u8::from_str_radix(hex.get(index * 2..index * 2 + 2)?, 16).ok()?;
        }
        Some(out)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct LoadOptions {
    /// Allow files between [`SOFT_SIZE_LIMIT`] and [`HARD_SIZE_LIMIT`].
    pub allow_large: bool,
    /// Decode with this encoding instead of detecting one.
    pub forced_encoding: Option<TextEncoding>,
}

#[derive(Clone, Debug)]
pub struct LoadedFile {
    pub path: PathBuf,
    /// Text with `\n` line endings.
    pub text: String,
    pub encoding: TextEncoding,
    pub utf16_bom: bool,
    pub eol: LineEnding,
    pub eol_counts: EolCounts,
    pub invalid_sequences: usize,
    pub first_invalid_offset: Option<usize>,
    pub contains_nul: bool,
    pub read_only: bool,
    pub snapshot: DiskSnapshot,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadError {
    TooLarge { size: u64, hard_limit: bool },
    NotAFile,
    NotFound,
    Io(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::TooLarge {
                size,
                hard_limit: true,
            } => write!(
                f,
                "the file is {} - larger than NoteStrike's {} limit for plain-text editing",
                human_size(*size),
                human_size(HARD_SIZE_LIMIT)
            ),
            LoadError::TooLarge { size, .. } => write!(
                f,
                "the file is {}; large files can make editing slow",
                human_size(*size)
            ),
            LoadError::NotAFile => write!(f, "the path is not a regular file"),
            LoadError::NotFound => write!(f, "the file does not exist"),
            LoadError::Io(message) => write!(f, "{message}"),
        }
    }
}

pub fn human_size(bytes: u64) -> String {
    const MIB: f64 = 1024.0 * 1024.0;
    if bytes >= 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / MIB)
    } else if bytes >= 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} bytes")
    }
}

pub fn load(path: &Path, options: LoadOptions) -> Result<LoadedFile, LoadError> {
    let metadata = fs::metadata(path).map_err(|error| match error.kind() {
        io::ErrorKind::NotFound => LoadError::NotFound,
        _ => LoadError::Io(error.to_string()),
    })?;
    if !metadata.is_file() {
        return Err(LoadError::NotAFile);
    }
    check_size(metadata.len(), options.allow_large)?;
    let mut file = fs::File::open(path).map_err(|e| LoadError::Io(e.to_string()))?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    // Read at most one byte past the hard limit in case the file grew.
    Read::by_ref(&mut file)
        .take(HARD_SIZE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| LoadError::Io(e.to_string()))?;
    check_size(bytes.len() as u64, options.allow_large)?;
    let modified = file.metadata().ok().and_then(|m| m.modified().ok());
    let decoded = match options.forced_encoding {
        Some(enc) => encoding::decode_as(&bytes, enc),
        None => encoding::detect_and_decode(&bytes),
    };
    let eol_counts = eol::count(&decoded.text);
    Ok(LoadedFile {
        path: path.to_path_buf(),
        text: eol::normalize(&decoded.text),
        encoding: decoded.encoding,
        utf16_bom: decoded.had_bom,
        eol: eol_counts
            .dominant()
            .unwrap_or_else(LineEnding::platform_default),
        eol_counts,
        invalid_sequences: decoded.invalid_sequences,
        first_invalid_offset: decoded.first_invalid_offset,
        contains_nul: decoded.contains_nul,
        read_only: is_read_only(path, &metadata),
        snapshot: DiskSnapshot::of_bytes(&bytes, modified),
    })
}

fn check_size(size: u64, allow_large: bool) -> Result<(), LoadError> {
    if size > HARD_SIZE_LIMIT {
        Err(LoadError::TooLarge {
            size,
            hard_limit: true,
        })
    } else if size > SOFT_SIZE_LIMIT && !allow_large {
        Err(LoadError::TooLarge {
            size,
            hard_limit: false,
        })
    } else {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiskState {
    Unchanged,
    Changed,
    Missing,
}

/// Compare the file on disk with a snapshot. Length and modified time are
/// checked first; the file is only re-hashed when those differ, so touching a
/// file without changing its bytes is not reported as a change. This cheap
/// check is used for background polling; saves use [`disk_state_exact`].
pub fn disk_state(
    path: &Path,
    snapshot: &DiskSnapshot,
) -> io::Result<(DiskState, Option<DiskSnapshot>)> {
    compare_disk(path, snapshot, false)
}

/// Like [`disk_state`] but always re-hashes the file, so a same-size edit
/// within the file system's timestamp resolution is still detected.
pub fn disk_state_exact(
    path: &Path,
    snapshot: &DiskSnapshot,
) -> io::Result<(DiskState, Option<DiskSnapshot>)> {
    compare_disk(path, snapshot, true)
}

fn compare_disk(
    path: &Path,
    snapshot: &DiskSnapshot,
    exact: bool,
) -> io::Result<(DiskState, Option<DiskSnapshot>)> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok((DiskState::Missing, None));
        }
        Err(error) => return Err(error),
    };
    let modified = metadata.modified().ok();
    if !exact
        && metadata.len() == snapshot.len
        && modified.is_some()
        && modified == snapshot.modified
    {
        return Ok((DiskState::Unchanged, None));
    }
    if metadata.len() > HARD_SIZE_LIMIT {
        return Ok((DiskState::Changed, None));
    }
    let bytes = fs::read(path)?;
    let current = DiskSnapshot::of_bytes(&bytes, modified);
    if current.hash == snapshot.hash {
        Ok((DiskState::Unchanged, Some(current)))
    } else {
        Ok((DiskState::Changed, Some(current)))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveError {
    /// The file changed on disk since it was loaded or last saved.
    ChangedOnDisk,
    /// The file is marked read-only.
    ReadOnly,
    /// The OS refused access (permissions, locked folder, ...).
    PermissionDenied(String),
    Unrepresentable(Unrepresentable),
    Io(String),
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveError::ChangedOnDisk => write!(f, "the file was changed by another program"),
            SaveError::ReadOnly => write!(f, "the file is read-only"),
            SaveError::PermissionDenied(message) => write!(f, "access denied: {message}"),
            SaveError::Unrepresentable(details) => write!(f, "{details}"),
            SaveError::Io(message) => write!(f, "{message}"),
        }
    }
}

/// What to write and what we believe is on disk.
pub struct SaveRequest<'a> {
    pub path: &'a Path,
    /// Editor text; line endings are converted to `eol`.
    pub text: &'a str,
    pub encoding: TextEncoding,
    pub utf16_bom: bool,
    pub eol: LineEnding,
    /// Snapshot of the file as last loaded/saved. `None` for a new file or a
    /// Save As target (the OS dialog already confirmed the overwrite).
    pub expected: Option<&'a DiskSnapshot>,
    /// Overwrite even if the file changed on disk (explicit user choice).
    pub overwrite_external_changes: bool,
}

/// Test hook to simulate a save that is interrupted after the temporary file
/// is written but before it replaces the target (power loss, crash, kill).
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default)]
pub struct FaultPlan {
    pub fail_before_rename: bool,
    /// Leave the temporary file behind, as a killed process would.
    pub leave_temp_file: bool,
}

pub fn save(request: &SaveRequest<'_>) -> Result<DiskSnapshot, SaveError> {
    save_with_faults(request, FaultPlan::default())
}

#[doc(hidden)]
pub fn save_with_faults(
    request: &SaveRequest<'_>,
    faults: FaultPlan,
) -> Result<DiskSnapshot, SaveError> {
    let text = eol::apply(request.text, request.eol);
    let bytes = encoding::encode(&text, request.encoding, request.utf16_bom)
        .map_err(SaveError::Unrepresentable)?;
    let target = resolve_symlink(request.path);
    match fs::metadata(&target) {
        Ok(metadata) => {
            if !metadata.is_file() {
                return Err(SaveError::Io("the target is not a regular file".into()));
            }
            if is_read_only(&target, &metadata) {
                return Err(SaveError::ReadOnly);
            }
            if let Some(expected) = request.expected
                && !request.overwrite_external_changes
            {
                let (state, _) = disk_state_exact(&target, expected).map_err(io_error)?;
                if state != DiskState::Unchanged {
                    return Err(SaveError::ChangedOnDisk);
                }
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            // Saving a file that was deleted externally recreates it, but only
            // when the user asked to overwrite external changes.
            if request.expected.is_some() && !request.overwrite_external_changes {
                return Err(SaveError::ChangedOnDisk);
            }
        }
        Err(error) => return Err(io_error(error)),
    }
    write_atomic_impl(
        &target,
        &bytes,
        faults,
        || {
            if let Some(expected) = request.expected
                && !request.overwrite_external_changes
            {
                let (state, _) = disk_state_exact(&target, expected).map_err(io_error)?;
                if state != DiskState::Unchanged {
                    return Err(SaveError::ChangedOnDisk);
                }
            }
            Ok(())
        },
        io_error,
    )?;
    let modified = fs::metadata(&target).ok().and_then(|m| m.modified().ok());
    Ok(DiskSnapshot::of_bytes(&bytes, modified))
}

fn io_error(error: io::Error) -> SaveError {
    if error.kind() == io::ErrorKind::PermissionDenied {
        SaveError::PermissionDenied(error.to_string())
    } else {
        SaveError::Io(error.to_string())
    }
}

/// Save through a symlink to the file it points at, keeping the link.
fn resolve_symlink(path: &Path) -> PathBuf {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
        }
        _ => path.to_path_buf(),
    }
}

/// Name prefix of NoteStrike temporary files, so they are easy to recognise.
pub const TEMP_MARKER: &str = ".notestrike-save-";

/// Write `bytes` to `path` atomically: temp file in the same directory,
/// `fsync`, then rename over the target (and `fsync` the directory on Unix).
#[doc(hidden)]
pub fn write_atomic(path: &Path, bytes: &[u8], faults: FaultPlan) -> io::Result<()> {
    write_atomic_impl(path, bytes, faults, || Ok(()), |error| error)
}

fn write_atomic_impl<E>(
    path: &Path,
    bytes: &[u8],
    faults: FaultPlan,
    mut check_target: impl FnMut() -> Result<(), E>,
    map_io: impl Fn(io::Error) -> E,
) -> Result<(), E> {
    let dir = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());
    let temp = dir.join(format!(
        "{TEMP_MARKER}{}-{name}.tmp",
        Uuid::new_v4().simple()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(&map_io)?;
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    if let Err(error) = written {
        let _ = fs::remove_file(&temp);
        return Err(map_io(error));
    }
    if faults.fail_before_rename {
        if !faults.leave_temp_file {
            let _ = fs::remove_file(&temp);
        }
        return Err(map_io(io::Error::other(
            "simulated interruption before rename",
        )));
    }
    copy_permissions(path, &temp);
    if let Err(error) = rename_with_retry(&temp, path, &mut check_target, &map_io) {
        let _ = fs::remove_file(&temp);
        return Err(error);
    }
    sync_dir(&dir);
    Ok(())
}

/// Antivirus scanners and search indexers briefly lock files on Windows. A
/// sharing violation is retried for up to two seconds; anything else fails.
/// (Retry policy reused from the previous NoteStrike notebook store.)
fn rename_with_retry<E>(
    from: &Path,
    to: &Path,
    check_target: &mut impl FnMut() -> Result<(), E>,
    map_io: &impl Fn(io::Error) -> E,
) -> Result<(), E> {
    let mut attempt = 0;
    loop {
        // A Windows sharing lock can release after another program has edited
        // the file. Recheck immediately before every replacement attempt.
        check_target()?;
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(error) if transient_windows_sharing_error(&error) && attempt < 40 => {
                attempt += 1;
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(error) => return Err(map_io(error)),
        }
    }
}

fn transient_windows_sharing_error(error: &io::Error) -> bool {
    // 32 = ERROR_SHARING_VIOLATION, 33 = ERROR_LOCK_VIOLATION. Access denied (5)
    // is not retried: it is how Windows reports read-only or ACL refusals.
    cfg!(windows) && matches!(error.raw_os_error(), Some(32 | 33))
}

/// Read-only attribute (Windows) / no write bits (Unix), or, on Unix, this
/// user may not write the file. Opening for writing without truncation does
/// not change the file.
fn is_read_only(path: &Path, metadata: &fs::Metadata) -> bool {
    if metadata.permissions().readonly() {
        return true;
    }
    #[cfg(unix)]
    if let Err(error) = OpenOptions::new().write(true).open(path)
        && error.kind() == io::ErrorKind::PermissionDenied
    {
        return true;
    }
    #[cfg(not(unix))]
    let _ = path;
    false
}

/// The replacement file is new, so on Unix it would get default permissions
/// (umask). Keep the original's mode instead. Windows has only the read-only
/// attribute here, which is never set on a file being saved.
#[cfg(unix)]
fn copy_permissions(original: &Path, replacement: &Path) {
    if let Ok(metadata) = fs::metadata(original) {
        let _ = fs::set_permissions(replacement, metadata.permissions());
    }
}

#[cfg(not(unix))]
fn copy_permissions(_original: &Path, _replacement: &Path) {}

#[cfg(unix)]
fn sync_dir(dir: &Path) {
    if let Ok(handle) = fs::File::open(dir) {
        let _ = handle.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_dir(_dir: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_edit_after_staging_prevents_replacement() {
        let dir = std::env::temp_dir().join(format!("notestrike-guard-{}", Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let path = dir.join("document.txt");
        fs::write(&path, b"original").unwrap();
        let expected = DiskSnapshot::of_bytes(b"original", None);

        let result = write_atomic_impl(
            &path,
            b"my edit",
            FaultPlan::default(),
            || {
                fs::write(&path, b"external edit").unwrap();
                let (state, _) = disk_state_exact(&path, &expected).map_err(io_error)?;
                if state != DiskState::Unchanged {
                    return Err(SaveError::ChangedOnDisk);
                }
                Ok(())
            },
            io_error,
        );
        assert_eq!(result, Err(SaveError::ChangedOnDisk));
        assert_eq!(fs::read(&path).unwrap(), b"external edit");
        assert_eq!(
            fs::read_dir(&dir).unwrap().count(),
            1,
            "staged file cleaned up"
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}
