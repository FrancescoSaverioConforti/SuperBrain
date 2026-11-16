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

    // A directory sitting where a record is expected is data corruption,
    // not an aliasing attack — it must surface as an ordinary I/O failure
    // (StorageError::Io), not PathEscapesVaultRoot, matching this crate's
    // pre-B5.SEC behavior for this exact case. Checked via symlink_metadata
    // (does not follow the final component) before the verified open, so a
    // real directory is distinguished from a symlink pointing at one — the
    // open below still rejects the symlink case via O_NOFOLLOW as before.
    if let Ok(meta) = std::fs::symlink_metadata(candidate) {
        if meta.is_dir() {
            return Err(StorageError::Io(std::io::Error::other(format!(
                "record path for note {note_id_label} is a directory, not a file"
            ))));
        }
    }

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

    // Same rationale as the Unix branch: a directory where a record is
    // expected is data corruption, not an aliasing attack, and must
    // surface as StorageError::Io — checked via symlink_metadata (does
    // not follow the final component) before the verified open.
    if let Ok(meta) = std::fs::symlink_metadata(candidate) {
        if meta.is_dir() {
            return Err(StorageError::Io(std::io::Error::other(format!(
                "record path for note {note_id_label} is a directory, not a file"
            ))));
        }
    }

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

/// Reads the full contents of `root.join(candidate_relative)` through a
/// verified handle (B5.SEC): one call replaces the `open_verified(...)` +
/// separate `read_to_end` pair every call site would otherwise repeat.
pub(crate) fn read_verified(root: &Path, candidate_relative: &Path, note_id_label: &str) -> Result<Vec<u8>, StorageError> {
    use std::io::Read;
    let mut handle = open_verified(root, candidate_relative, OpenMode::ReadExisting, note_id_label)?;
    let mut bytes = Vec::new();
    handle.file.read_to_end(&mut bytes).map_err(StorageError::Io)?;
    Ok(bytes)
}

/// A verified handle to the vault root's own directory (or any directory
/// confirmed to resolve inside it) — distinct from `VerifiedRecordHandle`,
/// which verifies a FILE. Anchors `write_verified_atomic`'s temp-file
/// creation and rename so both happen relative to a directory identity
/// already proven safe, never a freshly-resolved path string.
pub(crate) struct VerifiedDirHandle {
    // Never read directly (Approach A re-verifies via a fresh
    // open_verified_dir call rather than reusing this handle — see
    // write_verified_atomic's doc comment) but kept open deliberately:
    // holding it for VerifiedDirHandle's lifetime is a RAII anchor, not
    // dead weight, even though nothing currently reads its value.
    #[allow(dead_code)]
    dir: File,
    path: PathBuf,
}

/// Opens and verifies `root` itself as a directory: not a symlink/reparse
/// point, and its real path resolves to `root`'s own canonical form. This
/// is the anchor every atomic write in this crate is now rooted to, rather
/// than a bare `root.join(...)` string re-resolved at write time.
pub(crate) fn open_verified_dir(root: &Path) -> Result<VerifiedDirHandle, StorageError> {
    let canonical_root = std::fs::canonicalize(root).map_err(StorageError::Io)?;

    // `root` (the vault's own configured root path) is RESOLVED, not
    // rejected, if it goes through a symlink/reparse point — the trust
    // anchor is the CANONICAL location the root resolves to, which the
    // caller already computed as `canonical_root`. This differs from
    // `open_verified`'s file-open path, where a link at the final
    // component is always an attack signal: the vault root is configured
    // once, by the trusted local operator, and "reached via a symlinked
    // ancestor" is an ordinary, legitimate deployment shape (ADR-equivalent
    // precedent: `vault_root_reached_through_a_symlinked_ancestor_still_works`,
    // pre-existing in this module's test suite). What this function still
    // must refuse is a MISMATCH: whatever `root` resolves to right now,
    // via the OS's own symlink-following open, must be exactly
    // `canonical_root` (computed via `std::fs::canonicalize`, itself
    // symlink-resolving) — any divergence means the two resolutions
    // disagree, which is the actual anomaly worth rejecting.
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt};
        use std::os::unix::io::AsRawFd;
        let mut opts = std::fs::OpenOptions::new();
        opts.read(true).custom_flags(libc::O_DIRECTORY);
        let dir = opts.open(root).map_err(|_| escapes("<root>"))?;
        let metadata = dir.metadata().map_err(|_| escapes("<root>"))?;
        if !metadata.is_dir() {
            return Err(escapes("<root>"));
        }
        let fd = dir.as_raw_fd();
        let real_path = std::fs::read_link(format!("/proc/self/fd/{fd}")).map_err(|_| escapes("<root>"))?;
        if real_path != canonical_root {
            return Err(escapes("<root>"));
        }
        Ok(VerifiedDirHandle { dir, path: canonical_root })
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::Foundation::{HANDLE, MAX_PATH};
        use windows::Win32::Storage::FileSystem::{
            GetFileInformationByHandle, GetFinalPathNameByHandleW, BY_HANDLE_FILE_INFORMATION,
            FILE_ATTRIBUTE_DIRECTORY, FILE_FLAG_BACKUP_SEMANTICS, GETFINALPATHNAMEBYHANDLE_FLAGS,
        };
        // No FILE_FLAG_OPEN_REPARSE_POINT here: `root` resolving through a
        // reparse point is legitimate (see this function's doc comment
        // above) — CreateFileW follows it normally, landing on the real
        // target directory, which is exactly what this function needs to
        // then verify against `canonical_root`.
        let mut opts = std::fs::OpenOptions::new();
        opts.read(true).custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0);
        let dir = opts.open(root).map_err(|_| escapes("<root>"))?;
        let raw_handle = HANDLE(dir.as_raw_handle());
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe {
            GetFileInformationByHandle(raw_handle, &mut info).map_err(|_| escapes("<root>"))?;
        }
        if (info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0) == 0 {
            return Err(escapes("<root>"));
        }
        let mut buf = vec![0u16; MAX_PATH as usize];
        let len = unsafe { GetFinalPathNameByHandleW(raw_handle, &mut buf, GETFINALPATHNAMEBYHANDLE_FLAGS(0)) };
        if len == 0 || (len as usize) > buf.len() {
            return Err(escapes("<root>"));
        }
        let real_path = PathBuf::from(String::from_utf16_lossy(&buf[..len as usize]));
        if real_path != canonical_root {
            return Err(escapes("<root>"));
        }
        Ok(VerifiedDirHandle { dir, path: canonical_root })
    }
}

/// B5.SEC Approach A (user decision, 2026-11-16, after a directory-
/// handle-relative rename was exhaustively tried — Rust `windows` crate
/// bindings AND an independent C reference against real Windows SDK
/// headers, both failing identically — and found non-functional on this
/// Windows build; see `d248e4`/`d249ef` in continuity for the full
/// evidence trail). Writes `contents` to
/// `verified_dir.path().join(file_name)` via a sibling-temp-file-then-
/// rename sequence, with the directory's identity re-verified
/// IMMEDIATELY before the rename and the final file's identity verified
/// immediately AFTER it. This is a deliberate, documented trade-off:
///
/// - **What this closes:** the TOCTOU gap SEC-001/002/003 originally
///   described — a symlink/hard-link/reparse-point planted at the
///   record's path BEFORE this function's verification runs is rejected
///   (verification happens on an open handle, never a bare path string),
///   and if the parent directory itself is swapped for something else
///   between this call's start and its rename, the immediate pre-rename
///   re-check catches it.
/// - **What this does NOT close:** a residual window between the
///   pre-rename re-verification call returning and the `std::fs::rename`
///   syscall immediately after it — on the order of a single thread
///   scheduling slice, not the duration of this function. Closing that
///   window completely would require a true directory-handle-relative
///   rename syscall; Windows' public, documented API for this
///   (`SetFileInformationByHandle` with `FileRenameInfoEx` and a
///   `RootDirectory` handle, using the `FILE_RENAME_FLAG_REPLACE_IF_EXISTS`/
///   `FILE_RENAME_FLAG_POSIX_SEMANTICS` flags per MS-FSCC) was tested
///   against this exact call shape in BOTH a Rust implementation and an
///   independent C reference compiled against real Windows SDK headers
///   (MinGW-w64 gcc 13.2.0, not windows-rs — eliminating any Rust-binding
///   explanation) and fails with `ERROR_INVALID_PARAMETER` in every
///   documented flag combination on this Windows build (10.0.26200). This
///   is accepted as a residual risk against the explicit B5 threat model:
///   an unprivileged local process capable of planting filesystem
///   entries, NOT a process capable of winning a single-syscall-wide race
///   window, and not Administrator/SYSTEM (which no approach here
///   defends against, since they can bypass any user-mode check).
///   SEC-001/002/003 are described as MITIGATED against this threat
///   model, never as mathematically race-free.
pub(crate) fn write_verified_atomic(verified_dir: &VerifiedDirHandle, file_name: &str, contents: &[u8]) -> Result<(), StorageError> {
    use std::io::Write;
    let dir_path = &verified_dir.path;
    let tmp_name = format!(
        "{file_name}.tmp-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos()
    );
    let tmp_path = dir_path.join(&tmp_name);
    {
        let mut f = File::create(&tmp_path).map_err(StorageError::Io)?;
        f.write_all(contents).map_err(StorageError::Io)?;
        f.sync_all().map_err(StorageError::Io)?;
        // dropped here: the temp file's own identity is not what's at
        // risk (it has a unique, unguessable name this process just
        // created) — what matters is the PARENT directory's identity at
        // rename time, re-checked next.
    }

    let final_path = dir_path.join(file_name);
    if let Err(e) = rename_with_immediate_reverification(verified_dir, &tmp_path, &final_path, file_name) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e);
    }

    #[cfg(unix)]
    {
        if let Ok(dir_file) = File::open(dir_path) {
            let _ = dir_file.sync_all();
        }
    }

    Ok(())
}

/// Re-verifies `verified_dir`'s identity (not a cached value — a fresh
/// `open_verified_dir` call) immediately before calling
/// `std::fs::rename`, then verifies the renamed-into-place file's own
/// identity immediately after (catching the case where the FINAL path
/// itself was something untrustworthy that the rename silently replaced
/// in a way this crate did not intend — e.g. the final name having become
/// a directory, or the rename having landed somewhere unexpected due to
/// a symlinked ancestor path component above `verified_dir` itself,
/// which `open_verified_dir`'s own canonicalize-based check already rules
/// out at the START of the call but is re-confirmed here at the END).
/// Fails closed (removing the temp file, per this function's caller) on
/// any mismatch — never silently proceeds with a weaker guarantee than
/// what was checked.
fn rename_with_immediate_reverification(verified_dir: &VerifiedDirHandle, tmp_path: &Path, final_path: &Path, file_name: &str) -> Result<(), StorageError> {
    let revalidated = open_verified_dir(&verified_dir.path)?;
    if revalidated.path != verified_dir.path {
        return Err(escapes("<root>"));
    }

    std::fs::rename(tmp_path, final_path).map_err(StorageError::Io)?;

    // Post-rename identity check: open the file we just renamed into
    // place through the SAME verified-handle path every reader will use,
    // confirming it is still a plain, single-link, in-root regular file
    // right now — not just "was, at the moment of the pre-rename check".
    let _ = open_verified(&verified_dir.path, Path::new(file_name), OpenMode::ReadExisting, "<post-rename-verify>")?;
    Ok(())
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
    fn open_verified_dir_accepts_a_plain_directory() {
        let dir = unique_test_dir();
        let result = open_verified_dir(&dir);
        assert!(result.is_ok(), "expected Ok, got {:?}", result.err());
    }

    #[test]
    fn write_verified_atomic_creates_a_file_with_exact_contents() {
        let dir = unique_test_dir();
        let verified_dir = open_verified_dir(&dir).unwrap();
        write_verified_atomic(&verified_dir, "note.record", b"hello world").unwrap();
        assert_eq!(std::fs::read(dir.join("note.record")).unwrap(), b"hello world");
    }

    #[test]
    fn write_verified_atomic_overwrites_existing_file_completely() {
        let dir = unique_test_dir();
        let verified_dir = open_verified_dir(&dir).unwrap();
        write_verified_atomic(&verified_dir, "note.record", b"first version, much longer content here").unwrap();
        write_verified_atomic(&verified_dir, "note.record", b"v2").unwrap();
        assert_eq!(std::fs::read(dir.join("note.record")).unwrap(), b"v2");
    }

    #[test]
    fn write_verified_atomic_leaves_no_temp_file_behind_on_success() {
        let dir = unique_test_dir();
        let verified_dir = open_verified_dir(&dir).unwrap();
        write_verified_atomic(&verified_dir, "note.record", b"content").unwrap();
        let leftover: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(leftover.is_empty());
    }

    #[test]
    fn write_verified_atomic_rejects_a_reparse_point_parent_directory() {
        // B5.SEC Approach A: the directory itself is swapped for a
        // reparse point between open_verified_dir's own check and this
        // call — simulated by opening a dir handle, then replacing the
        // directory on disk with a symlink before writing. Approach A's
        // immediate pre-rename re-verification must catch this and fail
        // closed (PathEscapesVaultRoot) — it must NEVER silently succeed
        // by writing through the stale handle into a path that, by the
        // time of the rename, no longer resolves to what was verified,
        // and it must NEVER let the swapped-in directory receive the
        // file either. On Windows this requires the ability to create
        // directory symlinks; skip (not fail) if the environment cannot.
        let parent = unique_test_dir();
        let real_dir = parent.join("real");
        std::fs::create_dir(&real_dir).unwrap();
        let verified_dir = open_verified_dir(&real_dir).unwrap();
        std::fs::remove_dir(&real_dir).unwrap();
        let elsewhere = unique_test_dir();
        #[cfg(windows)]
        let link_result = std::os::windows::fs::symlink_dir(&elsewhere, &real_dir);
        #[cfg(unix)]
        let link_result = std::os::unix::fs::symlink(&elsewhere, &real_dir);
        match link_result {
            Ok(()) => {
                let result = write_verified_atomic(&verified_dir, "note.record", b"content");
                assert!(matches!(result, Err(StorageError::PathEscapesVaultRoot { .. })), "got {result:?}");
                assert!(
                    !elsewhere.join("note.record").exists(),
                    "the swapped-in directory must never receive the file either"
                );
            }
            Err(e) => {
                eprintln!("skipping reparse-point-parent test: could not create directory symlink in this environment: {e}");
            }
        }
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
