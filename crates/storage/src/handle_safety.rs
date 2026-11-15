//! Handle-based filesystem safety primitive for SEC-001/002/003 (TOCTOU,
//! symlink/reparse-point aliasing, hard-link aliasing).
//!
//! `record_path_checked`/`before_image_path_checked` (`vault.rs`) canonicalize
//! a candidate path, then the caller performs I/O against that path string
//! separately — a classic check-then-use race. This module closes it by
//! opening the candidate without following a reparse point/symlink at the
//! final component, then verifying the identity of the handle that open
//! actually produced (regular file, single hard link, resolves inside the
//! vault root) — never re-deriving trust from a path string after the fact.
//!
//! Pattern note: this module's handle-verification approach (open, then call
//! GetFileInformationByHandle/MetadataExt on the open handle, reject on any
//! anomaly) follows the same shape as
//! `vault_security::deletion_log::identity_of_open_file`, but is a separate,
//! new function — not a modification of that one, which is crate-private to
//! `vault-security` and solves a narrower problem (device+inode equality
//! between two already-open handles). This module additionally checks link
//! count and rejects reparse points/symlinks, which `identity_of_open_file`
//! does not.

use std::fs::File;
use std::path::{Path, PathBuf};

use crate::vault::StorageError;

/// Whether `open_verified` is opening a record that must already exist, or
/// may be created if absent (the vault's "first write" case).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OpenMode {
    ReadExisting,
    CreateOrOpenForWrite,
}

/// A filesystem handle whose identity has already been verified: a regular
/// file, with exactly one hard link, whose real path resolves inside the
/// vault root — checked on the open handle itself, never re-derived from a
/// path string after the fact. Holding this value is the only way code in
/// this crate may read or write a record's bytes once B5.SEC's rewires land.
#[derive(Debug)]
pub(crate) struct VerifiedRecordHandle {
    pub(crate) file: File,
    #[allow(dead_code)] // read by callers once Tasks 4-6 rewire their call sites
    pub(crate) path: PathBuf,
}

fn escapes(note_id_label: &str) -> StorageError {
    StorageError::PathEscapesVaultRoot { note_id: note_id_label.to_string() }
}

fn not_found(note_id_label: &str) -> StorageError {
    StorageError::NotFound { note_id: note_id_label.to_string() }
}

/// Maps the initial `open()`'s own `io::Error` to `NotFound` when nothing
/// exists at the candidate path at all (the ordinary "no note here yet"
/// case every read must still be able to report), and to `escapes` for
/// every other failure — including `O_NOFOLLOW`/`FILE_FLAG_OPEN_REPARSE_POINT`
/// rejecting a symlink/reparse point at the final component, which is a
/// real entry existing there, just not a trustworthy one. Only this first
/// open call can produce `NotFound`; every check after a successful open
/// (link count, reparse-point attribute, root confinement) failing means
/// something untrustworthy DOES exist, so it is always `escapes`, never
/// `NotFound`.
fn open_or_not_found(opts: &std::fs::OpenOptions, candidate: &Path, note_id_label: &str) -> Result<File, StorageError> {
    opts.open(candidate).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            not_found(note_id_label)
        } else {
            escapes(note_id_label)
        }
    })
}

/// Opens `root.join(candidate_relative)` without following a reparse
/// point/symlink at the final component, then verifies the open handle:
/// regular file, single hard link, resolves inside `root`. Any failure at
/// any step returns [`StorageError::PathEscapesVaultRoot`] — a symlink, a
/// multi-linked file, and an out-of-root resolution all mean the same thing
/// to the caller: this path is not trustworthy.
///
/// `note_id_label` is used only to label the error (mirrors
/// `record_path_checked`'s existing error shape); it is not re-validated
/// here.
pub(crate) fn open_verified(
    root: &Path,
    candidate_relative: &Path,
    mode: OpenMode,
    note_id_label: &str,
) -> Result<VerifiedRecordHandle, StorageError> {
    let canonical_root = std::fs::canonicalize(root).map_err(StorageError::Io)?;
    let candidate = root.join(candidate_relative);

    #[cfg(unix)]
    {
        open_verified_unix(&canonical_root, &candidate, mode, note_id_label)
    }
    #[cfg(windows)]
    {
        open_verified_windows(&canonical_root, &candidate, mode, note_id_label)
    }
}

#[cfg(unix)]
fn open_verified_unix(
    canonical_root: &Path,
    candidate: &Path,
    mode: OpenMode,
    note_id_label: &str,
) -> Result<VerifiedRecordHandle, StorageError> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    use std::os::unix::io::AsRawFd;

    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    match mode {
        OpenMode::ReadExisting => {}
        OpenMode::CreateOrOpenForWrite => {
            opts.write(true).create(true);
        }
    }
    opts.custom_flags(libc::O_NOFOLLOW);

    let file = open_or_not_found(&opts, candidate, note_id_label)?;

    let metadata = file.metadata().map_err(|_| escapes(note_id_label))?;
    if !metadata.is_file() {
        return Err(escapes(note_id_label));
    }
    if metadata.nlink() != 1 {
        return Err(escapes(note_id_label));
    }

    let fd = file.as_raw_fd();
    let real_path = std::fs::read_link(format!("/proc/self/fd/{fd}")).map_err(|_| escapes(note_id_label))?;
    if !real_path.starts_with(canonical_root) {
        return Err(escapes(note_id_label));
    }

    Ok(VerifiedRecordHandle { file, path: candidate.to_path_buf() })
}

#[cfg(windows)]
fn open_verified_windows(
    canonical_root: &Path,
    candidate: &Path,
    mode: OpenMode,
    note_id_label: &str,
) -> Result<VerifiedRecordHandle, StorageError> {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::{HANDLE, MAX_PATH};
    use windows::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, GetFinalPathNameByHandleW, BY_HANDLE_FILE_INFORMATION,
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT, GETFINALPATHNAMEBYHANDLE_FLAGS,
    };

    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    match mode {
        OpenMode::ReadExisting => {}
        OpenMode::CreateOrOpenForWrite => {
            opts.write(true).create(true);
        }
    }
    opts.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0);

    let file = open_or_not_found(&opts, candidate, note_id_label)?;

    let raw_handle = HANDLE(file.as_raw_handle());
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe {
        GetFileInformationByHandle(raw_handle, &mut info).map_err(|_| escapes(note_id_label))?;
    }

    if (info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0) != 0 {
        // The open succeeded only because we asked to open the reparse
        // point itself (FILE_FLAG_OPEN_REPARSE_POINT); its presence means
        // this is a symlink/junction, which must be rejected outright,
        // never followed or treated as equivalent to a regular file.
        return Err(escapes(note_id_label));
    }
    if info.nNumberOfLinks != 1 {
        return Err(escapes(note_id_label));
    }

    let mut buf = vec![0u16; MAX_PATH as usize];
    let len = unsafe { GetFinalPathNameByHandleW(raw_handle, &mut buf, GETFINALPATHNAMEBYHANDLE_FLAGS(0)) };
    if len == 0 || (len as usize) > buf.len() {
        return Err(escapes(note_id_label));
    }
    let real_path_string = String::from_utf16_lossy(&buf[..len as usize]);
    let real_path = PathBuf::from(real_path_string);

    // GetFinalPathNameByHandleW returns an extended-length path
    // (\\?\C:\...); canonicalize's own output uses the same prefix on this
    // platform, so starts_with compares like-for-like.
    if !real_path.starts_with(canonical_root) {
        return Err(escapes(note_id_label));
    }

    Ok(VerifiedRecordHandle { file, path: candidate.to_path_buf() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_test_dir() -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "superbrain-handle-safety-test-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn accepts_a_plain_single_link_file_inside_root() {
        let dir = unique_test_dir();
        std::fs::write(dir.join("real.record"), b"content").unwrap();
        let result = open_verified(&dir, Path::new("real.record"), OpenMode::ReadExisting, "note-a");
        assert!(result.is_ok(), "expected Ok, got {:?}", result.err());
    }

    #[test]
    fn rejects_a_file_with_more_than_one_hard_link() {
        let dir = unique_test_dir();
        let target = dir.join("real.record");
        std::fs::write(&target, b"content").unwrap();
        let alias = dir.join("alias.record");
        std::fs::hard_link(&target, &alias).unwrap();
        let result = open_verified(&dir, Path::new("alias.record"), OpenMode::ReadExisting, "note-a");
        assert!(matches!(result, Err(StorageError::PathEscapesVaultRoot { .. })), "got {:?}", result);
    }

    #[cfg(windows)]
    #[test]
    fn rejects_a_symlink_at_the_final_component_even_when_its_target_is_inside_root() {
        let dir = unique_test_dir();
        let target = dir.join("real.record");
        std::fs::write(&target, b"content").unwrap();
        let alias = dir.join("alias.record");
        // Symlink creation on Windows normally requires elevation/Developer
        // Mode; skip (not fail) the test if this environment cannot create
        // one, since that's an environment limitation, not a code question.
        match std::os::windows::fs::symlink_file(&target, &alias) {
            Ok(()) => {
                let result = open_verified(&dir, Path::new("alias.record"), OpenMode::ReadExisting, "note-a");
                assert!(matches!(result, Err(StorageError::PathEscapesVaultRoot { .. })), "got {:?}", result);
            }
            Err(e) => {
                eprintln!("skipping symlink test: could not create symlink in this environment: {e}");
            }
        }
    }

    #[test]
    fn create_or_open_for_write_creates_a_new_file_when_nothing_exists_yet() {
        let dir = unique_test_dir();
        let result = open_verified(&dir, Path::new("new.record"), OpenMode::CreateOrOpenForWrite, "note-a");
        assert!(result.is_ok(), "expected Ok, got {:?}", result.err());
        assert!(dir.join("new.record").exists());
    }

    #[test]
    fn read_existing_a_nonexistent_file_returns_not_found_not_an_escape() {
        // A missing file is the ordinary "no note here yet" case, not an
        // attack signal — must be distinguishable from a real identity
        // verification failure (StorageError::PathEscapesVaultRoot).
        let dir = unique_test_dir();
        let result = open_verified(&dir, Path::new("missing.record"), OpenMode::ReadExisting, "note-a");
        assert!(matches!(result, Err(StorageError::NotFound { .. })), "got {result:?}");
    }
}
