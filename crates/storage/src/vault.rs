use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use domain::{IndexState, WriteState};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use vault_security::{DeletionLog, PiiStore, TrustedReferenceStore};

use crate::atomic::{atomic_write, AtomicWriteError};
use crate::fault::{FaultInjector, FaultPoint};
use crate::git::{commit_operation, commit_operation_contents, find_commit_for_operation, GitError, ScanDepth};
use crate::journal::{AdvanceFields, Journal};
use crate::note_id::NoteId;
use crate::recovery::{own_content, reconcile, ReconcileError};

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
pub(crate) fn parse_record(bytes: &[u8], note_id: &NoteId) -> Result<VaultRecord, StorageError> {
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
    /// A failure inside the SQLite-backed journal. The crate-private
    /// `journal::JournalError` cannot be stored directly here (`StorageError`
    /// is `pub`, and a `pub(crate)` type in a `pub` enum variant's field is a
    /// visibility leak rustc/clippy reject under `-D warnings`), so its
    /// rendered message is captured instead — this still folds it out of the
    /// public API, matching the existing `AtomicWriteError` -> `Io` pattern.
    #[error("journal error: {0}")]
    Journal(String),
    /// The on-disk bytes at the record path for `note_id` do not hash to the
    /// `content_hash` recorded by the journal's resolved committed row (a
    /// post-commit integrity failure), and an attempted rollback to the
    /// before-image also failed verification (or none existed). There is no
    /// read-time repair for this: the only way back to a readable vault is
    /// through `Vault::open`'s recovery pass (`reconcile`, Task 7), which
    /// performs the rollback as a journal state transition rather than a
    /// read substituting a different operation's before-image.
    #[error("note {note_id} is inconsistent: on-disk content does not match the journal's committed record")]
    Inconsistent { note_id: String },
    /// This operation's content is durably persisted and journal-readable
    /// (it reached `WriteState::JournalCommitted`) but could not be advanced
    /// to `GitCommitted`: the vault root is not (yet) a git repository, the
    /// repository has no commits yet (unborn `HEAD`), or git otherwise failed
    /// to commit. This is the spec's `git_pending` *response code* ("se git
    /// fallisce restituisce git_pending"), a non-error outcome from
    /// `propose_and_commit`'s perspective — distinct from a persisted
    /// `WriteState` — not a write failure: the journal row legitimately stays
    /// at `JournalCommitted` until a retry of the same `operation_id` or the
    /// next `Vault::open` commits it. Also returned when this operation's own
    /// bytes can no longer be recovered for its commit (see
    /// `recovery::own_content`).
    #[error("git commit pending for note {note_id}: the content is durably persisted, but git could not commit it yet")]
    GitPending { note_id: String },
    /// Reserved for a failure of the git-commit step that is not
    /// `git_pending`. Since the final whole-branch review (finding I2) the
    /// write path classifies git failures exactly as recovery does — every
    /// git failure after `JournalCommitted` is `GitPending` — so A2 never
    /// constructs this variant. It is kept so the public error type does not
    /// change shape within A2.
    #[error("git error: {0}")]
    Git(String),
    /// A pipeline was stopped at `FaultPoint` by a test's armed
    /// `FaultInjector`. Production code passes `FaultInjector::none()`
    /// everywhere and never produces this. It comes from
    /// `Vault::open_with_injector` propagating an interrupted `reconcile`, and
    /// from `write_record_unchecked_with_injector`. `FaultPoint` is nominally `pub` but
    /// lives in the private `fault` module, so code outside this crate can
    /// match this variant (`Interrupted(_)`) but cannot name the point.
    #[error("interrupted at {0:?}")]
    Interrupted(FaultPoint),
    /// A retry of an `operation_id` whose journal row is already `Failed`.
    /// Per C1 ("retry of the same operation_id: the outcome already reached,
    /// no double application"), the write is not attempted again. The caller
    /// must use a new `operation_id`.
    #[error("operation {operation_id} already failed; retry with a new operation_id")]
    OperationFailed { operation_id: Uuid },
    /// A retry of an `operation_id` whose journal row exists but has not
    /// reached a resolved state (`IntentRecorded`, `ContentPersisted` or
    /// `Recovering`) in this process: it is either still being written by
    /// another call, or was abandoned mid-flight. Nothing is applied. The
    /// next `Vault::open` recovery pass resolves the row, after which a retry
    /// reports that outcome.
    #[error("operation {operation_id} is still in flight; nothing was applied")]
    OperationInProgress { operation_id: Uuid },
    /// A call reused an existing `operation_id` for a DIFFERENT request: a
    /// different note, expected version or content than the operation the
    /// journal recorded under that id. It is not a retry, so neither the
    /// original outcome nor the new request is applied or reported. The
    /// caller must use a new `operation_id`.
    #[error("operation {operation_id} already names a different request (note, expected version or content differ)")]
    OperationIdReused { operation_id: Uuid },
    /// A new write to `note_id` was refused because an earlier operation on
    /// the same note, `operation_id`, stopped partway after it may already
    /// have changed the record (its row is at `ContentPersisted`, or at
    /// `IntentRecorded` with its intended hash recorded) and was never
    /// committed or failed. Accepting a write now would build on content no
    /// operation committed (evidence item 26, second variant). Nothing was
    /// applied: the refusal happens before the record or any before-image is
    /// touched, and this call's own journal row is `Failed`. The next
    /// `Vault::open` recovery pass resolves `operation_id`; the caller then
    /// retries with a new `operation_id`.
    #[error("note {note_id} has an unresolved earlier operation {operation_id}; nothing was applied, reopen the vault to recover it")]
    UnresolvedOperation { note_id: String, operation_id: Uuid },
    /// A failure inside `PiiStore` itself (Task 6), folded the same way
    /// `JournalError` is: `vault-security`'s `PiiStoreError` cannot be
    /// stored directly in this `pub` enum's field for the same
    /// crate-boundary-visibility reason `StorageError::Journal` documents,
    /// so its rendered message is captured instead. No variant of
    /// `PiiStoreError` ever carries plaintext or key material (see its own
    /// doc comment), so this capture cannot leak either.
    #[error("PII store error: {0}")]
    PiiStore(String),
    /// A failure inside `vault-security`'s `DeletionLog` while recording a
    /// sensitive-note deletion (Task 6, A3 review round, IMPORTANT finding
    /// #1: the deletion-log write now runs inside this same journaled
    /// operation, under the per-note lock, after the version check —
    /// rather than before `write_sensitive_record_unchecked` is even
    /// called). Folded the same way `PiiStore`/`Journal` are: no variant of
    /// `DeletionLogError` ever carries plaintext (deletion log rows are
    /// note ids, generations and hashes only).
    #[error("deletion log error: {0}")]
    DeletionLog(String),
    /// A non-tombstone write targeted a note id the deletion log records as
    /// deleted (final whole-branch review, I2). A deleted note id is retired
    /// permanently, consistent with the deletion log's own monotonic,
    /// append-only model: accepting new content under it would leave a note
    /// that is live and "deleted" at once, which every later restore would
    /// then drop. Nothing was applied: the refusal happens under the
    /// per-note lock, before the record, any before-image or the PII store
    /// is touched, and this call's own journal row is `Failed`. A tombstone
    /// write to an already-deleted id is still accepted (it re-applies the
    /// deletion, e.g. to finish one whose tombstone step failed).
    #[error("note {note_id} has been deleted; a deleted note id is retired and cannot receive new content")]
    NoteDeleted { note_id: String },
}

/// Folds a crate-private `journal::JournalError` into `StorageError::Journal`
/// by capturing its rendered message, since the error type itself cannot be
/// stored in a `pub` enum's field (see `StorageError::Journal`'s doc comment).
fn journal_err(e: crate::journal::JournalError) -> StorageError {
    StorageError::Journal(e.to_string())
}

/// Folds a `vault_security::PiiStoreError` into `StorageError::PiiStore` by
/// capturing its rendered message (see that variant's doc comment).
fn pii_store_err(e: vault_security::PiiStoreError) -> StorageError {
    StorageError::PiiStore(e.to_string())
}

/// Bundles the `DeletionLog`/trusted-reference pair
/// `Vault::write_sensitive_record_unchecked_with_injector` calls
/// `record_deletion` against, when, and only when, this write is a
/// tombstone (A3 review round, IMPORTANT finding #1: the deletion log must
/// be written AFTER the per-note lock is held and the version check has
/// passed — i.e. inside the SAME journaled operation the pointer write
/// itself goes through — never before it, where a stale `expected_version`
/// could mark a note deleted in the log while its actual content stays
/// fully live and recoverable). Passed BY the caller
/// (`propose::commit_sensitive`) rather than constructed inside `vault.rs`,
/// since `vault.rs` has no opinion on where the deletion log's files live
/// — `propose.rs` already derives those paths from `PiiStore`'s root.
///
/// Carrying `&dyn TrustedReferenceStore` (not a concrete type) keeps
/// `vault.rs` from needing to know whether the real, DPAPI-authenticated
/// store or a test-only one is in use — exactly the abstraction
/// `vault-security`'s own `DeletionLog::record_deletion` already uses.
pub(crate) struct PendingDeletion<'a> {
    pub(crate) log: &'a DeletionLog,
    pub(crate) trusted_ref: &'a dyn TrustedReferenceStore,
}

/// Undoes `PiiStore::commit_staged`'s effect for `operation_id` as part of
/// rolling a sensitive write back to `Failed`, from the LIVE write path
/// (`write_sensitive_record_unchecked_with_injector`'s step 7 verify-mismatch
/// rollback): restores the live `.pii` file from this operation's own PII
/// before-image if one exists (an UPDATE — the live file must go back to the
/// PRIOR operation's ciphertext, never be left at this failed operation's
/// NEW ciphertext while the pointer rolls back to the OLD version), or
/// removes the live file outright if no before-image exists (a FIRST write
/// — there is no prior version to restore it to). The before-image, once
/// consumed by a successful restore, is deleted; the staged (already
/// promoted to live by `commit_staged`, so nothing remains under the
/// staging path) entry needs no separate discard here.
///
/// Looks up the before-image's EXPECTED plaintext hash and restores (or, on
/// a first write, removes) the live `.pii` file accordingly (A3 review
/// round 2, item #2 fix). The before-image is THIS operation's own copy of
/// the PRIOR, already-committed operation's ciphertext (per
/// `PiiStore::commit_staged`'s contract), so the hash it must decrypt to is
/// that PRIOR operation's recorded `pii_content_hash` — looked up via
/// `Journal::last_committed_for_note`, exactly as `recovery.rs`'s own
/// `roll_back_pii` does — never this failing operation's own hash (which
/// describes the NEW, not-yet-valid content the failed write was trying to
/// commit, and can never match the before-image's bytes).
///
/// `recovery.rs`'s `fix_intent_recorded`/`fix_content_persisted` have their
/// own, structurally identical rollback for the crash-recovery path (it
/// cannot call this directly: it has no `&Vault` to call it on, and needs
/// the extra `discard_staged` step for the staged-but-never-promoted case
/// this function does not need to handle, since by the time this runs
/// `commit_staged` has already consumed the staging entry).
///
/// A3 review round 3: this now delegates to `recovery::roll_back_pii`, the
/// single implementation of the fail-closed decision table, instead of
/// keeping a second copy. The copy it replaced removed the live file
/// whenever `has_before_image` said `false`, and that `bool` also reads
/// `false` on an I/O error, so an unreadable before-image led straight to
/// deleting the live file. An error from here leaves the row un-advanced
/// (the caller advances to `Failed` only on `Ok`), for recovery to resolve.
fn roll_back_pii(journal: &Mutex<Journal>, pii_store: &PiiStore, note_id: &NoteId, operation_id: Uuid) -> Result<(), StorageError> {
    let journal = journal.lock().unwrap();
    let row = journal
        .get(operation_id)
        .map_err(journal_err)?
        .ok_or_else(|| StorageError::Inconsistent { note_id: note_id.as_str().to_string() })?;
    crate::recovery::roll_back_pii(&journal, Some(pii_store), note_id, operation_id, &row).map_err(|e| reconcile_err(note_id, e))
}

/// Folds a crate-private `git::GitError` from the git step of the write chain
/// into `StorageError`, with exactly the classification recovery's row 3
/// (`recovery::fix_journal_committed`) applies to the same errors:
///
/// - `GitError::Journal`: a journal failure, not a git one, so it surfaces as
///   `StorageError::Journal`.
/// - every other `GitError` (`NoRepository`, and any `Git2` failure, such as
///   an unborn `HEAD` on a repository with no commits yet): the non-error
///   `GitPending` response code. Spec C1: "se git fallisce restituisce
///   git_pending". The git step only ever runs once the row is
///   `JournalCommitted`, so the content is already durable and readable and
///   the row rests there until a retry or the next `Vault::open` commits it.
///
/// Before the final whole-branch review (finding I2) only `NoRepository`
/// mapped to `GitPending` and everything else became a hard
/// `StorageError::Git`, so the live path reported a write that had durably
/// happened as an error, while recovery treated the identical condition as
/// pending.
fn git_err(note_id: &NoteId, e: GitError) -> StorageError {
    match e {
        GitError::Journal(e) => journal_err(e),
        GitError::NoRepository | GitError::Git2(_) => StorageError::GitPending {
            note_id: note_id.as_str().to_string(),
        },
    }
}

/// Folds a crate-private `recovery::ReconcileError`, from the shared
/// `recovery::own_content` helper used by the retry path, into `StorageError`.
fn reconcile_err(note_id: &NoteId, e: ReconcileError) -> StorageError {
    match e {
        ReconcileError::Io(e) => StorageError::Io(e),
        ReconcileError::Journal(e) => journal_err(e),
        ReconcileError::Interrupted(point) => StorageError::Interrupted(point),
        ReconcileError::Inconsistent { .. } => StorageError::Inconsistent {
            note_id: note_id.as_str().to_string(),
        },
    }
}

/// Step 0's guard (Task 8 fix round, finding I1): a call reusing an existing
/// `operation_id` is a retry only if it asks for exactly what the original
/// call asked for. Checked against the journal row: the same note id, the
/// same expected version and, once the row records a `content_hash`
/// (`ContentPersisted` onward), the same content, compared by hashing the
/// record this call would have written at the row's `new_version`. Any
/// mismatch is `OperationIdReused`. Otherwise the original outcome would be
/// reported for a request that was never applied (for example
/// `propose_and_commit` pairing the original version with link state
/// computed from the new, unapplied text).
fn ensure_retry_matches(
    row: &crate::journal::OperationRow,
    note_id: &NoteId,
    content: &str,
    expected_version: Version,
) -> Result<(), StorageError> {
    let content_differs = match (&row.content_hash, row.new_version) {
        (Some(recorded), Some(new_version)) => {
            content_hash(&serialize_record(content, Version(new_version))) != *recorded
        }
        _ => false,
    };
    if row.note_id != note_id.as_str() || row.expected_version != expected_version.0 || content_differs {
        return Err(StorageError::OperationIdReused {
            operation_id: row.operation_id,
        });
    }
    Ok(())
}

/// [`ensure_retry_matches`] plus a `pii_content_hash` comparison against
/// `plaintext` (IMPORTANT finding #3, A3 Task 6 review round): for a
/// `Sensitive` note, `content` is always the SAME pointer bytes
/// (frontmatter plus the fixed `pii://<id>` URI) regardless of what the
/// real plaintext is, so `ensure_retry_matches`'s pointer-hash check alone
/// can never distinguish a genuine retry from a reused `operation_id`
/// carrying DIFFERENT sensitive content. Checked the same way: by hashing
/// the plaintext this call would stage at the row's recorded
/// `new_version`'s point in the sequence and comparing against the row's
/// already-recorded `pii_content_hash`, once one exists (before
/// `AfterPiiStagedBeforeCommit`, no `pii_content_hash` is recorded yet, so
/// nothing to compare — exactly mirroring how `ensure_retry_matches` treats
/// `content_hash` before the pointer rename).
fn ensure_retry_matches_sensitive(
    row: &crate::journal::OperationRow,
    note_id: &NoteId,
    pointer_content: &str,
    expected_version: Version,
    plaintext: &[u8],
) -> Result<(), StorageError> {
    ensure_retry_matches(row, note_id, pointer_content, expected_version)?;
    if let Some(recorded) = &row.pii_content_hash {
        if content_hash(plaintext) != *recorded {
            return Err(StorageError::OperationIdReused {
                operation_id: row.operation_id,
            });
        }
    }
    Ok(())
}

/// The version a journal row at `JournalCommitted` or later committed.
fn committed_version(row: &crate::journal::OperationRow) -> Result<Version, StorageError> {
    row.new_version.map(Version).ok_or_else(|| StorageError::Inconsistent {
        note_id: row.note_id.clone(),
    })
}

/// The index step of the write pipeline (Task 6). Pre-B1 the only
/// implementation is [`NullIndexer`]; a real indexer arrives in phase B1.
pub(crate) trait Indexer: Send + Sync {
    /// Called once a write has reached `GitCommitted`, to let the index record
    /// that `note_id` is now at `version`.
    fn mark_current(&self, note_id: &str, version: u64);

    /// The index's own verdict on whether it reflects `note_id`'s current
    /// content. This is an index-freshness claim, separate from the journal's
    /// `WriteState`.
    fn state_for(&self, note_id: &str) -> IndexState;
}

/// The A2 stand-in indexer. There is no index to update, so `mark_current` is
/// a true no-op, and `state_for` always reports `Stale`: no indexer exists yet,
/// so no note can honestly be claimed to be indexed and current.
pub(crate) struct NullIndexer;

impl Indexer for NullIndexer {
    fn mark_current(&self, _note_id: &str, _version: u64) {}

    fn state_for(&self, _note_id: &str) -> IndexState {
        IndexState::Stale
    }
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
    ///
    /// A2 widens this from a writer-vs-writer mutex to a full
    /// reader-vs-writer exclusion: `read_note`/`current_version` now
    /// acquire the SAME per-note lock for the duration of their read, so a
    /// read can never observe the filesystem mid-write (see the Task 4
    /// architectural-review correction on the race between the writer's
    /// rename and its `JournalCommitted` journal advance). Still
    /// process-local/in-memory only — this does not change the mutex's
    /// documented cross-process limitations.
    locks: Mutex<HashMap<NoteId, Arc<Mutex<()>>>>,
    /// The SQLite-backed write-operation journal (Task 2) that every write
    /// now drives through, and that reads now resolve the current committed
    /// row from, instead of trusting the raw filesystem.
    ///
    /// Wrapped in its own `Mutex` (independent of the per-note locks above)
    /// because `rusqlite::Connection` is `!Sync`: two different notes can be
    /// read/written concurrently on this one `Vault` instance (they take
    /// different per-note locks), and both paths touch this same
    /// connection, so the connection itself needs its own short-lived lock
    /// around each individual journal call. This is unrelated to, and does
    /// not substitute for, the per-note lock's reader-vs-writer exclusion.
    journal: Mutex<Journal>,
    /// Serializes the git-commit sequence (`find_commit_for_operation` +
    /// `commit_operation` + the journal advance that records the resulting
    /// `Oid`) across ALL notes, not just within one note (Task 5 fix round,
    /// independent-review finding I1).
    ///
    /// The per-note lock map above only serializes writers (and readers) of
    /// the SAME note. Two threads writing to two DIFFERENT notes take
    /// different per-note locks and can run their chain extensions
    /// concurrently. Both would read the same `HEAD`, build their own tree
    /// via `TreeBuilder`, and call `repo.commit(Some(branch), ..., &[&parent])`
    /// — but a git branch ref update requires the ref's current tip to
    /// still equal the commit's first parent, so the second commit to
    /// actually update the ref fails (libgit2's "current tip is not the
    /// first parent" / `GIT_EMODIFIED`). Without this lock that failure
    /// surfaced as a hard `StorageError::Git` for the loser's otherwise-
    /// successful write (its content was already durably `JournalCommitted`),
    /// rather than the resumable `GitPending`-style outcome the rest of this
    /// plan treats git-layer contention as.
    ///
    /// Lock ordering (deadlock-free, matching the per-note lock's existing
    /// discipline): the per-note lock is always acquired FIRST, in
    /// `write_record_unchecked`'s step 2, before this lock is ever touched.
    /// This lock is acquired only within that per-note critical section,
    /// held only for the duration of the git-commit sequence itself (step
    /// 7, which since the final whole-branch review's finding I1 also
    /// commits, and takes through the index step, any earlier git-pending
    /// operation on the SAME note, whose per-note lock is the one already
    /// held), and is never held while attempting to acquire a per-note lock or
    /// the journal mutex's own short-lived locks (which are taken and
    /// released individually, not held across the git sequence). Since this
    /// lock is always acquired after the per-note lock and never before it,
    /// and no code path re-enters a per-note lock while already holding
    /// one, two threads can never form a lock-ordering cycle through it.
    git_lock: Mutex<()>,
    /// The index step of the write pipeline (Task 6). Its `mark_current` runs
    /// after `GitCommitted`, and its `state_for` backs `index_state_for`.
    indexer: Box<dyn Indexer>,
}

impl Vault {
    /// Opens a vault rooted at `root`, backed by the write-operation journal
    /// at `journal_path` (created if it does not yet exist), using `indexer`
    /// for the index step. Crate-private because [`Indexer`] is: callers
    /// outside this crate use [`Vault::open_with_default_indexer`].
    ///
    /// Before returning, runs the startup reconciliation pass
    /// (`recovery::reconcile`, Task 7) over every non-terminal journal row,
    /// with the vault root as the git repository. A `Vault` value exists only
    /// if that pass succeeded. If any operation cannot be brought to a safe
    /// state, this returns `StorageError::Inconsistent` (fail closed at
    /// startup), so no caller can use a vault with a known-bad operation left
    /// unresolved. Rows left at `JournalCommitted` because git is unavailable
    /// (`git_pending`) are an allowed resting state and do not block opening.
    pub(crate) fn open(root: PathBuf, journal_path: &Path, indexer: Box<dyn Indexer>) -> Result<Self, StorageError> {
        Self::open_reconciled(root, journal_path, indexer, None, &FaultInjector::none())
    }

    /// Opens a vault rooted at `root` with the A2 default indexer
    /// ([`NullIndexer`]), backed by the journal at `journal_path`. Replaces
    /// A1's `Vault::new`: every caller supplies a journal path, since A1 had
    /// no production caller to break. Runs the same startup reconciliation
    /// as `Vault::open` and fails closed the same way.
    ///
    /// No `PiiStore` is available to this constructor's recovery pass, so a
    /// non-terminal row left by a `Sensitive`-note write (one with a
    /// non-null `pii_content_hash`) cannot be verified against its staged
    /// ciphertext here; see [`Vault::open_with_pii_store`] for a vault whose
    /// recovery pass CAN resolve those rows. A caller that never writes
    /// `Sensitive` notes (or that always pairs this constructor with its own
    /// separate reconciliation of the PII store) is unaffected.
    pub fn open_with_default_indexer(root: PathBuf, journal_path: &Path) -> Result<Self, StorageError> {
        Self::open(root, journal_path, Box::new(NullIndexer))
    }

    /// [`Vault::open_with_default_indexer`], but also threads `pii_store`
    /// into the startup reconciliation pass (Task 6), so a non-terminal row
    /// left by an interrupted `Sensitive`-note write can be verified against
    /// its staged-or-committed ciphertext and rolled forward or back
    /// accordingly. This is the constructor a production caller that writes
    /// `Sensitive` notes must use.
    ///
    /// It also runs `DeletionLog::reconcile` against the deletion log at
    /// `pii_store.deletion_log_path()` and its DPAPI trusted reference
    /// first: a crash mid-deletion is rolled forward, and any other
    /// log/reference disagreement fails this call with
    /// `StorageError::DeletionLog` (final whole-branch review, I3).
    pub fn open_with_pii_store(root: PathBuf, journal_path: &Path, pii_store: &PiiStore) -> Result<Self, StorageError> {
        Self::open_reconciled(root, journal_path, Box::new(NullIndexer), Some(pii_store), &FaultInjector::none())
    }

    /// Test-only sibling of [`Vault::open_with_default_indexer`] (also uses
    /// [`NullIndexer`]) that threads `injector` into the internal `reconcile`
    /// call instead of `FaultInjector::none()`. This lets a test interrupt
    /// recovery itself, for example at
    /// `FaultPoint::AfterMarkedRecoveringBeforeFixup`. An interrupted pass
    /// comes back as `StorageError::Interrupted`, never as a usable `Vault`.
    #[cfg(test)]
    pub(crate) fn open_with_injector(root: PathBuf, journal_path: &Path, injector: &FaultInjector) -> Result<Self, StorageError> {
        Self::open_reconciled(root, journal_path, Box::new(NullIndexer), None, injector)
    }

    /// Test-only sibling that threads BOTH a `PiiStore` and an `injector`
    /// into the recovery pass, for crash-recovery tests over the sensitive
    /// write path.
    #[cfg(test)]
    pub(crate) fn open_with_pii_store_and_injector(
        root: PathBuf,
        journal_path: &Path,
        pii_store: &PiiStore,
        injector: &FaultInjector,
    ) -> Result<Self, StorageError> {
        Self::open_reconciled(root, journal_path, Box::new(NullIndexer), Some(pii_store), injector)
    }

    fn open_reconciled(
        root: PathBuf,
        journal_path: &Path,
        indexer: Box<dyn Indexer>,
        pii_store: Option<&PiiStore>,
        injector: &FaultInjector,
    ) -> Result<Self, StorageError> {
        // Final whole-branch review, I3: the deletion log's own C5
        // reconciliation runs on the same startup path as the journal's,
        // before any `Vault` exists, whenever a `PiiStore` (and so a
        // deletion log beside it) is in play. It rolls forward the one
        // explainable crash pattern (a `Pending` entry one generation ahead
        // of the trusted reference) and fails `open` closed on any other
        // disagreement ("modalità di riconciliazione"), exactly as the
        // journal's reconcile does for an unrecoverable operation. Runs first
        // so a half-finished deletion is completed before anything else.
        if let Some(store) = pii_store {
            let log = DeletionLog::open(&store.deletion_log_path()).map_err(|e| StorageError::DeletionLog(e.to_string()))?;
            let trusted_ref = vault_security::DpapiTrustedReferenceStore::new(store.deletion_log_reference_path());
            log.reconcile(&trusted_ref).map_err(|e| StorageError::DeletionLog(e.to_string()))?;
        }

        let journal = Journal::open(journal_path).map_err(journal_err)?;
        // No Vault lock is taken here: this runs before the Vault exists, so
        // nothing else can be writing or committing through it. See
        // `recovery`'s module docs, "Concurrency".
        match reconcile(&root, &journal, &root, indexer.as_ref(), pii_store, injector) {
            Ok(_report) => {}
            Err(ReconcileError::Inconsistent { operation_id, reason: _ }) => {
                let note_id = journal
                    .get(operation_id)
                    .map_err(journal_err)?
                    .map(|row| row.note_id)
                    .unwrap_or_else(|| format!("<unknown note for operation {operation_id}>"));
                return Err(StorageError::Inconsistent { note_id });
            }
            Err(ReconcileError::Interrupted(point)) => return Err(StorageError::Interrupted(point)),
            Err(ReconcileError::Journal(e)) => return Err(journal_err(e)),
            Err(ReconcileError::Io(e)) => return Err(StorageError::Io(e)),
        }
        Ok(Vault {
            root,
            locks: Mutex::new(HashMap::new()),
            journal: Mutex::new(journal),
            git_lock: Mutex::new(()),
            indexer,
        })
    }

    /// The indexer's own verdict on whether the index reflects `note_id`'s
    /// current content. Deliberately decoupled from the journal's `WriteState`:
    /// reaching `WriteState::IndexCurrent` only means the write pipeline's
    /// index step completed, and says nothing about index freshness. With
    /// `NullIndexer` this is always `IndexState::Stale`.
    // No non-test caller exists until a real index consumes it (phase B1).
    #[allow(dead_code)]
    pub(crate) fn index_state_for(&self, note_id: &NoteId) -> IndexState {
        self.indexer.state_for(note_id.as_str())
    }

    pub fn current_version(&self, note_id: &NoteId) -> Result<Version, StorageError> {
        self.read_note(note_id).map(|record| record.version)
    }

    /// Reads the current record for `note_id` by resolving it through the
    /// journal rather than trusting the raw filesystem: the committed row
    /// (`Journal::last_committed_for_note`) says which on-disk bytes are
    /// "current" and what hash they must have. A write in flight
    /// (`IntentRecorded`/`ContentPersisted`, not yet `JournalCommitted`) is
    /// invisible to readers, per C1.
    ///
    /// Shares the same per-note lock `write_record_unchecked` holds for its
    /// whole critical section, so a read can never be interleaved with
    /// another thread's in-progress rename on the same note within this
    /// process (Task 4 architectural-review correction).
    ///
    /// On a hash mismatch against the resolved row there is NO before-image
    /// fallback: this is unconditionally `StorageError::Inconsistent`. A
    /// mismatch means the vault's officially linearized file is corrupted —
    /// a genuine post-commit integrity failure that only `Vault::open`'s
    /// recovery pass (`reconcile`, Task 7) has the authority to repair, by
    /// demoting the affected operation to `Failed` as a journal state
    /// transition (after which `last_committed_for_note` resolves to the
    /// prior, still-correctly-paired, operation). Substituting a different
    /// operation's before-image at read time would pair that operation's
    /// overwritten content with THIS row's version number — exactly the
    /// cross-operation mislabeling the second architectural review forbids.
    pub fn read_note(&self, note_id: &NoteId) -> Result<VaultRecord, StorageError> {
        let note_lock = self.note_lock(note_id);
        let _guard = note_lock.lock().unwrap();
        self.read_note_locked(note_id)
    }

    /// The body of `read_note`, run while the caller already holds the
    /// per-note lock. Split out so `write_record_unchecked` can re-read the
    /// current committed record (for its own version check) without trying
    /// to re-acquire a lock it already holds.
    fn read_note_locked(&self, note_id: &NoteId) -> Result<VaultRecord, StorageError> {
        // The containment check runs before the journal lookup (A1's own
        // check order, unchanged): a symlink escape is a structural problem
        // with the path itself and must be caught regardless of whether a
        // committed journal row happens to exist yet for this note id.
        let path = self.record_path_checked(note_id)?;
        let Some(row) = self.journal.lock().unwrap().last_committed_for_note(note_id.as_str()).map_err(journal_err)? else {
            return Err(StorageError::NotFound {
                note_id: note_id.as_str().to_string(),
            });
        };
        let bytes = std::fs::read(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                StorageError::NotFound {
                    note_id: note_id.as_str().to_string(),
                }
            } else {
                StorageError::Io(e)
            }
        })?;
        let expected_hash = row.content_hash.as_deref();
        if expected_hash != Some(content_hash(&bytes).as_str()) {
            return Err(StorageError::Inconsistent {
                note_id: note_id.as_str().to_string(),
            });
        }
        parse_record(&bytes, note_id)
    }

    /// Returns (creating if absent) the `Arc<Mutex<()>>` for `note_id`,
    /// without locking it. Shared by `read_note` and
    /// `write_record_unchecked` so both sides serialize on the exact same
    /// lock object per note id.
    fn note_lock(&self, note_id: &NoteId) -> Arc<Mutex<()>> {
        let mut locks = self.locks.lock().unwrap();
        Arc::clone(
            locks
                .entry(note_id.clone())
                .or_insert_with(|| Arc::new(Mutex::new(()))),
        )
    }

    /// Writes `content` as the new record for `note_id`, provided
    /// `expected_version` matches the version currently on disk (or
    /// `Version(0)` if the note does not yet exist).
    ///
    /// `operation_id` is the journal key and the idempotency key: a call
    /// with an `operation_id` the journal already knows never writes again
    /// and reports the outcome that operation already reached (step 0
    /// below; see `resume_existing_operation`).
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
    ///
    /// A2 restructures the body to drive the write through the journal
    /// (Task 2) to `WriteState::JournalCommitted`, and to persist a
    /// before-image (Task 3) of any content being overwritten. The exact
    /// sequence, each step's journal/filesystem effect followed immediately
    /// by a `FaultInjector::check` call (production callers always pass
    /// `FaultInjector::none()`, so this never interrupts them — the seam
    /// exists for Task 8's crash tests):
    ///
    /// 0. If the journal already has a row for `operation_id`: unless this
    ///    call asks for exactly the same note, expected version and content
    ///    (`ensure_retry_matches`), return `OperationIdReused`. Otherwise return
    ///    that operation's outcome via `resume_existing_operation` (`Failed` ->
    ///    `OperationFailed`, finished -> its version, mid-flight ->
    ///    `OperationInProgress`, `JournalCommitted`/`GitCommitted` -> finish
    ///    the remaining git/index steps). Nothing is written twice.
    /// 1. `journal.insert_intent` — this IS `IntentRecorded`, persisted
    ///    before anything else happens. Safe to run before the per-note
    ///    lock below because it touches only the journal, never the
    ///    filesystem: nothing a concurrent reader could observe on disk
    ///    changes as a result of this row existing.
    /// 2. Acquire the per-note lock (now also shared with `read_note`). If
    ///    another operation on this note stopped partway after it may have
    ///    changed the record (`Journal::unresolved_operation_on_note`), advance
    ///    this row to `Failed` and return `UnresolvedOperation` without touching
    ///    the filesystem (evidence item 26, second variant). Otherwise
    ///    read the actual current version exactly as A1 did. On a version
    ///    mismatch, advance the journal row to `Failed` (a rejected
    ///    attempt is terminal) and return `Conflict`.
    /// 3. If a record already exists, persist its current bytes as the
    ///    before-image for this `operation_id`.
    /// 4. Record the intended `new_version`, new content hash and (if one
    ///    exists) before-image hash on the still-`IntentRecorded` row
    ///    (`Journal::record_fields`, no state change), then write the new
    ///    record via `atomic_write`, then advance the journal to
    ///    `ContentPersisted` with the same fields. Recording the hash before
    ///    the rename is what lets recovery tell a rename that landed from
    ///    one that never happened if the process stops in between (final
    ///    whole-branch review, finding C1).
    /// 5. Re-read the just-written file and verify its hash. On mismatch,
    ///    attempt rollback to the before-image and verify THAT hash (on
    ///    success the before-image is deleted, the row advanced to `Failed`,
    ///    and `Inconsistent` returned to report the write did not land); if
    ///    rollback also fails verification (or no before-image exists),
    ///    return `StorageError::Inconsistent` without advancing the journal
    ///    past `ContentPersisted`, so the next `Vault::open` recovery pass
    ///    finds it and also fails closed.
    /// 6. On successful verification, advance the journal to
    ///    `JournalCommitted` — the linearization point.
    /// 7. (Task 5) Extend the chain from `JournalCommitted` to
    ///    `GitCommitted`: call `find_commit_for_operation(..., ScanDepth::Bounded(500))`
    ///    first. If it returns `Some(oid)` (via either tier), make sure the
    ///    row's `git_commit_oid` is set to that `oid` and advance to
    ///    `GitCommitted` (an idempotent re-entry). If it returns `None`,
    ///    call `commit_operation`; on success, advance to `GitCommitted`
    ///    with `AdvanceFields::git_oid(oid)` on the SAME journal write that
    ///    advances the state, so there is never a window where the state
    ///    says `GitCommitted` but `git_commit_oid` is unset. Either way,
    ///    once `GitCommitted` is reached, delete the before-image (per
    ///    spec: "La before-image si elimina solo dopo git_committed"). On
    ///    any git failure (`NoRepository`, an unborn `HEAD`, any other
    ///    `git2` error), do NOT advance the journal past
    ///    `JournalCommitted`; return `StorageError::GitPending` (the spec's
    ///    `git_pending` response code, not a persisted `WriteState` — the
    ///    row legitimately stays at `JournalCommitted` until a later retry
    ///    succeeds). This is the same classification recovery's row 3 uses
    ///    (see `git_err`). Only a journal failure inside the git step is a
    ///    hard `StorageError::Journal`.
    /// 8. Release the per-note lock (by scope exit).
    /// 9. Return the new version.
    ///
    /// The chain ends at `IndexCurrent` (Task 6): after `GitCommitted`, the
    /// index step calls `Indexer::mark_current` and advances the journal to
    /// `WriteState::IndexCurrent`. That state marks pipeline completion only;
    /// it does not claim the index is current (see `index_state_for`).
    ///
    /// Production goes through [`Vault::write_record_unchecked_with_pii_store`]
    /// (A3 review round 3); this `PiiStore`-less form remains the primitive
    /// the crate's tests drive, hence the `dead_code` allowance outside tests.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn write_record_unchecked(
        &self,
        note_id: &NoteId,
        content: &str,
        expected_version: Version,
        operation_id: Uuid,
    ) -> Result<Version, StorageError> {
        self.write_record_unchecked_with_injector(note_id, content, expected_version, operation_id, &FaultInjector::none())
    }

    /// [`Vault::write_record_unchecked`] for a caller that has a `PiiStore`
    /// in hand (`propose::propose_and_commit`'s non-sensitive branch). The
    /// write itself never touches `pii_store`. It is threaded through only
    /// so that `commit_to_git`'s catch-up of an EARLIER git-pending
    /// SENSITIVE operation on the same note can delete that operation's PII
    /// before-image when it commits it (A3 review round 3, item 1(b)).
    /// Without a store, as from plain `write_record_unchecked`, such a
    /// catch-up is refused and this write rests as `GitPending` instead.
    pub(crate) fn write_record_unchecked_with_pii_store(
        &self,
        note_id: &NoteId,
        content: &str,
        expected_version: Version,
        operation_id: Uuid,
        pii_store: Option<&PiiStore>,
    ) -> Result<Version, StorageError> {
        self.write_record_impl(note_id, content, expected_version, operation_id, pii_store, &FaultInjector::none())
    }

    /// [`Vault::write_record_unchecked`] with an explicit `injector`. Production
    /// callers go through `write_record_unchecked`, which passes
    /// `FaultInjector::none()`. An armed injector stops the write right after
    /// the named step boundary and returns `StorageError::Interrupted(point)`
    /// (a `pause_at`-armed injector instead pauses and always resumes to
    /// completion, never returning `Interrupted`). The six write-path
    /// points are checked, in order: `AfterPathCheckedBeforeAtomicWrite`
    /// (right after `record_path_checked`, Task 9's SEC-001 characterization
    /// seam), `AfterIntentRecorded` (after step 1),
    /// `AfterRenameBeforeContentPersisted` (right after step 4's rename, row
    /// still `IntentRecorded`), `AfterRenameBeforeJournalCommitted` (after step 4's `ContentPersisted`
    /// advance), `AfterJournalCommittedBeforeGit` (after step 6) and
    /// `AfterGitCommittedBeforeIndex` (after step 7). The sixth,
    /// `DuringBeforeImageRestore`, lives in `recovery::reconcile`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn write_record_unchecked_with_injector(
        &self,
        note_id: &NoteId,
        content: &str,
        expected_version: Version,
        operation_id: Uuid,
        injector: &FaultInjector,
    ) -> Result<Version, StorageError> {
        self.write_record_impl(note_id, content, expected_version, operation_id, None, injector)
    }

    /// The body of `write_record_unchecked_with_injector`, with the optional
    /// `pii_store` described on [`Vault::write_record_unchecked_with_pii_store`].
    fn write_record_impl(
        &self,
        note_id: &NoteId,
        content: &str,
        expected_version: Version,
        operation_id: Uuid,
        pii_store: Option<&PiiStore>,
        injector: &FaultInjector,
    ) -> Result<Version, StorageError> {
        let interrupted = |crate::fault::Interrupted::At(point)| StorageError::Interrupted(point);

        // Step 0: retry idempotency (C1, "Retry dello stesso operation_id").
        // An operation_id the journal already knows is never applied a second
        // time; the outcome it already reached is reported instead. Bound to
        // a local first so the journal guard is released before resuming.
        let existing = self.journal.lock().unwrap().get(operation_id).map_err(journal_err)?;
        if let Some(row) = existing {
            ensure_retry_matches(&row, note_id, content, expected_version)?;
            return self.resume_existing_operation(row, pii_store);
        }

        let record_path = self.record_path_checked(note_id)?;
        // Task 9 seam: lets a test pause right after the containment check
        // judged `record_path` safe and swap it for a symlink before the
        // write below touches it, to characterize SEC-001. Never armed by
        // `FaultInjector::none()` (every production caller).
        injector.check(FaultPoint::AfterPathCheckedBeforeAtomicWrite).map_err(interrupted)?;

        // Step 1: intent_recorded. Deliberately before the per-note lock —
        // see the doc comment above for why that is safe.
        self.journal.lock().unwrap().insert_intent(operation_id, note_id.as_str(), expected_version.0).map_err(journal_err)?;
        injector.check(FaultPoint::AfterIntentRecorded).map_err(interrupted)?;

        // Step 2: acquire the per-note lock, now shared with read_note, and
        // hold it for the rest of this critical section — reading the
        // current version, comparing it to `expected_version`, persisting
        // the before-image, writing the new record, and advancing the
        // journal to JournalCommitted are all done while this lock is held,
        // so no concurrent reader on this Vault instance can observe the
        // filesystem mid-write (Task 4 architectural-review correction).
        let note_lock = self.note_lock(note_id);
        let _guard = note_lock.lock().unwrap();

        // Step 2a (evidence item 26, second variant): refuse the write if an
        // earlier operation on this note stopped partway after it may have
        // changed the record. The raw file may then hold bytes no operation
        // committed, and the version check below would accept a write built
        // on them. Checked under the per-note lock, where such a row can only
        // be stalled, never in flight (see `unresolved_operation_on_note`),
        // and before the record is read, the before-image is persisted or
        // anything is renamed. This call's own intent row becomes Failed, as
        // for a Conflict.
        let unresolved = self
            .journal
            .lock()
            .unwrap()
            .unresolved_operation_on_note(note_id.as_str(), operation_id)
            .map_err(journal_err)?;
        if let Some(blocking) = unresolved {
            self.journal.lock().unwrap().advance(operation_id, WriteState::Failed, AdvanceFields::none()).map_err(journal_err)?;
            return Err(StorageError::UnresolvedOperation {
                note_id: note_id.as_str().to_string(),
                operation_id: blocking.operation_id,
            });
        }

        let current_bytes = match std::fs::read(&record_path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(StorageError::Io(e)),
        };
        let actual_version = match &current_bytes {
            Some(bytes) => parse_record(bytes, note_id)?.version,
            None => Version(0),
        };

        if actual_version != expected_version {
            self.journal.lock().unwrap().advance(operation_id, WriteState::Failed, AdvanceFields::none()).map_err(journal_err)?;
            return Err(StorageError::Conflict {
                note_id: note_id.as_str().to_string(),
                expected: expected_version,
                actual: actual_version,
            });
        }

        // Step 2b (final whole-branch review, I2): a note id the deletion
        // log records as deleted is retired, whatever classification this
        // new write carries (a deleted sensitive note must not come back as
        // an Internal one either). Only checkable when a `PiiStore` is at
        // hand, since the deletion log lives beside it;
        // `propose_and_commit` passes one whenever the caller configured it.
        if let Some(store) = pii_store {
            self.refuse_if_deleted(store, note_id, operation_id)?;
        }

        // Step 3: preserve what this write is about to overwrite, if
        // anything, keyed by THIS operation's own operation_id.
        let before_image_hash = match &current_bytes {
            Some(bytes) => Some(persist_before_image(&self.root, note_id, operation_id, bytes)?),
            None => None,
        };

        // Step 4: write the new record, then advance the journal to
        // ContentPersisted carrying both hashes on the same row.
        let new_version = Version(actual_version.0 + 1);
        let buffer = serialize_record(content, new_version);
        let new_hash = content_hash(&buffer);
        let fields = match &before_image_hash {
            Some(before_hash) => {
                AdvanceFields::content_with_before_image(new_version.0, new_hash.clone(), before_hash.clone())
            }
            None => AdvanceFields::content(new_version.0, new_hash.clone()),
        };
        // Step 4a (final whole-branch review, finding C1): record the
        // INTENDED version and hashes on the still-IntentRecorded row before
        // the rename. If the process stops between the rename below and the
        // ContentPersisted advance, recovery's row 1 can then check the
        // record on disk against this hash: a match means the rename landed
        // and the operation rolls forward; anything else means it never did.
        // Without this, both cases looked identical and row 1 marked a
        // completed rename Failed, stranding the note.
        self.journal
            .lock()
            .unwrap()
            .record_fields(operation_id, WriteState::IntentRecorded, fields.clone())
            .map_err(journal_err)?;
        atomic_write(&record_path, &buffer).map_err(|AtomicWriteError::Io(e)| StorageError::Io(e))?;
        injector.check(FaultPoint::AfterRenameBeforeContentPersisted).map_err(interrupted)?;
        self.journal.lock().unwrap().advance(operation_id, WriteState::ContentPersisted, fields).map_err(journal_err)?;
        // Checked AFTER the ContentPersisted advance (Task 7 fix round,
        // finding I-1): "the rename happened AND the journal records it".
        // The earlier window, after the rename but before this advance, is
        // `AfterRenameBeforeContentPersisted` above; it is recoverable
        // because step 4a recorded the intended hash first (final
        // whole-branch review, finding C1).
        injector.check(FaultPoint::AfterRenameBeforeJournalCommitted).map_err(interrupted)?;

        // Step 5: verify the bytes actually on disk hash to what was just
        // recorded. This closes the "hash della nuova versione verificato"
        // fault-matrix row.
        let written_bytes = std::fs::read(&record_path).map_err(StorageError::Io)?;
        if content_hash(&written_bytes) != new_hash {
            if let Some(before_hash) = &before_image_hash {
                if let Ok(before_bytes) = read_before_image(&self.root, note_id, operation_id) {
                    if content_hash(&before_bytes) == *before_hash
                        && atomic_write(&record_path, &before_bytes).is_ok()
                    {
                        let rolled_back = std::fs::read(&record_path).map_err(StorageError::Io)?;
                        if content_hash(&rolled_back) == *before_hash {
                            // Delete the before-image BEFORE advancing to the
                            // terminal Failed state, matching recovery's row 2:
                            // a Failed row is never revisited, so the opposite
                            // order would orphan the file (Task 8).
                            delete_before_image(&self.root, note_id, operation_id)?;
                            self.journal.lock().unwrap().advance(operation_id, WriteState::Failed, AdvanceFields::none()).map_err(journal_err)?;
                            return Err(StorageError::Inconsistent {
                                note_id: note_id.as_str().to_string(),
                            });
                        }
                    }
                }
            }
            // Rollback was not possible, or did not itself verify: fail
            // closed WITHOUT advancing the journal past ContentPersisted,
            // per the spec's allowance that recovery itself may not be able
            // to resolve this one fault-matrix row.
            return Err(StorageError::Inconsistent {
                note_id: note_id.as_str().to_string(),
            });
        }

        // Step 6: the linearization point.
        self.journal.lock().unwrap().advance(operation_id, WriteState::JournalCommitted, AdvanceFields::none()).map_err(journal_err)?;
        injector.check(FaultPoint::AfterJournalCommittedBeforeGit).map_err(interrupted)?;

        // Step 7 (Task 5): JournalCommitted -> GitCommitted, committing the
        // record this call just wrote and verified (still under the per-note
        // lock). A brand-new operation cannot already have a commit far back
        // in history, so the bounded scan suffices here.
        self.commit_to_git(note_id, &record_path, operation_id, new_version, ScanDepth::Bounded(500), None, pii_store)?;
        injector.check(FaultPoint::AfterGitCommittedBeforeIndex).map_err(interrupted)?;

        // Step 7b (Task 6): the index step, GitCommitted -> IndexCurrent.
        self.mark_index_current(note_id, operation_id, new_version)?;

        // Step 8: the per-note lock is released when `_guard` drops at the
        // end of this scope.
        // Step 9:
        Ok(new_version)
    }

    /// The `Sensitive`-note sibling of [`Vault::write_record_unchecked`]
    /// (Task 6). Production callers (`propose::propose_and_commit`) reach
    /// this instead of `write_record_unchecked` when, and only when, the
    /// note being written is `Sensitive`-classified; every other
    /// classification is completely unaffected — this method shares the
    /// SAME journal, the SAME `WriteState` machine and the SAME per-note
    /// lock as `write_record_unchecked`, but is a separate method rather
    /// than a parameter on it so that none of `write_record_unchecked`'s
    /// ~90 existing call sites (across this crate's own test suite) need to
    /// change.
    ///
    /// This is deliberately ONE crash-recoverable operation, not two
    /// independently-recovered stores: `pointer_content` (what actually
    /// lands in the Markdown record — the frontmatter plus a `pii://<id>`
    /// pointer body) and `plaintext` (the real sensitive content, staged
    /// into `pii_store`) are both driven through this one `operation_id`'s
    /// journal row. The row's `content_hash` describes the POINTER bytes
    /// (exactly as it always has for `write_record_unchecked`); the new,
    /// additive `pii_content_hash` column describes the PLAINTEXT staged
    /// into `PiiStore`. `recovery::reconcile` treats a row with a non-null
    /// `pii_content_hash` as needing its staged-or-committed ciphertext
    /// verified before rolling forward — see that module's docs.
    ///
    /// Sequence (steps 0-3 identical to `write_record_unchecked_with_injector`;
    /// divergence starts at step 4):
    ///
    /// 0-3. Same retry/idempotency check, per-note lock acquisition, stalled-
    ///    operation guard, version check and before-image persistence as
    ///    `write_record_unchecked_with_injector`.
    /// 4. The PLAINTEXT's hash is recorded on the still-`IntentRecorded` row
    ///    as `pii_content_hash`, alongside the pointer's own intended
    ///    `content_hash` (mirroring how the ordinary path records its
    ///    intended hash before the rename, final whole-branch review, finding
    ///    C1), and THEN `PiiStore::stage(note_id, plaintext, operation_id)`
    ///    durably fsyncs the ciphertext to the staging location, NOT yet the
    ///    live `.pii` file, and its returned hash is checked against the
    ///    recorded one. Recording first means no staged file can exist under
    ///    a row recovery does not know is sensitive (A3 review round 3).
    ///    `FaultPoint::AfterPiiStagedBeforeCommit` is checked after staging.
    /// 5. `PiiStore::commit_staged` — renames the staged ciphertext onto the
    ///    live `.pii` path. `FaultPoint::AfterPiiCommittedBeforePointerRename`
    ///    is checked immediately after.
    /// 6. The Markdown pointer is written via `atomic_write` and the journal
    ///    advanced to `ContentPersisted`, exactly as step 4 of the ordinary
    ///    path does for ordinary content — `FaultPoint::AfterRenameBeforeContentPersisted`
    ///    is checked at the same point in the sequence.
    /// 7 onward: identical to `write_record_unchecked_with_injector`'s steps
    ///    5-9 (verify, `JournalCommitted`, git, index).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn write_sensitive_record_unchecked(
        &self,
        note_id: &NoteId,
        pointer_content: &str,
        plaintext: &[u8],
        expected_version: Version,
        operation_id: Uuid,
        pii_store: &PiiStore,
        deletion: Option<PendingDeletion<'_>>,
    ) -> Result<Version, StorageError> {
        self.write_sensitive_record_unchecked_with_injector(
            note_id,
            pointer_content,
            plaintext,
            expected_version,
            operation_id,
            pii_store,
            deletion,
            &FaultInjector::none(),
        )
    }

    /// [`Vault::write_sensitive_record_unchecked`] with an explicit
    /// `injector`. See that method's doc comment for the full sequence.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn write_sensitive_record_unchecked_with_injector(
        &self,
        note_id: &NoteId,
        pointer_content: &str,
        plaintext: &[u8],
        expected_version: Version,
        operation_id: Uuid,
        pii_store: &PiiStore,
        deletion: Option<PendingDeletion<'_>>,
        injector: &FaultInjector,
    ) -> Result<Version, StorageError> {
        let interrupted = |crate::fault::Interrupted::At(point)| StorageError::Interrupted(point);

        // Step 0: identical retry/idempotency handling.
        let existing = self.journal.lock().unwrap().get(operation_id).map_err(journal_err)?;
        if let Some(row) = existing {
            ensure_retry_matches_sensitive(&row, note_id, pointer_content, expected_version, plaintext)?;
            return self.resume_existing_operation(row, Some(pii_store));
        }

        let record_path = self.record_path_checked(note_id)?;
        injector.check(FaultPoint::AfterPathCheckedBeforeAtomicWrite).map_err(interrupted)?;

        // Step 1: intent_recorded.
        self.journal.lock().unwrap().insert_intent(operation_id, note_id.as_str(), expected_version.0).map_err(journal_err)?;
        injector.check(FaultPoint::AfterIntentRecorded).map_err(interrupted)?;

        // Step 2: per-note lock, held for the rest of this critical section.
        let note_lock = self.note_lock(note_id);
        let _guard = note_lock.lock().unwrap();

        let unresolved = self
            .journal
            .lock()
            .unwrap()
            .unresolved_operation_on_note(note_id.as_str(), operation_id)
            .map_err(journal_err)?;
        if let Some(blocking) = unresolved {
            self.journal.lock().unwrap().advance(operation_id, WriteState::Failed, AdvanceFields::none()).map_err(journal_err)?;
            return Err(StorageError::UnresolvedOperation {
                note_id: note_id.as_str().to_string(),
                operation_id: blocking.operation_id,
            });
        }

        let current_bytes = match std::fs::read(&record_path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(StorageError::Io(e)),
        };
        let actual_version = match &current_bytes {
            Some(bytes) => parse_record(bytes, note_id)?.version,
            None => Version(0),
        };

        if actual_version != expected_version {
            self.journal.lock().unwrap().advance(operation_id, WriteState::Failed, AdvanceFields::none()).map_err(journal_err)?;
            return Err(StorageError::Conflict {
                note_id: note_id.as_str().to_string(),
                expected: expected_version,
                actual: actual_version,
            });
        }

        // Step 2b (Task 6, A3 review round, IMPORTANT finding #1): record a
        // deletion in the log HERE — after the per-note lock is held and the
        // version check above has passed, inside the SAME critical section
        // the pointer write itself goes through — never before any of that,
        // where a stale `expected_version` could mark a note deleted in the
        // log while its actual content stays fully live and recoverable.
        // `DeletionLog::record_deletion` is NOT journaled by the A2
        // machine itself (it is a separate, independently-authenticated
        // mechanism per spec contract C5 — see `PendingDeletion`'s doc
        // comment), but running it only after this operation has already
        // proven it is a legitimate write at the expected version makes the
        // two consistent in practice: a deletion is logged if and only if
        // the tombstone pointer this same call is about to commit is also
        // about to be accepted.
        //
        // Final whole-branch review, I2: a deleted note id is retired. A
        // NON-tombstone write to it is refused here (`refuse_if_deleted`,
        // same lock, same point). A tombstone write to an id that is
        // already deleted re-applies the tombstone but does not append a
        // second log entry: the log already says "deleted", and a second
        // generation for the same id would add nothing but chain length.
        match &deletion {
            Some(deletion) => {
                let already_deleted = deletion
                    .log
                    .is_deleted(note_id.as_str())
                    .map_err(|e| StorageError::DeletionLog(e.to_string()))?;
                if !already_deleted {
                    deletion
                        .log
                        .record_deletion(note_id.as_str(), deletion.trusted_ref)
                        .map_err(|e| StorageError::DeletionLog(e.to_string()))?;
                }
            }
            None => self.refuse_if_deleted(pii_store, note_id, operation_id)?,
        }

        // Step 3: before-image of the POINTER record (never the plaintext —
        // the before-image mechanism guards the Markdown vault's own
        // optimistic-concurrency rollback; it is never asked to carry
        // sensitive content). A sensitive note being overwritten always had
        // a pointer-only before-image in the first place, since no path
        // ever writes plaintext into the Markdown record.
        let before_image_hash = match &current_bytes {
            Some(bytes) => Some(persist_before_image(&self.root, note_id, operation_id, bytes)?),
            None => None,
        };

        // Step 4 (Task 6, the new step): stage the ciphertext durably BEFORE
        // the pointer rename. The staged file's content_hash is the
        // PLAINTEXT's hash; recorded on the row as `pii_content_hash`
        // alongside the pointer's own intended `content_hash`, both on the
        // still-IntentRecorded row, before either effect is checked against
        // its fault point.
        //
        // A3 review round 3: the plaintext hash is recorded on the row BEFORE
        // the staging file is written (it is a plain SHA-256, identical to
        // what `stage` returns, which is checked below). The opposite order
        // left a window in which a crash produced a staged ciphertext under a
        // row with no `pii_content_hash`, which recovery cannot recognize as
        // sensitive and so never discards. Now every staged file belongs to a
        // row that recovery treats as sensitive.
        let pii_hash = content_hash(plaintext);
        let new_version = Version(actual_version.0 + 1);
        let buffer = serialize_record(pointer_content, new_version);
        let new_hash = content_hash(&buffer);
        let mut fields = match &before_image_hash {
            Some(before_hash) => {
                AdvanceFields::content_with_before_image(new_version.0, new_hash.clone(), before_hash.clone())
            }
            None => AdvanceFields::content(new_version.0, new_hash.clone()),
        };
        fields = AdvanceFields::merge_pii_content_hash(fields, pii_hash.clone());

        self.journal
            .lock()
            .unwrap()
            .record_fields(operation_id, WriteState::IntentRecorded, fields.clone())
            .map_err(journal_err)?;
        let staged = pii_store.stage(note_id.as_str(), plaintext, operation_id).map_err(pii_store_err)?;
        if staged.content_hash != pii_hash {
            return Err(StorageError::Inconsistent { note_id: note_id.as_str().to_string() });
        }
        injector.check(FaultPoint::AfterPiiStagedBeforeCommit).map_err(interrupted)?;

        // Step 5: commit the staged ciphertext onto the live `.pii` path.
        pii_store.commit_staged(note_id.as_str(), operation_id).map_err(pii_store_err)?;
        injector.check(FaultPoint::AfterPiiCommittedBeforePointerRename).map_err(interrupted)?;

        // Step 6: the Markdown pointer rename, exactly as ordinary content's
        // rename works.
        atomic_write(&record_path, &buffer).map_err(|AtomicWriteError::Io(e)| StorageError::Io(e))?;
        injector.check(FaultPoint::AfterRenameBeforeContentPersisted).map_err(interrupted)?;
        self.journal.lock().unwrap().advance(operation_id, WriteState::ContentPersisted, fields).map_err(journal_err)?;
        injector.check(FaultPoint::AfterRenameBeforeJournalCommitted).map_err(interrupted)?;

        // Step 7: verify the pointer bytes actually on disk, exactly as the
        // ordinary path does. A mismatch here rolls back BOTH the pointer
        // (restoring its Markdown before-image, as always) AND the `.pii`
        // ciphertext (restoring PiiStore's own before-image, if
        // `commit_staged` already overwrote the live file for this
        // operation) — never leaving the live `.pii` file at the NEW
        // version while the pointer rolls back to the OLD one. On a first
        // write (no PII before-image exists), the live `.pii` file this
        // operation just committed is instead removed outright, since there
        // is no prior version to restore it to.
        let written_bytes = std::fs::read(&record_path).map_err(StorageError::Io)?;
        if content_hash(&written_bytes) != new_hash {
            if let Some(before_hash) = &before_image_hash {
                if let Ok(before_bytes) = read_before_image(&self.root, note_id, operation_id) {
                    if content_hash(&before_bytes) == *before_hash
                        && atomic_write(&record_path, &before_bytes).is_ok()
                    {
                        let rolled_back = std::fs::read(&record_path).map_err(StorageError::Io)?;
                        if content_hash(&rolled_back) == *before_hash {
                            // PII first: if its fail-closed rollback refuses,
                            // nothing has been deleted yet and the row stays
                            // at ContentPersisted for recovery.
                            roll_back_pii(&self.journal, pii_store, note_id, operation_id)?;
                            delete_before_image(&self.root, note_id, operation_id)?;
                            self.journal.lock().unwrap().advance(operation_id, WriteState::Failed, AdvanceFields::none()).map_err(journal_err)?;
                            return Err(StorageError::Inconsistent {
                                note_id: note_id.as_str().to_string(),
                            });
                        }
                    }
                }
            }
            return Err(StorageError::Inconsistent {
                note_id: note_id.as_str().to_string(),
            });
        }

        // Step 8: the linearization point.
        self.journal.lock().unwrap().advance(operation_id, WriteState::JournalCommitted, AdvanceFields::none()).map_err(journal_err)?;
        injector.check(FaultPoint::AfterJournalCommittedBeforeGit).map_err(interrupted)?;

        // Step 9: JournalCommitted -> GitCommitted, committing the pointer
        // record (never the plaintext — this is the whole point of the
        // pointer scheme: git history, which this project cannot selectively
        // purge, never sees sensitive content). Passing `Some(pii_store)`
        // means the PII before-image (if this was an update) is deleted on
        // the same schedule as the Markdown before-image, once this
        // operation reaches GitCommitted.
        self.commit_to_git(note_id, &record_path, operation_id, new_version, ScanDepth::Bounded(500), None, Some(pii_store))?;
        injector.check(FaultPoint::AfterGitCommittedBeforeIndex).map_err(interrupted)?;

        // Step 10: the index step.
        self.mark_index_current(note_id, operation_id, new_version)?;

        Ok(new_version)
    }

    /// Step 0 of `write_record_unchecked_with_injector`: `row` is the existing
    /// journal row for the caller's `operation_id`. Reports the outcome that
    /// operation already reached, finishing any remaining pipeline step that
    /// can be finished safely, and never writes the record again (C1, "Retry
    /// dello stesso operation_id in ciascuno stato").
    ///
    /// The caller has already checked, via `ensure_retry_matches`, that the
    /// retry asks for the same note, expected version and content as `row`;
    /// a call that does not is `OperationIdReused` and never reaches here.
    ///
    /// - `Failed`: `OperationFailed`. The caller must use a new id.
    /// - `IndexCurrent`: `Ok` with the version it committed.
    /// - `IntentRecorded`, `ContentPersisted`, `Recovering`: `OperationInProgress`.
    ///   In a live process such a row is either still being written by another
    ///   call or was abandoned mid-flight, and the two cannot be told apart
    ///   from here, so it is neither resumed nor failed. Recovery on the next
    ///   `Vault::open` resolves it.
    /// - `JournalCommitted` (for example after `git_pending`): resumes the git
    ///   step and then the index step. The commit carries this operation's OWN
    ///   bytes (the record if it still hashes to the row's `content_hash`,
    ///   otherwise a successor's before-image), exactly as recovery's row 3
    ///   does. If they cannot be found, or git is still unavailable, the
    ///   result is `GitPending` and the row keeps resting.
    /// - `GitCommitted`: resumes the index step.
    ///
    /// The last two run under the note's per-note lock, after re-reading the
    /// row, so a concurrent call finishing the same operation is observed
    /// rather than repeated, and `commit_to_git` looks for an existing commit
    /// first (unbounded, as on the recovery path).
    fn resume_existing_operation(&self, row: crate::journal::OperationRow, pii_store: Option<&PiiStore>) -> Result<Version, StorageError> {
        let operation_id = row.operation_id;
        match row.state {
            WriteState::Failed => return Err(StorageError::OperationFailed { operation_id }),
            WriteState::IndexCurrent => return committed_version(&row),
            WriteState::IntentRecorded | WriteState::ContentPersisted | WriteState::Recovering => {
                return Err(StorageError::OperationInProgress { operation_id });
            }
            WriteState::JournalCommitted | WriteState::GitCommitted => {}
        }

        let note_id = NoteId::parse(&row.note_id).map_err(|_| StorageError::Inconsistent {
            note_id: row.note_id.clone(),
        })?;
        let record_path = self.record_path_checked(&note_id)?;
        let note_lock = self.note_lock(&note_id);
        let _guard = note_lock.lock().unwrap();

        let row = self
            .journal
            .lock()
            .unwrap()
            .get(operation_id)
            .map_err(journal_err)?
            .ok_or_else(|| StorageError::Inconsistent { note_id: note_id.as_str().to_string() })?;
        let version = committed_version(&row)?;
        match row.state {
            WriteState::IndexCurrent => return Ok(version),
            WriteState::JournalCommitted => {
                self.commit_to_git(&note_id, &record_path, operation_id, version, ScanDepth::Unbounded, Some(&row), pii_store)?;
            }
            // A3 review round 3, item 1(c): a crash between the GitCommitted
            // advance and the before-image deletes leaves the row here, so
            // the deletes are (idempotently) repeated before the index step.
            // A sensitive row with no store to delete with is left as it is
            // for the next `Vault::open`, which resolves it with one.
            WriteState::GitCommitted => {
                if row.pii_content_hash.is_some() && pii_store.is_none() {
                    return Err(StorageError::OperationInProgress { operation_id });
                }
                delete_before_image(&self.root, &note_id, operation_id)?;
                if let Some(store) = pii_store {
                    store.delete_before_image(note_id.as_str(), operation_id).map_err(pii_store_err)?;
                }
            }
            // Nothing moves a JournalCommitted/GitCommitted row anywhere else
            // on a live vault (Recovering is only written by `reconcile`,
            // before any Vault exists).
            WriteState::Failed | WriteState::IntentRecorded | WriteState::ContentPersisted | WriteState::Recovering => {
                return Err(StorageError::OperationInProgress { operation_id });
            }
        }
        self.mark_index_current(&note_id, operation_id, version)?;
        Ok(version)
    }

    /// JournalCommitted -> GitCommitted for `operation_id`, then deletes its
    /// before-image (per spec, only once git_committed is reached). The caller
    /// must hold `note_id`'s per-note lock.
    ///
    /// `find_commit_for_operation` is consulted FIRST (Tier 1 is the
    /// journal's own `git_commit_oid`, Tier 2 a history scan to `scan`'s
    /// depth), so an operation that already has a commit never gets a second
    /// one. Only on a miss is a commit made: of the record on disk when
    /// `own_content_of` is `None` (the live write path, which has just written
    /// and verified it), or of that row's own bytes as recovered by
    /// `recovery::own_content` (the retry path, where a successor may have
    /// overwritten the record since). If those bytes cannot be recovered,
    /// returns `GitPending` without committing anything.
    ///
    /// The whole find-then-commit-then-record sequence is serialized by
    /// `git_lock` across ALL notes (Task 5 fix round, finding I1): two threads
    /// writing two DIFFERENT notes hold different per-note locks and would
    /// otherwise race on the same branch ref. Acquired strictly after the
    /// per-note lock, never before it; see `git_lock`'s doc comment. The Oid
    /// is recorded on the SAME journal write that advances to GitCommitted,
    /// also under `git_lock`, so there is no window where the state says
    /// GitCommitted but `git_commit_oid` is unset, and no second thread can
    /// re-run the lookup between the commit and that write.
    ///
    /// Earlier git-pending operations on the same note are committed FIRST,
    /// in ascending `new_version` order, under the same `git_lock` (final
    /// whole-branch review, finding I1). An earlier operation resting at
    /// `JournalCommitted` may have its own bytes only in a later operation's
    /// before-image (the later write overwrote the record), and this method
    /// deletes `operation_id`'s before-image once it commits. Committing the
    /// earlier ones first, each from its own recovered bytes and each then
    /// taken through the index step, means no before-image is deleted while
    /// an earlier operation still needs it. It also makes commits for one
    /// note land in version order, so `HEAD` never moves back to older
    /// content. An earlier operation whose own bytes cannot be recovered is
    /// left resting (no commit is ever made from another operation's
    /// bytes); a git failure on any of them is `GitPending` for this call
    /// too, leaving every row at `JournalCommitted`.
    /// `pii_store` (Task 6): when given, this operation's PII before-image
    /// (and any earlier git-pending operation's, on the same note) is also
    /// deleted once each reaches `GitCommitted`, on the SAME schedule as the
    /// Markdown before-image, per `PiiStore::commit_staged`'s contract
    /// ("delete the before-image only once this operation has reached a
    /// point of no return"). When `None`, no sensitive row (this one or an
    /// earlier one) is committed at all: the call returns `GitPending` and
    /// leaves every row at `JournalCommitted` rather than advance a row whose
    /// cleanup it cannot do (A3 review round 3, item 1(b)).
    #[allow(clippy::too_many_arguments)]
    fn commit_to_git(
        &self,
        note_id: &NoteId,
        record_path: &Path,
        operation_id: Uuid,
        version: Version,
        scan: ScanDepth,
        own_content_of: Option<&crate::journal::OperationRow>,
        pii_store: Option<&PiiStore>,
    ) -> Result<(), StorageError> {
        {
            let _git_guard = self.git_lock.lock().unwrap();

            let earlier = self
                .journal
                .lock()
                .unwrap()
                .git_pending_operations_before(note_id.as_str(), version.0)
                .map_err(journal_err)?;
            // A3 review round 3, item 1(b): a SENSITIVE row (this one or an
            // earlier one being caught up) must have its PII before-image
            // deleted when it reaches GitCommitted. Without a `PiiStore`
            // that cannot happen, so nothing is committed: every row stays
            // resting at JournalCommitted (GitPending, exactly as for a git
            // failure on any of them) until a caller or recovery pass that
            // has the store finishes them.
            if pii_store.is_none() {
                let own_is_sensitive = self
                    .journal
                    .lock()
                    .unwrap()
                    .get(operation_id)
                    .map_err(journal_err)?
                    .is_some_and(|row| row.pii_content_hash.is_some());
                let earlier_is_sensitive = earlier
                    .iter()
                    .any(|row| row.operation_id != operation_id && row.pii_content_hash.is_some());
                if own_is_sensitive || earlier_is_sensitive {
                    return Err(StorageError::GitPending { note_id: note_id.as_str().to_string() });
                }
            }
            for row in earlier.iter().filter(|row| row.operation_id != operation_id) {
                // Unbounded: an earlier operation is not just-written, so a
                // bounded miss could not prove it has no commit yet.
                if self.commit_one_locked(note_id, record_path, row.operation_id, ScanDepth::Unbounded, Some(row))? {
                    delete_before_image(&self.root, note_id, row.operation_id)?;
                    if let Some(store) = pii_store {
                        store.delete_before_image(note_id.as_str(), row.operation_id).map_err(pii_store_err)?;
                    }
                    self.mark_index_current(note_id, row.operation_id, committed_version(row)?)?;
                }
            }

            if !self.commit_one_locked(note_id, record_path, operation_id, scan, own_content_of)? {
                return Err(StorageError::GitPending { note_id: note_id.as_str().to_string() });
            }
            // _git_guard drops here, releasing git_lock.
        }
        delete_before_image(&self.root, note_id, operation_id)?;
        if let Some(store) = pii_store {
            store.delete_before_image(note_id.as_str(), operation_id).map_err(pii_store_err)?;
        }
        Ok(())
    }

    /// One operation's JournalCommitted -> GitCommitted step. The caller must
    /// hold the note's per-note lock and `git_lock`. Returns `Ok(false)`,
    /// committing nothing, when `own_content_of` is given and that row's own
    /// bytes can no longer be recovered; `Ok(true)` once the row is
    /// `GitCommitted` with its Oid recorded on the same journal write.
    fn commit_one_locked(
        &self,
        note_id: &NoteId,
        record_path: &Path,
        operation_id: Uuid,
        scan: ScanDepth,
        own_content_of: Option<&crate::journal::OperationRow>,
    ) -> Result<bool, StorageError> {
        let existing_oid = find_commit_for_operation(&self.journal.lock().unwrap(), &self.root, operation_id, scan)
            .map_err(|e| git_err(note_id, e))?;

        let git_oid = match (existing_oid, own_content_of) {
            (Some(oid), _) => oid,
            (None, None) => commit_operation(&self.root, operation_id, note_id.as_str(), record_path)
                .map_err(|e| git_err(note_id, e))?,
            (None, Some(row)) => {
                let contents = own_content(&self.root, &self.journal.lock().unwrap(), row, note_id, record_path)
                    .map_err(|e| reconcile_err(note_id, e))?;
                let Some(contents) = contents else {
                    return Ok(false);
                };
                commit_operation_contents(&self.root, operation_id, note_id.as_str(), record_path, &contents)
                    .map_err(|e| git_err(note_id, e))?
            }
        };
        self.journal
            .lock()
            .unwrap()
            .advance(operation_id, WriteState::GitCommitted, AdvanceFields::git_oid(git_oid))
            .map_err(journal_err)?;
        Ok(true)
    }

    /// Final whole-branch review, I2: refuses a non-tombstone write to a note
    /// id the deletion log (at `pii_store.deletion_log_path()`) records as
    /// deleted, in any state. Must be called under `note_id`'s per-note lock,
    /// after the version check and before anything is touched, so it cannot
    /// race a concurrent tombstone on the same note: that tombstone either
    /// committed first (and this call's version check already failed) or has
    /// already logged its deletion (and this check sees it). On refusal, or
    /// if the log cannot be consulted (fail closed, never "not deleted"),
    /// this call's own journal row is advanced to `Failed`, as for a
    /// `Conflict`.
    fn refuse_if_deleted(&self, pii_store: &PiiStore, note_id: &NoteId, operation_id: Uuid) -> Result<(), StorageError> {
        let deleted = DeletionLog::open(&pii_store.deletion_log_path())
            .and_then(|log| log.is_deleted(note_id.as_str()))
            .map_err(|e| StorageError::DeletionLog(e.to_string()));
        let refusal = match deleted {
            Ok(false) => return Ok(()),
            Ok(true) => StorageError::NoteDeleted { note_id: note_id.as_str().to_string() },
            Err(e) => e,
        };
        self.journal.lock().unwrap().advance(operation_id, WriteState::Failed, AdvanceFields::none()).map_err(journal_err)?;
        Err(refusal)
    }

    /// The index step, GitCommitted -> IndexCurrent (Task 6). `mark_current`
    /// is the indexer's hook (a no-op for `NullIndexer`). The journal advance
    /// records only that this pipeline step completed. It is NOT an
    /// index-freshness claim; that is `index_state_for`'s job.
    fn mark_index_current(&self, note_id: &NoteId, operation_id: Uuid, version: Version) -> Result<(), StorageError> {
        self.indexer.mark_current(note_id.as_str(), version.0);
        self.journal
            .lock()
            .unwrap()
            .advance(operation_id, WriteState::IndexCurrent, AdvanceFields::none())
            .map_err(journal_err)
    }

    /// Test-only introspection: the journal row for `operation_id`, or
    /// `None` if no such row exists. Lets tests assert on journal state
    /// (e.g. `WriteState`, `new_version`) without widening any production
    /// visibility — `Journal`/`OperationRow` stay crate-private.
    #[cfg(test)]
    pub(crate) fn journal_row_for_test(&self, operation_id: Uuid) -> Option<crate::journal::OperationRow> {
        self.journal.lock().unwrap().get(operation_id).unwrap()
    }

    /// Test-only introspection: inserts an intent row directly against this
    /// vault's journal, so tests can simulate a crash frozen at
    /// `IntentRecorded` without going through `write_record_unchecked` at
    /// all.
    #[cfg(test)]
    pub(crate) fn insert_intent_for_test(&self, operation_id: Uuid, note_id: &str, expected_version: u64) {
        self.journal.lock().unwrap().insert_intent(operation_id, note_id, expected_version).unwrap()
    }

    /// Test-only introspection: this vault's root directory.
    #[cfg(test)]
    pub(crate) fn root_for_test(&self) -> &Path {
        &self.root
    }

    /// Test-only introspection: a locked handle onto this vault's journal,
    /// so a test can call crate-private `Journal`-taking functions (like
    /// `find_commit_for_operation`) directly against the exact journal
    /// `write_record_unchecked` drove, without widening any production
    /// visibility.
    #[cfg(test)]
    pub(crate) fn journal_for_test(&self) -> std::sync::MutexGuard<'_, Journal> {
        self.journal.lock().unwrap()
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
        record_path_checked(&self.root, note_id)
    }
}

/// The free-function body of [`Vault::record_path_checked`], shared with
/// `recovery::reconcile`, which runs before any `Vault` value exists but
/// must touch record files under exactly the same containment check.
pub(crate) fn record_path_checked(root: &Path, note_id: &NoteId) -> Result<PathBuf, StorageError> {
    let candidate = root.join(format!("{}.record", note_id.as_str()));
    let canonical_root = std::fs::canonicalize(root).map_err(StorageError::Io)?;
    ensure_resolves_inside_root(&canonical_root, &candidate, note_id)?;
    Ok(candidate)
}

/// Test-only access to the private on-disk record serializer, so tests in
/// other modules (`recovery`) can seed record files byte-identical to what
/// `write_record_unchecked` would have written.
#[cfg(test)]
pub(crate) fn serialize_record_for_test(content: &str, version: Version) -> Vec<u8> {
    serialize_record(content, version)
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

/// Dotfile directory, inside the vault root, that holds before-images. It is
/// a sibling of the `.record` files, so the containment check applies to it
/// exactly as it does to records.
const BEFORE_IMAGE_DIR: &str = ".before-images";

/// Hex-encoded SHA-256 of `bytes`.
pub(crate) fn content_hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// `<root>/.before-images/<note_id>-<operation_id>.record`. Pure path
/// construction; nothing is created or checked here.
pub(crate) fn before_image_path(root: &Path, note_id: &NoteId, operation_id: Uuid) -> PathBuf {
    root.join(BEFORE_IMAGE_DIR)
        .join(format!("{}-{}.record", note_id.as_str(), operation_id))
}

/// Resolves the before-image path and applies the same containment check as
/// `record_path_checked`. If the `.before-images` directory does not exist,
/// nothing can be reached through it, so the bare path is returned and the
/// caller's I/O reports `NotFound` (or a no-op delete).
fn before_image_path_checked(
    root: &Path,
    note_id: &NoteId,
    operation_id: Uuid,
) -> Result<PathBuf, StorageError> {
    let candidate = before_image_path(root, note_id, operation_id);
    let dir = root.join(BEFORE_IMAGE_DIR);
    if let Err(e) = dir.symlink_metadata() {
        if e.kind() == std::io::ErrorKind::NotFound {
            return Ok(candidate);
        }
    }
    let canonical_root = std::fs::canonicalize(root).map_err(StorageError::Io)?;
    ensure_resolves_inside_root(&canonical_root, &candidate, note_id)?;
    Ok(candidate)
}

/// Writes `current_record_bytes` as the before-image for `operation_id` via
/// the crate-private `atomic_write`, and returns the content hash of the
/// bytes written.
pub(crate) fn persist_before_image(
    root: &Path,
    note_id: &NoteId,
    operation_id: Uuid,
    current_record_bytes: &[u8],
) -> Result<String, StorageError> {
    std::fs::create_dir_all(root.join(BEFORE_IMAGE_DIR)).map_err(StorageError::Io)?;
    let path = before_image_path_checked(root, note_id, operation_id)?;
    atomic_write(&path, current_record_bytes).map_err(|AtomicWriteError::Io(e)| StorageError::Io(e))?;
    Ok(content_hash(current_record_bytes))
}

/// Removes the before-image for `operation_id`. Idempotent: a missing file
/// is not an error.
pub(crate) fn delete_before_image(
    root: &Path,
    note_id: &NoteId,
    operation_id: Uuid,
) -> Result<(), StorageError> {
    let path = before_image_path_checked(root, note_id, operation_id)?;
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(StorageError::Io(e)),
    }
}

/// Reads back the before-image for `operation_id`.
pub(crate) fn read_before_image(
    root: &Path,
    note_id: &NoteId,
    operation_id: Uuid,
) -> Result<Vec<u8>, StorageError> {
    let path = before_image_path_checked(root, note_id, operation_id)?;
    std::fs::read(&path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            StorageError::NotFound {
                note_id: note_id.as_str().to_string(),
            }
        } else {
            StorageError::Io(e)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_test_dir() -> PathBuf {
        // A per-process counter on top of the timestamp: parallel tests can read
        // the same clock value, which made two tests share one directory.
        static UNIQUE_DIR_COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "superbrain-vault-test-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            UNIQUE_DIR_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// `Vault::open` with a fresh root and a fresh journal path, for tests
    /// that don't care about sharing either across calls.
    /// `Vault::open` with a fresh root and a fresh journal path.
    ///
    /// Task 5 note: the root is ALSO a fresh git repository (with an
    /// initial empty commit, so `HEAD` exists), because `write_record_unchecked`
    /// now extends the chain past `JournalCommitted` to `GitCommitted` —
    /// every pre-Task-5 test calling this helper expects a write to fully
    /// succeed (readable, committed content), which now requires a real
    /// repo at the root. The one test that deliberately exercises the
    /// no-repository case (`write_to_a_vault_root_that_is_not_a_git_repo_returns_git_pending_not_an_error`)
    /// builds its own non-git vault instead of using this helper.
    fn open_test_vault() -> Vault {
        open_test_vault_with_git_repo().0
    }

    /// A vault whose root is ALSO a fresh git repository (with an initial
    /// empty commit, so `HEAD` exists), for tests that exercise the
    /// `JournalCommitted` -> `GitCommitted` chain extension. Returns the
    /// vault and its root path (which is also the repo path).
    fn open_test_vault_with_git_repo() -> (Vault, PathBuf) {
        open_test_vault_with_git_repo_and_default_indexer()
    }

    /// Same git-backed fixture as `open_test_vault_with_git_repo`, opened with
    /// the A2 default indexer (`NullIndexer`) via `open_with_default_indexer`.
    fn open_test_vault_with_git_repo_and_default_indexer() -> (Vault, PathBuf) {
        let root = crate::git::test_support::init_test_repo();
        let vault = Vault::open_with_default_indexer(root.clone(), &unique_test_dir().join("journal.sqlite")).unwrap();
        (vault, root)
    }

    // --- Task 6: IndexState stub, NullIndexer, and the IndexCurrent chain step ---

    #[test]
    fn null_indexer_never_reports_current_for_any_note_or_version() {
        let indexer = NullIndexer;
        // Property-style check over several arbitrary note ids/versions, not just one,
        // so this cannot be satisfied by special-casing a single hard-coded input.
        for note_id in ["note-a", "note-b", "freshly-written-note"] {
            assert_eq!(indexer.state_for(note_id), IndexState::Stale, "NullIndexer must never claim Current — no indexer exists in A2");
        }
    }

    #[test]
    fn write_record_unchecked_reaches_index_current_pipeline_state_but_index_state_for_stays_stale() {
        let (vault, _repo_dir) = open_test_vault_with_git_repo_and_default_indexer();
        let id = NoteId::parse("note-a").unwrap();
        let op = Uuid::new_v4();
        vault.write_record_unchecked(&id, "content", Version(0), op).unwrap();
        let row = vault.journal_row_for_test(op).unwrap();
        assert_eq!(row.state, WriteState::IndexCurrent, "the write pipeline has nothing further to do");
        assert_eq!(vault.index_state_for(&id), IndexState::Stale, "but the index itself is honestly reported as not current — no indexer exists yet");
    }

    // --- Task 5: JournalCommitted -> GitCommitted chain extension ---

    #[test]
    fn write_record_unchecked_reaches_git_committed_when_repo_exists() {
        let (vault, repo_dir) = open_test_vault_with_git_repo();
        let id = NoteId::parse("note-a").unwrap();
        let op = Uuid::new_v4();
        vault.write_record_unchecked(&id, "content", Version(0), op).unwrap();
        let row = vault.journal_row_for_test(op).unwrap();
        // Task 6 extends the chain past GitCommitted; the Oid is still recorded.
        assert_eq!(row.state, WriteState::IndexCurrent);
        assert!(row.git_commit_oid.is_some(), "the commit's Oid must be recorded on the row itself (Tier 1 idempotency source)");
        assert!(find_commit_for_operation(&vault.journal_for_test(), &repo_dir, op, ScanDepth::Bounded(500)).unwrap().is_some());
    }

    #[test]
    fn concurrent_writes_to_different_notes_both_reach_git_committed_without_a_hard_git_error() {
        // Fix round, independent-review finding I1: two threads writing to
        // TWO DIFFERENT notes take DIFFERENT per-note locks and, before the
        // git_lock fix, could run their git-commit sequences concurrently —
        // both reading the same HEAD, both building a tree via TreeBuilder,
        // both calling repo.commit against the same branch ref. Only one
        // ref update can win; the loser's commit_operation call would fail
        // outright (libgit2's "current tip is not the first parent" /
        // GIT_EMODIFIED), surfacing as a hard StorageError::Git for a write
        // whose content was already durably JournalCommitted. With git_lock
        // serializing the find-then-commit-then-record sequence across all
        // notes, both writers must succeed cleanly.
        let (vault, repo_dir) = open_test_vault_with_git_repo();
        let vault = Arc::new(vault);
        let id_a = NoteId::parse("note-a").unwrap();
        let id_b = NoteId::parse("note-b").unwrap();
        let op_a = Uuid::new_v4();
        let op_b = Uuid::new_v4();

        let barrier = Arc::new(std::sync::Barrier::new(2));

        let vault_a = Arc::clone(&vault);
        let id_a2 = id_a.clone();
        let barrier_a = Arc::clone(&barrier);
        let thread_a = std::thread::spawn(move || {
            barrier_a.wait();
            vault_a.write_record_unchecked(&id_a2, "content-a", Version(0), op_a)
        });

        let vault_b = Arc::clone(&vault);
        let id_b2 = id_b.clone();
        let barrier_b = Arc::clone(&barrier);
        let thread_b = std::thread::spawn(move || {
            barrier_b.wait();
            vault_b.write_record_unchecked(&id_b2, "content-b", Version(0), op_b)
        });

        let result_a = thread_a.join().unwrap();
        let result_b = thread_b.join().unwrap();

        result_a.unwrap_or_else(|e| panic!("writer for note-a must succeed, got {e:?}"));
        result_b.unwrap_or_else(|e| panic!("writer for note-b must succeed, got {e:?}"));

        let row_a = vault.journal_row_for_test(op_a).unwrap();
        let row_b = vault.journal_row_for_test(op_b).unwrap();
        assert_eq!(row_a.state, WriteState::IndexCurrent, "note-a's write must reach the end of the chain, not get stuck behind a lost ref race");
        assert_eq!(row_b.state, WriteState::IndexCurrent, "note-b's write must reach the end of the chain, not get stuck behind a lost ref race");
        assert!(row_a.git_commit_oid.is_some());
        assert!(row_b.git_commit_oid.is_some());
        assert_ne!(row_a.git_commit_oid, row_b.git_commit_oid, "each note's write must have its own distinct commit");

        // Both commits must actually be reachable from HEAD: the repository
        // ends up with one commit per note, neither write's commit lost or
        // silently dropped by the ref race.
        assert!(find_commit_for_operation(&vault.journal_for_test(), &repo_dir, op_a, ScanDepth::Unbounded).unwrap().is_some());
        assert!(find_commit_for_operation(&vault.journal_for_test(), &repo_dir, op_b, ScanDepth::Unbounded).unwrap().is_some());
    }

    #[test]
    fn before_image_is_deleted_only_after_git_committed() {
        let (vault, _repo_dir) = open_test_vault_with_git_repo();
        let id = NoteId::parse("note-a").unwrap();
        vault.write_record_unchecked(&id, "v1", Version(0), Uuid::new_v4()).unwrap();
        let op2 = Uuid::new_v4();
        vault.write_record_unchecked(&id, "v2", Version(1), op2).unwrap();
        assert!(read_before_image(vault.root_for_test(), &id, op2).is_err(), "before-image for a git-committed op must be gone");
    }

    #[test]
    fn write_to_a_vault_root_that_is_not_a_git_repo_returns_git_pending_not_an_error() {
        // Deliberately NOT open_test_vault(): that helper's root is now a
        // git repository (Task 5), so this test builds its own non-git
        // vault to exercise the GitPending path specifically.
        let vault = Vault::open_with_default_indexer(unique_test_dir(), &unique_test_dir().join("journal.sqlite")).unwrap();
        let id = NoteId::parse("note-a").unwrap();
        let op = Uuid::new_v4();
        let result = vault.write_record_unchecked(&id, "content", Version(0), op);
        assert!(matches!(result, Err(StorageError::GitPending { .. })));
        // but the content IS persisted and journal-readable — git_pending is not a write failure
        let record = vault.read_note(&id).unwrap();
        assert_eq!(record.content, "content");
    }

    // --- Task 4: journal- and before-image-wired write/read path ---

    #[test]
    fn write_record_unchecked_advances_journal_to_journal_committed_on_success() {
        // Task 4 named this test for its own stopping point, JournalCommitted.
        // Task 5 extends the chain past it to GitCommitted (open_test_vault's
        // root is now a git repository), so the row this test observes is at
        // the NEW terminus — a row that reached JournalCommitted necessarily
        // passed through it on the way, which is still what this test's name
        // is pinning. The dedicated Task 5 test
        // `write_record_unchecked_reaches_git_committed_when_repo_exists`
        // covers the git_commit_oid/find_commit_for_operation specifics.
        let vault = open_test_vault();
        let id = NoteId::parse("note-a").unwrap();
        let op = Uuid::new_v4();
        vault.write_record_unchecked(&id, "content v1", Version(0), op).unwrap();
        let row = vault.journal_row_for_test(op).unwrap();
        assert_eq!(row.state, domain::WriteState::IndexCurrent);
        assert_eq!(row.new_version, Some(1));
    }

    #[test]
    fn read_note_resolves_through_the_journal_not_the_raw_file() {
        let vault = open_test_vault();
        let id = NoteId::parse("note-a").unwrap();
        vault.write_record_unchecked(&id, "committed content", Version(0), Uuid::new_v4()).unwrap();
        let record = vault.read_note(&id).unwrap();
        assert_eq!(record.content, "committed content");
        assert_eq!(record.version, Version(1));
    }

    #[test]
    fn before_image_is_persisted_and_old_content_recoverable_before_deletion() {
        // Deliberately a non-git vault: A2 deletes the before-image only
        // after GitCommitted (pinned by Task 5's own
        // `before_image_is_deleted_only_after_git_committed`, which uses a
        // git-backed vault and expects the OPPOSITE of this assertion). This
        // test stops at JournalCommitted on purpose, so op2's before-image
        // is still there to observe.
        let vault = Vault::open_with_default_indexer(unique_test_dir(), &unique_test_dir().join("journal.sqlite")).unwrap();
        let id = NoteId::parse("note-a").unwrap();
        let err = vault.write_record_unchecked(&id, "v1", Version(0), Uuid::new_v4()).unwrap_err();
        assert!(matches!(err, StorageError::GitPending { .. }));
        let op2 = Uuid::new_v4();
        let err = vault.write_record_unchecked(&id, "v2", Version(1), op2).unwrap_err();
        assert!(matches!(err, StorageError::GitPending { .. }));
        // the before-image for op2 (the v1->v2 transition) must still exist:
        // A2 deletes it only after git_committed, which this (non-git) vault never reaches.
        let before = read_before_image(vault.root_for_test(), &id, op2).unwrap();
        assert!(String::from_utf8(before).unwrap().contains("v1"));
    }

    #[test]
    fn reading_a_note_whose_only_write_never_reached_journal_committed_returns_not_found() {
        // Simulates a crash frozen at IntentRecorded by inserting the intent row
        // directly against the journal without ever calling write_record_unchecked's
        // file-writing steps — exercising the "uncommitted write is invisible" rule
        // without needing the full fault-injection harness (that lands in Task 6).
        let vault = open_test_vault();
        let id = NoteId::parse("note-a").unwrap();
        vault.insert_intent_for_test(Uuid::new_v4(), id.as_str(), 0);
        assert!(matches!(vault.read_note(&id), Err(StorageError::NotFound { .. })));
    }

    #[test]
    fn concurrent_read_during_rename_never_observes_uncommitted_bytes_against_the_wrong_row() {
        // Reproduces the exact race the architectural review identified: a writer
        // has renamed new content onto disk (step 4) but not yet advanced the
        // journal to JournalCommitted (step 6). Without the shared per-note lock,
        // a concurrent read_note would open the ALREADY-RENAMED file and hash it
        // against the PREVIOUS row's content_hash, producing a spurious mismatch.
        // With the lock shared between read_note and write_record_unchecked, the
        // reader cannot run until the writer's whole critical section — including
        // the journal.advance to JournalCommitted — has completed.
        let vault = Arc::new(open_test_vault());
        let id = NoteId::parse("note-a").unwrap();
        vault.write_record_unchecked(&id, "v1", Version(0), Uuid::new_v4()).unwrap();

        let barrier = Arc::new(std::sync::Barrier::new(2));
        let writer_vault = Arc::clone(&vault);
        let writer_id = id.clone();
        let writer_barrier = Arc::clone(&barrier);
        let writer = std::thread::spawn(move || {
            writer_barrier.wait();
            writer_vault.write_record_unchecked(&writer_id, "v2", Version(1), Uuid::new_v4())
        });

        let reader_vault = Arc::clone(&vault);
        let reader_id = id.clone();
        let reader_barrier = Arc::clone(&barrier);
        let reader = std::thread::spawn(move || {
            reader_barrier.wait();
            reader_vault.read_note(&reader_id)
        });

        writer.join().unwrap().unwrap();
        let read_result = reader.join().unwrap();
        // Whichever order the lock actually serialized them in, the reader must
        // see EITHER the fully-committed v1 OR the fully-committed v2 — never an
        // Inconsistent error from comparing renamed-but-uncommitted bytes against
        // a stale row, which is what the unlocked version of read_note could do.
        let record = read_result.expect("read must never spuriously report Inconsistent due to the writer's in-flight rename");
        assert!(record.content == "v1" || record.content == "v2");
    }

    #[test]
    fn read_note_blocks_deterministically_while_another_thread_holds_the_notes_lock() {
        // Storage Task 4 review, finding I2: the barrier-started race in
        // `concurrent_read_during_rename_never_observes_uncommitted_bytes_against_the_wrong_row`
        // above is too narrow a window to reliably prove `read_note` takes
        // the per-note lock at all — the reviewer confirmed that test still
        // passes 40/40 runs even with the lock acquisition deleted from
        // `read_note`. This test holds the lock explicitly, with no race
        // window, and asserts the reader thread is still blocked after a
        // fixed sleep: it fails deterministically if `read_note` does not
        // actually take `self.note_lock(note_id)`.
        let vault = Arc::new(open_test_vault());
        let id = NoteId::parse("note-a").unwrap();
        vault.write_record_unchecked(&id, "v1", Version(0), Uuid::new_v4()).unwrap();

        let note_lock = vault.note_lock(&id);
        let _guard = note_lock.lock().unwrap();

        // Simulate the window between the writer's rename (step 4) and its
        // JournalCommitted advance (step 6): new, not-yet-committed bytes
        // are on disk, but the lock is still held, exactly as
        // write_record_unchecked holds it across that whole span. These
        // bytes would hash-mismatch against v1's committed row if a reader
        // ever reached them, which is exactly why the lock must keep any
        // reader out until they are put back.
        let record_path = vault.root_for_test().join("note-a.record");
        let original_bytes = std::fs::read(&record_path).unwrap();
        atomic_write(&record_path, &serialize_record("v2-in-flight", Version(2))).unwrap();

        let reader_vault = Arc::clone(&vault);
        let reader_id = id.clone();
        let handle = std::thread::spawn(move || reader_vault.read_note(&reader_id));

        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(
            !handle.is_finished(),
            "read_note must still be blocked on the per-note lock while this thread holds it"
        );

        // Restore the committed bytes before releasing the lock, so the
        // reader (once unblocked) observes a consistent, fully-committed
        // record rather than the in-flight bytes this test planted.
        atomic_write(&record_path, &original_bytes).unwrap();
        drop(_guard);
        let result = handle.join().unwrap();
        assert_eq!(result.unwrap().content, "v1", "after the lock is released, the reader sees the last fully-committed record");
    }

    #[test]
    fn read_note_returns_inconsistent_on_hash_mismatch_never_a_before_image_substitution() {
        // Pins the second user architectural review's correction to issue 1: a
        // corrupted on-disk file for the committed row must surface as
        // Inconsistent, never as a before-image's content silently relabeled with
        // the committed row's version. This replaces the original (defective)
        // test, which asserted read_note returned op2's before-image ("v1")
        // paired with op2's own committed version (2) — a content/version
        // mismatch across two different operations.
        let vault = open_test_vault();
        let id = NoteId::parse("note-a").unwrap();
        vault.write_record_unchecked(&id, "v1", Version(0), Uuid::new_v4()).unwrap();
        let op2 = Uuid::new_v4();
        vault.write_record_unchecked(&id, "v2", Version(1), op2).unwrap();
        // op2's row is now committed as version 2, content "v2". Corrupt the
        // on-disk file so its hash no longer matches op2's row.
        std::fs::write(vault.root_for_test().join("note-a.record"), b"CORRUPTED-ON-DISK").unwrap();
        let result = vault.read_note(&id);
        assert!(matches!(result, Err(StorageError::Inconsistent { .. })), "a hash mismatch on the committed row must be reported, never silently repaired by a read with a different operation's before-image, got {result:?}");
    }

    #[test]
    fn persist_before_image_is_keyed_by_the_writing_operations_own_operation_id_not_a_different_ones() {
        // Retains the before-image/operation_id association coverage the first
        // review flagged — now verified directly against the persistence
        // primitive (Task 3) rather than through read_note, since read_note no
        // longer consults before-images at all (see the correction above).
        //
        // Deliberately a non-git vault: this test wants to observe op2's
        // before-image BEFORE it is deleted, and A2 deletes it only after
        // GitCommitted (Task 5) — using a git-backed vault here would have
        // op2's before-image already gone by the time this test reads it.
        let vault = Vault::open_with_default_indexer(unique_test_dir(), &unique_test_dir().join("journal.sqlite")).unwrap();
        let id = NoteId::parse("note-a").unwrap();
        let op1 = Uuid::new_v4();
        let err = vault.write_record_unchecked(&id, "v1", Version(0), op1).unwrap_err();
        assert!(matches!(err, StorageError::GitPending { .. }));
        let op2 = Uuid::new_v4();
        let err = vault.write_record_unchecked(&id, "v2", Version(1), op2).unwrap_err();
        assert!(matches!(err, StorageError::GitPending { .. }));
        // op2's before-image (persisted under op2's own operation_id) must hold
        // what op2 itself overwrote ("v1"); no before-image should exist under op1.
        let op2_before = read_before_image(vault.root_for_test(), &id, op2).unwrap();
        assert!(String::from_utf8(op2_before).unwrap().contains("v1"), "op2's before-image must hold what op2 overwrote");
        assert!(read_before_image(vault.root_for_test(), &id, op1).is_err(), "op1 never had a prior version to preserve a before-image for");
    }

    #[test]
    fn non_not_found_io_error_on_read_propagates_as_io_error_not_version_zero() {
        // If the record path happens to be a directory (not a plain missing
        // file), std::fs::read fails with PermissionDenied (on this
        // platform), not NotFound. write_record_unchecked must propagate
        // that as StorageError::Io, not silently treat it as "note doesn't
        // exist yet" (Version(0)) and proceed to write over/through it.
        let root = unique_test_dir();
        let vault = Vault::open_with_default_indexer(root.clone(), &unique_test_dir().join("journal.sqlite")).unwrap();
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
        crate::git::test_support::init_git_repo_at(&root);
        let vault = Vault::open_with_default_indexer(root.clone(), &unique_test_dir().join("journal.sqlite")).unwrap();
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
        crate::git::test_support::init_git_repo_at(&real);
        let vault = Vault::open_with_default_indexer(base.join("via"), &unique_test_dir().join("journal.sqlite")).unwrap();
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
        let vault = Vault::open_with_default_indexer(root, &unique_test_dir().join("journal.sqlite")).unwrap();
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
        let vault = Vault::open_with_default_indexer(root.clone(), &unique_test_dir().join("journal.sqlite")).unwrap();
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
        let vault = Vault::open_with_default_indexer(root.clone(), &unique_test_dir().join("journal.sqlite")).unwrap();
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
        let vault = Vault::open_with_default_indexer(root.clone(), &unique_test_dir().join("journal.sqlite")).unwrap();
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
        //
        // A2 note: `read_note` now resolves through the journal, so the
        // record must actually be written via `write_record_unchecked`
        // first (establishing a committed row) rather than placed directly
        // on disk as A1's version of this test did — a raw `fs::write` with
        // no journal row is now correctly invisible to readers regardless
        // of containment. The symlink is then swapped in afterward, still
        // pointing at a copy of the exact bytes the committed row expects,
        // so this test still isolates the containment check from the
        // journal-resolution check.
        let root = unique_test_dir();
        crate::git::test_support::init_git_repo_at(&root);
        let vault = Vault::open_with_default_indexer(root.clone(), &unique_test_dir().join("journal.sqlite")).unwrap();
        let id = NoteId::parse("note-a").unwrap();
        vault.write_record_unchecked(&id, "inside", Version(0), Uuid::new_v4()).unwrap();
        std::fs::create_dir(root.join("store")).unwrap();
        let target = root.join("store").join("real.record");
        std::fs::rename(root.join("note-a.record"), &target).unwrap();
        symlink_file(&target, &root.join("note-a.record"));
        let record = vault.read_note(&id).unwrap();
        assert_eq!(record.content, "inside");
        assert_eq!(record.version, Version(1));
    }

    #[test]
    fn write_then_read_returns_same_content_and_version_one() {
        let vault = open_test_vault();
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
        let vault = open_test_vault();
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
        let vault = open_test_vault();
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
        let vault = open_test_vault();
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
        let vault = Arc::new(open_test_vault());
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
        let vault = open_test_vault();
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

    #[test]
    fn before_image_persists_and_reads_back_identical_bytes() {
        let root = unique_test_dir();
        let id = NoteId::parse("note-a").unwrap();
        let op = Uuid::new_v4();
        let bytes = b"1\n---\noriginal content";
        let hash = persist_before_image(&root, &id, op, bytes).unwrap();
        assert_eq!(hash, content_hash(bytes));
        let read_back = read_before_image(&root, &id, op).unwrap();
        assert_eq!(read_back, bytes);
    }

    #[test]
    fn before_image_lives_inside_vault_root_under_dotfile_dir() {
        let root = unique_test_dir();
        let id = NoteId::parse("note-a").unwrap();
        let op = Uuid::new_v4();
        persist_before_image(&root, &id, op, b"content").unwrap();
        let path = before_image_path(&root, &id, op);
        assert!(path.starts_with(&root));
        assert!(path.is_file());
    }

    #[test]
    fn delete_before_image_twice_is_not_an_error() {
        let root = unique_test_dir();
        let id = NoteId::parse("note-a").unwrap();
        let op = Uuid::new_v4();
        persist_before_image(&root, &id, op, b"content").unwrap();
        delete_before_image(&root, &id, op).unwrap();
        delete_before_image(&root, &id, op).unwrap(); // second call: no error
        assert!(read_before_image(&root, &id, op).is_err());
    }

    #[test]
    fn content_hash_is_stable_and_detects_any_byte_change() {
        assert_eq!(content_hash(b"abc"), content_hash(b"abc"));
        assert_ne!(content_hash(b"abc"), content_hash(b"abd"));
    }

    // --- Task 8: crash injection against the C1 fault matrix, end to end ---
    //
    // Each test drives the REAL write path, interrupts it with
    // `FaultInjector::trigger_at` (or stops it with `pause_at`), then
    // simulates a restart by dropping the `Vault` and calling `Vault::open`
    // again on the same root and journal path. Row 2's valid-hash
    // roll-forward (first write and overwrite) is already pinned end to end
    // by Task 7's fix-round tests in `recovery.rs`
    // (`crash_at_after_rename_on_a_first_write_...` and
    // `crash_at_after_rename_over_a_prior_version_...`), so it is not repeated
    // here.

    use crate::git::test_support::{commit_unrelated_change_for_test, init_git_repo_at};

    /// A fresh vault root (NOT a git repository) and a journal path in a
    /// separate fresh directory.
    fn fresh_root_and_journal_path() -> (PathBuf, PathBuf) {
        (unique_test_dir(), unique_test_dir().join("journal.sqlite"))
    }

    /// Git-initializes `root` in place and returns it as the repository path.
    fn init_test_repo_at(root: &Path) -> PathBuf {
        init_git_repo_at(root);
        root.to_path_buf()
    }

    /// How many commits reachable from `HEAD` carry `op`'s marker. The
    /// "exactly once" counter on the git side.
    fn commits_for_operation(repo_dir: &Path, op: Uuid) -> usize {
        let repo = git2::Repository::open(repo_dir).unwrap();
        let mut walk = repo.revwalk().unwrap();
        walk.push_head().unwrap();
        let marker = op.to_string();
        walk.filter(|oid| {
            let commit = repo.find_commit(*oid.as_ref().unwrap()).unwrap();
            commit.message().is_ok_and(|m| m.contains(&marker))
        })
        .count()
    }

    /// The bytes of `<note>.record` in commit `oid`'s tree.
    fn committed_record_bytes(repo_dir: &Path, oid: &str, note: &str) -> Vec<u8> {
        let repo = git2::Repository::open(repo_dir).unwrap();
        let commit = repo.find_commit(git2::Oid::from_str(oid).unwrap()).unwrap();
        let tree = commit.tree().unwrap();
        let entry = tree.get_name(&format!("{note}.record")).unwrap();
        let bytes = repo.find_blob(entry.id()).unwrap().content().to_vec();
        bytes
    }

    /// Every `(note_id, version)` a `CountingIndexer` was told is current.
    type IndexCalls = Arc<Mutex<Vec<(String, u64)>>>;

    /// An indexer that records every `mark_current` call: the "exactly once"
    /// counter on the index side.
    struct CountingIndexer {
        calls: IndexCalls,
    }

    impl Indexer for CountingIndexer {
        fn mark_current(&self, note_id: &str, version: u64) {
            self.calls.lock().unwrap().push((note_id.to_string(), version));
        }

        fn state_for(&self, _note_id: &str) -> IndexState {
            IndexState::Stale
        }
    }

    fn counting_indexer() -> (Box<dyn Indexer>, IndexCalls) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        (Box::new(CountingIndexer { calls: Arc::clone(&calls) }), calls)
    }

    // Matrix row 1: "Dopo intent_recorded, prima della scrittura".

    #[test]
    fn crash_after_intent_recorded_before_any_write_results_in_failed_no_file_touched() {
        let (root, journal_path) = fresh_root_and_journal_path();
        // Deviation from the brief: the root is a git repository, so the
        // retry below can reach `Ok` (on a non-git root it is GitPending).
        init_test_repo_at(&root);
        let vault = Vault::open_with_default_indexer(root.clone(), &journal_path).unwrap();
        let id = NoteId::parse("note-a").unwrap();
        let op = Uuid::new_v4();
        let injector = FaultInjector::trigger_at(FaultPoint::AfterIntentRecorded);
        let result = vault.write_record_unchecked_with_injector(&id, "content", Version(0), op, &injector);
        assert!(matches!(result, Err(StorageError::Interrupted(FaultPoint::AfterIntentRecorded))));
        assert!(!root.join("note-a.record").exists(), "no file touched");
        drop(vault);
        let vault2 = Vault::open_with_default_indexer(root.clone(), &journal_path).unwrap();
        assert_eq!(vault2.journal_row_for_test(op).unwrap().state, WriteState::Failed);
        assert!(!root.join("note-a.record").exists(), "recovery did not touch a file either");
        assert!(matches!(vault2.read_note(&id), Err(StorageError::NotFound { .. })));
        // caller can retry only with a NEW operation_id, per C1:
        let retry = vault2.write_record_unchecked(&id, "content", Version(0), Uuid::new_v4()).unwrap();
        assert_eq!(retry, Version(1));
    }

    // Matrix row 2, invalid-hash branch: "Dopo il rename, prima di
    // journal_committed ... altrimenti ripristino esatto della before-image".

    #[test]
    fn crash_after_rename_with_an_invalid_hash_restores_the_exact_before_image_on_reopen() {
        let (root, journal_path) = fresh_root_and_journal_path();
        init_test_repo_at(&root);
        let id = NoteId::parse("note-a").unwrap();
        let op1 = Uuid::new_v4();
        let op2 = Uuid::new_v4();
        {
            let vault = Vault::open_with_default_indexer(root.clone(), &journal_path).unwrap();
            vault.write_record_unchecked(&id, "v1", Version(0), op1).unwrap();
            let injector = FaultInjector::trigger_at(FaultPoint::AfterRenameBeforeJournalCommitted);
            let result = vault.write_record_unchecked_with_injector(&id, "v2", Version(1), op2, &injector);
            assert!(matches!(result, Err(StorageError::Interrupted(FaultPoint::AfterRenameBeforeJournalCommitted))));
            assert_eq!(vault.journal_row_for_test(op2).unwrap().state, WriteState::ContentPersisted);
        }
        // The new version is damaged on disk before the restart.
        std::fs::write(root.join("note-a.record"), b"TORN WRITE").unwrap();

        let vault = Vault::open_with_default_indexer(root.clone(), &journal_path).unwrap();
        let row2 = vault.journal_row_for_test(op2).unwrap();
        assert_eq!(row2.state, WriteState::Failed, "rolled back, not rolled forward");
        assert_eq!(row2.previous_state, Some(WriteState::ContentPersisted));
        assert_eq!(
            std::fs::read(root.join("note-a.record")).unwrap(),
            serialize_record("v1", Version(1)),
            "exact restore: same bytes, so same hash and same version"
        );
        assert!(!before_image_path(&root, &id, op2).exists(), "the before-image is consumed by the restore");
        assert_eq!(vault.read_note(&id).unwrap(), VaultRecord { content: "v1".into(), version: Version(1) });
        assert_eq!(commits_for_operation(&root, op2), 0, "a rolled-back operation is never committed to git");
    }

    // Final whole-branch review, finding I1: an earlier git-pending operation
    // must not be stranded when a later write to the same note commits and
    // deletes the before-image that held the earlier operation's bytes.

    #[test]
    fn a_git_pending_operation_is_committed_first_when_a_later_write_to_the_same_note_commits() {
        let (root, journal_path) = fresh_root_and_journal_path(); // not a git repository yet
        let id = NoteId::parse("note-a").unwrap();
        let (op_a, op_b) = (Uuid::new_v4(), Uuid::new_v4());
        {
            let vault = Vault::open_with_default_indexer(root.clone(), &journal_path).unwrap();
            assert!(matches!(vault.write_record_unchecked(&id, "content A", Version(0), op_a), Err(StorageError::GitPending { .. })));
            // The ordinary workflow: git is initialized while the vault is in use.
            init_test_repo_at(&root);
            assert_eq!(vault.write_record_unchecked(&id, "content B", Version(1), op_b).unwrap(), Version(2));

            let row_a = vault.journal_row_for_test(op_a).unwrap();
            assert_eq!(row_a.state, WriteState::IndexCurrent, "A is committed before B, not left behind");
            assert_eq!(
                committed_record_bytes(&root, row_a.git_commit_oid.as_deref().unwrap(), "note-a"),
                serialize_record("content A", Version(1)),
                "A's commit carries A's own bytes, recovered from B's before-image before it was deleted"
            );
            let row_b = vault.journal_row_for_test(op_b).unwrap();
            assert_eq!(row_b.state, WriteState::IndexCurrent);
            // Commits land in version order, so HEAD ends at the newest content.
            let repo = git2::Repository::open(&root).unwrap();
            let head = repo.head().unwrap().peel_to_commit().unwrap();
            assert_eq!(head.id().to_string(), row_b.git_commit_oid.clone().unwrap());
            assert_eq!(head.parent_id(0).unwrap().to_string(), row_a.git_commit_oid.clone().unwrap());
            assert!(!before_image_path(&root, &id, op_b).exists());
        }
        for _ in 0..2 {
            let vault = Vault::open_with_default_indexer(root.clone(), &journal_path).unwrap();
            let row_a = vault.journal_row_for_test(op_a).unwrap();
            assert_eq!(row_a.state, WriteState::IndexCurrent, "not stuck at JournalCommitted across reopens");
            assert!(row_a.git_commit_oid.is_some());
            assert_eq!(commits_for_operation(&root, op_a), 1);
            assert_eq!(commits_for_operation(&root, op_b), 1);
            assert_eq!(vault.read_note(&id).unwrap(), VaultRecord { content: "content B".into(), version: Version(2) });
        }
    }

    #[test]
    fn a_retry_of_the_newer_git_pending_operation_commits_the_older_one_first_so_head_never_moves_backward() {
        // Closes Task 8's deferred I3 as a side effect: the caller retries
        // the NEWER git-pending operation before the older one. The older one
        // is committed first, then the newer one, so HEAD ends at the newest
        // content; the later retry of the older one creates nothing new.
        let (root, journal_path) = fresh_root_and_journal_path();
        let id = NoteId::parse("note-a").unwrap();
        let (op_a, op_b) = (Uuid::new_v4(), Uuid::new_v4());
        let vault = Vault::open_with_default_indexer(root.clone(), &journal_path).unwrap();
        let _ = vault.write_record_unchecked(&id, "content A", Version(0), op_a);
        let _ = vault.write_record_unchecked(&id, "content B", Version(1), op_b);
        init_test_repo_at(&root);
        assert_eq!(vault.write_record_unchecked(&id, "content B", Version(1), op_b).unwrap(), Version(2));
        assert_eq!(vault.write_record_unchecked(&id, "content A", Version(0), op_a).unwrap(), Version(1));
        assert_eq!(commits_for_operation(&root, op_a), 1);
        assert_eq!(commits_for_operation(&root, op_b), 1);
        let repo = git2::Repository::open(&root).unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(Some(head.id().to_string()), vault.journal_row_for_test(op_b).unwrap().git_commit_oid);
    }

    // Final whole-branch review, finding I2: the live path must classify git
    // failures exactly as recovery does.

    #[test]
    fn write_on_a_git_repo_with_no_commits_is_git_pending_then_commits_once_head_exists() {
        let (root, journal_path) = fresh_root_and_journal_path();
        git2::Repository::init(&root).unwrap(); // unborn HEAD: no commits yet
        let id = NoteId::parse("note-a").unwrap();
        let op = Uuid::new_v4();
        {
            let vault = Vault::open_with_default_indexer(root.clone(), &journal_path).unwrap();
            let result = vault.write_record_unchecked(&id, "content", Version(0), op);
            assert!(matches!(result, Err(StorageError::GitPending { .. })), "a durably-persisted write must not be a hard git error, got {result:?}");
            assert_eq!(vault.journal_row_for_test(op).unwrap().state, WriteState::JournalCommitted);
            assert_eq!(vault.read_note(&id).unwrap().content, "content");
        }
        // Once HEAD exists, the next open commits it, exactly as recovery
        // already did for this same condition.
        commit_initial_for_test(&root);
        let vault = Vault::open_with_default_indexer(root.clone(), &journal_path).unwrap();
        assert_eq!(vault.journal_row_for_test(op).unwrap().state, WriteState::IndexCurrent);
        assert_eq!(commits_for_operation(&root, op), 1);
    }

    /// Makes an initial empty commit in an already-initialized repository.
    fn commit_initial_for_test(root: &Path) {
        let repo = git2::Repository::open(root).unwrap();
        let signature = git2::Signature::now("SuperBrain Test", "test@localhost").unwrap();
        let tree_oid = repo.treebuilder(None).unwrap().write().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        repo.commit(Some("HEAD"), &signature, &signature, "initial empty commit", &tree, &[]).unwrap();
    }

    // Matrix row 2 at its earliest point (final whole-branch review, finding
    // C1): the rename landed but the row is still IntentRecorded, because the
    // crash (or a journal I/O error) came before the ContentPersisted advance.
    // Recovery must verify the intended hash recorded before the rename and
    // roll forward, not mark the row Failed and strand the new bytes.

    #[test]
    fn crash_after_rename_before_content_persisted_rolls_forward_on_reopen_not_stranded() {
        let (root, journal_path) = fresh_root_and_journal_path();
        init_test_repo_at(&root);
        let id = NoteId::parse("note-a").unwrap();
        let op2 = Uuid::new_v4();
        {
            let vault = Vault::open_with_default_indexer(root.clone(), &journal_path).unwrap();
            vault.write_record_unchecked(&id, "v1", Version(0), Uuid::new_v4()).unwrap();
            let injector = FaultInjector::trigger_at(FaultPoint::AfterRenameBeforeContentPersisted);
            let result = vault.write_record_unchecked_with_injector(&id, "v2", Version(1), op2, &injector);
            assert!(matches!(result, Err(StorageError::Interrupted(FaultPoint::AfterRenameBeforeContentPersisted))));
            assert_eq!(vault.journal_row_for_test(op2).unwrap().state, WriteState::IntentRecorded, "the crash came before the ContentPersisted advance");
            assert_eq!(std::fs::read(root.join("note-a.record")).unwrap(), serialize_record("v2", Version(2)), "but the rename already landed the new bytes");
        }

        let vault = Vault::open_with_default_indexer(root.clone(), &journal_path).unwrap();
        let row2 = vault.journal_row_for_test(op2).unwrap();
        assert_eq!(row2.state, WriteState::IndexCurrent, "rolled forward through the rest of the chain, not marked Failed");
        assert_eq!(row2.previous_state, Some(WriteState::IntentRecorded));
        assert_eq!(vault.read_note(&id).unwrap(), VaultRecord { content: "v2".into(), version: Version(2) }, "readable at the new version, not Inconsistent");
        assert_eq!(commits_for_operation(&root, op2), 1);
        assert!(!before_image_path(&root, &id, op2).exists(), "deleted once git_committed is reached, not orphaned");
        // The vault is not stuck: the next write builds on v2, with no false Conflict.
        assert_eq!(vault.write_record_unchecked(&id, "v3", Version(2), Uuid::new_v4()).unwrap(), Version(3));
    }

    #[test]
    fn crash_after_rename_before_content_persisted_on_a_first_write_rolls_forward_on_reopen() {
        let (root, journal_path) = fresh_root_and_journal_path();
        init_test_repo_at(&root);
        let id = NoteId::parse("note-a").unwrap();
        let op = Uuid::new_v4();
        {
            let vault = Vault::open_with_default_indexer(root.clone(), &journal_path).unwrap();
            let injector = FaultInjector::trigger_at(FaultPoint::AfterRenameBeforeContentPersisted);
            let result = vault.write_record_unchecked_with_injector(&id, "first", Version(0), op, &injector);
            assert!(matches!(result, Err(StorageError::Interrupted(_))));
            assert_eq!(vault.journal_row_for_test(op).unwrap().state, WriteState::IntentRecorded);
        }
        let vault = Vault::open_with_default_indexer(root.clone(), &journal_path).unwrap();
        assert_eq!(vault.journal_row_for_test(op).unwrap().state, WriteState::IndexCurrent);
        assert_eq!(vault.read_note(&id).unwrap(), VaultRecord { content: "first".into(), version: Version(1) });
    }

    // Matrix row 3: "Nuova versione con hash non valido -> Ripristino della
    // before-image", detected by the writer's own step-5 verification.

    #[test]
    fn new_version_with_an_invalid_hash_is_detected_by_the_writer_and_restored_from_the_before_image() {
        let (root, journal_path) = fresh_root_and_journal_path();
        init_test_repo_at(&root);
        let vault = Arc::new(Vault::open_with_default_indexer(root.clone(), &journal_path).unwrap());
        let id = NoteId::parse("note-a").unwrap();
        vault.write_record_unchecked(&id, "v1", Version(0), Uuid::new_v4()).unwrap();

        let (paused_tx, paused_rx) = std::sync::mpsc::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        let injector = FaultInjector::pause_at(FaultPoint::AfterRenameBeforeJournalCommitted, paused_tx, resume_rx);
        let op2 = Uuid::new_v4();
        let writer_vault = Arc::clone(&vault);
        let writer_id = id.clone();
        let writer = std::thread::spawn(move || {
            writer_vault.write_record_unchecked_with_injector(&writer_id, "v2", Version(1), op2, &injector)
        });
        paused_rx.recv_timeout(std::time::Duration::from_secs(10)).expect("writer must reach the rename boundary");
        // The writer has renamed v2 into place and recorded its hash, and is
        // stopped before verifying it. Damage the file now.
        std::fs::write(root.join("note-a.record"), b"TORN WRITE").unwrap();
        resume_tx.send(()).unwrap();

        let result = writer.join().unwrap();
        assert!(matches!(result, Err(StorageError::Inconsistent { .. })), "the writer reports that its write did not land, got {result:?}");
        assert_eq!(vault.journal_row_for_test(op2).unwrap().state, WriteState::Failed);
        assert_eq!(std::fs::read(root.join("note-a.record")).unwrap(), serialize_record("v1", Version(1)));
        assert!(!before_image_path(&root, &id, op2).exists(), "a terminal Failed row must not leave its before-image behind");
        assert_eq!(vault.read_note(&id).unwrap(), VaultRecord { content: "v1".into(), version: Version(1) });
        assert_eq!(commits_for_operation(&root, op2), 0);
    }

    // Matrix row 4: "Durante il ripristino della before-image -> Il
    // ripristino riparte in modo idempotente; se la before-image e'
    // illeggibile, stato inconsistente".

    #[test]
    fn crash_during_before_image_restore_restarts_idempotently_on_the_next_open() {
        let (root, journal_path) = fresh_root_and_journal_path();
        init_test_repo_at(&root);
        let id = NoteId::parse("note-a").unwrap();
        let op2 = Uuid::new_v4();
        {
            let vault = Vault::open_with_default_indexer(root.clone(), &journal_path).unwrap();
            vault.write_record_unchecked(&id, "v1", Version(0), Uuid::new_v4()).unwrap();
            let injector = FaultInjector::trigger_at(FaultPoint::AfterRenameBeforeJournalCommitted);
            let _ = vault.write_record_unchecked_with_injector(&id, "v2", Version(1), op2, &injector);
        }
        std::fs::write(root.join("note-a.record"), b"TORN WRITE").unwrap();

        // Second process: recovery restores the before-image and crashes
        // before it can finish the operation.
        let injector = FaultInjector::trigger_at(FaultPoint::DuringBeforeImageRestore);
        let result = Vault::open_with_injector(root.clone(), &journal_path, &injector);
        assert!(matches!(result, Err(StorageError::Interrupted(FaultPoint::DuringBeforeImageRestore))), "recovery must be interrupted mid-restore");
        {
            let journal = Journal::open(&journal_path).unwrap();
            let row = journal.get(op2).unwrap().unwrap();
            assert_eq!(row.state, WriteState::Recovering);
            assert_eq!(row.previous_state, Some(WriteState::ContentPersisted));
        }
        assert_eq!(std::fs::read(root.join("note-a.record")).unwrap(), serialize_record("v1", Version(1)), "the restore itself happened");
        assert!(before_image_path(&root, &id, op2).exists(), "the crash came before the before-image was consumed");

        // Third process: the restore restarts and completes, without a
        // second (corrupting) application.
        let vault = Vault::open_with_default_indexer(root.clone(), &journal_path).unwrap();
        assert_eq!(vault.journal_row_for_test(op2).unwrap().state, WriteState::Failed);
        assert_eq!(std::fs::read(root.join("note-a.record")).unwrap(), serialize_record("v1", Version(1)));
        assert!(!before_image_path(&root, &id, op2).exists());
        assert_eq!(vault.read_note(&id).unwrap(), VaultRecord { content: "v1".into(), version: Version(1) });
    }

    #[test]
    fn unreadable_before_image_during_restore_is_inconsistent_and_open_refuses_to_start() {
        let (root, journal_path) = fresh_root_and_journal_path();
        init_test_repo_at(&root);
        let id = NoteId::parse("note-a").unwrap();
        let op2 = Uuid::new_v4();
        {
            let vault = Vault::open_with_default_indexer(root.clone(), &journal_path).unwrap();
            vault.write_record_unchecked(&id, "v1", Version(0), Uuid::new_v4()).unwrap();
            let injector = FaultInjector::trigger_at(FaultPoint::AfterRenameBeforeJournalCommitted);
            let _ = vault.write_record_unchecked_with_injector(&id, "v2", Version(1), op2, &injector);
        }
        std::fs::write(root.join("note-a.record"), b"TORN WRITE").unwrap();
        std::fs::remove_file(before_image_path(&root, &id, op2)).unwrap();

        let result = Vault::open_with_default_indexer(root.clone(), &journal_path);
        assert!(matches!(result, Err(StorageError::Inconsistent { ref note_id }) if note_id == "note-a"), "got {:?}", result.err());
        assert_eq!(Journal::open(&journal_path).unwrap().get(op2).unwrap().unwrap().state, WriteState::Recovering, "left unresolved, never reported as Failed or committed");
    }

    // Matrix row 5: "Dopo journal_committed, prima di git_committed ->
    // Roll-forward del commit git".

    #[test]
    fn crash_after_journal_committed_before_git_rolls_the_git_commit_forward_on_reopen() {
        let (root, journal_path) = fresh_root_and_journal_path();
        init_test_repo_at(&root);
        let id = NoteId::parse("note-a").unwrap();
        let op2 = Uuid::new_v4();
        {
            let vault = Vault::open_with_default_indexer(root.clone(), &journal_path).unwrap();
            vault.write_record_unchecked(&id, "v1", Version(0), Uuid::new_v4()).unwrap();
            let injector = FaultInjector::trigger_at(FaultPoint::AfterJournalCommittedBeforeGit);
            let result = vault.write_record_unchecked_with_injector(&id, "v2", Version(1), op2, &injector);
            assert!(matches!(result, Err(StorageError::Interrupted(FaultPoint::AfterJournalCommittedBeforeGit))));
            let row = vault.journal_row_for_test(op2).unwrap();
            assert_eq!(row.state, WriteState::JournalCommitted);
            assert_eq!(row.git_commit_oid, None);
            assert_eq!(commits_for_operation(&root, op2), 0, "the crash came before any git commit");
            assert!(before_image_path(&root, &id, op2).exists(), "kept until git_committed");
            // JournalCommitted is the linearization point: already readable.
            assert_eq!(vault.read_note(&id).unwrap(), VaultRecord { content: "v2".into(), version: Version(2) });
        }
        let vault = Vault::open_with_default_indexer(root.clone(), &journal_path).unwrap();
        let row = vault.journal_row_for_test(op2).unwrap();
        assert_eq!(row.state, WriteState::IndexCurrent);
        assert_eq!(row.previous_state, Some(WriteState::JournalCommitted));
        assert_eq!(commits_for_operation(&root, op2), 1);
        assert_eq!(committed_record_bytes(&root, row.git_commit_oid.as_deref().unwrap(), "note-a"), serialize_record("v2", Version(2)));
        assert!(!before_image_path(&root, &id, op2).exists(), "deleted once git_committed is reached");
        assert_eq!(vault.read_note(&id).unwrap(), VaultRecord { content: "v2".into(), version: Version(2) });
    }

    #[test]
    fn crash_after_journal_committed_without_git_rests_as_git_pending_then_commits_once_git_exists() {
        let (root, journal_path) = fresh_root_and_journal_path(); // not a git repository yet
        let id = NoteId::parse("note-a").unwrap();
        let op = Uuid::new_v4();
        {
            let vault = Vault::open_with_default_indexer(root.clone(), &journal_path).unwrap();
            let injector = FaultInjector::trigger_at(FaultPoint::AfterJournalCommittedBeforeGit);
            let result = vault.write_record_unchecked_with_injector(&id, "content", Version(0), op, &injector);
            assert!(matches!(result, Err(StorageError::Interrupted(FaultPoint::AfterJournalCommittedBeforeGit))));
        }
        {
            // git_pending is an allowed resting state: open succeeds and the
            // committed content is served.
            let vault = Vault::open_with_default_indexer(root.clone(), &journal_path).unwrap();
            assert_eq!(vault.journal_row_for_test(op).unwrap().state, WriteState::JournalCommitted);
            assert_eq!(vault.read_note(&id).unwrap(), VaultRecord { content: "content".into(), version: Version(1) });
        }
        init_test_repo_at(&root);
        let vault = Vault::open_with_default_indexer(root.clone(), &journal_path).unwrap();
        let row = vault.journal_row_for_test(op).unwrap();
        assert_eq!(row.state, WriteState::IndexCurrent);
        assert_eq!(commits_for_operation(&root, op), 1);
        assert_eq!(committed_record_bytes(&root, row.git_commit_oid.as_deref().unwrap(), "note-a"), serialize_record("content", Version(1)));
    }

    // Matrix row 6: "Dopo git_committed, prima di index_current ->
    // Ricostruzione dell'indice, nel frattempo index_stale".

    #[test]
    fn crash_after_git_committed_before_index_current_reaches_index_current_on_reopen_and_is_stale_meanwhile() {
        let (root, journal_path) = fresh_root_and_journal_path();
        init_test_repo_at(&root);
        let id = NoteId::parse("note-a").unwrap();
        let op = Uuid::new_v4();
        {
            let (indexer, calls) = counting_indexer();
            let vault = Vault::open(root.clone(), &journal_path, indexer).unwrap();
            let injector = FaultInjector::trigger_at(FaultPoint::AfterGitCommittedBeforeIndex);
            let result = vault.write_record_unchecked_with_injector(&id, "content", Version(0), op, &injector);
            assert!(matches!(result, Err(StorageError::Interrupted(FaultPoint::AfterGitCommittedBeforeIndex))));
            let row = vault.journal_row_for_test(op).unwrap();
            assert_eq!(row.state, WriteState::GitCommitted);
            assert!(row.git_commit_oid.is_some());
            assert!(calls.lock().unwrap().is_empty(), "the index step never ran");
            assert_eq!(vault.index_state_for(&id), IndexState::Stale, "index_stale in the meantime");
            assert_eq!(vault.read_note(&id).unwrap().content, "content");
        }
        let (indexer, calls) = counting_indexer();
        let vault = Vault::open(root.clone(), &journal_path, indexer).unwrap();
        assert_eq!(vault.journal_row_for_test(op).unwrap().state, WriteState::IndexCurrent);
        assert_eq!(*calls.lock().unwrap(), vec![("note-a".to_string(), 1)], "the index step ran exactly once, on recovery");
        assert_eq!(commits_for_operation(&root, op), 1, "recovery did not commit a second time");
    }

    // Matrix row 7: "Lettura concorrente in ciascuno dei momenti sopra ->
    // L'API restituisce sempre la versione committata, anche se il file
    // definitivo e' gia' sostituito".

    #[test]
    fn read_blocks_behind_a_paused_writer_holding_the_same_note_lock_and_then_sees_the_fully_committed_result() {
        // Deviation from the brief: the root is a git repository (via
        // open_test_vault_with_git_repo) so the resumed writer can return Ok,
        // which the brief's `writer.join().unwrap().unwrap()` requires.
        let (vault, _root) = open_test_vault_with_git_repo();
        let vault = Arc::new(vault);
        let id = NoteId::parse("note-a").unwrap();
        vault.write_record_unchecked(&id, "v1", Version(0), Uuid::new_v4()).unwrap();

        let (pause_tx, pause_rx) = std::sync::mpsc::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        let injector = FaultInjector::pause_at(FaultPoint::AfterRenameBeforeJournalCommitted, pause_tx, resume_rx);
        let writer_vault = Arc::clone(&vault);
        let writer_id = id.clone();
        let writer = std::thread::spawn(move || {
            writer_vault.write_record_unchecked_with_injector(&writer_id, "v2", Version(1), Uuid::new_v4(), &injector)
        });

        pause_rx.recv().unwrap(); // writer has renamed the file but is paused BEFORE releasing its per-note lock

        // Start a reader now. It must not be able to acquire the lock while the
        // writer holds it, so prove it hasn't completed yet by racing it against
        // a short timeout BEFORE resuming the writer.
        let reader_vault = Arc::clone(&vault);
        let reader_id = id.clone();
        let (reader_done_tx, reader_done_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = reader_vault.read_note(&reader_id);
            reader_done_tx.send(result).unwrap();
        });
        assert!(
            reader_done_rx.recv_timeout(std::time::Duration::from_millis(200)).is_err(),
            "reader must still be blocked on the per-note lock while the writer holds it"
        );

        resume_tx.send(()).unwrap();
        writer.join().unwrap().unwrap();
        let record = reader_done_rx.recv().unwrap().unwrap();
        assert_eq!(record.content, "v2", "reader unblocks only after the writer's full critical section, including journal_committed, completes");
    }

    #[test]
    fn a_read_concurrent_with_each_write_fault_point_returns_only_a_committed_version() {
        // The test above covers one point. This walks every write-path point.
        // At AfterIntentRecorded the writer has not taken the per-note lock
        // yet, and its intent row is invisible, so the reader is NOT blocked
        // and sees v1. At every later point the writer holds the lock, so the
        // reader waits and then sees the fully committed v2. It never sees the
        // replaced file paired with v1's row, nor an error.
        // (DuringBeforeImageRestore fires inside recovery, before any Vault
        // exists, so no concurrent reader is possible there.)
        for point in [
            FaultPoint::AfterIntentRecorded,
            FaultPoint::AfterRenameBeforeContentPersisted,
            FaultPoint::AfterRenameBeforeJournalCommitted,
            FaultPoint::AfterJournalCommittedBeforeGit,
            FaultPoint::AfterGitCommittedBeforeIndex,
        ] {
            let (vault, _root) = open_test_vault_with_git_repo();
            let vault = Arc::new(vault);
            let id = NoteId::parse("note-a").unwrap();
            vault.write_record_unchecked(&id, "v1", Version(0), Uuid::new_v4()).unwrap();

            let (pause_tx, pause_rx) = std::sync::mpsc::channel();
            let (resume_tx, resume_rx) = std::sync::mpsc::channel();
            let injector = FaultInjector::pause_at(point, pause_tx, resume_rx);
            let writer_vault = Arc::clone(&vault);
            let writer_id = id.clone();
            let writer = std::thread::spawn(move || {
                writer_vault.write_record_unchecked_with_injector(&writer_id, "v2", Version(1), Uuid::new_v4(), &injector)
            });
            pause_rx
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap_or_else(|_| panic!("writer never paused at {point:?}"));

            let reader_vault = Arc::clone(&vault);
            let reader_id = id.clone();
            let (reader_done_tx, reader_done_rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = reader_done_tx.send(reader_vault.read_note(&reader_id));
            });

            if point == FaultPoint::AfterIntentRecorded {
                let record = reader_done_rx
                    .recv_timeout(std::time::Duration::from_secs(10))
                    .unwrap_or_else(|_| panic!("reader must not block at {point:?}"))
                    .unwrap();
                assert_eq!(record, VaultRecord { content: "v1".into(), version: Version(1) }, "at {point:?}");
                resume_tx.send(()).unwrap();
                writer.join().unwrap().unwrap();
            } else {
                assert!(
                    reader_done_rx.recv_timeout(std::time::Duration::from_millis(200)).is_err(),
                    "reader must be blocked behind the writer at {point:?}"
                );
                resume_tx.send(()).unwrap();
                writer.join().unwrap().unwrap();
                let record = reader_done_rx.recv().unwrap().unwrap();
                assert_eq!(record, VaultRecord { content: "v2".into(), version: Version(2) }, "at {point:?}");
            }
        }
    }

    // Matrix row 8: "Retry dello stesso operation_id in ciascuno stato ->
    // Esito gia' raggiunto, nessuna doppia applicazione".

    #[test]
    fn retry_of_a_failed_operation_id_returns_failed_again_without_reapplying() {
        // Deviation from the brief: the brief retried in the SAME process,
        // claiming the row was already Failed. Nothing marks it Failed at that
        // point: a crash at AfterIntentRecorded leaves the row at
        // IntentRecorded (that is what row 1 of reconcile resolves). The
        // Failed state is reached on restart, so the retry happens after one.
        // The in-process case is covered by
        // `retry_of_an_operation_still_in_flight_is_refused_without_reapplying`.
        let (root, journal_path) = fresh_root_and_journal_path();
        init_test_repo_at(&root);
        let id = NoteId::parse("note-a").unwrap();
        let op = Uuid::new_v4();
        {
            let vault = Vault::open_with_default_indexer(root.clone(), &journal_path).unwrap();
            let injector = FaultInjector::trigger_at(FaultPoint::AfterIntentRecorded);
            let _ = vault.write_record_unchecked_with_injector(&id, "content", Version(0), op, &injector);
        }
        let vault = Vault::open_with_default_indexer(root.clone(), &journal_path).unwrap();
        assert_eq!(vault.journal_row_for_test(op).unwrap().state, WriteState::Failed);
        let retry_same_id = vault.write_record_unchecked(&id, "content", Version(0), op);
        assert!(
            matches!(retry_same_id, Err(StorageError::OperationFailed { operation_id }) if operation_id == op),
            "same operation_id on a failed op must not retry the write, got {retry_same_id:?}"
        );
        assert!(!root.join("note-a.record").exists(), "nothing was applied");
        assert_eq!(vault.journal_row_for_test(op).unwrap().state, WriteState::Failed);
    }

    #[test]
    fn retry_of_a_conflict_failed_operation_id_returns_operation_failed_not_a_second_attempt() {
        let (vault, _root) = open_test_vault_with_git_repo();
        let id = NoteId::parse("note-a").unwrap();
        vault.write_record_unchecked(&id, "v1", Version(0), Uuid::new_v4()).unwrap();
        let op = Uuid::new_v4();
        assert!(matches!(vault.write_record_unchecked(&id, "stale", Version(0), op), Err(StorageError::Conflict { .. })));
        // A genuine retry (identical request): the outcome already reached
        // (Failed) stands; the conflicting write is not attempted again.
        let retry = vault.write_record_unchecked(&id, "stale", Version(0), op);
        assert!(matches!(retry, Err(StorageError::OperationFailed { operation_id }) if operation_id == op), "got {retry:?}");
        // The same id with an expected version that WOULD now match is not a
        // retry but a different request (fix round, finding I1).
        let reused = vault.write_record_unchecked(&id, "stale", Version(1), op);
        assert!(matches!(reused, Err(StorageError::OperationIdReused { operation_id }) if operation_id == op), "got {reused:?}");
        assert_eq!(vault.read_note(&id).unwrap(), VaultRecord { content: "v1".into(), version: Version(1) });
    }

    #[test]
    fn retry_of_an_operation_still_in_flight_is_refused_without_reapplying() {
        // A row at IntentRecorded or ContentPersisted, in a process that has
        // not restarted, is either still being written by another call or was
        // abandoned mid-flight. Neither can be safely resumed or failed from
        // here; recovery on the next open resolves it.
        for point in [FaultPoint::AfterIntentRecorded, FaultPoint::AfterRenameBeforeJournalCommitted] {
            let (vault, root) = open_test_vault_with_git_repo();
            let id = NoteId::parse("note-a").unwrap();
            let op = Uuid::new_v4();
            let injector = FaultInjector::trigger_at(point);
            let _ = vault.write_record_unchecked_with_injector(&id, "content", Version(0), op, &injector);
            let state_before = vault.journal_row_for_test(op).unwrap().state;
            let bytes_before = std::fs::read(root.join("note-a.record")).ok();

            let retry = vault.write_record_unchecked(&id, "content", Version(0), op);
            assert!(matches!(retry, Err(StorageError::OperationInProgress { operation_id }) if operation_id == op), "at {point:?}, got {retry:?}");
            assert_eq!(vault.journal_row_for_test(op).unwrap().state, state_before, "at {point:?}: the row is untouched");
            assert_eq!(std::fs::read(root.join("note-a.record")).ok(), bytes_before, "at {point:?}: the file is untouched");
            assert_eq!(commits_for_operation(&root, op), 0, "at {point:?}");
        }
    }

    #[test]
    fn retry_of_a_journal_committed_operation_commits_to_git_exactly_once() {
        let (root, journal_path) = fresh_root_and_journal_path();
        init_test_repo_at(&root);
        let (indexer, calls) = counting_indexer();
        let vault = Vault::open(root.clone(), &journal_path, indexer).unwrap();
        let id = NoteId::parse("note-a").unwrap();
        let op = Uuid::new_v4();
        let injector = FaultInjector::trigger_at(FaultPoint::AfterJournalCommittedBeforeGit);
        let _ = vault.write_record_unchecked_with_injector(&id, "content", Version(0), op, &injector);
        assert_eq!(vault.journal_row_for_test(op).unwrap().state, WriteState::JournalCommitted);

        // The retry resumes the chain from where it stopped.
        assert_eq!(vault.write_record_unchecked(&id, "content", Version(0), op).unwrap(), Version(1));
        // A second retry finds the outcome already reached.
        assert_eq!(vault.write_record_unchecked(&id, "content", Version(0), op).unwrap(), Version(1));

        assert_eq!(vault.journal_row_for_test(op).unwrap().state, WriteState::IndexCurrent);
        assert_eq!(commits_for_operation(&root, op), 1, "two retries, one commit");
        assert_eq!(*calls.lock().unwrap(), vec![("note-a".to_string(), 1)], "two retries, one index step");
        assert_eq!(vault.read_note(&id).unwrap(), VaultRecord { content: "content".into(), version: Version(1) });
    }

    #[test]
    fn retry_of_a_git_pending_operation_commits_its_own_content_once_git_exists() {
        // CommitResult::GitPending documents that a retry of the SAME
        // operation_id advances the operation once a repository exists.
        let (root, journal_path) = fresh_root_and_journal_path(); // not a git repository yet
        let vault = Vault::open_with_default_indexer(root.clone(), &journal_path).unwrap();
        let id = NoteId::parse("note-a").unwrap();
        let op = Uuid::new_v4();
        assert!(matches!(vault.write_record_unchecked(&id, "content", Version(0), op), Err(StorageError::GitPending { .. })));
        // Still no repository: the retry reports the same outcome, nothing new applied.
        assert!(matches!(vault.write_record_unchecked(&id, "content", Version(0), op), Err(StorageError::GitPending { .. })));
        assert_eq!(vault.journal_row_for_test(op).unwrap().state, WriteState::JournalCommitted);

        init_test_repo_at(&root);
        assert_eq!(vault.write_record_unchecked(&id, "content", Version(0), op).unwrap(), Version(1));
        let row = vault.journal_row_for_test(op).unwrap();
        assert_eq!(row.state, WriteState::IndexCurrent);
        assert_eq!(commits_for_operation(&root, op), 1);
        assert_eq!(committed_record_bytes(&root, row.git_commit_oid.as_deref().unwrap(), "note-a"), serialize_record("content", Version(1)));
    }

    #[test]
    fn retry_of_a_git_pending_operation_after_a_successor_overwrote_it_commits_its_own_content_not_the_successors() {
        // The retry-path counterpart of Task 7's fix-round finding I-2: the
        // record on disk now belongs to op_b, so the retry of op_a must take
        // op_a's bytes from op_b's before-image.
        let (root, journal_path) = fresh_root_and_journal_path();
        let vault = Vault::open_with_default_indexer(root.clone(), &journal_path).unwrap();
        let id = NoteId::parse("note-a").unwrap();
        let (op_a, op_b) = (Uuid::new_v4(), Uuid::new_v4());
        assert!(matches!(vault.write_record_unchecked(&id, "content A", Version(0), op_a), Err(StorageError::GitPending { .. })));
        assert!(matches!(vault.write_record_unchecked(&id, "content B", Version(1), op_b), Err(StorageError::GitPending { .. })));
        init_test_repo_at(&root);

        assert_eq!(vault.write_record_unchecked(&id, "content A", Version(0), op_a).unwrap(), Version(1));
        let row_a = vault.journal_row_for_test(op_a).unwrap();
        assert_eq!(committed_record_bytes(&root, row_a.git_commit_oid.as_deref().unwrap(), "note-a"), serialize_record("content A", Version(1)));
        assert_eq!(vault.read_note(&id).unwrap(), VaultRecord { content: "content B".into(), version: Version(2) }, "the retry never touched the record");
    }

    #[test]
    fn retry_of_a_git_committed_operation_completes_the_index_step_exactly_once() {
        let (root, journal_path) = fresh_root_and_journal_path();
        init_test_repo_at(&root);
        let (indexer, calls) = counting_indexer();
        let vault = Vault::open(root.clone(), &journal_path, indexer).unwrap();
        let id = NoteId::parse("note-a").unwrap();
        let op = Uuid::new_v4();
        let injector = FaultInjector::trigger_at(FaultPoint::AfterGitCommittedBeforeIndex);
        let _ = vault.write_record_unchecked_with_injector(&id, "content", Version(0), op, &injector);
        assert_eq!(vault.journal_row_for_test(op).unwrap().state, WriteState::GitCommitted);

        assert_eq!(vault.write_record_unchecked(&id, "content", Version(0), op).unwrap(), Version(1));
        assert_eq!(vault.write_record_unchecked(&id, "content", Version(0), op).unwrap(), Version(1));
        assert_eq!(vault.journal_row_for_test(op).unwrap().state, WriteState::IndexCurrent);
        assert_eq!(*calls.lock().unwrap(), vec![("note-a".to_string(), 1)]);
        assert_eq!(commits_for_operation(&root, op), 1);
    }

    #[test]
    fn retry_of_an_index_current_operation_returns_its_version_without_a_second_write_commit_or_index_step() {
        let (root, journal_path) = fresh_root_and_journal_path();
        init_test_repo_at(&root);
        let (indexer, calls) = counting_indexer();
        let vault = Vault::open(root.clone(), &journal_path, indexer).unwrap();
        let id = NoteId::parse("note-a").unwrap();
        let op = Uuid::new_v4();
        assert_eq!(vault.write_record_unchecked(&id, "content", Version(0), op).unwrap(), Version(1));
        let bytes = std::fs::read(root.join("note-a.record")).unwrap();

        // A genuine retry, the identical request: its outcome is already reached.
        assert_eq!(vault.write_record_unchecked(&id, "content", Version(0), op).unwrap(), Version(1));
        assert_eq!(std::fs::read(root.join("note-a.record")).unwrap(), bytes, "no second write");
        assert_eq!(commits_for_operation(&root, op), 1, "no second commit");
        assert_eq!(calls.lock().unwrap().len(), 1, "no second index step");
        assert!(read_before_image(&root, &id, op).is_err(), "no before-image was taken for the retry");
    }

    #[test]
    fn reusing_an_operation_id_for_a_different_request_is_refused_not_reported_as_the_original_outcome() {
        // Fix round, finding I1: the journal row records what the original
        // request was (note, expected version, content hash). A call that
        // reuses the id for anything else must not get the original outcome
        // back, and must not be applied either.
        let (root, journal_path) = fresh_root_and_journal_path();
        init_test_repo_at(&root);
        let (indexer, calls) = counting_indexer();
        let vault = Vault::open(root.clone(), &journal_path, indexer).unwrap();
        let id = NoteId::parse("note-a").unwrap();
        let other_id = NoteId::parse("note-b").unwrap();
        let op = Uuid::new_v4();
        assert_eq!(vault.write_record_unchecked(&id, "content", Version(0), op).unwrap(), Version(1));
        let bytes = std::fs::read(root.join("note-a.record")).unwrap();

        for (label, note, content, expected) in [
            ("different content", &id, "other content", Version(0)),
            ("different expected version", &id, "content", Version(1)),
            ("different note", &other_id, "content", Version(0)),
            ("everything different", &other_id, "other content", Version(1)),
        ] {
            let result = vault.write_record_unchecked(note, content, expected, op);
            assert!(
                matches!(result, Err(StorageError::OperationIdReused { operation_id }) if operation_id == op),
                "{label}: got {result:?}"
            );
        }
        assert_eq!(std::fs::read(root.join("note-a.record")).unwrap(), bytes, "nothing applied");
        assert!(!root.join("note-b.record").exists(), "nothing applied to the other note");
        assert_eq!(commits_for_operation(&root, op), 1);
        assert_eq!(calls.lock().unwrap().len(), 1);
        assert_eq!(vault.journal_row_for_test(op).unwrap().state, WriteState::IndexCurrent, "the original row is untouched");
    }

    #[test]
    fn reusing_an_in_flight_operation_id_for_different_content_is_refused_as_reused_not_in_progress() {
        // The content check also applies before the operation resolves, once
        // the row records a content_hash (ContentPersisted onward).
        let (vault, root) = open_test_vault_with_git_repo();
        let id = NoteId::parse("note-a").unwrap();
        let op = Uuid::new_v4();
        let injector = FaultInjector::trigger_at(FaultPoint::AfterRenameBeforeJournalCommitted);
        let _ = vault.write_record_unchecked_with_injector(&id, "content", Version(0), op, &injector);
        assert_eq!(vault.journal_row_for_test(op).unwrap().state, WriteState::ContentPersisted);

        let reused = vault.write_record_unchecked(&id, "other content", Version(0), op);
        assert!(matches!(reused, Err(StorageError::OperationIdReused { .. })), "got {reused:?}");
        let same = vault.write_record_unchecked(&id, "content", Version(0), op);
        assert!(matches!(same, Err(StorageError::OperationInProgress { .. })), "got {same:?}");
        assert_eq!(commits_for_operation(&root, op), 0);
    }

    #[test]
    fn retry_concurrent_with_a_live_writer_paused_before_journal_committed_returns_in_progress_without_blocking() {
        // Fix round, finding I2(a): the original writer is genuinely still
        // running (paused at AfterRenameBeforeJournalCommitted, holding the
        // per-note lock). The retry checks the row at step 0, before any lock,
        // so it must return OperationInProgress promptly rather than wait.
        let (vault, root) = open_test_vault_with_git_repo();
        let vault = Arc::new(vault);
        let id = NoteId::parse("note-a").unwrap();
        let op = Uuid::new_v4();

        let (pause_tx, pause_rx) = std::sync::mpsc::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        let injector = FaultInjector::pause_at(FaultPoint::AfterRenameBeforeJournalCommitted, pause_tx, resume_rx);
        let writer_vault = Arc::clone(&vault);
        let writer_id = id.clone();
        let writer = std::thread::spawn(move || {
            writer_vault.write_record_unchecked_with_injector(&writer_id, "content", Version(0), op, &injector)
        });
        pause_rx.recv_timeout(std::time::Duration::from_secs(10)).expect("writer must pause");

        let retry_vault = Arc::clone(&vault);
        let retry_id = id.clone();
        let (retry_tx, retry_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = retry_tx.send(retry_vault.write_record_unchecked(&retry_id, "content", Version(0), op));
        });
        let retry = retry_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the retry must return while the writer still holds the lock, not block on it");
        assert!(matches!(retry, Err(StorageError::OperationInProgress { operation_id }) if operation_id == op), "got {retry:?}");

        resume_tx.send(()).unwrap();
        assert_eq!(writer.join().unwrap().unwrap(), Version(1), "the live writer completes normally");
        assert_eq!(vault.journal_row_for_test(op).unwrap().state, WriteState::IndexCurrent);
        assert_eq!(vault.write_record_unchecked(&id, "content", Version(0), op).unwrap(), Version(1), "a later retry sees the outcome");
        assert_eq!(commits_for_operation(&root, op), 1);
        assert_eq!(vault.read_note(&id).unwrap(), VaultRecord { content: "content".into(), version: Version(1) });
    }

    #[test]
    fn retry_concurrent_with_a_live_writer_paused_after_journal_committed_waits_then_resolves_without_a_second_commit() {
        // Fix round, finding I2(b): the writer is paused at
        // AfterJournalCommittedBeforeGit, still holding the per-note lock. The
        // retry sees JournalCommitted at step 0, so it goes for the lock to
        // finish the remaining steps, and must wait there. Once the writer
        // completes and releases the lock, the retry re-reads the row, finds
        // IndexCurrent, and reports it: no second commit, no second index call.
        let (root, journal_path) = fresh_root_and_journal_path();
        init_test_repo_at(&root);
        let (indexer, calls) = counting_indexer();
        let vault = Arc::new(Vault::open(root.clone(), &journal_path, indexer).unwrap());
        let id = NoteId::parse("note-a").unwrap();
        let op = Uuid::new_v4();

        let (pause_tx, pause_rx) = std::sync::mpsc::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        let injector = FaultInjector::pause_at(FaultPoint::AfterJournalCommittedBeforeGit, pause_tx, resume_rx);
        let writer_vault = Arc::clone(&vault);
        let writer_id = id.clone();
        let writer = std::thread::spawn(move || {
            writer_vault.write_record_unchecked_with_injector(&writer_id, "content", Version(0), op, &injector)
        });
        pause_rx.recv_timeout(std::time::Duration::from_secs(10)).expect("writer must pause");
        assert_eq!(vault.journal_row_for_test(op).unwrap().state, WriteState::JournalCommitted);

        let retry_vault = Arc::clone(&vault);
        let retry_id = id.clone();
        let (retry_tx, retry_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = retry_tx.send(retry_vault.write_record_unchecked(&retry_id, "content", Version(0), op));
        });
        assert!(
            retry_rx.recv_timeout(std::time::Duration::from_millis(200)).is_err(),
            "the retry must wait on the per-note lock while the writer holds it"
        );
        assert_eq!(commits_for_operation(&root, op), 0, "nobody has committed yet");

        resume_tx.send(()).unwrap();
        assert_eq!(writer.join().unwrap().unwrap(), Version(1));
        let retry = retry_rx.recv_timeout(std::time::Duration::from_secs(10)).expect("the retry unblocks once the writer releases the lock");
        assert_eq!(retry.unwrap(), Version(1), "the retry reports the outcome the writer reached");

        assert_eq!(vault.journal_row_for_test(op).unwrap().state, WriteState::IndexCurrent);
        assert_eq!(commits_for_operation(&root, op), 1, "exactly one commit");
        assert_eq!(*calls.lock().unwrap(), vec![("note-a".to_string(), 1)], "exactly one index step");
    }

    #[test]
    fn retry_after_a_restart_returns_the_outcome_recovery_reached() {
        // ContentPersisted at crash time, rolled forward to IndexCurrent by
        // recovery: the retry reports success without applying anything.
        let (root, journal_path) = fresh_root_and_journal_path();
        init_test_repo_at(&root);
        let id = NoteId::parse("note-a").unwrap();
        let op = Uuid::new_v4();
        {
            let vault = Vault::open_with_default_indexer(root.clone(), &journal_path).unwrap();
            let injector = FaultInjector::trigger_at(FaultPoint::AfterRenameBeforeJournalCommitted);
            let _ = vault.write_record_unchecked_with_injector(&id, "content", Version(0), op, &injector);
        }
        let vault = Vault::open_with_default_indexer(root.clone(), &journal_path).unwrap();
        assert_eq!(vault.journal_row_for_test(op).unwrap().state, WriteState::IndexCurrent);
        assert_eq!(vault.write_record_unchecked(&id, "content", Version(0), op).unwrap(), Version(1));
        assert_eq!(commits_for_operation(&root, op), 1);
    }

    // Review Focus item 2: a second crash during recovery of the same operation.

    #[test]
    fn second_crash_during_recovery_of_the_same_operation_still_converges() {
        let (root, journal_path) = fresh_root_and_journal_path();
        let repo_dir = init_test_repo_at(&root);
        let id = NoteId::parse("note-a").unwrap();
        let op = Uuid::new_v4();
        {
            let vault = Vault::open_with_default_indexer(root.clone(), &journal_path).unwrap();
            let injector = FaultInjector::trigger_at(FaultPoint::AfterJournalCommittedBeforeGit);
            let _ = vault.write_record_unchecked_with_injector(&id, "content", Version(0), op, &injector);
        } // first "crash": process 1 ends, op is stuck at JournalCommitted
        {
            // Second restart: reconcile is given an injector that interrupts it
            // right after it marks the row Recovering (Task 7's
            // FaultPoint::AfterMarkedRecoveringBeforeFixup) but before row 3's
            // git-commit fix-up completes — reproducing a genuine second crash
            // mid-recovery, not just a second ordinary open.
            let second_open_injector = FaultInjector::trigger_at(FaultPoint::AfterMarkedRecoveringBeforeFixup);
            let result = Vault::open_with_injector(root.clone(), &journal_path, &second_open_injector);
            assert!(matches!(result, Err(StorageError::Interrupted(_))), "the second open's own recovery must be genuinely interrupted, not merely re-run to completion");
            // The row is now genuinely AT Recovering, with previous_state recording
            // JournalCommitted — not re-labeled back to JournalCommitted itself.
            let journal = Journal::open(&journal_path).unwrap();
            let row = journal.get(op).unwrap().unwrap();
            assert_eq!(row.state, WriteState::Recovering, "second crash left the row genuinely at Recovering, mid-fixup");
            assert_eq!(row.previous_state, Some(WriteState::JournalCommitted), "previous_state preserves what row 3's dispatch was resuming from");
            assert_eq!(commits_for_operation(&repo_dir, op), 0);
        }
        let vault3 = Vault::open_with_default_indexer(root.clone(), &journal_path).unwrap();
        assert_eq!(vault3.journal_row_for_test(op).unwrap().state, WriteState::IndexCurrent);
        // and the git history has exactly one commit for this operation_id, not two,
        // despite two separate reconcile passes touching this row — verified via
        // Tier 1 (the journal's own recorded Oid), the actual idempotency source:
        assert!(find_commit_for_operation(&vault3.journal_for_test(), &repo_dir, op, ScanDepth::Unbounded).unwrap().is_some());
        assert_eq!(commits_for_operation(&repo_dir, op), 1, "exactly one commit, counted in history");
        assert_eq!(vault3.read_note(&id).unwrap().content, "content");
    }

    // Review Focus item 5: a repository dirtied between journal_committed
    // and recovery.

    #[test]
    fn a_manual_git_commit_between_journal_committed_and_recovery_does_not_break_idempotent_commit() {
        // Deviation from the brief: the JournalCommitted row is produced by the
        // real write path interrupted at AfterJournalCommittedBeforeGit
        // (instead of hand-seeded journal rows), and the end state is
        // IndexCurrent, not GitCommitted: a row recovered from JournalCommitted
        // runs row 3 and then row 4 in the same pass (Task 7's cascade).
        let (root, journal_path) = fresh_root_and_journal_path();
        let repo_dir = init_test_repo_at(&root);
        let id = NoteId::parse("note-a").unwrap();
        let op = Uuid::new_v4();
        {
            let vault = Vault::open_with_default_indexer(root.clone(), &journal_path).unwrap();
            let injector = FaultInjector::trigger_at(FaultPoint::AfterJournalCommittedBeforeGit);
            let _ = vault.write_record_unchecked_with_injector(&id, "content", Version(0), op, &injector);
        }
        // A manual, unrelated commit lands in the repo before recovery runs.
        commit_unrelated_change_for_test(&repo_dir);
        let manual_head = git2::Repository::open(&repo_dir).unwrap().head().unwrap().peel_to_commit().unwrap().id();

        let vault = Vault::open_with_default_indexer(root, &journal_path).unwrap();
        let row = vault.journal_row_for_test(op).unwrap();
        assert_eq!(row.state, WriteState::IndexCurrent);
        let oid = find_commit_for_operation(&vault.journal_for_test(), &repo_dir, op, ScanDepth::Unbounded).unwrap();
        assert!(oid.is_some(), "recovery's own commit still exists alongside the manual one");
        assert_eq!(commits_for_operation(&repo_dir, op), 1);
        let repo = git2::Repository::open(&repo_dir).unwrap();
        let recovery_commit = repo.find_commit(oid.unwrap()).unwrap();
        assert_eq!(recovery_commit.parent_id(0).unwrap(), manual_head, "built on top of the manual commit, which is kept");
        assert_eq!(committed_record_bytes(&repo_dir, &oid.unwrap().to_string(), "note-a"), serialize_record("content", Version(1)));
    }

    // --- Task 9: SEC-001/002/003 re-evaluation against the journal design ---
    //
    // These are adversarial characterization tests, not regression tests:
    // each is EXPECTED to currently PASS, but a pass does NOT uniformly mean
    // "the exploit still succeeds" — it means the assertion correctly
    // captures whatever A2's actual behavior measures out to be, which
    // differs across these tests and even within a single test's scenarios.
    // SEC-001's scenarios prove the race still proceeds through a
    // mid-flight-swapped path (the exploit's core mechanism still works, with
    // two different concrete consequences depending on what the swapped-in
    // target is). SEC-002/003's Scenario A proves the opposite of what SEC-002
    // was originally named for on the READ side: A2's journal-first
    // resolution now REJECTS that specific read-side alias shape rather than
    // serving it, while their Scenario B proves a different read-side shape
    // IS still rejected for an unrelated reason (the hash check), and their
    // write-path scenario proves the alias is NOT rejected on the write side
    // at all — see each test's own comments and the verdicts in
    // `eval/results/A2/sec-debt-reevaluation.md`, which is the authoritative,
    // scoped statement of what remains open vs. narrowed vs. closed. A
    // FAILING assertion here is not a test bug to fix — it means A2's
    // measured behavior changed and the verdict must be re-derived from that
    // failure. Every assertion checks a concrete, checkable outcome (bytes
    // landing outside the root; a specific `StorageError` variant; a
    // before-image file's actual bytes), never a bare print with no
    // assertion.

    /// A fresh, unique SQLite journal path with no corresponding journal
    /// file yet created — `Vault::open*` creates it. Named to match this
    /// task's brief; equivalent to `unique_test_dir().join("journal.sqlite")`
    /// used inline elsewhere in this module.
    fn fresh_journal_path() -> PathBuf {
        unique_test_dir().join("journal.sqlite")
    }

    #[test]
    fn sec_001_toctou_window_between_path_check_and_write_still_exists() {
        // This test is EXPECTED to demonstrate the race still succeeds (NOT a
        // regression to fix in this task) — it is the adversarial proof that
        // hash verification alone does not close SEC-001, because the hash
        // check happens AFTER the write, not before the path resolution race.
        let root = unique_test_dir();
        init_git_repo_at(&root);
        let vault = Arc::new(Vault::open_with_default_indexer(root.clone(), &fresh_journal_path()).unwrap());
        let id = NoteId::parse("note-a").unwrap();
        let outside = unique_test_dir();

        let (pause_tx, pause_rx) = std::sync::mpsc::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        // Pause AFTER record_path_checked's canonicalization has already run
        // and returned "inside root", but BEFORE atomic_write's own
        // open/rename.
        let injector = FaultInjector::pause_at(FaultPoint::AfterPathCheckedBeforeAtomicWrite, pause_tx, resume_rx);
        let writer_vault = Arc::clone(&vault);
        let writer_id = id.clone();
        let writer = std::thread::spawn(move || {
            writer_vault.write_record_unchecked_with_injector(&writer_id, "content", Version(0), Uuid::new_v4(), &injector)
        });

        pause_rx.recv().unwrap();
        // Swap the now-checked-safe path for a symlink pointing outside,
        // during the window.
        let _ = std::fs::remove_file(root.join("note-a.record"));
        symlink_file(&outside.join("victim"), &root.join("note-a.record"));
        resume_tx.send(()).unwrap();
        let write_result = writer.join().unwrap();

        // DEVIATION FROM THE BRIEF, recorded here rather than silently
        // "fixed" to match the brief's prediction (per this task's explicit
        // instructions): the brief's original assertion checked
        // `outside.join("victim").exists()`, predicting that the race makes
        // `atomic_write`'s rename FOLLOW the symlink and land the new bytes
        // at its target, inside `outside`. Measured directly against this
        // platform's `atomic_write` (a bare `std::fs::rename(tmp, path)`,
        // `crates/storage/src/atomic.rs`), that is not what happens: a
        // rename onto a destination that is itself a symlink replaces the
        // link (unlinks it and puts the new inode/file in its place) rather
        // than writing through it to the link's target. So `outside/victim`
        // is never created, and the race's observable effect is instead
        // that the attacker's symlink is silently destroyed and replaced by
        // the vault's own record — still a bug (the write proceeded through
        // a path component the containment check had already approved
        // before an attacker swapped it, violating the "write what you
        // checked" invariant) but NOT the specific "write bytes outside the
        // root" consequence the original finding predicted via THIS write
        // primitive. Both facts are asserted below so this test fails loudly
        // if either changes.
        assert!(write_result.is_ok(), "the write must succeed despite the mid-flight symlink swap, got {write_result:?}");
        let link_meta = root.join("note-a.record").symlink_metadata().unwrap();
        let race_replaced_the_symlink_without_erroring = !link_meta.file_type().is_symlink();
        assert!(
            race_replaced_the_symlink_without_erroring,
            "SEC-001 verdict check: if this assertion fails (the path is still the attacker's symlink), the race no longer silently proceeds through a swapped path — re-evaluate the verdict rather than treating this as a test bug"
        );
        let race_wrote_outside_the_root = outside.join("victim").exists();
        assert!(
            !race_wrote_outside_the_root,
            "atomic_write's std::fs::rename onto the final path replaces a destination symlink rather than following it — this holds under both Windows and POSIX rename(2) semantics, so it is not a platform quirk. If this assertion fails, atomic_write's rename semantics changed and bytes now land outside the root, which is a MORE severe form of SEC-001 than previously measured"
        );
    }

    #[test]
    fn sec_001_toctou_read_side_leaks_an_outside_records_version_and_bytes_into_the_before_image() {
        // Independent-review finding I-1: the write-side scenario above only
        // exercises atomic_write's own rename, which happens to replace a
        // destination symlink rather than follow it. But the SAME race
        // window also covers write_record_unchecked's STEP 2 — the
        // version-check `std::fs::read(&record_path)` that runs AFTER the
        // per-note lock is acquired but still against whatever is actually
        // at `record_path` right now. Unlike atomic_write's rename,
        // std::fs::read DOES follow a symlink. If the attacker's swapped-in
        // symlink points at an EXISTING, readable, parseable record (not a
        // nonexistent path like the write-side test's `outside/victim`),
        // step 2 reads THAT file's version and bytes through the link,
        // producing two live, concrete consequences the write-side test
        // cannot see:
        //   (a) a version mismatch surfaces `Conflict` reporting the
        //       attacker-chosen "actual" version (a denial-of-service lever:
        //       the attacker can make legitimate writes with the real
        //       expected version fail); and
        //   (b) a version MATCH causes `persist_before_image` to copy the
        //       OUTSIDE file's bytes into the vault's OWN before-image
        //       directory under this note's name — external data pulled
        //       into the vault, a real (if narrow) exfiltration-ADJACENT
        //       consequence distinct from (and not captured by) the
        //       write-side test's "symlink destroyed, no write-through"
        //       finding.
        // This test demonstrates (b), the before-image contamination,
        // because it is the more concrete and more surprising of the two;
        // (a) is the ordinary Conflict path and is not re-demonstrated here
        // since Conflict-with-an-attacker-chosen-version needs no new
        // mechanism beyond what `stale_expected_version_is_rejected_with_explicit_conflict`
        // already covers elsewhere in this module — the only new fact is
        // WHERE the "actual" version came from.
        // Deliberately NOT a git-backed root: `write_record_unchecked`
        // deletes a note's before-image once `GitCommitted` is reached
        // (doc comment on step 7, above), and this test wants to inspect
        // the before-image the write persisted, so the write must stop at
        // `JournalCommitted`/`GitPending` instead of running all the way to
        // completion. See `write_to_a_vault_root_that_is_not_a_git_repo_returns_git_pending_not_an_error`
        // elsewhere in this module for the same pattern.
        let root = unique_test_dir();
        let outside_dir = unique_test_dir();
        let outside_file = outside_dir.join("victim.record");
        // A real, parseable, READABLE record at Version(7) — root_and_outside_record's
        // own fixture shape, inlined here so this test controls `expected_version` directly.
        std::fs::write(&outside_file, serialize_record("OUTSIDE CONTENT", Version(7))).unwrap();
        let vault = Arc::new(Vault::open_with_default_indexer(root.clone(), &fresh_journal_path()).unwrap());
        let id = NoteId::parse("note-a").unwrap();

        let (pause_tx, pause_rx) = std::sync::mpsc::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        let injector = FaultInjector::pause_at(FaultPoint::AfterPathCheckedBeforeAtomicWrite, pause_tx, resume_rx);
        let writer_vault = Arc::clone(&vault);
        let writer_id = id.clone();
        let op = Uuid::new_v4();
        // expected_version = Version(7): matches the OUTSIDE file's version,
        // so step 2's version check (reading THROUGH the about-to-be-swapped-in
        // symlink) passes and the pipeline proceeds to persist a before-image
        // of what step 2 just read.
        let writer = std::thread::spawn(move || {
            writer_vault.write_record_unchecked_with_injector(&writer_id, "new content", Version(7), op, &injector)
        });

        pause_rx.recv().unwrap();
        let _ = std::fs::remove_file(root.join("note-a.record"));
        symlink_file(&outside_file, &root.join("note-a.record"));
        resume_tx.send(()).unwrap();
        let write_result = writer.join().unwrap();

        // Step 2 read Version(7) through the symlink, matched
        // expected_version, so this is NOT a Conflict. The content itself
        // is still durably persisted and journal-readable at this point
        // (`JournalCommitted`) — `GitPending` is the spec's non-error
        // response code for "no git repository yet", not a write failure.
        assert!(
            matches!(write_result, Err(StorageError::GitPending { .. })),
            "the write's content/journal steps must succeed (step 2 reads the outside file's real Version(7) through the symlink, matching expected_version); GitPending is expected here only because this test's root is deliberately not a git repository — got {write_result:?}"
        );
        let before_image_bytes = read_before_image(&root, &id, op).expect("a before-image must have been persisted: step 2 saw an existing record (the outside file, through the symlink) before overwriting it");
        assert_eq!(
            before_image_bytes,
            serialize_record("OUTSIDE CONTENT", Version(7)),
            "SEC-001 read-side finding: the before-image persisted for THIS note now holds the OUTSIDE file's own bytes, pulled in through the race window's step-2 read, which DOES follow a symlink (unlike atomic_write's rename). If this assertion fails, step 2 no longer reads through a mid-race symlink swap and this read-side consequence no longer applies — re-evaluate the verdict rather than treating this as a test bug."
        );
    }

    #[test]
    fn sec_002_read_path_rejects_both_aliasing_shapes_but_write_path_pre_read_does_not() {
        // Renamed (independent-review finding M-2) from
        // `sec_002_journal_hash_check_does_not_detect_aliasing_present_before_the_write`:
        // that name asserted the opposite of what this test's Scenario A
        // actually proves — read_note's journal-first resolution REJECTS
        // the never-written-alias shape (NotFound), it does not silently
        // alias it. The new name describes all three scenarios this test
        // now covers: two on the read path (both rejected, for two
        // different reasons) and one on the write path (NOT rejected).
        //
        // Scenario A: note-a.record is a symlink to note-b.record BEFORE
        // any write to note-a ever happens. The ORIGINAL SEC-002 finding's
        // premise was that write_record_unchecked's own
        // hash-of-what-it-just-wrote check cannot detect this on a
        // subsequent read (it hashes content it itself produced and reads
        // back the same aliased path, so the hash matches by construction)
        // — but A2's read path resolves through the journal FIRST, before
        // any hash is even consulted, which closes this specific shape for
        // an unrelated, structural reason (see the assertion below).
        let root = unique_test_dir();
        init_git_repo_at(&root);
        let vault = Vault::open_with_default_indexer(root.clone(), &fresh_journal_path()).unwrap();
        let id_b = NoteId::parse("note-b").unwrap();
        vault.write_record_unchecked(&id_b, "note-b real content", Version(0), Uuid::new_v4()).unwrap();
        symlink_file(&root.join("note-b.record"), &root.join("note-a.record"));
        let id_a = NoteId::parse("note-a").unwrap();
        // record_path_checked's existing I-C fix (A1) already rejects a
        // symlink whose target is OUTSIDE the root; root/note-a.record ->
        // root/note-b.record resolves INSIDE the root, so I-C's fix does not
        // reject it — this is specifically the aliasing case I-C's own doc
        // comment says is out of its scope.
        //
        // DEVIATION FROM THE BRIEF, recorded rather than silently "fixed" to
        // match the prediction: the brief's code calls `read_note(&id_a)`
        // directly here and asserts it returns note-b's content. Measured
        // against the actual A2 read path, that call instead returns
        // `NotFound`, because `read_note_locked` resolves a note exclusively
        // through `Journal::last_committed_for_note(note_id)` — keyed by the
        // LOGICAL note id being read, never by "whatever bytes are at this
        // path" — and no journal row has ever been created under the key
        // "note-a" in this scenario (only "note-b" has one). This is a real,
        // reproducible narrowing the brief's own prediction did not
        // anticipate: a pure "alias a never-written note id to another
        // note's file" read is now rejected, not aliased.
        let never_written_alias_result = vault.read_note(&id_a);
        assert!(
            matches!(&never_written_alias_result, Err(StorageError::NotFound { note_id }) if note_id == "note-a"),
            "SEC-002 finding (narrower than predicted): aliasing a note id that was NEVER itself written is now rejected as NotFound by the journal lookup, not silently aliased — got {never_written_alias_result:?}. If this assertion fails, the journal's per-note-id resolution changed and this specific variant may no longer be rejected."
        );

        // A second, more realistic variant of the same exploit family: have
        // note-a-v2 reach its OWN committed journal row and hash first (a
        // real write through the normal path), THEN swap its on-disk file
        // for an in-root symlink aliasing note-b. This is the sub-case the
        // brief's narration ("the alias is in place from before the write
        // that created the journal row") does NOT literally cover — here
        // the write happens first, the alias second — included because it
        // is the shape an attacker with filesystem access after a note is
        // already committed would actually use.
        let id_a_v2 = NoteId::parse("note-a-v2").unwrap();
        vault.write_record_unchecked(&id_a_v2, "note-a-v2's own original content", Version(0), Uuid::new_v4()).unwrap();
        std::fs::remove_file(root.join("note-a-v2.record")).unwrap();
        symlink_file(&root.join("note-b.record"), &root.join("note-a-v2.record"));
        // FINDING (a genuine narrowing, not predicted by this task's plan):
        // read_note's hash check is unconditional and keyed to the resolved
        // row's OWN content_hash, computed at write time from note-a-v2's
        // ORIGINAL bytes. note-b's bytes hash differently, so swapping the
        // file for note-b's content after the fact is caught: the read
        // fails closed as Inconsistent rather than silently returning
        // note-b's content under note-a-v2's name. This is the SECOND
        // read-path sub-case A2 happens to catch (Scenario A above catches
        // the first). Both are caught by machinery that was not built to
        // detect aliasing as such (per-note-id journal keying; an
        // unconditional content-hash check), which is why this is a
        // NARROWING, not a purpose-built fix: a shape that defeats BOTH
        // incidental mechanisms — aliasing a note that already has a
        // committed row to content that happens to hash-IDENTICALLY to
        // what the row expects — would pass completely undetected (fixed
        // bytes in, identical hash out; the check cannot distinguish "the
        // original file" from "a different file with the same bytes"), and
        // the hash check is a fail-CLOSED integrity gate, not a security
        // boundary: it turns a successful alias into a denial of service
        // (Inconsistent) rather than silently accepting or cryptographically
        // proving the content is what it claims — it offers no defense at
        // all against an attacker who can also write to the journal
        // directly (the resolved row's `content_hash` is just a column a
        // sufficiently-privileged attacker could set to match anything).
        let result = vault.read_note(&id_a_v2);
        assert!(
            matches!(&result, Err(StorageError::Inconsistent { note_id }) if note_id == "note-a-v2"),
            "SEC-002 finding (this read-path sub-case is CLOSED): once a note has its own committed journal row, read_note's unconditional hash check rejects a later swap to DIFFERENT content as Inconsistent, rather than aliasing it — got {result:?}. If this assertion fails, that hash check stopped catching this swap and the verdict must be re-evaluated toward OPEN."
        );

        // Independent-review finding I-2: everything above is scoped to the
        // READ path (`read_note`). `write_record_unchecked`'s own step 2 —
        // the pre-write version-check read, which runs against whatever is
        // ACTUALLY at `record_path` right now, through any in-root symlink
        // sitting there — is a completely separate code path from
        // `read_note_locked`'s journal-first resolution, and it is NOT
        // gated by a per-note-id journal lookup at all: step 2 reads
        // `record_path` directly with `std::fs::read`, falls back to
        // `Version(0)` only on `NotFound`, and otherwise trusts whatever
        // version it parsed. Aliasing a NEVER-WRITTEN note id to an
        // ALREADY-COMMITTED different note's file, then writing through the
        // aliased id, reproduces the version-based consequences this test's
        // sibling `sec_001_toctou_read_side_leaks_an_outside_records_version_and_bytes_into_the_before_image`
        // demonstrated for an OUTSIDE file — but here entirely with ordinary,
        // no-race, in-root symlinks: no pause/resume timing is even needed,
        // because write_record_unchecked never re-validates that
        // `record_path` still points at a plain, un-aliased file once
        // `record_path_checked` approves it, and a plain in-root symlink is
        // simply never excluded by that check at all (it is explicitly
        // in scope — see `record_symlink_resolving_inside_the_vault_root_is_not_an_escape`
        // elsewhere in this module).
        let id_c = NoteId::parse("note-c").unwrap();
        symlink_file(&root.join("note-b.record"), &root.join("note-c.record"));
        // note-b is at Version(1) (its one write above). A write to the
        // never-written note-c, aliased to note-b, with expected_version
        // matching note-b's ACTUAL version, takes that path: step 2 reads
        // Version(1) through the alias, matches, and proceeds to persist
        // note-b's CURRENT bytes as note-c's before-image, then overwrites
        // the symlink with note-c's own new record (replaced exactly as
        // SEC-001's write-side scenario showed: the rename destroys the
        // symlink rather than writing through it, so note-b's own file is
        // untouched — confirmed by the post-write assertions below).
        let op_c = Uuid::new_v4();
        // Interrupted right after the before-image is persisted and the
        // rename lands (AfterRenameBeforeJournalCommitted), i.e. BEFORE
        // write_record_unchecked's later git-commit step would delete the
        // before-image on reaching GitCommitted (doc comment on step 7,
        // above) — this test wants to inspect the before-image the write
        // persisted, not watch it get cleaned up by normal completion.
        let interrupt_before_git = FaultInjector::trigger_at(FaultPoint::AfterRenameBeforeJournalCommitted);
        let write_through_alias = vault.write_record_unchecked_with_injector(&id_c, "note-c's new content", Version(1), op_c, &interrupt_before_git);
        assert!(
            matches!(write_through_alias, Err(StorageError::Interrupted(FaultPoint::AfterRenameBeforeJournalCommitted))),
            "the write's content/before-image steps must succeed (step 2 reads note-b's real Version(1) through the alias, matching expected_version) and reach exactly this fault point — got {write_through_alias:?}"
        );
        let before_image_bytes = read_before_image(&root, &id_c, op_c).expect("step 2 must have persisted note-b's bytes as note-c's before-image: it read them through the alias before overwriting");
        assert_eq!(
            before_image_bytes,
            serialize_record("note-b real content", Version(1)),
            "SEC-002 write-path finding: note-c's before-image now holds note-b's OWN content — a cross-note version/content leak through write_record_unchecked's step 2, which is NOT gated by the per-note-id journal lookup that protects read_note. This is the write-side half of SEC-002 that remains fully open."
        );
        assert_eq!(
            vault.read_note(&id_b).unwrap().content,
            "note-b real content",
            "note-b's own file must be untouched by note-c's write: the rename at the end of write_record_unchecked replaces the symlink at note-c.record, exactly as SEC-001's write-side scenario showed, rather than writing through it to note-b's actual file"
        );
    }

    #[test]
    fn sec_003_hard_link_aliasing_is_invisible_to_canonicalization_after_a2() {
        let root = unique_test_dir();
        init_git_repo_at(&root);
        let outside = unique_test_dir();
        let outside_file = outside.join("victim.record");
        std::fs::write(&outside_file, serialize_record_for_test("OUTSIDE", Version(7))).unwrap();
        let vault = Vault::open_with_default_indexer(root.clone(), &fresh_journal_path()).unwrap();
        std::fs::hard_link(&outside_file, root.join("note-a.record")).unwrap();
        // DEVIATION FROM THE BRIEF, recorded rather than silently "fixed" to
        // match the prediction: the brief's code calls
        // `read_note(&NoteId::parse("note-a").unwrap())` directly and
        // asserts it returns "OUTSIDE". Measured against the actual A2 read
        // path, this returns `NotFound`, for the same reason as SEC-002's
        // never-written-alias variant: `note-a` has no journal row at all
        // (nothing was ever written under that note id through
        // `write_record_unchecked`), so `last_committed_for_note("note-a")`
        // finds nothing and `read_note` never reaches the point of reading
        // the hard-linked bytes off disk. Canonicalization still cannot see
        // the hard link (unchanged from A1) — but the journal lookup now
        // gates every read before canonicalization's blind spot is ever
        // reached for a note id with no committed history.
        let result = vault.read_note(&NoteId::parse("note-a").unwrap());
        assert!(
            matches!(&result, Err(StorageError::NotFound { note_id }) if note_id == "note-a"),
            "SEC-003 finding (narrower than predicted): a hard link planted under a note id that was NEVER itself written is now rejected as NotFound by the journal lookup before canonicalization's blind spot matters — got {result:?}. If this assertion fails, the journal's per-note-id gating changed and this variant may again return the hard-linked OUTSIDE content."
        );

        // Second variant, symmetric with SEC-002's second variant and closer
        // to the ORIGINAL A1 finding's shape: note-a-v2 is written for real
        // first (its own committed row and hash exist), THEN its on-disk
        // file is replaced with a hard link to the outside victim file.
        // Canonicalization genuinely cannot distinguish the hard-linked
        // path from an ordinary file — there is no separate link to resolve,
        // unlike a symlink — so if anything still catches this, it can only
        // be the content-hash check, not the containment check.
        let id_a_v2 = NoteId::parse("note-a-v2").unwrap();
        vault.write_record_unchecked(&id_a_v2, "note-a-v2's own original content", Version(0), Uuid::new_v4()).unwrap();
        std::fs::remove_file(root.join("note-a-v2.record")).unwrap();
        std::fs::hard_link(&outside_file, root.join("note-a-v2.record")).unwrap();
        // FINDING: exactly like SEC-002's second variant, read_note's
        // unconditional hash check rejects this as Inconsistent (the
        // committed row's hash was computed over note-a-v2's ORIGINAL
        // bytes, not the OUTSIDE file's "OUTSIDE" content), rather than
        // returning the hard-linked victim content under note-a-v2's name.
        // This sub-case is CLOSED by the same mechanism as SEC-002's.
        let result2 = vault.read_note(&id_a_v2);
        assert!(
            matches!(&result2, Err(StorageError::Inconsistent { note_id }) if note_id == "note-a-v2"),
            "SEC-003 finding (this sub-case is CLOSED): once a note has its own committed journal row, read_note's unconditional hash check rejects a later hard-link swap to different content as Inconsistent — got {result2:?}. If this assertion fails, that hash check stopped catching this swap and the verdict must be re-evaluated toward OPEN."
        );
    }

    // SEC-00x status anchor (audit pointer, not an executable check).
    //
    // Current status per Continuity: [d5b9] SEC-001 open, [c16c6] SEC-002
    // narrowed, [c17e8] SEC-003 narrowed.
    //
    // The drift oracles for this status are the four existing tests below,
    // each of which already panics with "re-evaluate the verdict" when its
    // observed behavior changes:
    //   - sec_001_toctou_window_between_path_check_and_write_still_exists
    //     (vault.rs, ~line 3052)
    //   - sec_001_toctou_read_side_leaks_an_outside_records_version_and_bytes_into_the_before_image
    //     (vault.rs, ~line 3118)
    //   - sec_002_read_path_rejects_both_aliasing_shapes_but_write_path_pre_read_does_not
    //     (vault.rs, ~line 3204)
    //   - sec_003_hard_link_aliasing_is_invisible_to_canonicalization_after_a2
    //     (vault.rs, ~line 3356)
    //
    // This comment only exists so a human or future change can find the
    // status by searching for "SEC-00x status". It does not re-verify
    // anything. If any of those four tests is renamed, moved, or changes
    // behavior, update this comment in the same change, together with the
    // Continuity record for the status.

    // --- Task 6: the single, crash-recoverable sensitive-write pipeline ---
    //
    // These tests exercise `write_sensitive_record_unchecked`/
    // `_with_injector` directly (crate-private, so they must live here
    // rather than in an external `tests/*.rs` file — see
    // `a3_pii_integration.rs`'s module doc comment for the split). The
    // public-surface happy-path and orphan-detection tests
    // (`a_sensitive_note_is_written_to_the_pii_store_...`, etc.) live in
    // `crates/storage/tests/a3_pii_integration.rs` instead, driven through
    // `propose_and_commit` alone.
    mod a3_sensitive_write_pipeline {
        use super::*;
        use vault_security::PiiStore;

        fn unique_dir() -> PathBuf {
            static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "superbrain-a3-sensitive-pipeline-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
                COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&dir).unwrap();
            dir
        }

        fn open_pii_store(vault_root: &Path) -> PiiStore {
            let pii_root = vault_root.join(".pii-store");
            PiiStore::open(pii_root.clone(), &pii_root.join("master.key")).unwrap()
        }

        fn pointer_content_for(note_id: &str) -> String {
            format!(
                "id: {note_id}\nschema_version: 1\nclassification: sensitive\n---\npii://{note_id}"
            )
        }

        #[test]
        fn crash_after_pii_staged_before_pointer_committed_recovers_without_a_readable_half_written_pointer() {
            let root = crate::git::test_support::init_test_repo();
            let journal_path = unique_dir().join("journal.sqlite");
            let pii_store = open_pii_store(&root);
            let id = NoteId::parse("note-crash").unwrap();
            let op = Uuid::new_v4();
            let pointer_content = pointer_content_for("note-crash");

            {
                let vault = Vault::open_with_pii_store(root.clone(), &journal_path, &pii_store).unwrap();
                let injector = FaultInjector::trigger_at(FaultPoint::AfterPiiStagedBeforeCommit);
                let result = vault.write_sensitive_record_unchecked_with_injector(
                    &id,
                    &pointer_content,
                    b"secret content v1",
                    Version(0),
                    op,
                    &pii_store,
                    None,
                    &injector,
                );
                assert!(
                    matches!(result, Err(StorageError::Interrupted(FaultPoint::AfterPiiStagedBeforeCommit))),
                    "got {result:?}"
                );

                // (a) Before recovery runs, the note must never read as a
                // pointer to ciphertext that doesn't exist yet: this is a
                // first write, so the only correct observable state is
                // NotFound.
                let before_recovery = vault.read_note(&id);
                assert!(
                    matches!(before_recovery, Err(StorageError::NotFound { .. })),
                    "a note that never existed before must read NotFound, not a dangling pointer, got {before_recovery:?}"
                );
            }

            // (b) After Vault::open (reconcile) runs, the operation resolves
            // deterministically — either rolled forward (pointer now
            // committed, PiiStore::get works) or rolled back (Failed, no
            // orphaned staged ciphertext left behind) — never left
            // ambiguous.
            let reopened = Vault::open_with_pii_store(root, &journal_path, &pii_store).unwrap();
            let after = reopened.read_note(&id);
            match after {
                Ok(record) => {
                    assert!(record.content.contains("pii://note-crash"));
                    let plaintext = pii_store.get("note-crash").expect("rolled forward: ciphertext must be readable");
                    assert_eq!(plaintext, b"secret content v1");
                }
                Err(StorageError::NotFound { .. }) => {
                    assert!(!pii_store.has_live("note-crash"), "rolled back: no live ciphertext must survive");
                    assert!(
                        pii_store.get_staged("note-crash", op).is_err(),
                        "rolled back: no orphaned staged ciphertext must survive either"
                    );
                }
                other => panic!("must resolve deterministically to either a readable pointer or NotFound, got {other:?}"),
            }
        }

        #[test]
        fn crash_mid_update_of_an_already_sensitive_note_never_exposes_a_pointer_whose_ciphertext_is_the_wrong_version() {
            // The single most important test in this task (Review Focus item
            // 2, update path): write v1 fully, start an update to v2 with an
            // injector at AfterPiiStagedBeforeCommit, and verify that at
            // every observable point — mid-crash and after recovery
            // completes — read_note plus PiiStore::get together are
            // self-consistent: either the full v1 pair (pointer + v1
            // ciphertext) or the full v2 pair, NEVER a v2 pointer reading
            // back v1 ciphertext or vice versa.
            let root = crate::git::test_support::init_test_repo();
            let journal_path = unique_dir().join("journal.sqlite");
            let pii_store = open_pii_store(&root);
            let id = NoteId::parse("note-update").unwrap();
            let pointer_content = pointer_content_for("note-update");

            // v1 fully committed.
            {
                let vault = Vault::open_with_pii_store(root.clone(), &journal_path, &pii_store).unwrap();
                let op1 = Uuid::new_v4();
                vault
                    .write_sensitive_record_unchecked(&id, &pointer_content, b"v1 secret", Version(0), op1, &pii_store, None)
                    .unwrap();
            }
            let v1_record = {
                let vault = Vault::open_with_pii_store(root.clone(), &journal_path, &pii_store).unwrap();
                vault.read_note(&id).unwrap()
            };
            assert_eq!(pii_store.get("note-update").unwrap(), b"v1 secret");

            // Start an update to v2, crashing at AfterPiiStagedBeforeCommit:
            // the new ciphertext is durably staged, but the live `.pii` has
            // NOT been updated and the pointer has NOT been renamed.
            let op2 = Uuid::new_v4();
            {
                let vault = Vault::open_with_pii_store(root.clone(), &journal_path, &pii_store).unwrap();
                let injector = FaultInjector::trigger_at(FaultPoint::AfterPiiStagedBeforeCommit);
                let result = vault.write_sensitive_record_unchecked_with_injector(
                    &id,
                    &pointer_content,
                    b"v2 secret",
                    v1_record.version,
                    op2,
                    &pii_store,
                    None,
                    &injector,
                );
                assert!(matches!(result, Err(StorageError::Interrupted(FaultPoint::AfterPiiStagedBeforeCommit))));

                // Mid-crash: the pointer must still read as v1, and its
                // ciphertext must still be v1's. Never a v2 pointer / v1
                // ciphertext mismatch, and never a v1 pointer / v2
                // ciphertext mismatch.
                let mid_crash_record = vault.read_note(&id).unwrap();
                assert_eq!(mid_crash_record.content, v1_record.content, "mid-crash: pointer must still be v1's");
                assert_eq!(pii_store.get("note-update").unwrap(), b"v1 secret", "mid-crash: ciphertext must still be v1's");
            }

            // After recovery: still self-consistent, either fully v1 or
            // fully v2 — never a mismatched pairing.
            let reopened = Vault::open_with_pii_store(root, &journal_path, &pii_store).unwrap();
            let after_record = reopened.read_note(&id).unwrap();
            let after_plaintext = pii_store.get("note-update").unwrap();
            if after_record.version == v1_record.version {
                assert_eq!(after_record.content, v1_record.content, "rolled back: pointer must be exactly v1's");
                assert_eq!(after_plaintext, b"v1 secret", "rolled back: ciphertext must be exactly v1's");
            } else {
                assert_eq!(after_plaintext, b"v2 secret", "rolled forward: ciphertext must be exactly v2's");
                assert!(after_record.content.contains("pii://note-update"));
            }
        }

        #[test]
        fn before_image_of_a_sensitive_notes_prior_pointer_never_holds_plaintext_while_it_exists() {
            // Part of the A3 confidentiality-boundary audit (Task 6): proves
            // the before-image persisted DURING a sensitive-note update is
            // the pointer-only record, never the real content, at the exact
            // moment it exists on disk (paused via
            // `AfterJournalCommittedBeforeGit` -- the latest point before
            // `commit_to_git` deletes the before-image once `GitCommitted`
            // is reached -- so this is a true mid-write observation, not an
            // after-the-fact steady-state check like the public-surface
            // test in `a3_confidentiality_boundary.rs`).
            let root = crate::git::test_support::init_test_repo();
            let journal_path = unique_dir().join("journal.sqlite");
            let pii_store = open_pii_store(&root);
            let id = NoteId::parse("note-before-image-pause").unwrap();
            let pointer_content = pointer_content_for("note-before-image-pause");

            let vault = Vault::open_with_pii_store(root.clone(), &journal_path, &pii_store).unwrap();
            vault
                .write_sensitive_record_unchecked(&id, &pointer_content, b"v1 plaintext secret", Version(0), Uuid::new_v4(), &pii_store, None)
                .unwrap();

            let (paused_tx, paused_rx) = std::sync::mpsc::channel();
            let (resume_tx, resume_rx) = std::sync::mpsc::channel();
            let injector = FaultInjector::pause_at(FaultPoint::AfterJournalCommittedBeforeGit, paused_tx, resume_rx);
            let op2 = Uuid::new_v4();
            let vault_arc = std::sync::Arc::new(vault);
            let pii_store_arc = std::sync::Arc::new(pii_store);
            let (v, p, pc) = (std::sync::Arc::clone(&vault_arc), std::sync::Arc::clone(&pii_store_arc), pointer_content.clone());
            let id2 = id.clone();
            let handle = std::thread::spawn(move || {
                v.write_sensitive_record_unchecked_with_injector(&id2, &pc, b"v2 plaintext secret", Version(1), op2, &p, None, &injector)
            });

            paused_rx.recv().unwrap();
            let before_image_path = root.join(".before-images").join(format!("note-before-image-pause-{op2}.record"));
            let bytes = std::fs::read(&before_image_path).expect("the before-image must exist while paused");
            let as_text = String::from_utf8_lossy(&bytes);
            assert!(!as_text.contains("v1 plaintext secret"), "the before-image must hold the pointer, not v1's plaintext, got {as_text:?}");
            assert!(!as_text.contains("v2 plaintext secret"), "nor the new write's plaintext, got {as_text:?}");
            assert!(as_text.contains("pii://note-before-image-pause"), "got {as_text:?}");

            resume_tx.send(()).unwrap();
            handle.join().unwrap().unwrap();
        }

        #[test]
        fn reconcile_itself_can_be_interrupted_mid_fixup_of_a_sensitive_rows_pii_content_hash_check() {
            // Uses `open_with_pii_store_and_injector` to interrupt
            // `recovery::reconcile` itself (not the original write) right
            // after it marks a stalled sensitive row `Recovering`, mirroring
            // `recovery.rs`'s own `reconcile_marks_a_row_recovering_...`
            // test but with a `PiiStore` threaded through. Proves the PII
            // verification branch is reached via the SAME generic
            // `Recovering`-marking step every other row goes through, not a
            // special-cased shortcut, and that a second, uninterrupted pass
            // still resolves it afterward.
            let root = crate::git::test_support::init_test_repo();
            let journal_path = unique_dir().join("journal.sqlite");
            let pii_store = open_pii_store(&root);
            let id = NoteId::parse("note-reconcile-pause").unwrap();
            let pointer_content = pointer_content_for("note-reconcile-pause");
            let op = Uuid::new_v4();

            {
                let vault = Vault::open_with_pii_store(root.clone(), &journal_path, &pii_store).unwrap();
                let injector = FaultInjector::trigger_at(FaultPoint::AfterPiiStagedBeforeCommit);
                let result = vault.write_sensitive_record_unchecked_with_injector(
                    &id, &pointer_content, b"secret", Version(0), op, &pii_store, None, &injector,
                );
                assert!(matches!(result, Err(StorageError::Interrupted(FaultPoint::AfterPiiStagedBeforeCommit))));
            }

            let interrupt_recovery = FaultInjector::trigger_at(FaultPoint::AfterMarkedRecoveringBeforeFixup);
            let interrupted = Vault::open_with_pii_store_and_injector(root.clone(), &journal_path, &pii_store, &interrupt_recovery);
            assert!(matches!(interrupted, Err(StorageError::Interrupted(FaultPoint::AfterMarkedRecoveringBeforeFixup))));

            // A second, uninterrupted pass (ordinary open) must still resolve
            // the row deterministically.
            let reopened = Vault::open_with_pii_store(root, &journal_path, &pii_store).unwrap();
            let after = reopened.read_note(&id);
            match after {
                Ok(record) => {
                    assert!(record.content.contains("pii://note-reconcile-pause"));
                    assert_eq!(pii_store.get("note-reconcile-pause").unwrap(), b"secret");
                }
                Err(StorageError::NotFound { .. }) => {
                    assert!(!pii_store.has_live("note-reconcile-pause"));
                }
                other => panic!("must resolve deterministically, got {other:?}"),
            }
        }

        #[test]
        fn crash_after_pii_committed_before_pointer_rename_rolls_back_cleanly_on_a_first_write() {
            // The second new FaultPoint, on a first write: the live `.pii`
            // file exists, but the pointer rename never ran. Recovery must
            // never leave a `.pii` file with no corresponding pointer at
            // all for a note that never had one before this operation.
            let root = crate::git::test_support::init_test_repo();
            let journal_path = unique_dir().join("journal.sqlite");
            let pii_store = open_pii_store(&root);
            let id = NoteId::parse("note-commit-crash").unwrap();
            let op = Uuid::new_v4();
            let pointer_content = pointer_content_for("note-commit-crash");

            {
                let vault = Vault::open_with_pii_store(root.clone(), &journal_path, &pii_store).unwrap();
                let injector = FaultInjector::trigger_at(FaultPoint::AfterPiiCommittedBeforePointerRename);
                let result = vault.write_sensitive_record_unchecked_with_injector(
                    &id,
                    &pointer_content,
                    b"secret content",
                    Version(0),
                    op,
                    &pii_store,
                    None,
                    &injector,
                );
                assert!(matches!(result, Err(StorageError::Interrupted(FaultPoint::AfterPiiCommittedBeforePointerRename))));
                // The live ciphertext IS committed at this exact point.
                assert!(pii_store.has_live("note-commit-crash"));
                assert!(matches!(vault.read_note(&id), Err(StorageError::NotFound { .. })), "no pointer exists yet");
            }

            let reopened = Vault::open_with_pii_store(root, &journal_path, &pii_store).unwrap();
            let after = reopened.read_note(&id);
            match after {
                Ok(record) => {
                    assert!(record.content.contains("pii://note-commit-crash"));
                    assert_eq!(pii_store.get("note-commit-crash").unwrap(), b"secret content");
                }
                Err(StorageError::NotFound { .. }) => {
                    assert!(!pii_store.has_live("note-commit-crash"), "rolled back: the orphaned live ciphertext must be removed too");
                }
                other => panic!("must resolve deterministically, got {other:?}"),
            }
        }

        #[test]
        fn crash_after_pii_committed_before_pointer_rename_on_an_update_never_loses_or_mispairs_committed_content() {
            // A3 review round, CRITICAL fix verification: on an UPDATE,
            // `AfterPiiCommittedBeforePointerRename` fires AFTER
            // `commit_staged` has already overwritten the version-agnostic
            // live `<note_id>.pii` file with v2's ciphertext — unlike the
            // AfterPiiStagedBeforeCommit tests above, where the live file is
            // still untouched. Before the before-image fix, recovery's
            // `fix_intent_recorded` would find the pointer still at v1 (the
            // rename never ran) and DELETE the live `.pii` file outright,
            // permanently destroying v1's committed content — the exact
            // "v1 pointer, v2 (or no) ciphertext" pairing this task's
            // design forbids. This test asserts, mid-crash AND after
            // recovery, that only the full v1 pair or the full v2 pair is
            // EVER observable, and that v1's committed content is never
            // lost.
            let root = crate::git::test_support::init_test_repo();
            let journal_path = unique_dir().join("journal.sqlite");
            let pii_store = open_pii_store(&root);
            let id = NoteId::parse("note-upd-commit-crash").unwrap();
            let pointer_content = pointer_content_for("note-upd-commit-crash");

            // v1 fully committed.
            {
                let vault = Vault::open_with_pii_store(root.clone(), &journal_path, &pii_store).unwrap();
                let op1 = Uuid::new_v4();
                vault
                    .write_sensitive_record_unchecked(&id, &pointer_content, b"v1 secret", Version(0), op1, &pii_store, None)
                    .unwrap();
            }
            let v1_record = {
                let vault = Vault::open_with_pii_store(root.clone(), &journal_path, &pii_store).unwrap();
                vault.read_note(&id).unwrap()
            };
            assert_eq!(pii_store.get("note-upd-commit-crash").unwrap(), b"v1 secret");

            // Start an update to v2, crashing at
            // AfterPiiCommittedBeforePointerRename: commit_staged has ALREADY
            // overwritten the live file with v2's ciphertext, but the
            // pointer rename has NOT run — the pointer still reads v1.
            let op2 = Uuid::new_v4();
            {
                let vault = Vault::open_with_pii_store(root.clone(), &journal_path, &pii_store).unwrap();
                let injector = FaultInjector::trigger_at(FaultPoint::AfterPiiCommittedBeforePointerRename);
                let result = vault.write_sensitive_record_unchecked_with_injector(
                    &id,
                    &pointer_content,
                    b"v2 secret",
                    v1_record.version,
                    op2,
                    &pii_store,
                    None,
                    &injector,
                );
                assert!(matches!(result, Err(StorageError::Interrupted(FaultPoint::AfterPiiCommittedBeforePointerRename))));

                // MID-CRASH: the pointer reads v1, but the live ciphertext
                // is ALREADY v2's — this is the exact mismatched state the
                // reviewer's repro observed. It is allowed to exist
                // TRANSIENTLY (that is what "crash recovery" means), but it
                // must never be the FINAL, resolved state, and v1's content
                // must still be recoverable at this very moment via the PII
                // before-image.
                let mid_crash_record = vault.read_note(&id).unwrap();
                assert_eq!(mid_crash_record.content, v1_record.content, "mid-crash: pointer must still read v1");
                assert_eq!(pii_store.get("note-upd-commit-crash").unwrap(), b"v2 secret", "mid-crash: live ciphertext is already v2's (this is the transient mismatch recovery must resolve)");
                assert!(pii_store.has_before_image("note-upd-commit-crash", op2), "v1's content must still be recoverable as op2's PII before-image at this exact crash point");
            }

            // AFTER RECOVERY: must resolve to EXACTLY ONE of the two valid
            // pairs — never a pointer with no ciphertext, never a v1
            // pointer paired with v2's ciphertext, never v1's content lost.
            let reopened = Vault::open_with_pii_store(root, &journal_path, &pii_store).unwrap();
            let after_record = reopened.read_note(&id).expect("the pointer must always be readable after recovery — never NotFound, since v1 was already fully committed before this crash");
            let after_plaintext = pii_store.get("note-upd-commit-crash").expect("ciphertext must always be readable after recovery — v1's content must never be permanently lost");
            if after_record.version == v1_record.version {
                assert_eq!(after_record.content, v1_record.content, "rolled back: pointer must be exactly v1's");
                assert_eq!(after_plaintext, b"v1 secret", "rolled back: ciphertext must be restored to exactly v1's — this is the CRITICAL assertion: before the fix, this read NotFound because the live file was deleted outright instead of restored from the before-image");
            } else {
                assert_eq!(after_plaintext, b"v2 secret", "rolled forward: ciphertext must be exactly v2's");
                assert!(after_record.content.contains("pii://note-upd-commit-crash"));
            }
            // No orphaned staging or before-image artifacts survive either outcome.
            assert!(pii_store.get_staged("note-upd-commit-crash", op2).is_err());
            assert!(!pii_store.has_before_image("note-upd-commit-crash", op2));
        }

        #[test]
        fn opening_without_a_pii_store_fails_closed_on_a_stalled_sensitive_row_instead_of_silently_rolling_back() {
            // A3 review round, IMPORTANT finding #5: `Vault::open_with_default_indexer`
            // (no `PiiStore` configured for the reconcile pass) must refuse
            // to open at all when a non-terminal row carries a
            // `pii_content_hash` — never silently resolve it (forward or
            // back) without ever touching `.pii`, which could leave `.pii`
            // and the Markdown pointer permanently out of sync with no
            // error ever raised.
            let root = crate::git::test_support::init_test_repo();
            let journal_path = unique_dir().join("journal.sqlite");
            let pii_store = open_pii_store(&root);
            let id = NoteId::parse("note-no-store-stall").unwrap();
            let op = Uuid::new_v4();
            let pointer_content = pointer_content_for("note-no-store-stall");

            {
                let vault = Vault::open_with_pii_store(root.clone(), &journal_path, &pii_store).unwrap();
                let injector = FaultInjector::trigger_at(FaultPoint::AfterPiiStagedBeforeCommit);
                let result = vault.write_sensitive_record_unchecked_with_injector(
                    &id,
                    &pointer_content,
                    b"secret",
                    Version(0),
                    op,
                    &pii_store,
                    None,
                    &injector,
                );
                assert!(matches!(result, Err(StorageError::Interrupted(FaultPoint::AfterPiiStagedBeforeCommit))));
            }

            // Reopening WITHOUT a PiiStore must fail closed, never silently succeed.
            let result = Vault::open_with_default_indexer(root, &journal_path);
            let described = result.as_ref().map(|_| "Ok(Vault)");
            assert!(
                matches!(result, Err(StorageError::Inconsistent { .. })),
                "opening without a PiiStore over a stalled sensitive row must fail closed as Inconsistent, got {described:?}"
            );
        }

        #[test]
        fn a_successful_roll_forward_recovery_deletes_the_pii_before_image_on_an_ordinary_update() {
            // A3 review round 2, NEW finding #1 (ordinary-update half): every
            // successful roll-forward path must delete the PII before-image
            // once the operation reaches its point of no return, mirroring
            // exactly how the Markdown before-image is deleted once
            // GitCommitted is reached. Before this fix, `fix_journal_committed`
            // took no `pii_store` parameter at all, so a row resumed from
            // JournalCommitted (crash after JournalCommitted, before
            // GitCommitted) rolled all the way forward to IndexCurrent
            // without ever deleting its PII before-image.
            let root = crate::git::test_support::init_test_repo();
            let journal_path = unique_dir().join("journal.sqlite");
            let pii_store = open_pii_store(&root);
            let id = NoteId::parse("note-before-image-cleanup").unwrap();
            let pointer_content = pointer_content_for("note-before-image-cleanup");

            // v1 fully committed.
            {
                let vault = Vault::open_with_pii_store(root.clone(), &journal_path, &pii_store).unwrap();
                let op1 = Uuid::new_v4();
                vault
                    .write_sensitive_record_unchecked(&id, &pointer_content, b"v1 secret", Version(0), op1, &pii_store, None)
                    .unwrap();
            }
            let v1_version = {
                let vault = Vault::open_with_pii_store(root.clone(), &journal_path, &pii_store).unwrap();
                vault.read_note(&id).unwrap().version
            };

            // v2 update, interrupted at AfterJournalCommittedBeforeGit: the
            // PII before-image (op2's copy of v1's ciphertext) exists at
            // this point, and must be cleaned up once reconcile rolls this
            // row all the way forward through GitCommitted to IndexCurrent.
            let op2 = Uuid::new_v4();
            {
                let vault = Vault::open_with_pii_store(root.clone(), &journal_path, &pii_store).unwrap();
                let injector = FaultInjector::trigger_at(FaultPoint::AfterJournalCommittedBeforeGit);
                let result = vault.write_sensitive_record_unchecked_with_injector(
                    &id,
                    &pointer_content,
                    b"v2 secret",
                    v1_version,
                    op2,
                    &pii_store,
                    None,
                    &injector,
                );
                assert!(matches!(result, Err(StorageError::Interrupted(FaultPoint::AfterJournalCommittedBeforeGit))));
                assert!(pii_store.has_before_image("note-before-image-cleanup", op2), "the before-image must exist at this crash point");
            }

            let reopened = Vault::open_with_pii_store(root, &journal_path, &pii_store).unwrap();
            let after = reopened.read_note(&id).unwrap();
            assert!(after.content.contains("pii://note-before-image-cleanup"));
            assert_eq!(pii_store.get("note-before-image-cleanup").unwrap(), b"v2 secret", "rolled forward to v2");

            // THE ASSERTION THAT MATTERS: the before-image must be gone.
            assert!(
                !pii_store.has_before_image("note-before-image-cleanup", op2),
                "the PII before-image must be deleted once the operation successfully reaches IndexCurrent — it must never survive a successful roll-forward"
            );
        }

        #[test]
        fn a_successful_roll_forward_recovery_deletes_the_pii_before_image_on_a_tombstone_deletion() {
            // A3 review round 2, NEW finding #1 (the finding's actual point):
            // for a TOMBSTONE/deletion write specifically, `commit_staged`
            // saves the note's REAL (now supposedly deleted) ciphertext as a
            // before-image before overwriting the live file with the empty
            // tombstone ciphertext. If that before-image is never purged,
            // the deleted sensitive content survives its own deletion, still
            // decryptable by the live master key — directly undermining
            // spec contract C5. This is the test that actually matters for
            // the spec guarantee.
            let root = crate::git::test_support::init_test_repo();
            let journal_path = unique_dir().join("journal.sqlite");
            let pii_store = open_pii_store(&root);
            let id = NoteId::parse("note-tombstone-cleanup").unwrap();
            let pointer_content = pointer_content_for("note-tombstone-cleanup");

            // v1 fully committed with real sensitive content.
            {
                let vault = Vault::open_with_pii_store(root.clone(), &journal_path, &pii_store).unwrap();
                let op1 = Uuid::new_v4();
                vault
                    .write_sensitive_record_unchecked(&id, &pointer_content, b"REAL SECRET TO BE DELETED", Version(0), op1, &pii_store, None)
                    .unwrap();
            }
            let v1_version = {
                let vault = Vault::open_with_pii_store(root.clone(), &journal_path, &pii_store).unwrap();
                vault.read_note(&id).unwrap().version
            };

            // Tombstone write (empty plaintext, mirroring propose.rs's
            // commit_sensitive for a deletion), with the deletion log
            // recorded via PendingDeletion, interrupted at
            // AfterJournalCommittedBeforeGit — the before-image (the real,
            // now-deleted content) exists at this exact point.
            let deletion_log_path = pii_store.root().join("deletion-log.sqlite");
            let deletion_log = vault_security::DeletionLog::open(&deletion_log_path).unwrap();
            let trusted_ref_path = pii_store.root().join("deletion-log-reference.dpapi");
            let trusted_ref = vault_security::DpapiTrustedReferenceStore::new(trusted_ref_path);
            let op2 = Uuid::new_v4();
            {
                let vault = Vault::open_with_pii_store(root.clone(), &journal_path, &pii_store).unwrap();
                let injector = FaultInjector::trigger_at(FaultPoint::AfterJournalCommittedBeforeGit);
                let deletion = PendingDeletion { log: &deletion_log, trusted_ref: &trusted_ref };
                let result = vault.write_sensitive_record_unchecked_with_injector(
                    &id,
                    &pointer_content,
                    b"",
                    v1_version,
                    op2,
                    &pii_store,
                    Some(deletion),
                    &injector,
                );
                assert!(matches!(result, Err(StorageError::Interrupted(FaultPoint::AfterJournalCommittedBeforeGit))));
                assert!(
                    pii_store.has_before_image("note-tombstone-cleanup", op2),
                    "the before-image (the real, now-deleted content) must exist at this crash point"
                );
                assert!(deletion_log.is_deleted("note-tombstone-cleanup").unwrap(), "the deletion must already be logged");
            }

            let reopened = Vault::open_with_pii_store(root, &journal_path, &pii_store).unwrap();
            let after = reopened.read_note(&id).unwrap();
            assert!(after.content.contains("pii://note-tombstone-cleanup"));
            assert_eq!(pii_store.get("note-tombstone-cleanup").unwrap(), b"", "the live ciphertext is now the empty tombstone");

            // THE ASSERTION THAT ACTUALLY MATTERS FOR C5: the before-image
            // holding the REAL, deleted content must be gone — it must not
            // be decryptable by the live master key after a successful
            // deletion completes.
            assert!(
                !pii_store.has_before_image("note-tombstone-cleanup", op2),
                "the PII before-image holding the REAL, now-deleted content must be purged once the tombstone write successfully completes — \
                 deleted sensitive content must never survive its own deletion as a recoverable before-image"
            );
        }

        #[test]
        fn live_write_step_7_verify_mismatch_on_an_update_rolls_back_cleanly_in_process_without_a_pii_store_error() {
            // A3 review round 2, item #2: the LIVE-WRITE step-7 verify-
            // mismatch rollback (inside write_sensitive_record_unchecked
            // itself, synchronous, no crash/reopen involved) must look up
            // the PRIOR operation's pii_content_hash to verify the PII
            // before-image, never this failing operation's OWN hash (which
            // describes the NEW content, not what the before-image holds).
            // Before the fix, this call always failed with
            // StorageError::PiiStore("...Inconsistent...") on an update,
            // even though the rollback should succeed cleanly.
            let root = crate::git::test_support::init_test_repo();
            let journal_path = unique_dir().join("journal.sqlite");
            let pii_store = open_pii_store(&root);
            let id = NoteId::parse("note-live-rollback").unwrap();
            let pointer_content = pointer_content_for("note-live-rollback");

            let vault = std::sync::Arc::new(Vault::open_with_pii_store(root.clone(), &journal_path, &pii_store).unwrap());
            vault
                .write_sensitive_record_unchecked(&id, &pointer_content, b"v1 secret", Version(0), Uuid::new_v4(), &pii_store, None)
                .unwrap();

            // Pause right after the pointer rename lands but before the
            // verify-and-advance step (AfterRenameBeforeJournalCommitted):
            // commit_staged has ALREADY run (the live .pii is v2's, and
            // op2's before-image holds v1's), and the pointer rename has
            // ALREADY happened too — this is exactly the window step 7's
            // hash check examines next.
            let (paused_tx, paused_rx) = std::sync::mpsc::channel();
            let (resume_tx, resume_rx) = std::sync::mpsc::channel();
            let injector = FaultInjector::pause_at(FaultPoint::AfterRenameBeforeJournalCommitted, paused_tx, resume_rx);
            let op2 = Uuid::new_v4();
            let writer_vault = std::sync::Arc::clone(&vault);
            let writer_id = id.clone();
            let writer_pointer_content = pointer_content.clone();
            let writer_pii_store = open_pii_store(&root); // separate handle, same on-disk store
            let writer = std::thread::spawn(move || {
                writer_vault.write_sensitive_record_unchecked_with_injector(
                    &writer_id,
                    &writer_pointer_content,
                    b"v2 secret",
                    Version(1),
                    op2,
                    &writer_pii_store,
                    None,
                    &injector,
                )
            });
            paused_rx.recv_timeout(std::time::Duration::from_secs(10)).expect("writer must reach the post-rename boundary");
            assert!(pii_store.has_before_image("note-live-rollback", op2), "commit_staged must have already run and preserved v1's ciphertext");
            // Damage the pointer file now, forcing step 7's hash check to fail.
            std::fs::write(root.join("note-live-rollback.record"), b"TORN WRITE").unwrap();
            resume_tx.send(()).unwrap();

            let result = writer.join().unwrap();
            assert!(
                matches!(result, Err(StorageError::Inconsistent { .. })),
                "the writer must report a clean Inconsistent (the write did not land), not a confusing PiiStore error, got {result:?}"
            );
            assert_eq!(vault.journal_row_for_test(op2).unwrap().state, WriteState::Failed);

            // The pointer must have rolled back to v1, and the live
            // ciphertext must have been restored to v1's as well — a clean
            // in-process rollback, no crash or reopen needed.
            assert_eq!(vault.read_note(&id).unwrap().content, pointer_content);
            assert_eq!(pii_store.get("note-live-rollback").unwrap(), b"v1 secret");
            assert!(!pii_store.has_before_image("note-live-rollback", op2), "the before-image must be consumed by the successful restore");
        }

        // ---- A3 Task 6, review round 3: fail-closed PII before-image handling ----

        /// The PII store's live-file path for `note_id`, mirroring
        /// `PiiStore::live_path` (private to `vault-security`).
        fn live_pii_path(pii_store: &PiiStore, note_id: &str) -> PathBuf {
            pii_store.root().join(format!("{note_id}.pii"))
        }

        /// Opens `path` with a sharing mode that refuses every other read
        /// (and write) handle, so any `std::fs::read` of it fails with a
        /// sharing violation (an `io::Error` that is NOT `NotFound`) for as
        /// long as the returned handle lives. This is the reviewer's repro
        /// technique for round 2's "not ours" regression: a transient,
        /// real-world read failure (backup agent, antivirus scanner, ...).
        #[cfg(windows)]
        fn hold_without_read_share(path: &Path) -> std::fs::File {
            use std::os::windows::fs::OpenOptionsExt;
            std::fs::OpenOptions::new().read(true).share_mode(0).open(path).unwrap()
        }

        /// Writes v1 fully, then starts a v2 update interrupted at `point`.
        /// Returns `(root, journal_path, pii_store, id, pointer_content, op2)`.
        fn v1_committed_then_v2_interrupted_at(
            note: &str,
            point: FaultPoint,
        ) -> (PathBuf, PathBuf, PiiStore, NoteId, String, Uuid) {
            let root = crate::git::test_support::init_test_repo();
            let journal_path = unique_dir().join("journal.sqlite");
            let pii_store = open_pii_store(&root);
            let id = NoteId::parse(note).unwrap();
            let pointer_content = pointer_content_for(note);
            let op2 = Uuid::new_v4();
            {
                let vault = Vault::open_with_pii_store(root.clone(), &journal_path, &pii_store).unwrap();
                vault
                    .write_sensitive_record_unchecked(&id, &pointer_content, b"v1 secret", Version(0), Uuid::new_v4(), &pii_store, None)
                    .unwrap();
                let injector = FaultInjector::trigger_at(point);
                let result = vault.write_sensitive_record_unchecked_with_injector(
                    &id, &pointer_content, b"v2 secret", Version(1), op2, &pii_store, None, &injector,
                );
                assert!(matches!(result, Err(StorageError::Interrupted(p)) if p == point), "got {result:?}");
                assert!(pii_store.has_before_image(note, op2), "op2's PII before-image (v1's ciphertext) must exist at this crash point");
            }
            (root, journal_path, pii_store, id, pointer_content, op2)
        }

        fn journal_state(journal_path: &Path, op: Uuid) -> WriteState {
            Journal::open(journal_path).unwrap().get(op).unwrap().unwrap().state
        }

        #[test]
        fn round3_1a_reopening_without_a_pii_store_over_a_git_pending_sensitive_row_fails_closed_instead_of_orphaning_its_before_image() {
            // Item 1(a): row 3 (`fix_journal_committed`) used to skip the PII
            // before-image delete when no PiiStore was supplied, yet still
            // advanced the row to GitCommitted/IndexCurrent — the before-image
            // (v1's real content) was orphaned forever under a terminal row.
            let (root, journal_path, pii_store, id, _pc, op2) =
                v1_committed_then_v2_interrupted_at("note-r3-1a", FaultPoint::AfterJournalCommittedBeforeGit);

            let result = Vault::open_with_default_indexer(root.clone(), &journal_path);
            assert!(
                matches!(result, Err(StorageError::Inconsistent { .. })),
                "no PiiStore to clean up a sensitive row's before-image: must fail closed, got {:?}",
                result.as_ref().map(|_| "Ok(Vault)")
            );
            assert!(pii_store.has_before_image("note-r3-1a", op2), "nothing may be deleted on the fail-closed path");
            assert_ne!(journal_state(&journal_path, op2), WriteState::IndexCurrent, "the row must not advance past the cleanup it could not do");
            assert_ne!(journal_state(&journal_path, op2), WriteState::GitCommitted);

            // With the store, the next pass completes the row AND the cleanup.
            let reopened = Vault::open_with_pii_store(root, &journal_path, &pii_store).unwrap();
            assert_eq!(journal_state(&journal_path, op2), WriteState::IndexCurrent);
            assert_eq!(pii_store.get("note-r3-1a").unwrap(), b"v2 secret");
            assert!(reopened.read_note(&id).unwrap().content.contains("pii://note-r3-1a"));
            assert!(!pii_store.has_before_image("note-r3-1a", op2), "cleaned up once a store is available");
        }

        #[test]
        fn round3_1b_a_non_sensitive_write_without_a_pii_store_never_catches_up_a_git_pending_sensitive_row() {
            // Item 1(b): `commit_to_git`'s catch-up loop committed an earlier
            // git-pending SENSITIVE row with `pii_store: None`, skipping its
            // PII before-image delete while advancing it to IndexCurrent.
            let root = crate::git::test_support::init_test_repo();
            let journal_path = unique_dir().join("journal.sqlite");
            let pii_store = open_pii_store(&root);
            let id = NoteId::parse("note-r3-1b").unwrap();
            let pointer_content = pointer_content_for("note-r3-1b");
            let vault = Vault::open_with_pii_store(root.clone(), &journal_path, &pii_store).unwrap();
            vault
                .write_sensitive_record_unchecked(&id, &pointer_content, b"v1 secret", Version(0), Uuid::new_v4(), &pii_store, None)
                .unwrap();
            let op2 = Uuid::new_v4();
            let injector = FaultInjector::trigger_at(FaultPoint::AfterJournalCommittedBeforeGit);
            let stalled = vault.write_sensitive_record_unchecked_with_injector(
                &id, &pointer_content, b"v2 secret", Version(1), op2, &pii_store, None, &injector,
            );
            assert!(matches!(stalled, Err(StorageError::Interrupted(_))));
            assert_eq!(vault.journal_row_for_test(op2).unwrap().state, WriteState::JournalCommitted);
            assert!(pii_store.has_before_image("note-r3-1b", op2));

            // An unrelated, NON-sensitive write on the same note, with no
            // PiiStore in hand, triggers the catch-up.
            let op3 = Uuid::new_v4();
            let result = vault.write_record_unchecked(&id, "plain v3", Version(2), op3);
            assert!(matches!(result, Err(StorageError::GitPending { .. })), "must rest as git-pending, got {result:?}");
            assert_eq!(vault.journal_row_for_test(op2).unwrap().state, WriteState::JournalCommitted, "the sensitive row must not be advanced without its cleanup");
            assert!(pii_store.has_before_image("note-r3-1b", op2), "nothing may be orphaned: the before-image is still owned by a non-terminal row");
            drop(vault);

            // A later pass that has the store resolves both rows and cleans up.
            let _reopened = Vault::open_with_pii_store(root, &journal_path, &pii_store).unwrap();
            assert_eq!(journal_state(&journal_path, op2), WriteState::IndexCurrent);
            assert_eq!(journal_state(&journal_path, op3), WriteState::IndexCurrent);
            assert!(!pii_store.has_before_image("note-r3-1b", op2));
        }

        #[test]
        fn round3_1b_a_non_sensitive_write_with_the_pii_store_threaded_through_catches_up_and_cleans_up() {
            // Item 1(b), the threaded half: `propose_and_commit`'s
            // non-sensitive branch has the PiiStore in hand and passes it
            // through, so the catch-up does the cleanup itself.
            let root = crate::git::test_support::init_test_repo();
            let journal_path = unique_dir().join("journal.sqlite");
            let pii_store = open_pii_store(&root);
            let id = NoteId::parse("note-r3-1b-threaded").unwrap();
            let pointer_content = pointer_content_for("note-r3-1b-threaded");
            let vault = Vault::open_with_pii_store(root.clone(), &journal_path, &pii_store).unwrap();
            vault
                .write_sensitive_record_unchecked(&id, &pointer_content, b"v1 secret", Version(0), Uuid::new_v4(), &pii_store, None)
                .unwrap();
            let op2 = Uuid::new_v4();
            let injector = FaultInjector::trigger_at(FaultPoint::AfterJournalCommittedBeforeGit);
            let _ = vault.write_sensitive_record_unchecked_with_injector(
                &id, &pointer_content, b"v2 secret", Version(1), op2, &pii_store, None, &injector,
            );
            assert!(pii_store.has_before_image("note-r3-1b-threaded", op2));

            let op3 = Uuid::new_v4();
            let version = vault.write_record_unchecked_with_pii_store(&id, "plain v3", Version(2), op3, Some(&pii_store)).unwrap();
            assert_eq!(version, Version(3));
            assert_eq!(vault.journal_row_for_test(op2).unwrap().state, WriteState::IndexCurrent);
            assert!(!pii_store.has_before_image("note-r3-1b-threaded", op2), "caught-up sensitive row's before-image must be deleted");
        }

        /// Puts op2 exactly in the crash window between its `GitCommitted`
        /// advance and its before-image deletes: commits its record to git
        /// and advances the journal (recording the Oid), deleting nothing.
        fn advance_to_git_committed_without_cleanup(root: &Path, journal_path: &Path, id: &NoteId, op: Uuid) {
            let record_path = root.join(format!("{}.record", id.as_str()));
            let oid = crate::git::commit_operation(root, op, id.as_str(), &record_path).unwrap();
            Journal::open(journal_path).unwrap().advance(op, WriteState::GitCommitted, AdvanceFields::git_oid(oid)).unwrap();
        }

        #[test]
        fn round3_1c_a_row_resumed_from_git_committed_still_deletes_its_pii_before_image() {
            // Item 1(c): crash after the GitCommitted advance, before the
            // before-image delete. Recovery resumes such a row from row 4
            // (index step), never through `fix_journal_committed`, so the
            // before-image was orphaned.
            let (root, journal_path, pii_store, id, _pc, op2) =
                v1_committed_then_v2_interrupted_at("note-r3-1c", FaultPoint::AfterJournalCommittedBeforeGit);
            advance_to_git_committed_without_cleanup(&root, &journal_path, &id, op2);
            assert!(pii_store.has_before_image("note-r3-1c", op2));

            let _reopened = Vault::open_with_pii_store(root.clone(), &journal_path, &pii_store).unwrap();
            assert_eq!(journal_state(&journal_path, op2), WriteState::IndexCurrent);
            assert!(!pii_store.has_before_image("note-r3-1c", op2), "a row resumed from GitCommitted must still clean up its PII before-image");
            assert!(
                crate::vault::read_before_image(&root, &id, op2).is_err(),
                "and its Markdown before-image, on the same schedule"
            );
            assert_eq!(pii_store.get("note-r3-1c").unwrap(), b"v2 secret");
        }

        #[test]
        fn round3_1c_a_row_resumed_from_git_committed_without_a_pii_store_fails_closed() {
            let (root, journal_path, pii_store, id, _pc, op2) =
                v1_committed_then_v2_interrupted_at("note-r3-1c-nostore", FaultPoint::AfterJournalCommittedBeforeGit);
            advance_to_git_committed_without_cleanup(&root, &journal_path, &id, op2);

            let result = Vault::open_with_default_indexer(root.clone(), &journal_path);
            assert!(matches!(result, Err(StorageError::Inconsistent { .. })), "got {:?}", result.as_ref().map(|_| "Ok(Vault)"));
            assert!(pii_store.has_before_image("note-r3-1c-nostore", op2));
            assert_ne!(journal_state(&journal_path, op2), WriteState::IndexCurrent);

            let _reopened = Vault::open_with_pii_store(root, &journal_path, &pii_store).unwrap();
            assert_eq!(journal_state(&journal_path, op2), WriteState::IndexCurrent);
            assert!(!pii_store.has_before_image("note-r3-1c-nostore", op2));
        }

        #[test]
        fn round3_1c_a_retry_of_a_git_committed_sensitive_operation_deletes_its_pii_before_image() {
            // Same crash window, resumed through the live retry path
            // (`resume_existing_operation`) instead of recovery.
            let (root, journal_path, pii_store, id, pointer_content, op2) =
                v1_committed_then_v2_interrupted_at("note-r3-1c-retry", FaultPoint::AfterJournalCommittedBeforeGit);
            advance_to_git_committed_without_cleanup(&root, &journal_path, &id, op2);
            // Build the vault directly, skipping reconcile (which would
            // resolve op2 itself), to model the live process that crashed
            // being replaced by a caller retrying the same operation_id.
            let vault = Vault {
                root: root.clone(),
                locks: Mutex::new(HashMap::new()),
                journal: Mutex::new(Journal::open(&journal_path).unwrap()),
                git_lock: Mutex::new(()),
                indexer: Box::new(NullIndexer),
            };
            let version = vault
                .write_sensitive_record_unchecked(&id, &pointer_content, b"v2 secret", Version(1), op2, &pii_store, None)
                .unwrap();
            assert_eq!(version, Version(2));
            assert_eq!(vault.journal_row_for_test(op2).unwrap().state, WriteState::IndexCurrent);
            assert!(!pii_store.has_before_image("note-r3-1c-retry", op2), "the retry path must clean up too");
        }

        /// Shared body of the two item-2 tests: v1 committed, v2 crashed
        /// with op2's ciphertext live and v1's as op2's before-image; the
        /// reopen happens while `obstruct` makes the live file unreadable.
        fn assert_unreadable_live_file_never_destroys_the_only_copy_of_v1(
            note: &str,
            obstruct: impl FnOnce(&Path) -> Box<dyn FnOnce()>,
        ) {
            let (root, journal_path, pii_store, id, pointer_content, op2) =
                v1_committed_then_v2_interrupted_at(note, FaultPoint::AfterPiiCommittedBeforePointerRename);
            assert_eq!(pii_store.get(note).unwrap(), b"v2 secret", "live is op2's own ciphertext");

            let release = obstruct(&live_pii_path(&pii_store, note));
            let result = Vault::open_with_pii_store(root.clone(), &journal_path, &pii_store);
            assert!(result.is_err(), "an unreadable live file is ambiguous: recovery must fail closed, not resolve the row");
            drop(result);
            assert!(
                pii_store.has_before_image(note, op2),
                "THE CRITICAL ASSERTION: the before-image is the only copy of committed v1 content and must survive an ambiguous read error"
            );
            assert_ne!(journal_state(&journal_path, op2), WriteState::Failed, "the row must be left for the next pass, not resolved now");
            release();

            // Next pass, with the obstruction gone: v1 is restored intact.
            let reopened = Vault::open_with_pii_store(root, &journal_path, &pii_store).unwrap();
            assert_eq!(journal_state(&journal_path, op2), WriteState::Failed);
            assert_eq!(reopened.read_note(&id).unwrap().content, pointer_content);
            assert_eq!(pii_store.get(note).unwrap(), b"v1 secret", "v1's committed content must never be lost");
            assert!(!pii_store.has_before_image(note, op2));
        }

        #[cfg(windows)]
        #[test]
        fn round3_2_a_sharing_violation_reading_the_live_pii_file_never_deletes_the_before_image() {
            // Item 2, the reviewer's exact repro: hold a no-read-share
            // handle on the live `.pii` file across the reopen.
            assert_unreadable_live_file_never_destroys_the_only_copy_of_v1("note-r3-2-lock", |live| {
                let handle = hold_without_read_share(live);
                Box::new(move || drop(handle))
            });
        }

        #[test]
        fn round3_audit_a_live_file_that_is_neither_ours_nor_the_before_images_content_never_deletes_the_before_image() {
            // Self-audit: round 2's "not ours" branch deleted the
            // before-image whenever the live file was not this operation's,
            // even when the before-image was then the ONLY copy of v1. Here
            // an unrelated third ciphertext replaces the live file (op2's
            // own v2) before recovery runs; op2's before-image (v1) is not a
            // copy of what is live, so it must be kept and the row refused.
            let (root, journal_path, pii_store, _id, _pc, op2) =
                v1_committed_then_v2_interrupted_at("note-r3-other", FaultPoint::AfterPiiCommittedBeforePointerRename);
            let intruder = Uuid::new_v4();
            pii_store.stage("note-r3-other", b"unrelated content", intruder).unwrap();
            pii_store.commit_staged("note-r3-other", intruder).unwrap();

            let result = Vault::open_with_pii_store(root, &journal_path, &pii_store);
            assert!(matches!(result, Err(StorageError::Inconsistent { .. })), "got {:?}", result.as_ref().map(|_| "Ok(Vault)"));
            assert!(pii_store.has_before_image("note-r3-other", op2), "v1's only copy must survive");
            assert_ne!(journal_state(&journal_path, op2), WriteState::Failed);
        }

        #[test]
        fn round3_audit_a_missing_live_file_never_deletes_the_before_image() {
            // Self-audit: same branch, live file absent (NotFound). The
            // before-image may be the only copy left; it is kept.
            let (root, journal_path, pii_store, _id, _pc, op2) =
                v1_committed_then_v2_interrupted_at("note-r3-absent", FaultPoint::AfterPiiCommittedBeforePointerRename);
            pii_store.remove_live("note-r3-absent").unwrap();

            let result = Vault::open_with_pii_store(root, &journal_path, &pii_store);
            assert!(matches!(result, Err(StorageError::Inconsistent { .. })), "got {:?}", result.as_ref().map(|_| "Ok(Vault)"));
            assert!(pii_store.has_before_image("note-r3-absent", op2));
            assert_ne!(journal_state(&journal_path, op2), WriteState::Failed);
        }

        #[test]
        fn round3_audit_an_update_with_unchanged_plaintext_that_stops_before_commit_staged_never_removes_the_committed_ciphertext() {
            // Self-audit: "the live file hashes to my content" was treated as
            // "the live file is mine", and with no before-image the live file
            // was removed as a first write's leftover. If an update carries
            // the SAME plaintext as the committed version (a frontmatter-only
            // edit, say) and stops before commit_staged, the live file is v1's
            // committed ciphertext, which hashes identically. Removing it
            // destroyed committed content.
            let root = crate::git::test_support::init_test_repo();
            let journal_path = unique_dir().join("journal.sqlite");
            let pii_store = open_pii_store(&root);
            let id = NoteId::parse("note-r3-same").unwrap();
            let pointer_content = pointer_content_for("note-r3-same");
            {
                let vault = Vault::open_with_pii_store(root.clone(), &journal_path, &pii_store).unwrap();
                vault
                    .write_sensitive_record_unchecked(&id, &pointer_content, b"unchanged secret", Version(0), Uuid::new_v4(), &pii_store, None)
                    .unwrap();
                let injector = FaultInjector::trigger_at(FaultPoint::AfterPiiStagedBeforeCommit);
                let result = vault.write_sensitive_record_unchecked_with_injector(
                    &id, &pointer_content, b"unchanged secret", Version(1), Uuid::new_v4(), &pii_store, None, &injector,
                );
                assert!(matches!(result, Err(StorageError::Interrupted(_))));
            }
            let reopened = Vault::open_with_pii_store(root, &journal_path, &pii_store).unwrap();
            assert_eq!(reopened.read_note(&id).unwrap().version, Version(1), "rolled back to v1");
            assert_eq!(
                pii_store.get("note-r3-same").expect("v1's committed ciphertext must still be live"),
                b"unchanged secret"
            );
        }

        #[test]
        fn round3_audit_live_rollback_with_an_unreadable_before_image_never_removes_the_live_file() {
            // Self-audit: the live write path's own rollback copy decided
            // "first write, remove the live file" from `has_before_image`,
            // a bool that also reads `false` on an I/O error. Here op2's
            // before-image path is unreadable as a file at step 7's
            // rollback. The old code removed the live file (op2's v2) and
            // failed the row, leaving v1's content only in the before-image
            // of a terminal row. Now nothing is touched and the row stays
            // ContentPersisted for recovery.
            let root = crate::git::test_support::init_test_repo();
            let journal_path = unique_dir().join("journal.sqlite");
            let pii_store = open_pii_store(&root);
            let id = NoteId::parse("note-r3-live-bi").unwrap();
            let pointer_content = pointer_content_for("note-r3-live-bi");
            let vault = std::sync::Arc::new(Vault::open_with_pii_store(root.clone(), &journal_path, &pii_store).unwrap());
            vault
                .write_sensitive_record_unchecked(&id, &pointer_content, b"v1 secret", Version(0), Uuid::new_v4(), &pii_store, None)
                .unwrap();

            let (paused_tx, paused_rx) = std::sync::mpsc::channel();
            let (resume_tx, resume_rx) = std::sync::mpsc::channel();
            let injector = FaultInjector::pause_at(FaultPoint::AfterRenameBeforeJournalCommitted, paused_tx, resume_rx);
            let op2 = Uuid::new_v4();
            let (writer_vault, writer_id, writer_pc) = (std::sync::Arc::clone(&vault), id.clone(), pointer_content.clone());
            let writer_pii_store = open_pii_store(&root);
            let writer = std::thread::spawn(move || {
                writer_vault.write_sensitive_record_unchecked_with_injector(
                    &writer_id, &writer_pc, b"v2 secret", Version(1), op2, &writer_pii_store, None, &injector,
                )
            });
            paused_rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
            let bi = pii_store.root().join(".before-images").join(format!("note-r3-live-bi-{op2}.pii"));
            let aside = bi.with_extension("pii-aside");
            std::fs::rename(&bi, &aside).unwrap();
            std::fs::create_dir(&bi).unwrap();
            std::fs::write(root.join("note-r3-live-bi.record"), b"TORN WRITE").unwrap();
            resume_tx.send(()).unwrap();
            let result = writer.join().unwrap();
            assert!(result.is_err());
            assert_ne!(vault.journal_row_for_test(op2).unwrap().state, WriteState::Failed, "the row must be left for recovery");
            assert_eq!(pii_store.get("note-r3-live-bi").unwrap(), b"v2 secret", "the live file must not be removed on an ambiguous before-image check");
            drop(vault);

            std::fs::remove_dir(&bi).unwrap();
            std::fs::rename(&aside, &bi).unwrap();
            let reopened = Vault::open_with_pii_store(root, &journal_path, &pii_store).unwrap();
            assert_eq!(journal_state(&journal_path, op2), WriteState::Failed);
            assert_eq!(reopened.read_note(&id).unwrap().content, pointer_content);
            assert_eq!(pii_store.get("note-r3-live-bi").unwrap(), b"v1 secret");
            assert!(!pii_store.has_before_image("note-r3-live-bi", op2));
        }

        #[test]
        fn round3_2_a_live_pii_path_that_cannot_be_read_as_a_file_never_deletes_the_before_image() {
            // Item 2, cross-platform variant: the live path is temporarily a
            // directory (read fails with PermissionDenied on Windows,
            // IsADirectory elsewhere — never NotFound).
            assert_unreadable_live_file_never_destroys_the_only_copy_of_v1("note-r3-2-dir", |live| {
                let live = live.to_path_buf();
                let aside = live.with_extension("pii-aside");
                std::fs::rename(&live, &aside).unwrap();
                std::fs::create_dir(&live).unwrap();
                Box::new(move || {
                    std::fs::remove_dir(&live).unwrap();
                    std::fs::rename(&aside, &live).unwrap();
                })
            });
        }
    }
}
