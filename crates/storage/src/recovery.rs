//! Startup reconciliation against the C1 fault matrix.
//!
//! [`reconcile`] runs inside `Vault::open`, before any `Vault` value exists,
//! and drives every non-terminal journal row to a terminal state (`Failed`,
//! `IndexCurrent`) or to the one allowed resting state (`JournalCommitted`
//! with git unavailable — the spec's `git_pending`). If even recovery cannot
//! establish a safe state for one operation, it returns
//! [`ReconcileError::Inconsistent`] and `Vault::open` fails closed.
//!
//! Every row goes through the same sequence, with no per-row shortcut:
//!
//! 1. Mark it `Recovering`, recording the state it is leaving as
//!    `previous_state` on the same journal write. A row already at
//!    `Recovering` (an earlier pass crashed mid-fix-up) is not re-marked; its
//!    stored `previous_state` is used instead.
//! 2. Check [`FaultPoint::AfterMarkedRecoveringBeforeFixup`].
//! 3. Starting from `previous_state`, apply that state's fault-matrix row, then
//!    keep applying the row matching the operation's new state, in this same
//!    pass, until it is terminal or resting. A row recovered from
//!    `JournalCommitted` therefore reaches `GitCommitted` and then
//!    `IndexCurrent` in one call.
//!
//! The five rows, keyed by the state being resumed from:
//!
//! - `IntentRecorded`: if the row carries an intended `content_hash` (the
//!   write path records it before the rename) and the record on disk matches
//!   it, the rename completed and only the `ContentPersisted` advance was
//!   lost: handed to the `ContentPersisted` row below, which rolls it
//!   forward. Otherwise nothing new reached the record: any before-image is
//!   deleted and the row advanced to `Failed`. The caller must retry with a
//!   new `operation_id` (final whole-branch review, finding C1).
//! - `ContentPersisted`: if the record on disk hashes to the row's
//!   `content_hash`, roll forward to `JournalCommitted`. Otherwise restore this
//!   operation's before-image, verify it, delete the before-image and advance
//!   the row to `Failed`. Its `new_version` stays as the version it failed to
//!   commit. No git commit is made: the restored content was already
//!   linearized by the earlier operation that wrote it, which
//!   `last_committed_for_note` still resolves to. If the row has no
//!   `before_image_hash` (a first write, so there is nothing to restore), it
//!   becomes `Failed`. If no operation has ever committed the note, the
//!   unverified record is also removed. A recorded before-image that cannot be
//!   restored or does not verify gives `Inconsistent`. A pass interrupted
//!   after the restore (`FaultPoint::DuringBeforeImageRestore`) is resumed
//!   idempotently: the next pass finds the record already equal to the
//!   before-image and only completes the deletion and the `Failed` advance.
//! - `JournalCommitted`: `find_commit_for_operation` with
//!   `ScanDepth::Unbounded`, always. A bounded miss would be ambiguous, and a
//!   `commit_operation` call on that ambiguity could duplicate a commit. A
//!   found `Oid` (including a Tier 2 find the row never recorded) is written
//!   back on the same advance to `GitCommitted`. If there is none, this
//!   operation's OWN bytes are committed, never just whatever is on disk:
//!   either the record, if it still hashes to the row's `content_hash`, or
//!   the before-image of the successor that overwrote it. If neither is
//!   available, the row rests at `JournalCommitted` with that reason. If git
//!   is unavailable the row also rests at
//!   `JournalCommitted` and is reported as `git_pending`; this does not block
//!   `Vault::open`. As `ScanDepth` documents, an unbounded miss proves absence
//!   only among commits reachable from `HEAD`; rewriting history out of band
//!   is an accepted residual risk.
//! - `GitCommitted`: `Indexer::mark_current`, then `IndexCurrent`. This marks
//!   pipeline completion only, not index freshness.
//! - `Recovering`: re-dispatch from the stored `previous_state` (step 1 above).
//!
//! # Concurrency
//!
//! `reconcile` takes no `Vault` lock (not the per-note locks, not `git_lock`).
//! Its only caller, `Vault::open`, runs it before the `Vault` is
//! constructed, so no other thread can hold a reference through which to
//! write, read or commit on this vault. The per-instance `git_lock` therefore
//! has nothing to serialize against yet. It also could not cover a second
//! `Vault` opened on the same root, because each instance has its own lock.
//! Cross-instance and cross-process exclusion is outside A2's threat model,
//! as the `Vault::locks` doc comment says. Anyone calling `reconcile` on a
//! live vault would need to take `git_lock` around the `JournalCommitted`
//! fix-up first.

use std::path::Path;

use domain::WriteState;
use uuid::Uuid;

use crate::atomic::atomic_write;
use crate::fault::{FaultInjector, FaultPoint, Interrupted};
use crate::git::{commit_operation_contents, find_commit_for_operation, GitError, ScanDepth};
use crate::journal::{AdvanceFields, Journal, JournalError, OperationRow};
use crate::note_id::NoteId;
use crate::vault::{content_hash, delete_before_image, read_before_image, record_path_checked, Indexer, StorageError};

#[derive(Debug, thiserror::Error)]
pub(crate) enum ReconcileError {
    /// Recovery could not bring `operation_id` to a safe state. `Vault::open`
    /// refuses to start.
    #[error("operation {operation_id} is inconsistent and could not be recovered: {reason}")]
    Inconsistent { operation_id: Uuid, reason: String },
    /// A test's armed `FaultInjector` stopped the pass at this point.
    /// Production callers pass `FaultInjector::none()` and never get this.
    #[error("reconcile interrupted at {0:?}")]
    Interrupted(FaultPoint),
    #[error(transparent)]
    Journal(#[from] JournalError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// What one [`reconcile`] pass did, per operation.
///
/// - `recovered`: operations moved forward to a non-`Failed` state, either
///   `IndexCurrent` or (if git is unavailable) `JournalCommitted`.
/// - `failed`: operations resolved to `Failed`, including row-2 rollbacks.
/// - `git_pending`: operations left resting at `JournalCommitted` because git
///   is unavailable. An operation that rolled forward from `ContentPersisted`
///   and then hit an unavailable git appears in both `recovered` and
///   `git_pending`. One that was already at `JournalCommitted` appears only
///   in `git_pending`, since it made no progress.
/// - `git_pending_reasons`: for each `git_pending` entry, in the same order,
///   why the row could not be committed. That is either git being unavailable
///   or this operation's own content not being recoverable (see row 3 in the
///   module docs).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct ReconcileReport {
    pub recovered: Vec<Uuid>,
    pub failed: Vec<Uuid>,
    pub git_pending: Vec<Uuid>,
    pub git_pending_reasons: Vec<(Uuid, String)>,
}

/// How one row's cascade ended.
enum Outcome {
    /// Reached `IndexCurrent`.
    Completed,
    /// Reached `Failed`.
    Failed,
    /// Resting at `JournalCommitted`, for the given reason.
    GitPending(String),
}

/// Reconciles every non-terminal row in `journal` against the C1 fault
/// matrix. See the module docs for the per-row handling and why no lock is
/// taken. `repo_path` is the git repository (the vault root in production).
pub(crate) fn reconcile(
    root: &Path,
    journal: &Journal,
    repo_path: &Path,
    indexer: &dyn Indexer,
    injector: &FaultInjector,
) -> Result<ReconcileReport, ReconcileError> {
    let mut report = ReconcileReport::default();
    for row in journal.non_terminal_operations()? {
        let op = row.operation_id;
        let origin = mark_recovering(journal, &row)?;
        injector
            .check(FaultPoint::AfterMarkedRecoveringBeforeFixup)
            .map_err(|Interrupted::At(point)| ReconcileError::Interrupted(point))?;
        match resolve(root, journal, repo_path, indexer, &row, origin, injector)? {
            Outcome::Completed => report.recovered.push(op),
            Outcome::Failed => report.failed.push(op),
            Outcome::GitPending(reason) => {
                if origin != WriteState::JournalCommitted {
                    report.recovered.push(op);
                }
                report.git_pending.push(op);
                report.git_pending_reasons.push((op, reason));
            }
        }
    }
    Ok(report)
}

/// Step 1 for one row: persists `Recovering` plus `previous_state` and
/// returns the state to resume from. A row already at `Recovering` is
/// left as is, and its stored `previous_state` is returned.
fn mark_recovering(journal: &Journal, row: &OperationRow) -> Result<WriteState, ReconcileError> {
    let op = row.operation_id;
    if row.state != WriteState::Recovering {
        journal.advance(op, WriteState::Recovering, AdvanceFields::previous_state(row.state))?;
        return Ok(row.state);
    }
    match row.previous_state {
        Some(WriteState::Recovering) | None => Err(inconsistent(
            op,
            format!("row is at Recovering with no usable previous_state ({:?})", row.previous_state),
        )),
        Some(previous) => Ok(previous),
    }
}

/// Step 3 for one row: applies the fault-matrix row for `origin`, then for
/// each state it advances to, until the row is terminal or resting.
fn resolve(
    root: &Path,
    journal: &Journal,
    repo_path: &Path,
    indexer: &dyn Indexer,
    row: &OperationRow,
    origin: WriteState,
    injector: &FaultInjector,
) -> Result<Outcome, ReconcileError> {
    let op = row.operation_id;
    let mut at = origin;
    loop {
        at = match at {
            // Row 1: the write never got past its intent, unless its rename
            // already landed (then it is row 2's roll-forward case).
            WriteState::IntentRecorded => match fix_intent_recorded(root, journal, row)? {
                WriteState::Failed => return Ok(Outcome::Failed),
                next => next,
            },
            // Row 2: roll forward on a verified hash, else roll back to Failed.
            WriteState::ContentPersisted => match fix_content_persisted(root, journal, row, injector)? {
                WriteState::Failed => return Ok(Outcome::Failed),
                next => next,
            },
            // Row 3: idempotent git commit, or rest as git_pending.
            WriteState::JournalCommitted => match fix_journal_committed(root, journal, repo_path, row)? {
                GitStep::Committed => WriteState::GitCommitted,
                GitStep::Pending(reason) => {
                    // Only a row resumed directly from JournalCommitted is still
                    // persisted at Recovering here. One rolled forward this pass
                    // is already at JournalCommitted.
                    if current_state(journal, op)? == WriteState::Recovering {
                        journal.advance(op, WriteState::JournalCommitted, AdvanceFields::none())?;
                    }
                    return Ok(Outcome::GitPending(reason));
                }
            },
            // Row 4: the index step (pipeline completion, not index freshness).
            WriteState::GitCommitted => {
                let version = row
                    .new_version
                    .ok_or_else(|| inconsistent(op, "GitCommitted row has no new_version"))?;
                indexer.mark_current(&row.note_id, version);
                journal.advance(op, WriteState::IndexCurrent, AdvanceFields::none())?;
                return Ok(Outcome::Completed);
            }
            // Never produced by mark_recovering for a row reconcile marked
            // itself (it only marks non-terminal rows), but X -> Recovering is
            // a legal edge for these too. Resolve back to where the row was.
            WriteState::IndexCurrent => {
                journal.advance(op, WriteState::IndexCurrent, AdvanceFields::none())?;
                return Ok(Outcome::Completed);
            }
            WriteState::Failed => {
                journal.advance(op, WriteState::Failed, AdvanceFields::none())?;
                return Ok(Outcome::Failed);
            }
            WriteState::Recovering => {
                return Err(inconsistent(op, "cannot resume a row from Recovering itself"));
            }
        };
    }
}

/// Row 1. Returns `ContentPersisted` (the rename landed: hand over to row 2,
/// which re-verifies the hash and rolls forward) or `Failed`.
///
/// The write path records the operation's intended `content_hash` on its
/// `IntentRecorded` row BEFORE the rename (final whole-branch review, finding
/// C1). So an `IntentRecorded` row is one of:
///
/// - no `content_hash`: the process stopped before the rename could start.
///   Nothing new is on disk.
/// - a `content_hash` the record on disk matches: the rename completed and
///   only the `ContentPersisted` advance is missing. This is the spec's
///   "Dopo il rename, prima di journal_committed" row with a valid hash, so
///   it rolls forward exactly as row 2 does. The hash covers the version
///   header too, so a match means these exact intended bytes, at the
///   intended version, are on disk.
/// - a `content_hash` the record does not match (or no record): the rename
///   never happened, so the record still holds the prior version (or no note
///   exists yet).
///
/// In the two "never happened" cases the record is not touched, any
/// before-image this operation persisted is deleted (it is no longer needed
/// and would otherwise be orphaned under a terminal row), and the row
/// becomes `Failed`.
fn fix_intent_recorded(root: &Path, journal: &Journal, row: &OperationRow) -> Result<WriteState, ReconcileError> {
    let op = row.operation_id;
    let note_id = parse_note_id(row)?;
    if let Some(intended) = row.content_hash.as_deref() {
        let record_path = record_path_checked(root, &note_id).map_err(|e| from_storage(op, e))?;
        let on_disk_hash = match std::fs::read(&record_path) {
            Ok(bytes) => Some(content_hash(&bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(ReconcileError::Io(e)),
        };
        if on_disk_hash.as_deref() == Some(intended) {
            return Ok(WriteState::ContentPersisted);
        }
    }
    // Deleted BEFORE the terminal advance, as in row 2, so a crash in
    // between cannot orphan it under a row that is never revisited.
    delete_before_image(root, &note_id, op).map_err(|e| from_storage(op, e))?;
    journal.advance(op, WriteState::Failed, AdvanceFields::none())?;
    Ok(WriteState::Failed)
}

/// Row 2. Returns `JournalCommitted` (rolled forward) or `Failed` (rolled back).
///
/// The before-image is located by deriving its path from
/// `(root, note_id, operation_id)` via `read_before_image`. The journal's
/// `before_image_path` column is not used, because the write path never fills
/// it. The row's `operation_id` always identifies the before-image uniquely,
/// so the derived path needs no second source of truth.
fn fix_content_persisted(
    root: &Path,
    journal: &Journal,
    row: &OperationRow,
    injector: &FaultInjector,
) -> Result<WriteState, ReconcileError> {
    let op = row.operation_id;
    let note_id = parse_note_id(row)?;
    let record_path = record_path_checked(root, &note_id).map_err(|e| from_storage(op, e))?;
    let expected_hash = row
        .content_hash
        .as_deref()
        .ok_or_else(|| inconsistent(op, "ContentPersisted row has no content_hash"))?;

    // A missing record counts as "does not match". The restore below can
    // recreate it from the before-image.
    let on_disk_hash = match std::fs::read(&record_path) {
        Ok(bytes) => Some(content_hash(&bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(ReconcileError::Io(e)),
    };

    if on_disk_hash.as_deref() == Some(expected_hash) {
        journal.advance(op, WriteState::JournalCommitted, AdvanceFields::none())?;
        return Ok(WriteState::JournalCommitted);
    }

    // No before_image_hash means the record did not exist when this
    // operation wrote it: a first write with no prior version to restore.
    // Nothing was ever validly written, so the operation simply failed
    // (Task 7 fix round, finding I-1(b)). Inconsistent is kept for a recorded
    // before-image that cannot be restored. If no operation has ever committed
    // this note, the unverified record is removed, which restores the
    // pre-operation state of "no note". Otherwise the record is left alone
    // rather than deleting content another row may describe.
    let Some(before_hash) = row.before_image_hash.as_deref() else {
        if on_disk_hash.is_some() && journal.last_committed_for_note(note_id.as_str())?.is_none() {
            match std::fs::remove_file(&record_path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(ReconcileError::Io(e)),
            }
        }
        journal.advance(op, WriteState::Failed, AdvanceFields::none())?;
        return Ok(WriteState::Failed);
    };

    // If the record already holds the before-image, an earlier pass restored
    // it and crashed before advancing the row. The restore is idempotent, so
    // skip straight to completing it.
    if on_disk_hash.as_deref() != Some(before_hash) {
        let before_bytes = read_before_image(root, &note_id, op)
            .map_err(|e| inconsistent(op, format!("before-image restore failed: cannot read before-image: {e}")))?;
        if content_hash(&before_bytes) != before_hash {
            return Err(inconsistent(op, "before-image restore failed: before-image does not match its recorded hash"));
        }
        atomic_write(&record_path, &before_bytes)
            .map_err(|e| inconsistent(op, format!("before-image restore failed: write failed: {e}")))?;
        let restored = std::fs::read(&record_path)
            .map_err(|e| inconsistent(op, format!("before-image restore failed: cannot re-read record: {e}")))?;
        if content_hash(&restored) != before_hash {
            return Err(inconsistent(op, "before-image restore failed: restored record does not verify"));
        }
        // "Durante il ripristino della before-image": the record is restored
        // but the operation is not yet resolved and its before-image still
        // exists. Checked only when this pass performed the restore itself.
        injector
            .check(FaultPoint::DuringBeforeImageRestore)
            .map_err(|Interrupted::At(point)| ReconcileError::Interrupted(point))?;
    }

    // The restore is complete and verified. Delete the before-image BEFORE
    // advancing to Failed. If we crash between the two, the next pass sees the
    // record already matching before_image_hash and completes without the file.
    // The opposite order would orphan the before-image of a row that is
    // already terminal and never revisited.
    delete_before_image(root, &note_id, op).map_err(|e| from_storage(op, e))?;
    journal.advance(op, WriteState::Failed, AdvanceFields::none())?;
    Ok(WriteState::Failed)
}

/// Row 3. Returns `GitStep::Committed` once the row records its commit
/// (now `GitCommitted`), or `GitStep::Pending(reason)` when git is
/// unavailable or this operation's own content cannot be recovered. In the
/// pending case the caller leaves the row resting.
///
/// Any `GitError` other than a journal failure counts as "git unavailable".
/// The content is already durable and readable at `JournalCommitted`, which
/// is a safe state, so a git failure is not an inconsistency and does not
/// block `Vault::open`.
fn fix_journal_committed(
    root: &Path,
    journal: &Journal,
    repo_path: &Path,
    row: &OperationRow,
) -> Result<GitStep, ReconcileError> {
    let op = row.operation_id;
    let note_id = parse_note_id(row)?;
    // Always Unbounded on the recovery path. See the module docs.
    let found = match find_commit_for_operation(journal, repo_path, op, ScanDepth::Unbounded) {
        Ok(found) => found,
        Err(GitError::Journal(e)) => return Err(e.into()),
        Err(e) => return Ok(GitStep::Pending(format!("git unavailable: {e}"))),
    };
    let oid = match found {
        Some(oid) => oid,
        None => {
            let record_path = record_path_checked(root, &note_id).map_err(|e| from_storage(op, e))?;
            let Some(contents) = own_content(root, journal, row, &note_id, &record_path)? else {
                return Ok(GitStep::Pending(
                    "this operation's own content is no longer on disk and no successor's before-image \
                     holding it could be read; refusing to commit another operation's content under \
                     this operation's marker"
                        .to_string(),
                ));
            };
            match commit_operation_contents(repo_path, op, note_id.as_str(), &record_path, &contents) {
                Ok(oid) => oid,
                Err(GitError::Journal(e)) => return Err(e.into()),
                Err(e) => return Ok(GitStep::Pending(format!("git unavailable: {e}"))),
            }
        }
    };
    // The Oid is recorded on the same write that advances the state, so Tier 1
    // is authoritative from here on. This includes repairing a Tier 2 find.
    journal.advance(op, WriteState::GitCommitted, AdvanceFields::git_oid(oid))?;
    // Per spec, the before-image is deleted only once GitCommitted is reached.
    delete_before_image(root, &note_id, op).map_err(|e| from_storage(op, e))?;
    Ok(GitStep::Committed)
}

/// Row 3's result: committed (the row is now `GitCommitted`) or pending, with
/// the reason.
enum GitStep {
    Committed,
    Pending(String),
}

/// The bytes this operation itself persisted, i.e. the bytes that hash to
/// its `content_hash` (Task 7 fix round, finding I-2).
///
/// The per-note lock serializes writers, but it does not stop a later
/// operation from overwriting a note whose earlier operation is still resting
/// at `JournalCommitted` (git pending). Committing "whatever is on disk" would
/// then put the LATER operation's content under the EARLIER operation's
/// marker. So the bytes come from:
///
/// 1. the record on disk, if it still hashes to this row's `content_hash`;
///    otherwise
/// 2. a successor's before-image. That is the operation on the same note
///    whose `before_image_hash` equals this row's `content_hash`, since it
///    overwrote exactly this content. It is used only if the file reads back
///    with that hash.
///
/// `None` if neither is available. The caller then leaves the row pending
/// rather than guessing.
pub(crate) fn own_content(
    root: &Path,
    journal: &Journal,
    row: &OperationRow,
    note_id: &NoteId,
    record_path: &Path,
) -> Result<Option<Vec<u8>>, ReconcileError> {
    let Some(expected) = row.content_hash.as_deref() else {
        return Ok(None);
    };
    match std::fs::read(record_path) {
        Ok(bytes) if content_hash(&bytes) == expected => return Ok(Some(bytes)),
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(ReconcileError::Io(e)),
    }
    for successor in journal.operations_with_before_image_hash(note_id.as_str(), expected)? {
        if successor.operation_id == row.operation_id {
            continue;
        }
        if let Ok(bytes) = read_before_image(root, note_id, successor.operation_id) {
            if content_hash(&bytes) == expected {
                return Ok(Some(bytes));
            }
        }
    }
    Ok(None)
}

fn current_state(journal: &Journal, op: Uuid) -> Result<WriteState, ReconcileError> {
    journal
        .get(op)?
        .map(|row| row.state)
        .ok_or_else(|| inconsistent(op, "journal row disappeared during reconcile"))
}

fn parse_note_id(row: &OperationRow) -> Result<NoteId, ReconcileError> {
    NoteId::parse(&row.note_id)
        .map_err(|e| inconsistent(row.operation_id, format!("journal row has an invalid note id {:?}: {e}", row.note_id)))
}

fn inconsistent(operation_id: Uuid, reason: impl std::fmt::Display) -> ReconcileError {
    ReconcileError::Inconsistent { operation_id, reason: reason.to_string() }
}

/// Plain I/O failures stay `Io`. Any other `StorageError` (e.g. a record path
/// that escapes the vault root) means this operation cannot be recovered
/// safely.
fn from_storage(operation_id: Uuid, e: StorageError) -> ReconcileError {
    match e {
        StorageError::Io(e) => ReconcileError::Io(e),
        other => inconsistent(operation_id, other),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use domain::WriteState;
    use uuid::Uuid;

    use super::*;
    use crate::fault::{FaultInjector, FaultPoint};
    use crate::git::test_support::{init_git_repo_at, unique_test_dir};
    use crate::git::{commit_operation, find_commit_for_operation, ScanDepth};
    use crate::journal::{AdvanceFields, Journal};
    use crate::note_id::NoteId;
    use crate::vault::{content_hash, persist_before_image, serialize_record_for_test, NullIndexer, StorageError, Vault, Version};

    fn fresh_root_and_journal() -> (PathBuf, Journal) {
        (unique_test_dir(), Journal::open_in_memory().unwrap())
    }

    /// A fresh directory that is never git-initialized.
    fn no_git_repo() -> PathBuf {
        unique_test_dir()
    }

    /// Git-initializes `root` in place (with an initial empty commit) and
    /// returns it as the repository path.
    fn init_test_repo_at(root: &std::path::Path) -> PathBuf {
        init_git_repo_at(root);
        root.to_path_buf()
    }

    #[test]
    fn intent_recorded_with_no_further_progress_is_marked_failed_on_reconcile() {
        let (root, journal) = fresh_root_and_journal();
        let op = Uuid::new_v4();
        journal.insert_intent(op, "note-a", 0).unwrap();
        let report = reconcile(&root, &journal, &no_git_repo(), &NullIndexer, &FaultInjector::none()).unwrap();
        assert_eq!(journal.get(op).unwrap().unwrap().state, WriteState::Failed);
        assert_eq!(report.failed, vec![op]);
    }

    #[test]
    fn intent_recorded_with_intended_fields_but_no_rename_is_failed_and_its_before_image_cleaned_up() {
        // C1 inverse: the write path recorded the intended hash and persisted
        // a before-image, then stopped before the rename. The record still
        // holds the prior version, so the operation never landed: Failed,
        // record untouched, before-image removed rather than orphaned.
        let (root, journal) = fresh_root_and_journal();
        let id = NoteId::parse("note-a").unwrap();
        let old = serialize_record_for_test("old", Version(1));
        std::fs::write(root.join("note-a.record"), &old).unwrap();
        let op = Uuid::new_v4();
        let before_hash = persist_before_image(&root, &id, op, &old).unwrap();
        journal.insert_intent(op, id.as_str(), 1).unwrap();
        let intended = serialize_record_for_test("new", Version(2));
        journal
            .record_fields(op, WriteState::IntentRecorded, AdvanceFields::content_with_before_image(2, content_hash(&intended), before_hash))
            .unwrap();
        let report = reconcile(&root, &journal, &no_git_repo(), &NullIndexer, &FaultInjector::none()).unwrap();
        assert_eq!(journal.get(op).unwrap().unwrap().state, WriteState::Failed);
        assert_eq!(report.failed, vec![op]);
        assert_eq!(std::fs::read(root.join("note-a.record")).unwrap(), old, "the prior version is left exactly as it was");
        assert!(read_before_image(&root, &id, op).is_err(), "the unused before-image is deleted, not orphaned");
    }

    #[test]
    fn intent_recorded_whose_intended_bytes_are_on_disk_rolls_forward_like_content_persisted() {
        // C1 at the recovery level: the rename landed, the ContentPersisted
        // advance did not.
        let (root, journal) = fresh_root_and_journal();
        let op = Uuid::new_v4();
        let bytes = serialize_record_for_test("new", Version(1));
        std::fs::write(root.join("note-a.record"), &bytes).unwrap();
        journal.insert_intent(op, "note-a", 0).unwrap();
        journal.record_fields(op, WriteState::IntentRecorded, AdvanceFields::content(1, content_hash(&bytes))).unwrap();
        let report = reconcile(&root, &journal, &no_git_repo(), &NullIndexer, &FaultInjector::none()).unwrap();
        let row = journal.get(op).unwrap().unwrap();
        assert_eq!(row.state, WriteState::JournalCommitted, "rolled forward (no git repo here, so it rests as git_pending)");
        assert_eq!(row.previous_state, Some(WriteState::IntentRecorded));
        assert_eq!(report.recovered, vec![op]);
        assert_eq!(report.git_pending, vec![op]);
        assert_eq!(journal.last_committed_for_note("note-a").unwrap().unwrap().operation_id, op);
    }

    #[test]
    fn content_persisted_with_valid_hash_rolls_forward_to_journal_committed() {
        let (root, journal) = fresh_root_and_journal();
        let id = NoteId::parse("note-a").unwrap();
        let op = Uuid::new_v4();
        // Simulate: the rename happened, but the process crashed before advancing
        // the journal past ContentPersisted.
        let bytes = serialize_record_for_test("content", Version(1));
        std::fs::write(root.join("note-a.record"), &bytes).unwrap();
        journal.insert_intent(op, id.as_str(), 0).unwrap();
        journal.advance(op, WriteState::ContentPersisted, AdvanceFields::content(1, &content_hash(&bytes))).unwrap();
        let report = reconcile(&root, &journal, &no_git_repo(), &NullIndexer, &FaultInjector::none()).unwrap();
        assert_eq!(journal.get(op).unwrap().unwrap().state, WriteState::JournalCommitted);
        assert_eq!(report.recovered, vec![op]);
    }

    #[test]
    fn content_persisted_with_invalid_hash_and_a_valid_before_image_rolls_back_to_failed_not_journal_committed() {
        // Pins the user architectural review's point 3 correction: a rolled-back
        // operation ends at Failed, carrying its own operation_id, distinguishable
        // from the PRIOR operation (op_prev) that actually committed the content
        // now restored on disk. A caller inspecting op's row must see Failed, and
        // last_committed_for_note must resolve to op_prev's row, never op's.
        let (root, journal) = fresh_root_and_journal();
        let id = NoteId::parse("note-a").unwrap();

        // op_prev is the operation that legitimately committed "old content" at
        // version 1 — this is whose before-image op will later roll back to.
        let op_prev = Uuid::new_v4();
        journal.insert_intent(op_prev, id.as_str(), 0).unwrap();
        let before_bytes = serialize_record_for_test("old content", Version(1));
        journal.advance(op_prev, WriteState::ContentPersisted, AdvanceFields::content(1, &content_hash(&before_bytes))).unwrap();
        journal.advance(op_prev, WriteState::JournalCommitted, AdvanceFields::none()).unwrap();
        journal.advance(op_prev, WriteState::GitCommitted, AdvanceFields::none()).unwrap();
        journal.advance(op_prev, WriteState::IndexCurrent, AdvanceFields::none()).unwrap();

        // op is a later, failed attempt: its before-image (persisted under op's
        // OWN operation_id, per Task 4's association fix) is op_prev's content.
        let op = Uuid::new_v4();
        persist_before_image(&root, &id, op, &before_bytes).unwrap();
        // On-disk file is corrupted (does not match the hash op's row expects).
        std::fs::write(root.join("note-a.record"), b"CORRUPTED").unwrap();
        journal.insert_intent(op, id.as_str(), 1).unwrap();
        journal.advance(op, WriteState::ContentPersisted, AdvanceFields::content_with_before_image(
            2, "expected-hash-that-will-not-match", &content_hash(&before_bytes),
        )).unwrap();

        let report = reconcile(&root, &journal, &no_git_repo(), &NullIndexer, &FaultInjector::none()).unwrap();

        assert_eq!(journal.get(op).unwrap().unwrap().state, WriteState::Failed, "the rolled-back operation itself is Failed, not JournalCommitted");
        assert_eq!(journal.get(op).unwrap().unwrap().new_version, Some(2), "the row still truthfully records the version it FAILED to commit, for audit purposes — it is not reinterpreted as success");
        assert_eq!(std::fs::read(root.join("note-a.record")).unwrap(), before_bytes, "filesystem content matches op_prev's already-committed version");
        assert_eq!(journal.last_committed_for_note("note-a").unwrap().unwrap().operation_id, op_prev, "the linearized version is attributed to op_prev, which actually committed it — never to the failed op");
        assert_eq!(report.failed, vec![op], "reported as a failure, not a recovery success, in the reconcile report");
    }

    #[test]
    fn rollback_genuinely_passes_through_recovering_before_resolving_to_failed() {
        // Closes a gap the second independent review flagged: the test above
        // only checks the END state (Failed), which would also pass if an
        // implementer silently special-cased row 2 to skip row 5's generic
        // "mark Recovering first" step. This test pins that row 2 is NOT
        // special-cased: interrupting reconcile right after it marks THIS row
        // Recovering (before row 2's rollback fix-up runs) must leave the row
        // genuinely at Recovering with previous_state = ContentPersisted, exactly
        // like the row-5 test does for the GitCommitted -> IndexCurrent case.
        let (root, journal) = fresh_root_and_journal();
        let id = NoteId::parse("note-a").unwrap();
        let before_bytes = serialize_record_for_test("old content", Version(1));
        let op = Uuid::new_v4();
        persist_before_image(&root, &id, op, &before_bytes).unwrap();
        std::fs::write(root.join("note-a.record"), b"CORRUPTED").unwrap();
        journal.insert_intent(op, id.as_str(), 1).unwrap();
        journal.advance(op, WriteState::ContentPersisted, AdvanceFields::content_with_before_image(
            2, "expected-hash-that-will-not-match", &content_hash(&before_bytes),
        )).unwrap();

        let injector = FaultInjector::trigger_at(FaultPoint::AfterMarkedRecoveringBeforeFixup);
        let interrupted = reconcile(&root, &journal, &no_git_repo(), &NullIndexer, &injector);
        assert!(matches!(interrupted, Err(ReconcileError::Interrupted(_))), "row 2 must go through the same generic Recovering-marking step as every other row, not a special-cased shortcut");
        let mid_recovery_row = journal.get(op).unwrap().unwrap();
        assert_eq!(mid_recovery_row.state, WriteState::Recovering, "genuinely at Recovering, not fast-forwarded past it");
        assert_eq!(mid_recovery_row.previous_state, Some(WriteState::ContentPersisted));

        // A second, uninterrupted pass must still resolve to Failed, per the test above.
        let report = reconcile(&root, &journal, &no_git_repo(), &NullIndexer, &FaultInjector::none()).unwrap();
        assert_eq!(journal.get(op).unwrap().unwrap().state, WriteState::Failed);
        assert_eq!(report.failed, vec![op]);
    }

    #[test]
    fn content_persisted_with_invalid_hash_and_an_unreadable_before_image_is_inconsistent() {
        let (root, journal) = fresh_root_and_journal();
        let id = NoteId::parse("note-a").unwrap();
        let op = Uuid::new_v4();
        std::fs::write(root.join("note-a.record"), b"CORRUPTED").unwrap();
        journal.insert_intent(op, id.as_str(), 1).unwrap();
        journal.advance(op, WriteState::ContentPersisted, AdvanceFields::content_with_before_image(
            2, "expected-hash-that-will-not-match", "before-image-hash-but-no-file-exists",
        )).unwrap();
        let err = reconcile(&root, &journal, &no_git_repo(), &NullIndexer, &FaultInjector::none()).unwrap_err();
        assert!(matches!(err, ReconcileError::Inconsistent { operation_id, .. } if operation_id == op));
    }

    #[test]
    fn journal_committed_with_no_existing_commit_commits_on_reconcile() {
        let (root, journal) = fresh_root_and_journal();
        let repo_dir = init_test_repo_at(&root);
        let id = NoteId::parse("note-a").unwrap();
        let op = Uuid::new_v4();
        let bytes = serialize_record_for_test("content", Version(1));
        std::fs::write(root.join("note-a.record"), &bytes).unwrap();
        journal.insert_intent(op, id.as_str(), 0).unwrap();
        journal.advance(op, WriteState::ContentPersisted, AdvanceFields::content(1, &content_hash(&bytes))).unwrap();
        journal.advance(op, WriteState::JournalCommitted, AdvanceFields::none()).unwrap();
        let report = reconcile(&root, &journal, &repo_dir, &NullIndexer, &FaultInjector::none()).unwrap();
        // Deviation from the brief's literal text, which asserted the row
        // stops at GitCommitted. That predates the second independent-review
        // cascade correction: reconcile marks this row Recovering with
        // previous_state = JournalCommitted, and a row recovered from
        // JournalCommitted runs row 3 (to GitCommitted) and then row 4 (to
        // IndexCurrent) in the same pass. The GitCommitted step is still
        // pinned below by the Oid it recorded on the row.
        let row = journal.get(op).unwrap().unwrap();
        assert_eq!(row.state, WriteState::IndexCurrent);
        assert_eq!(row.previous_state, Some(WriteState::JournalCommitted));
        let oid = find_commit_for_operation(&journal, &repo_dir, op, ScanDepth::Unbounded).unwrap();
        assert!(oid.is_some());
        assert_eq!(row.git_commit_oid, oid.map(|o| o.to_string()), "row 3 recorded the new commit's Oid on its advance to GitCommitted");
        assert_eq!(report.recovered, vec![op]);
    }

    #[test]
    fn journal_committed_with_an_already_existing_commit_does_not_duplicate_on_reconcile() {
        // This is the Tier 2 (history-scan) repair path: the commit exists but
        // the journal row (deliberately) has no git_commit_oid recorded, since
        // commit_operation was called directly here rather than through the
        // normal chain that would have recorded it.
        let (root, journal) = fresh_root_and_journal();
        let repo_dir = init_test_repo_at(&root);
        let id = NoteId::parse("note-a").unwrap();
        let op = Uuid::new_v4();
        let bytes = serialize_record_for_test("content", Version(1));
        std::fs::write(root.join("note-a.record"), &bytes).unwrap();
        let existing_oid = commit_operation(&repo_dir, op, "note-a", &root.join("note-a.record")).unwrap();
        journal.insert_intent(op, id.as_str(), 0).unwrap();
        journal.advance(op, WriteState::ContentPersisted, AdvanceFields::content(1, &content_hash(&bytes))).unwrap();
        journal.advance(op, WriteState::JournalCommitted, AdvanceFields::none()).unwrap();
        reconcile(&root, &journal, &repo_dir, &NullIndexer, &FaultInjector::none()).unwrap();
        // After reconcile, Tier 1 must now be authoritative: the row itself carries the Oid.
        assert_eq!(journal.get(op).unwrap().unwrap().git_commit_oid, Some(existing_oid.to_string()), "reconcile must repair the journal row with the Tier-2-found Oid");
        assert_eq!(find_commit_for_operation(&journal, &repo_dir, op, ScanDepth::Unbounded).unwrap(), Some(existing_oid));
    }

    #[test]
    fn journal_committed_with_no_git_repository_stays_at_journal_committed_and_does_not_block_open() {
        let (root, journal) = fresh_root_and_journal(); // no git2::Repository::init
        let id = NoteId::parse("note-a").unwrap();
        let op = Uuid::new_v4();
        let bytes = serialize_record_for_test("content", Version(1));
        std::fs::write(root.join("note-a.record"), &bytes).unwrap();
        journal.insert_intent(op, id.as_str(), 0).unwrap();
        journal.advance(op, WriteState::ContentPersisted, AdvanceFields::content(1, &content_hash(&bytes))).unwrap();
        journal.advance(op, WriteState::JournalCommitted, AdvanceFields::none()).unwrap();
        let report = reconcile(&root, &journal, &root, &NullIndexer, &FaultInjector::none()).unwrap();
        assert_eq!(journal.get(op).unwrap().unwrap().state, WriteState::JournalCommitted, "stays pending, not blocked");
        assert_eq!(report.git_pending, vec![op]);
    }

    #[test]
    fn git_committed_reaches_index_current_on_reconcile() {
        let (root, journal) = fresh_root_and_journal();
        let id = NoteId::parse("note-a").unwrap();
        let op = Uuid::new_v4();
        let bytes = serialize_record_for_test("content", Version(1));
        std::fs::write(root.join("note-a.record"), &bytes).unwrap();
        journal.insert_intent(op, id.as_str(), 0).unwrap();
        journal.advance(op, WriteState::ContentPersisted, AdvanceFields::content(1, &content_hash(&bytes))).unwrap();
        journal.advance(op, WriteState::JournalCommitted, AdvanceFields::none()).unwrap();
        journal.advance(op, WriteState::GitCommitted, AdvanceFields::none()).unwrap();
        let report = reconcile(&root, &journal, &no_git_repo(), &NullIndexer, &FaultInjector::none()).unwrap();
        assert_eq!(journal.get(op).unwrap().unwrap().state, WriteState::IndexCurrent);
        assert_eq!(report.recovered, vec![op]);
    }

    #[test]
    fn reconcile_marks_a_row_recovering_with_its_previous_state_before_doing_fixup_work() {
        // Row 5, corrected per the user architectural review (point 6): this
        // proves reconcile itself genuinely writes Recovering as a real,
        // momentarily-persisted step — not a state this plan only describes in
        // prose. An injector interrupts reconcile immediately after it writes
        // Recovering but before it completes that row's fix-up (here: the
        // GitCommitted -> IndexCurrent row), so the row is left genuinely AT
        // Recovering, with previous_state recording what it was before.
        let (root, journal) = fresh_root_and_journal();
        let id = NoteId::parse("note-a").unwrap();
        let op = Uuid::new_v4();
        let bytes = serialize_record_for_test("content", Version(1));
        std::fs::write(root.join("note-a.record"), &bytes).unwrap();
        journal.insert_intent(op, id.as_str(), 0).unwrap();
        journal.advance(op, WriteState::ContentPersisted, AdvanceFields::content(1, &content_hash(&bytes))).unwrap();
        journal.advance(op, WriteState::JournalCommitted, AdvanceFields::none()).unwrap();
        journal.advance(op, WriteState::GitCommitted, AdvanceFields::none()).unwrap();

        let injector = FaultInjector::trigger_at(FaultPoint::AfterMarkedRecoveringBeforeFixup);
        let interrupted = reconcile(&root, &journal, &no_git_repo(), &NullIndexer, &injector);
        assert!(matches!(interrupted, Err(ReconcileError::Interrupted(_))), "reconcile itself must be interruptible mid-fixup, not only the original write");
        let mid_recovery_row = journal.get(op).unwrap().unwrap();
        assert_eq!(mid_recovery_row.state, WriteState::Recovering, "genuinely left at Recovering by the interrupted reconcile, not inserted artificially by the test");
        assert_eq!(mid_recovery_row.previous_state, Some(WriteState::GitCommitted), "previous_state records exactly what row-4 dispatch was about to resume from");

        // A second, uninterrupted reconcile pass must re-dispatch from previous_state
        // and still reach IndexCurrent — proving the re-dispatch-from-Recovering path.
        let report = reconcile(&root, &journal, &no_git_repo(), &NullIndexer, &FaultInjector::none()).unwrap();
        assert_eq!(journal.get(op).unwrap().unwrap().state, WriteState::IndexCurrent);
        assert_eq!(report.recovered, vec![op]);
    }

    // --- Vault::open integration ---

    #[test]
    fn vault_open_refuses_to_start_when_an_operation_is_inconsistent() {
        let root = unique_test_dir();
        let journal_path = root.join("journal.sqlite");
        { // Pre-seed a journal with an unrecoverable row, then drop the connection.
            let journal = Journal::open(&journal_path).unwrap();
            let id = NoteId::parse("note-a").unwrap();
            let op = Uuid::new_v4();
            std::fs::write(root.join("note-a.record"), b"CORRUPTED").unwrap();
            journal.insert_intent(op, id.as_str(), 1).unwrap();
            journal.advance(op, WriteState::ContentPersisted, AdvanceFields::content_with_before_image(
                2, "hash-that-wont-match", "before-image-hash-but-missing-file",
            )).unwrap();
        }
        let result = Vault::open_with_default_indexer(root, &journal_path);
        assert!(matches!(result, Err(StorageError::Inconsistent { .. })));
    }

    #[test]
    fn vault_open_succeeds_and_applies_recovery_for_a_resolvable_journal() {
        let root = unique_test_dir();
        // Deviation from the brief's literal text: the root is also a git
        // repository. Since Task 5, write_record_unchecked extends the chain
        // to GitCommitted, and on a non-git root it returns
        // StorageError::GitPending. The brief's `.unwrap()` on the fresh write
        // below would then panic for a reason that has nothing to do with
        // recovery.
        init_git_repo_at(&root);
        let journal_path = root.join("journal.sqlite");
        let stuck = Uuid::new_v4();
        {
            let journal = Journal::open(&journal_path).unwrap();
            journal.insert_intent(stuck, "note-a", 0).unwrap(); // -> Failed on recovery
        }
        let vault = Vault::open_with_default_indexer(root, &journal_path).unwrap();
        assert_eq!(vault.journal_row_for_test(stuck).unwrap().state, WriteState::Failed, "recovery ran inside open");
        // The vault is usable afterward — e.g. a fresh write to the same note id succeeds.
        let id = NoteId::parse("note-a").unwrap();
        vault.write_record_unchecked(&id, "fresh content", Version(0), Uuid::new_v4()).unwrap();
    }

    #[test]
    fn vault_open_with_injector_propagates_an_interrupted_reconcile_as_storage_error_interrupted() {
        let root = unique_test_dir();
        let journal_path = root.join("journal.sqlite");
        let op = Uuid::new_v4();
        {
            let journal = Journal::open(&journal_path).unwrap();
            journal.insert_intent(op, "note-a", 0).unwrap();
        }
        let injector = FaultInjector::trigger_at(FaultPoint::AfterMarkedRecoveringBeforeFixup);
        let result = Vault::open_with_injector(root.clone(), &journal_path, &injector);
        assert!(matches!(result, Err(StorageError::Interrupted(FaultPoint::AfterMarkedRecoveringBeforeFixup))));
        let row = Journal::open(&journal_path).unwrap().get(op).unwrap().unwrap();
        assert_eq!(row.state, WriteState::Recovering);
        assert_eq!(row.previous_state, Some(WriteState::IntentRecorded));

        // An uninterrupted open resumes from previous_state and resolves it.
        let vault = Vault::open_with_default_indexer(root, &journal_path).unwrap();
        assert_eq!(vault.journal_row_for_test(op).unwrap().state, WriteState::Failed);
    }

    // --- Fix round: independent review findings I-1 and I-2 ---

    #[test]
    fn crash_at_after_rename_on_a_first_write_rolls_forward_to_journal_committed_not_failed() {
        // I-1: AfterRenameBeforeJournalCommitted used to fire after the rename
        // but before the ContentPersisted advance. That left the row at
        // IntentRecorded with new bytes on disk, which row 1 then marked
        // Failed. The note read as NotFound for good. The point now fires after
        // the ContentPersisted advance, so recovery verifies the hash and
        // rolls forward.
        let root = unique_test_dir(); // not a git repo: the row rests at JournalCommitted
        let journal_path = unique_test_dir().join("journal.sqlite");
        let id = NoteId::parse("note-a").unwrap();
        let op = Uuid::new_v4();
        {
            let vault = Vault::open_with_default_indexer(root.clone(), &journal_path).unwrap();
            let injector = FaultInjector::trigger_at(FaultPoint::AfterRenameBeforeJournalCommitted);
            let result = vault.write_record_unchecked_with_injector(&id, "content", Version(0), op, &injector);
            assert!(matches!(result, Err(StorageError::Interrupted(FaultPoint::AfterRenameBeforeJournalCommitted))));
            assert_eq!(vault.journal_row_for_test(op).unwrap().state, WriteState::ContentPersisted, "the fault point fires only once the journal records the rename");
        }
        let vault = Vault::open_with_default_indexer(root, &journal_path).unwrap();
        assert_eq!(vault.journal_row_for_test(op).unwrap().state, WriteState::JournalCommitted, "rolled forward via the hash-verify path, not marked Failed");
        assert_eq!(vault.read_note(&id).unwrap().content, "content");
    }

    #[test]
    fn crash_at_after_rename_over_a_prior_version_rolls_forward_through_to_index_current() {
        // I-1, overwrite variant: before the fix, the prior row's hash no
        // longer matched the new on-disk bytes, so read_note returned
        // Inconsistent for good.
        let root = crate::git::test_support::init_test_repo();
        let journal_path = unique_test_dir().join("journal.sqlite");
        let id = NoteId::parse("note-a").unwrap();
        let op = Uuid::new_v4();
        {
            let vault = Vault::open_with_default_indexer(root.clone(), &journal_path).unwrap();
            vault.write_record_unchecked(&id, "v1", Version(0), Uuid::new_v4()).unwrap();
            let injector = FaultInjector::trigger_at(FaultPoint::AfterRenameBeforeJournalCommitted);
            let result = vault.write_record_unchecked_with_injector(&id, "v2", Version(1), op, &injector);
            assert!(matches!(result, Err(StorageError::Interrupted(_))));
        }
        let vault = Vault::open_with_default_indexer(root, &journal_path).unwrap();
        let row = vault.journal_row_for_test(op).unwrap();
        assert_eq!(row.state, WriteState::IndexCurrent);
        assert_eq!(row.previous_state, Some(WriteState::ContentPersisted));
        let record = vault.read_note(&id).unwrap();
        assert_eq!((record.content.as_str(), record.version), ("v2", Version(2)));
    }

    #[test]
    fn content_persisted_first_write_with_a_mismatched_record_resolves_to_failed_not_inconsistent() {
        // I-1(b): no before_image_hash means there was nothing to restore.
        // That is a failed operation, not an unrecoverable vault.
        let (root, journal) = fresh_root_and_journal();
        let op = Uuid::new_v4();
        std::fs::write(root.join("note-a.record"), b"CORRUPTED").unwrap();
        journal.insert_intent(op, "note-a", 0).unwrap();
        journal.advance(op, WriteState::ContentPersisted, AdvanceFields::content(1, "hash-that-wont-match")).unwrap();
        let report = reconcile(&root, &journal, &no_git_repo(), &NullIndexer, &FaultInjector::none()).unwrap();
        assert_eq!(journal.get(op).unwrap().unwrap().state, WriteState::Failed);
        assert_eq!(report.failed, vec![op]);
        assert!(!root.join("note-a.record").exists(), "no committed version exists, so the unverified record is removed (pre-operation state)");
    }

    /// The bytes of `note-a.record` in commit `oid`'s tree.
    fn committed_record_bytes(repo_dir: &std::path::Path, oid: &str) -> Vec<u8> {
        let repo = git2::Repository::open(repo_dir).unwrap();
        let commit = repo.find_commit(git2::Oid::from_str(oid).unwrap()).unwrap();
        let tree = commit.tree().unwrap();
        let entry = tree.get_name("note-a.record").unwrap();
        let bytes = repo.find_blob(entry.id()).unwrap().content().to_vec();
        bytes
    }

    #[test]
    fn superseded_journal_committed_operation_commits_its_own_content_not_its_successors() {
        // I-2, the reviewer's reproduction. A then B write the same note on a
        // non-git root, so both rest at JournalCommitted. Then the root is
        // git-initialized and the vault reopened. A's recovery commit used to
        // capture B's bytes (whatever was on disk) under A's marker. It must
        // carry A's own bytes, recovered from B's before-image.
        let root = unique_test_dir();
        let journal_path = unique_test_dir().join("journal.sqlite");
        let id = NoteId::parse("note-a").unwrap();
        let (op_a, op_b) = (Uuid::new_v4(), Uuid::new_v4());
        {
            let vault = Vault::open_with_default_indexer(root.clone(), &journal_path).unwrap();
            assert!(matches!(vault.write_record_unchecked(&id, "content A", Version(0), op_a), Err(StorageError::GitPending { .. })));
            assert!(matches!(vault.write_record_unchecked(&id, "content B", Version(1), op_b), Err(StorageError::GitPending { .. })));
        }
        init_git_repo_at(&root);
        let vault = Vault::open_with_default_indexer(root.clone(), &journal_path).unwrap();

        let row_a = vault.journal_row_for_test(op_a).unwrap();
        let row_b = vault.journal_row_for_test(op_b).unwrap();
        assert_eq!(row_a.state, WriteState::IndexCurrent);
        assert_eq!(row_b.state, WriteState::IndexCurrent);
        assert_eq!(
            committed_record_bytes(&root, row_a.git_commit_oid.as_deref().unwrap()),
            serialize_record_for_test("content A", Version(1)),
            "A's commit carries A's OWN content, never B's"
        );
        assert_eq!(
            committed_record_bytes(&root, row_b.git_commit_oid.as_deref().unwrap()),
            serialize_record_for_test("content B", Version(2)),
        );
        assert_ne!(row_a.git_commit_oid, row_b.git_commit_oid);
    }

    #[test]
    fn journal_committed_row_whose_own_content_is_unrecoverable_stays_pending_and_never_commits_other_content() {
        // I-2 fallback: the record no longer holds this operation's bytes and
        // no successor before-image holds them either. Never guess: no commit
        // is made, and the row rests with a reason.
        let (root, journal) = fresh_root_and_journal();
        let repo_dir = init_test_repo_at(&root);
        let op = Uuid::new_v4();
        let own = serialize_record_for_test("mine", Version(1));
        std::fs::write(root.join("note-a.record"), serialize_record_for_test("someone else's", Version(2))).unwrap();
        journal.insert_intent(op, "note-a", 0).unwrap();
        journal.advance(op, WriteState::ContentPersisted, AdvanceFields::content(1, content_hash(&own))).unwrap();
        journal.advance(op, WriteState::JournalCommitted, AdvanceFields::none()).unwrap();
        let report = reconcile(&root, &journal, &repo_dir, &NullIndexer, &FaultInjector::none()).unwrap();
        assert_eq!(journal.get(op).unwrap().unwrap().state, WriteState::JournalCommitted);
        assert_eq!(report.git_pending, vec![op]);
        assert!(report.git_pending_reasons[0].1.contains("own content"));
        assert_eq!(find_commit_for_operation(&journal, &repo_dir, op, ScanDepth::Unbounded).unwrap(), None, "no commit under this marker");
    }
}
