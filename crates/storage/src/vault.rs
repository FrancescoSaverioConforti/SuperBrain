use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use uuid::Uuid;

use crate::atomic::{atomic_write, AtomicWriteError};
use crate::note_id::NoteId;

/// The separator between the version header line and the raw note content
/// in the on-disk record format.
const RECORD_SEPARATOR: &str = "\n---\n";

/// Serializes `content` and `version` into the single buffer that is the
/// entire on-disk representation of a record: a decimal version line,
/// then [`RECORD_SEPARATOR`], then the raw content. Building this as one
/// `Vec<u8>` and handing it to `atomic_write` in one call is what makes
/// content and version be replaced together by a single rename — there is
/// no second file and no window where one can be read without the other.
fn serialize_record(content: &str, version: Version) -> Vec<u8> {
    let mut buffer = Vec::with_capacity(content.len() + 32);
    buffer.extend_from_slice(version.0.to_string().as_bytes());
    buffer.extend_from_slice(RECORD_SEPARATOR.as_bytes());
    buffer.extend_from_slice(content.as_bytes());
    buffer
}

/// Parses the single-buffer on-disk record format back into a
/// [`VaultRecord`]. `note_id` is used only to label errors.
fn parse_record(bytes: &[u8], note_id: &NoteId) -> Result<VaultRecord, StorageError> {
    let text = String::from_utf8(bytes.to_vec()).map_err(|_| StorageError::Io(
        std::io::Error::new(std::io::ErrorKind::InvalidData, "record is not valid utf-8"),
    ))?;

    let (version_part, content) = text.split_once(RECORD_SEPARATOR).ok_or_else(|| {
        StorageError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("malformed record for note {}: missing separator", note_id.as_str()),
        ))
    })?;

    let version_number: u64 = version_part.trim().parse().map_err(|_| {
        StorageError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("malformed record for note {}: invalid version header", note_id.as_str()),
        ))
    })?;

    Ok(VaultRecord {
        content: content.to_string(),
        version: Version(version_number),
    })
}

/// A monotonically increasing per-note version number, used for optimistic
/// concurrency: a write must present the version it believes is current, and
/// is rejected with an explicit [`StorageError::Conflict`] if that belief is
/// stale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Version(pub u64);

/// The single unit serialized to disk: content and version travel together
/// as one in-memory buffer and are replaced by exactly one `atomic_write`
/// call, so there is never a window where one can be read without the
/// other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultRecord {
    pub content: String,
    pub version: Version,
}

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("version conflict on note {note_id}: expected {expected:?}, actual {actual:?}")]
    Conflict {
        note_id: String,
        expected: Version,
        actual: Version,
    },
    #[error("note {note_id} not found")]
    NotFound { note_id: String },
    #[error("resolved path for note {note_id} escapes the vault root")]
    PathEscapesVaultRoot { note_id: String },
    /// Any filesystem I/O failure, including a failure inside the
    /// crate-private atomic-write step (temp-file create/write/fsync/rename).
    /// The crate-private `AtomicWriteError` is folded into this variant at
    /// the crate boundary so it never becomes public API.
    #[error("io error: {0}")]
    Io(std::io::Error),
}

/// The core storage type: a single-record-per-note vault rooted at `root`,
/// with explicit optimistic-concurrency conflicts on write.
pub struct Vault {
    root: PathBuf,
    /// Per-note-id locks, scoped to **this process and this `Vault`
    /// instance only**. They serialize concurrent callers that share this
    /// `Vault` object (e.g. multiple threads in the same running process)
    /// and make no claim whatsoever about two separate processes, two
    /// separate `Vault::new` calls on the same root, or a crash mid-write —
    /// those guarantees require the A2 journal and are explicitly not
    /// provided here.
    ///
    /// Keyed by the canonical [`NoteId`] itself (not a separately-derived
    /// string), so the lock key, the record path (`<note_id>.record`, built
    /// from `NoteId::as_str`) and the identity checked in
    /// `propose_and_commit` are one and the same value. Because `NoteId` is
    /// lowercase-only, two distinct keys can never name the same file on a
    /// case-insensitive filesystem (A1 final review, Finding C2).
    locks: Mutex<HashMap<NoteId, Arc<Mutex<()>>>>,
}

impl Vault {
    pub fn new(root: PathBuf) -> Self {
        Vault {
            root,
            locks: Mutex::new(HashMap::new()),
        }
    }

    pub fn current_version(&self, note_id: &NoteId) -> Result<Version, StorageError> {
        self.read_note(note_id).map(|record| record.version)
    }

    pub fn read_note(&self, note_id: &NoteId) -> Result<VaultRecord, StorageError> {
        let path = self.record_path_checked(note_id)?;
        let bytes = std::fs::read(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                StorageError::NotFound {
                    note_id: note_id.as_str().to_string(),
                }
            } else {
                StorageError::Io(e)
            }
        })?;
        parse_record(&bytes, note_id)
    }

    /// Writes `content` as the new record for `note_id`, provided
    /// `expected_version` matches the version currently on disk (or
    /// `Version(0)` if the note does not yet exist).
    ///
    /// `operation_id` is accepted and currently unused beyond being
    /// threaded through the signature: A1 does not implement cross-restart
    /// idempotency, so this parameter exists only so a later phase (A2) can
    /// wire it to journal-backed idempotency without changing this
    /// signature.
    ///
    /// Crate-private (`pub(crate)`, never `pub`): this is the single
    /// primitive through which every note write to the vault flows, and it
    /// performs **no policy validation**, so no code outside this crate may
    /// call it. The other raw primitive it builds on, `atomic::atomic_write`,
    /// is likewise `pub(crate)` and not re-exported (A1 final review, Finding
    /// C1). Both are pinned unreachable from other crates by the
    /// `compile_fail` doctests in `lib.rs`.
    ///
    /// `propose::propose_and_commit` (same crate) is this
    /// method's first and only production caller, reaching it only after
    /// `policy::Gate::evaluate` returns `GateDecision::Pass`. `pub(crate)`
    /// (not `pub`) is the minimum visibility that lets a sibling module in
    /// this same crate call it while keeping it completely unreachable from
    /// outside the crate — Rust's module-privacy model scopes a bare,
    /// non-`pub` item to its *defining module only*, not the whole crate, so
    /// `propose.rs` (a sibling of `vault.rs`, not a descendant) could not
    /// call a plain-private method here even though both live in `storage`.
    pub(crate) fn write_record_unchecked(
        &self,
        note_id: &NoteId,
        content: &str,
        expected_version: Version,
        operation_id: Uuid,
    ) -> Result<Version, StorageError> {
        let _ = operation_id;

        let record_path = self.record_path_checked(note_id)?;

        // Acquire (creating if absent) the per-note-id lock, then hold it
        // for the entire critical section below: reading the current
        // version, comparing it to `expected_version`, and writing the new
        // record are all done while this lock is held, so no other thread
        // sharing this `Vault` instance can observe or act on a stale
        // version between the check and the write. Releasing the lock
        // between the check and the write would reintroduce exactly the
        // race this method exists to close.
        let note_lock = {
            let mut locks = self.locks.lock().unwrap();
            Arc::clone(
                locks
                    .entry(note_id.clone())
                    .or_insert_with(|| Arc::new(Mutex::new(()))),
            )
        };
        let _guard = note_lock.lock().unwrap();

        let actual_version = match std::fs::read(&record_path) {
            Ok(bytes) => parse_record(&bytes, note_id)?.version,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Version(0),
            Err(e) => return Err(StorageError::Io(e)),
        };

        if actual_version != expected_version {
            return Err(StorageError::Conflict {
                note_id: note_id.as_str().to_string(),
                expected: expected_version,
                actual: actual_version,
            });
        }

        let new_version = Version(actual_version.0 + 1);
        let buffer = serialize_record(content, new_version);
        atomic_write(&record_path, &buffer).map_err(|AtomicWriteError::Io(e)| StorageError::Io(e))?;

        Ok(new_version)
    }

    /// Resolves the on-disk record path for `note_id` — `root/<id>.record`,
    /// the exact path that `read_note` and `write_record_unchecked` open —
    /// and verifies, via filesystem canonicalization, that this same path
    /// resolves inside the vault root. `NoteId::parse` only ever sees the id
    /// string, so it cannot detect a symlink planted inside the vault root
    /// that resolves elsewhere on disk; this check is what catches that. See
    /// [`ensure_resolves_inside_root`] for the exact rule.
    ///
    /// (A1 third review, Finding I-C: until then this function canonicalized
    /// the bare `root/<id>` path, without the `.record` suffix, which no I/O
    /// ever touches, and returned the `.record` path unchecked. A symlink at
    /// `root/<id>.record` was therefore followed by every read and by the
    /// version check.)
    ///
    /// KNOWN LIMITATION (accepted for A1, Continuity [c12ah]): this is still
    /// a check-then-use. The I-C fix changes *which* path is checked, not
    /// *when*: nothing here is atomic with the later `std::fs::read` or
    /// `atomic_write` create/rename, so a concurrent process could still swap
    /// the record path, or a component of it, for a symlink between this
    /// check and the I/O. Closing that window needs handle-relative
    /// (`openat`/`O_NOFOLLOW`-style) I/O and is not done in A1, whose threat
    /// model is explicitly single-process and non-adversarial (see
    /// `eval/results/A1/evidence.md`, "Known limitations").
    fn record_path_checked(&self, note_id: &NoteId) -> Result<PathBuf, StorageError> {
        let candidate = self.root.join(format!("{}.record", note_id.as_str()));
        let canonical_root = std::fs::canonicalize(&self.root).map_err(StorageError::Io)?;
        ensure_resolves_inside_root(&canonical_root, &candidate, note_id)?;
        Ok(candidate)
    }
}

/// Fails with [`StorageError::PathEscapesVaultRoot`] unless `candidate`
/// resolves to a location inside `canonical_root` (which must already be
/// canonical).
///
/// - If an entry exists at `candidate` — `symlink_metadata` succeeds, which
///   it does for a symlink whether or not the link's target exists — the
///   **full** `candidate` path is canonicalized. That resolves a symlink at
///   the final component *and* any symlinked directory component above it,
///   so both are caught by the same comparison. If canonicalization fails
///   (a dangling link, a self-referential link or loop, ...), the entry's
///   real location cannot be established and the check fails closed with
///   `PathEscapesVaultRoot`; it is never treated as "no note here yet".
/// - If nothing exists at `candidate` (first write), the parent directory is
///   canonicalized instead, which again resolves every symlinked directory
///   component on the way.
///
/// "Inside" means the canonical path starts with `canonical_root`. A link
/// whose target is elsewhere inside the root is accepted; that is aliasing,
/// not escape, and it is out of this check's scope.
fn ensure_resolves_inside_root(
    canonical_root: &Path,
    candidate: &Path,
    note_id: &NoteId,
) -> Result<(), StorageError> {
    let escapes = || StorageError::PathEscapesVaultRoot {
        note_id: note_id.as_str().to_string(),
    };
    let to_resolve = if candidate.symlink_metadata().is_ok() {
        candidate
    } else {
        candidate.parent().ok_or_else(escapes)?
    };
    let resolved = std::fs::canonicalize(to_resolve).map_err(|_| escapes())?;
    if resolved.starts_with(canonical_root) {
        Ok(())
    } else {
        Err(escapes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_test_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "superbrain-vault-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn non_not_found_io_error_on_read_propagates_as_io_error_not_version_zero() {
        // If the record path happens to be a directory (not a plain missing
        // file), std::fs::read fails with PermissionDenied (on this
        // platform), not NotFound. write_record_unchecked must propagate
        // that as StorageError::Io, not silently treat it as "note doesn't
        // exist yet" (Version(0)) and proceed to write over/through it.
        let root = unique_test_dir();
        let vault = Vault::new(root.clone());
        let id = NoteId::parse("note-a").unwrap();
        std::fs::create_dir_all(root.join("note-a.record")).unwrap();
        let err = vault
            .write_record_unchecked(&id, "content", Version(0), Uuid::new_v4())
            .unwrap_err();
        assert!(
            matches!(err, StorageError::Io(_)),
            "expected StorageError::Io, got {err:?}"
        );
        // Since fix C1 folds atomic-write failures into StorageError::Io too,
        // the error variant alone no longer distinguishes "stopped at the
        // read" from "treated it as Version(0) and attempted the write" (the
        // latter leaves an orphaned `note-a.record.tmp-*` file when the
        // rename onto the directory fails). Assert the write was never
        // attempted: the directory must contain only what we created.
        let entries: Vec<String> = std::fs::read_dir(&root)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(entries, vec!["note-a.record".to_string()], "no write may be attempted");
    }

    fn symlink_dir(target: &std::path::Path, link: &std::path::Path) {
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(target, link).unwrap();
    }

    #[test]
    fn symlink_at_the_bare_note_id_path_is_never_touched_by_io() {
        // Replaces the Task 6 test `write_target_resolving_outside_vault_root_via_symlink_is_rejected`,
        // which planted a directory symlink at `root/escape` (no `.record`
        // suffix) and expected a rejection. No I/O ever opens that path: the
        // record lives at `root/escape.record`, a sibling entry that does not
        // traverse the link. So the old test proved nothing about escapes
        // (Finding I-C). This test pins the actual behavior: the bare-path
        // entry is irrelevant, the write lands inside the root, and nothing
        // is written through the link.
        let root = unique_test_dir();
        let outside = unique_test_dir();
        symlink_dir(&outside, &root.join("escape"));
        let vault = Vault::new(root.clone());
        let id = NoteId::parse("escape").unwrap();
        let v = vault
            .write_record_unchecked(&id, "x", Version(0), Uuid::new_v4())
            .unwrap();
        assert_eq!(v, Version(1));
        assert!(root.join("escape.record").symlink_metadata().unwrap().is_file());
        assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0, "nothing written outside");
    }

    // Finding I-C, part (c): a symlinked *directory* component. With today's
    // layout the only directory above `<id>.record` is the root itself
    // (`NoteId` has no separators), so this cannot be reached through
    // `Vault`; it is tested on `ensure_resolves_inside_root` directly to
    // show that canonicalizing the full path covers it.

    #[test]
    fn existing_file_under_a_symlinked_directory_component_escapes() {
        let (root, outside_file) = root_and_outside_record();
        symlink_dir(outside_file.parent().unwrap(), &root.join("sub"));
        let canonical_root = std::fs::canonicalize(&root).unwrap();
        let id = NoteId::parse("victim").unwrap();
        let err = ensure_resolves_inside_root(&canonical_root, &root.join("sub").join("victim.record"), &id)
            .unwrap_err();
        assert!(matches!(err, StorageError::PathEscapesVaultRoot { .. }), "got {err:?}");
    }

    #[test]
    fn new_file_under_a_symlinked_directory_component_escapes() {
        let root = unique_test_dir();
        let outside = unique_test_dir();
        symlink_dir(&outside, &root.join("sub"));
        let canonical_root = std::fs::canonicalize(&root).unwrap();
        let id = NoteId::parse("fresh").unwrap();
        let err = ensure_resolves_inside_root(&canonical_root, &root.join("sub").join("fresh.record"), &id)
            .unwrap_err();
        assert!(matches!(err, StorageError::PathEscapesVaultRoot { .. }), "got {err:?}");
    }

    #[test]
    fn real_subdirectory_component_inside_the_root_is_accepted() {
        let root = unique_test_dir();
        std::fs::create_dir(root.join("sub")).unwrap();
        let canonical_root = std::fs::canonicalize(&root).unwrap();
        let id = NoteId::parse("fresh").unwrap();
        ensure_resolves_inside_root(&canonical_root, &root.join("sub").join("fresh.record"), &id).unwrap();
    }

    #[test]
    fn vault_root_reached_through_a_symlinked_ancestor_still_works() {
        // The root itself is the trust anchor: if the configured root path
        // goes through a symlink, both sides of the comparison are
        // canonicalized, so ordinary reads and writes are not rejected.
        let base = unique_test_dir();
        let real = unique_test_dir();
        symlink_dir(&real, &base.join("via"));
        let vault = Vault::new(base.join("via"));
        let id = NoteId::parse("note-a").unwrap();
        vault.write_record_unchecked(&id, "ok", Version(0), Uuid::new_v4()).unwrap();
        assert_eq!(vault.read_note(&id).unwrap().content, "ok");
        assert!(real.join("note-a.record").is_file());
    }

    // --- A1 third whole-branch review, Finding I-C ---
    //
    // Every test below plants the symlink at the REAL record path,
    // `root/<id>.record` (with the suffix), which is the exact path
    // `read_note` and `write_record_unchecked` open. The pre-I-C test planted
    // it at the bare `root/<id>` path, which no I/O ever touches, so it could
    // not detect an escape whether or not the check was correct.

    fn symlink_file(target: &std::path::Path, link: &std::path::Path) {
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(target, link).unwrap();
    }

    /// A vault root plus a sibling directory (outside the root) holding a
    /// well-formed record at version 7, so a read or version check that
    /// follows an escaping symlink *succeeds* and is observable.
    fn root_and_outside_record() -> (PathBuf, PathBuf) {
        let root = unique_test_dir();
        let outside_dir = unique_test_dir();
        let outside_file = outside_dir.join("victim.record");
        std::fs::write(&outside_file, serialize_record("OUTSIDE CONTENT", Version(7))).unwrap();
        (root, outside_file)
    }

    #[test]
    fn read_note_rejects_record_path_symlinked_to_a_file_outside_the_vault_root() {
        let (root, outside_file) = root_and_outside_record();
        symlink_file(&outside_file, &root.join("note-a.record"));
        let vault = Vault::new(root);
        let result = vault.read_note(&NoteId::parse("note-a").unwrap());
        assert!(
            matches!(result, Err(StorageError::PathEscapesVaultRoot { ref note_id }) if note_id == "note-a"),
            "read_note must refuse an escaping .record symlink, got {result:?}"
        );
        assert!(matches!(
            vault.current_version(&NoteId::parse("note-a").unwrap()),
            Err(StorageError::PathEscapesVaultRoot { .. })
        ));
    }

    #[test]
    fn write_rejects_record_path_symlinked_outside_and_never_reads_the_outside_version() {
        let (root, outside_file) = root_and_outside_record();
        let link = root.join("note-a.record");
        symlink_file(&outside_file, &link);
        let vault = Vault::new(root.clone());
        let id = NoteId::parse("note-a").unwrap();
        // Version(7) is the OUTSIDE file's version: before the fix the version
        // check read it through the link and accepted this write.
        let err = vault
            .write_record_unchecked(&id, "x", Version(7), Uuid::new_v4())
            .unwrap_err();
        assert!(matches!(err, StorageError::PathEscapesVaultRoot { .. }), "got {err:?}");
        // Version(0) must not surface as a Conflict carrying the outside version.
        let err = vault
            .write_record_unchecked(&id, "x", Version(0), Uuid::new_v4())
            .unwrap_err();
        assert!(matches!(err, StorageError::PathEscapesVaultRoot { .. }), "got {err:?}");
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink(), "link left untouched");
        assert_eq!(
            std::fs::read(&outside_file).unwrap(),
            serialize_record("OUTSIDE CONTENT", Version(7)),
            "outside file untouched"
        );
    }

    #[test]
    fn dangling_record_symlink_is_rejected_not_treated_as_a_missing_note() {
        // The link exists but its target does not. Before the fix, read_note
        // said NotFound and a Version(0) write went through (replacing the
        // link). The target's location cannot be verified, so it must fail
        // closed on both paths.
        let root = unique_test_dir();
        let outside_dir = unique_test_dir();
        let link = root.join("note-a.record");
        symlink_file(&outside_dir.join("does-not-exist.record"), &link);
        let vault = Vault::new(root.clone());
        let id = NoteId::parse("note-a").unwrap();
        let read = vault.read_note(&id);
        assert!(matches!(read, Err(StorageError::PathEscapesVaultRoot { .. })), "got {read:?}");
        let err = vault
            .write_record_unchecked(&id, "x", Version(0), Uuid::new_v4())
            .unwrap_err();
        assert!(matches!(err, StorageError::PathEscapesVaultRoot { .. }), "got {err:?}");
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink(), "no write through the link");
        assert!(!outside_dir.join("does-not-exist.record").exists());
    }

    #[test]
    fn self_referential_record_symlink_is_rejected_cleanly() {
        // A link to itself never resolves (ELOOP on unix, a resolution
        // failure on Windows). It must be an explicit rejection, not a panic.
        let root = unique_test_dir();
        let link = root.join("note-a.record");
        symlink_file(&link, &link);
        let vault = Vault::new(root.clone());
        let id = NoteId::parse("note-a").unwrap();
        let read = vault.read_note(&id);
        assert!(matches!(read, Err(StorageError::PathEscapesVaultRoot { .. })), "got {read:?}");
        let err = vault
            .write_record_unchecked(&id, "x", Version(0), Uuid::new_v4())
            .unwrap_err();
        assert!(matches!(err, StorageError::PathEscapesVaultRoot { .. }), "got {err:?}");
    }

    #[test]
    fn record_symlink_resolving_inside_the_vault_root_is_not_an_escape() {
        // The check is about leaving the root, not about symlinks as such: a
        // link whose target is inside the root is read normally.
        let root = unique_test_dir();
        std::fs::create_dir(root.join("store")).unwrap();
        let target = root.join("store").join("real.record");
        std::fs::write(&target, serialize_record("inside", Version(3))).unwrap();
        symlink_file(&target, &root.join("note-a.record"));
        let vault = Vault::new(root);
        let record = vault.read_note(&NoteId::parse("note-a").unwrap()).unwrap();
        assert_eq!(record.content, "inside");
        assert_eq!(record.version, Version(3));
    }

    #[test]
    fn write_then_read_returns_same_content_and_version_one() {
        let vault = Vault::new(unique_test_dir());
        let id = NoteId::parse("note-a").unwrap();
        let v = vault
            .write_record_unchecked(&id, "content v1", Version(0), Uuid::new_v4())
            .unwrap();
        assert_eq!(v, Version(1));
        let record = vault.read_note(&id).unwrap();
        assert_eq!(record.content, "content v1");
        assert_eq!(record.version, Version(1));
    }

    #[test]
    fn record_file_never_shows_mismatched_content_and_version() {
        // After N writes, the on-disk record's embedded version always matches
        // the version current_version() reports — there is exactly one file,
        // so this is a structural guarantee, not a race to test for.
        let vault = Vault::new(unique_test_dir());
        let id = NoteId::parse("note-a").unwrap();
        vault
            .write_record_unchecked(&id, "v1", Version(0), Uuid::new_v4())
            .unwrap();
        vault
            .write_record_unchecked(&id, "v2", Version(1), Uuid::new_v4())
            .unwrap();
        let record = vault.read_note(&id).unwrap();
        assert_eq!(record.version, Version(2));
        assert_eq!(record.content, "v2");
    }

    #[test]
    fn stale_expected_version_is_rejected_with_explicit_conflict() {
        let vault = Vault::new(unique_test_dir());
        let id = NoteId::parse("note-a").unwrap();
        vault
            .write_record_unchecked(&id, "v1", Version(0), Uuid::new_v4())
            .unwrap();
        let err = vault
            .write_record_unchecked(&id, "v2 attempt", Version(0), Uuid::new_v4())
            .unwrap_err();
        match err {
            StorageError::Conflict {
                note_id,
                expected,
                actual,
            } => {
                assert_eq!(note_id, "note-a");
                assert_eq!(expected, Version(0));
                assert_eq!(actual, Version(1));
            }
            other => panic!("expected Conflict, got {other:?}"),
        }
        let record = vault.read_note(&id).unwrap();
        assert_eq!(record.content, "v1");
        assert_eq!(record.version, Version(1));
    }

    #[test]
    fn version_numbers_do_not_collide_across_different_notes() {
        let vault = Vault::new(unique_test_dir());
        let a = NoteId::parse("note-a").unwrap();
        let b = NoteId::parse("note-b").unwrap();
        vault
            .write_record_unchecked(&a, "a-content", Version(0), Uuid::new_v4())
            .unwrap();
        let v = vault
            .write_record_unchecked(&b, "b-content", Version(0), Uuid::new_v4())
            .unwrap();
        assert_eq!(v, Version(1));
    }

    #[test]
    fn concurrent_writers_sharing_one_vault_instance_on_same_version_only_one_succeeds() {
        // Scope of this guarantee: both writers share `vault` (one process, one Vault
        // instance, two threads) — this is exactly what the in-memory mutex can promise.
        let vault = Arc::new(Vault::new(unique_test_dir()));
        let id = NoteId::parse("note-a").unwrap();
        vault
            .write_record_unchecked(&id, "initial", Version(0), Uuid::new_v4())
            .unwrap();

        let barrier = Arc::new(std::sync::Barrier::new(2));
        let mut handles = vec![];
        for i in 0..2 {
            let vault = Arc::clone(&vault);
            let barrier = Arc::clone(&barrier);
            let id = id.clone();
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                vault.write_record_unchecked(&id, &format!("writer-{i}"), Version(1), Uuid::new_v4())
            }));
        }
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let successes = results.iter().filter(|r| r.is_ok()).count();
        let conflicts = results
            .iter()
            .filter(|r| matches!(r, Err(StorageError::Conflict { .. })))
            .count();
        assert_eq!(successes, 1, "exactly one concurrent writer must succeed");
        assert_eq!(
            conflicts, 1,
            "the other must get an explicit conflict, not a silent overwrite or panic"
        );

        let record = vault.read_note(&id).unwrap();
        assert_eq!(record.version, Version(2));
    }

    #[test]
    fn losing_writer_can_retry_with_fresh_version_and_succeed() {
        let vault = Vault::new(unique_test_dir());
        let id = NoteId::parse("note-a").unwrap();
        vault
            .write_record_unchecked(&id, "initial", Version(0), Uuid::new_v4())
            .unwrap();
        let err = vault
            .write_record_unchecked(&id, "stale attempt", Version(0), Uuid::new_v4())
            .unwrap_err();
        assert!(matches!(err, StorageError::Conflict { .. }));
        let current = vault.read_note(&id).unwrap().version;
        let v = vault
            .write_record_unchecked(&id, "retry with fresh version", current, Uuid::new_v4())
            .unwrap();
        assert_eq!(v, Version(2));
    }
}
