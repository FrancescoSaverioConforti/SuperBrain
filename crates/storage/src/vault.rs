use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use domain::{IndexState, WriteState};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::atomic::{atomic_write, AtomicWriteError};
use crate::fault::{FaultInjector, FaultPoint};
use crate::git::{commit_operation, find_commit_for_operation, GitError, ScanDepth};
use crate::journal::{AdvanceFields, Journal};
use crate::note_id::NoteId;
use crate::recovery::{reconcile, ReconcileError};

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
    /// The vault root is not (yet) a git repository, so this operation's
    /// content is durably persisted and journal-readable (it reached
    /// `WriteState::JournalCommitted`) but could not be advanced to
    /// `GitCommitted`. This is the spec's `git_pending` *response code*, a
    /// non-error outcome from `propose_and_commit`'s perspective — distinct
    /// from a persisted `WriteState` — not a write failure: the journal row
    /// legitimately stays at `JournalCommitted` until a later retry
    /// succeeds in committing it to git.
    #[error("git commit pending for note {note_id}: no git repository at the vault root yet")]
    GitPending { note_id: String },
    /// Any other failure from the git-commit step of the write chain
    /// (Task 5). The crate-private `git::GitError` is folded into this
    /// variant (rendered message aside, its `Git2`/`Journal` variants are
    /// kept structured via `#[from]` on `GitError` itself, but `GitError`
    /// cannot appear directly in this `pub` enum's field — same reasoning
    /// as `StorageError::Journal` above).
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
}

/// Folds a crate-private `journal::JournalError` into `StorageError::Journal`
/// by capturing its rendered message, since the error type itself cannot be
/// stored in a `pub` enum's field (see `StorageError::Journal`'s doc comment).
fn journal_err(e: crate::journal::JournalError) -> StorageError {
    StorageError::Journal(e.to_string())
}

/// Folds a crate-private `git::GitError` into `StorageError`: `NoRepository`
/// becomes the non-error `GitPending` response code (per spec C1), anything
/// else becomes `StorageError::Git` by its rendered message (same reasoning
/// as `journal_err` above — `GitError` cannot appear directly in a `pub`
/// enum's field).
fn git_err(note_id: &NoteId, e: GitError) -> StorageError {
    match e {
        GitError::NoRepository => StorageError::GitPending {
            note_id: note_id.as_str().to_string(),
        },
        other => StorageError::Git(other.to_string()),
    }
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
    /// 7), and is never held while attempting to acquire a per-note lock or
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
        Self::open_reconciled(root, journal_path, indexer, &FaultInjector::none())
    }

    /// Opens a vault rooted at `root` with the A2 default indexer
    /// ([`NullIndexer`]), backed by the journal at `journal_path`. Replaces
    /// A1's `Vault::new`: every caller supplies a journal path, since A1 had
    /// no production caller to break. Runs the same startup reconciliation
    /// as `Vault::open` and fails closed the same way.
    pub fn open_with_default_indexer(root: PathBuf, journal_path: &Path) -> Result<Self, StorageError> {
        Self::open(root, journal_path, Box::new(NullIndexer))
    }

    /// Test-only sibling of [`Vault::open_with_default_indexer`] (also uses
    /// [`NullIndexer`]) that threads `injector` into the internal `reconcile`
    /// call instead of `FaultInjector::none()`. This lets a test interrupt
    /// recovery itself, for example at
    /// `FaultPoint::AfterMarkedRecoveringBeforeFixup`. An interrupted pass
    /// comes back as `StorageError::Interrupted`, never as a usable `Vault`.
    #[cfg(test)]
    pub(crate) fn open_with_injector(root: PathBuf, journal_path: &Path, injector: &FaultInjector) -> Result<Self, StorageError> {
        Self::open_reconciled(root, journal_path, Box::new(NullIndexer), injector)
    }

    fn open_reconciled(
        root: PathBuf,
        journal_path: &Path,
        indexer: Box<dyn Indexer>,
        injector: &FaultInjector,
    ) -> Result<Self, StorageError> {
        let journal = Journal::open(journal_path).map_err(journal_err)?;
        // No Vault lock is taken here: this runs before the Vault exists, so
        // nothing else can be writing or committing through it. See
        // `recovery`'s module docs, "Concurrency".
        match reconcile(&root, &journal, &root, indexer.as_ref(), injector) {
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
    ///
    /// A2 restructures the body to drive the write through the journal
    /// (Task 2) to `WriteState::JournalCommitted`, and to persist a
    /// before-image (Task 3) of any content being overwritten. The exact
    /// sequence, each step's journal/filesystem effect followed immediately
    /// by a `FaultInjector::check` call (production callers always pass
    /// `FaultInjector::none()`, so this never interrupts them — the seam
    /// exists for Task 8's crash tests):
    ///
    /// 1. `journal.insert_intent` — this IS `IntentRecorded`, persisted
    ///    before anything else happens. Safe to run before the per-note
    ///    lock below because it touches only the journal, never the
    ///    filesystem: nothing a concurrent reader could observe on disk
    ///    changes as a result of this row existing.
    /// 2. Acquire the per-note lock (now also shared with `read_note`) and
    ///    read the actual current version exactly as A1 did. On a version
    ///    mismatch, advance the journal row to `Failed` (a rejected
    ///    attempt is terminal) and return `Conflict`.
    /// 3. If a record already exists, persist its current bytes as the
    ///    before-image for this `operation_id`.
    /// 4. Write the new record via `atomic_write`, then advance the
    ///    journal to `ContentPersisted`, attaching both the new content's
    ///    hash and (if one exists) the before-image's hash on the SAME row.
    /// 5. Re-read the just-written file and verify its hash. On mismatch,
    ///    attempt rollback to the before-image and verify THAT hash; if
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
    ///    `GitError::NoRepository`, do NOT advance the journal past
    ///    `JournalCommitted`; return `StorageError::GitPending` instead of
    ///    an error (the spec's `git_pending` response code, not a
    ///    persisted `WriteState` — the row legitimately stays at
    ///    `JournalCommitted` until a later retry succeeds). On any other
    ///    `GitError`, return `StorageError::Git` without advancing — same
    ///    "stays recoverable at `JournalCommitted`" behavior.
    /// 8. Release the per-note lock (by scope exit).
    /// 9. Return the new version.
    ///
    /// The chain ends at `IndexCurrent` (Task 6): after `GitCommitted`, the
    /// index step calls `Indexer::mark_current` and advances the journal to
    /// `WriteState::IndexCurrent`. That state marks pipeline completion only;
    /// it does not claim the index is current (see `index_state_for`).
    pub(crate) fn write_record_unchecked(
        &self,
        note_id: &NoteId,
        content: &str,
        expected_version: Version,
        operation_id: Uuid,
    ) -> Result<Version, StorageError> {
        self.write_record_unchecked_with_injector(note_id, content, expected_version, operation_id, &FaultInjector::none())
    }

    /// [`Vault::write_record_unchecked`] with an explicit `injector`. Production
    /// callers go through `write_record_unchecked`, which passes
    /// `FaultInjector::none()`. An armed injector stops the write right after
    /// the named step boundary and returns `StorageError::Interrupted(point)`.
    /// Today only `AfterIntentRecorded` and `AfterRenameBeforeJournalCommitted`
    /// are wired; Task 8 threads the remaining points.
    pub(crate) fn write_record_unchecked_with_injector(
        &self,
        note_id: &NoteId,
        content: &str,
        expected_version: Version,
        operation_id: Uuid,
        injector: &FaultInjector,
    ) -> Result<Version, StorageError> {
        let interrupted = |crate::fault::Interrupted::At(point)| StorageError::Interrupted(point);

        let record_path = self.record_path_checked(note_id)?;

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
        atomic_write(&record_path, &buffer).map_err(|AtomicWriteError::Io(e)| StorageError::Io(e))?;
        let new_hash = content_hash(&buffer);
        let fields = match &before_image_hash {
            Some(before_hash) => {
                AdvanceFields::content_with_before_image(new_version.0, new_hash.clone(), before_hash.clone())
            }
            None => AdvanceFields::content(new_version.0, new_hash.clone()),
        };
        self.journal.lock().unwrap().advance(operation_id, WriteState::ContentPersisted, fields).map_err(journal_err)?;
        // Checked AFTER the ContentPersisted advance, not between the rename
        // and it (Task 7 fix round, finding I-1). The point now means "the
        // rename happened AND the journal records it, with its hash", which is
        // an orderly step boundary that recovery's row 2 can resolve. Before,
        // it fired while the row was still IntentRecorded with the new bytes
        // already on disk. Row 1 marked that row Failed without restoring the
        // file, and the note then read as Inconsistent/NotFound for good. A
        // real hardware crash between the rename and SQLite's write is
        // inherent to any journal design and out of scope for this harness.
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

        // Step 7 (Task 5): extend the chain from JournalCommitted to
        // GitCommitted. find_commit_for_operation is consulted FIRST (its
        // own Tier 1 is the journal's own git_commit_oid column, O(1) and
        // authoritative; Tier 2 is a bounded defensive history scan) so a
        // re-entrant call for an operation that already has a commit never
        // creates a second one.
        //
        // The whole find-then-commit-then-record sequence is serialized by
        // `git_lock` across ALL notes (fix round, independent-review finding
        // I1): two threads writing to two DIFFERENT notes hold DIFFERENT
        // per-note locks and would otherwise both read the same HEAD and
        // race to update the same branch ref, and the loser's otherwise-
        // successful commit_operation call would fail outright. Acquired
        // here, strictly after the per-note lock (already held since step
        // 2), never before it — see `git_lock`'s own doc comment for the
        // full lock-ordering argument.
        {
            let _git_guard = self.git_lock.lock().unwrap();

            let existing_oid = find_commit_for_operation(
                &self.journal.lock().unwrap(),
                &self.root,
                operation_id,
                ScanDepth::Bounded(500),
            )
            .map_err(|e| git_err(note_id, e))?;

            let git_oid = match existing_oid {
                Some(oid) => oid,
                None => commit_operation(&self.root, operation_id, note_id.as_str(), &record_path)
                    .map_err(|e| git_err(note_id, e))?,
            };
            // Recording the Oid and advancing to GitCommitted happen on the
            // SAME journal write (AdvanceFields::git_oid), so there is no
            // window where the state says GitCommitted but git_commit_oid
            // is unset — this covers both the idempotent re-entry (Tier 2
            // found it but the journal row didn't have it recorded yet) and
            // the fresh-commit case. Also still under git_lock: otherwise a
            // second thread could re-run find_commit_for_operation between
            // this commit and this journal write, find nothing (Tier 1
            // unset, Tier 2 racing the same HEAD this thread just moved past)
            // and attempt its own commit_operation against a now-stale
            // assumption.
            self.journal
                .lock()
                .unwrap()
                .advance(operation_id, WriteState::GitCommitted, AdvanceFields::git_oid(git_oid))
                .map_err(journal_err)?;
            // _git_guard drops here, releasing git_lock.
        }

        // Per spec: the before-image is deleted only after git_committed.
        delete_before_image(&self.root, note_id, operation_id)?;

        // Step 7b (Task 6): the index step, GitCommitted -> IndexCurrent.
        // `mark_current` is the indexer's hook (a no-op for `NullIndexer`).
        // The journal advance records only that this pipeline step completed.
        // It is NOT an index-freshness claim; that is `index_state_for`'s job.
        self.indexer.mark_current(note_id.as_str(), new_version.0);
        self.journal
            .lock()
            .unwrap()
            .advance(operation_id, WriteState::IndexCurrent, AdvanceFields::none())
            .map_err(journal_err)?;

        // Step 8: the per-note lock is released when `_guard` drops at the
        // end of this scope.
        // Step 9:
        Ok(new_version)
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
}
