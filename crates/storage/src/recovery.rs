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
//! - `IntentRecorded`: advanced to `Failed`. The caller must retry with a new
//!   `operation_id`.
//! - `ContentPersisted`: if the record on disk hashes to the row's
//!   `content_hash`, roll forward to `JournalCommitted`. Otherwise restore this
//!   operation's before-image, verify it, delete the before-image and advance
//!   the row to `Failed`. Its `new_version` stays as the version it failed to
//!   commit. No git commit is made: the restored content was already
//!   linearized by the earlier operation that wrote it, which
//!   `last_committed_for_note` still resolves to. If the restore is impossible
//!   or does not verify, the result is `Inconsistent`.
//! - `JournalCommitted`: `find_commit_for_operation` with
//!   `ScanDepth::Unbounded`, always. A bounded miss would be ambiguous, and a
//!   `commit_operation` call on that ambiguity could duplicate a commit. A
//!   found `Oid` (including a Tier 2 find the row never recorded) is written
//!   back on the same advance to `GitCommitted`. If there is none,
//!   `commit_operation` runs. If git is unavailable the row rests at
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
use crate::git::{commit_operation, find_commit_for_operation, GitError, ScanDepth};
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
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct ReconcileReport {
    pub recovered: Vec<Uuid>,
    pub failed: Vec<Uuid>,
    pub git_pending: Vec<Uuid>,
}

/// How one row's cascade ended.
enum Outcome {
    /// Reached `IndexCurrent`.
    Completed,
    /// Reached `Failed`.
    Failed,
    /// Resting at `JournalCommitted`, git unavailable.
    GitPending,
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
        match resolve(root, journal, repo_path, indexer, &row, origin)? {
            Outcome::Completed => report.recovered.push(op),
            Outcome::Failed => report.failed.push(op),
            Outcome::GitPending => {
                if origin != WriteState::JournalCommitted {
                    report.recovered.push(op);
                }
                report.git_pending.push(op);
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
) -> Result<Outcome, ReconcileError> {
    let op = row.operation_id;
    let mut at = origin;
    loop {
        at = match at {
            // Row 1: the write never got past its intent.
            WriteState::IntentRecorded => {
                journal.advance(op, WriteState::Failed, AdvanceFields::none())?;
                return Ok(Outcome::Failed);
            }
            // Row 2: roll forward on a verified hash, else roll back to Failed.
            WriteState::ContentPersisted => match fix_content_persisted(root, journal, row)? {
                WriteState::Failed => return Ok(Outcome::Failed),
                next => next,
            },
            // Row 3: idempotent git commit, or rest as git_pending.
            WriteState::JournalCommitted => match fix_journal_committed(root, journal, repo_path, row)? {
                Some(next) => next,
                None => {
                    // Only a row resumed directly from JournalCommitted is still
                    // persisted at Recovering here. One rolled forward this pass
                    // is already at JournalCommitted.
                    if current_state(journal, op)? == WriteState::Recovering {
                        journal.advance(op, WriteState::JournalCommitted, AdvanceFields::none())?;
                    }
                    return Ok(Outcome::GitPending);
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

/// Row 2. Returns `JournalCommitted` (rolled forward) or `Failed` (rolled back).
///
/// The before-image is located by deriving its path from
/// `(root, note_id, operation_id)` via `read_before_image`. The journal's
/// `before_image_path` column is not used, because the write path never fills
/// it. The row's `operation_id` always identifies the before-image uniquely,
/// so the derived path needs no second source of truth.
fn fix_content_persisted(root: &Path, journal: &Journal, row: &OperationRow) -> Result<WriteState, ReconcileError> {
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

    let before_hash = row.before_image_hash.as_deref().ok_or_else(|| {
        inconsistent(op, "record does not match the journal's content_hash and there is no before-image to restore")
    })?;

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

/// Row 3. Returns `Some(GitCommitted)` once the row records its commit, or
/// `None` when git is unavailable (the row is left for the caller to rest).
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
) -> Result<Option<WriteState>, ReconcileError> {
    let op = row.operation_id;
    let note_id = parse_note_id(row)?;
    // Always Unbounded on the recovery path. See the module docs.
    let found = match find_commit_for_operation(journal, repo_path, op, ScanDepth::Unbounded) {
        Ok(found) => found,
        Err(GitError::Journal(e)) => return Err(e.into()),
        Err(_) => return Ok(None),
    };
    let oid = match found {
        Some(oid) => oid,
        None => {
            let record_path = record_path_checked(root, &note_id).map_err(|e| from_storage(op, e))?;
            match commit_operation(repo_path, op, note_id.as_str(), &record_path) {
                Ok(oid) => oid,
                Err(GitError::Journal(e)) => return Err(e.into()),
                Err(_) => return Ok(None),
            }
        }
    };
    // The Oid is recorded on the same write that advances the state, so Tier 1
    // is authoritative from here on. This includes repairing a Tier 2 find.
    journal.advance(op, WriteState::GitCommitted, AdvanceFields::git_oid(oid))?;
    // Per spec, the before-image is deleted only once GitCommitted is reached.
    delete_before_image(root, &note_id, op).map_err(|e| from_storage(op, e))?;
    Ok(Some(WriteState::GitCommitted))
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
}
