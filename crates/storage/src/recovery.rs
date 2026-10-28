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
//! Rows are processed in ascending `new_version` order
//! (`Journal::non_terminal_operations`), so within one note an earlier
//! operation is always resolved before a later one. That matters for row 3:
//! an earlier git-pending operation's own bytes may survive only in its
//! successor's before-image, which the successor deletes once it commits
//! (final whole-branch review, findings I1 and M2). The live write path
//! enforces the same order in `Vault::commit_to_git`.
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
//!   unverified record is also removed. If a later operation has already
//!   committed the note at or past this row's `new_version` (this row
//!   stalled live and a later write committed on top of it), nothing is
//!   restored: the before-image is deleted and the row becomes `Failed`,
//!   leaving the later operation's record untouched (evidence item 26). The
//!   same happens if the before-image is the never-committed content of a
//!   stalled sibling (no committed row carries its hash, a non-committed one
//!   does): two writes stalled in a row, and restoring the second one's
//!   before-image would put the first one's never-committed bytes back on
//!   disk (evidence item 26, second variant). A
//!   recorded before-image that cannot be
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
use vault_security::PiiStore;

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
    pii_store: Option<&PiiStore>,
    injector: &FaultInjector,
) -> Result<ReconcileReport, ReconcileError> {
    let mut report = ReconcileReport::default();
    for row in journal.non_terminal_operations()? {
        let op = row.operation_id;
        let origin = mark_recovering(journal, &row)?;
        injector
            .check(FaultPoint::AfterMarkedRecoveringBeforeFixup)
            .map_err(|Interrupted::At(point)| ReconcileError::Interrupted(point))?;
        match resolve(root, journal, repo_path, indexer, pii_store, &row, origin, injector)? {
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
#[allow(clippy::too_many_arguments)]
fn resolve(
    root: &Path,
    journal: &Journal,
    repo_path: &Path,
    indexer: &dyn Indexer,
    pii_store: Option<&PiiStore>,
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
            WriteState::IntentRecorded => match fix_intent_recorded(root, journal, pii_store, row)? {
                WriteState::Failed => return Ok(Outcome::Failed),
                next => next,
            },
            // Row 2: roll forward on a verified hash, else roll back to Failed.
            WriteState::ContentPersisted => match fix_content_persisted(root, journal, pii_store, row, injector)? {
                WriteState::Failed => return Ok(Outcome::Failed),
                next => next,
            },
            // Row 3: idempotent git commit, or rest as git_pending.
            WriteState::JournalCommitted => match fix_journal_committed(root, journal, repo_path, pii_store, row)? {
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
fn fix_intent_recorded(
    root: &Path,
    journal: &Journal,
    pii_store: Option<&PiiStore>,
    row: &OperationRow,
) -> Result<WriteState, ReconcileError> {
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
            // Task 6: a sensitive write's pointer rename landed, but the
            // ContentPersisted advance did not. Before handing this row to
            // row 2's roll-forward path, verify the staged-or-committed
            // ciphertext this row's `pii_content_hash` names. A mismatch or
            // absence here means the pointer rename landed WITHOUT its
            // ciphertext having been staged/committed consistently — never
            // roll a pointer forward over that. This can only happen if
            // `AfterPiiCommittedBeforePointerRename`'s invariant were
            // violated upstream; checked here anyway, defensively, since
            // this is the one place recovery is authoritative over the
            // pairing.
            if pii_is_consistent(pii_store, &note_id, op, row) {
                return Ok(WriteState::ContentPersisted);
            }
            // A3 review round 2, item #4: the pointer that just landed is
            // the NEW version (whose PII pairing is inconsistent), so it
            // must be rolled BACK to the PRIOR version — restored from the
            // Markdown before-image, exactly as `fix_content_persisted`
            // restores its own before-image on a mismatch — never just
            // deleted out from under a pointer that is left stuck at the
            // bad new version. Only reachable via `.pii` corruption or
            // tampering upstream (never a live-traffic path, since
            // `AfterPiiCommittedBeforePointerRename`'s own invariant
            // guarantees consistency when nothing else interferes), but a
            // row still needs a safe, non-`Inconsistent` resolution here.
            if let Some(before_hash) = row.before_image_hash.as_deref() {
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
            } else if journal.last_committed_for_note(note_id.as_str())?.is_none() {
                // No before-image means this was a first write with no
                // prior pointer; nothing to restore to, so the record (if
                // any landed) is removed, mirroring `fix_content_persisted`'s
                // own first-write rollback.
                match std::fs::remove_file(&record_path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(ReconcileError::Io(e)),
                }
            }
            roll_back_pii(journal, pii_store, &note_id, op, row)?;
            delete_before_image(root, &note_id, op).map_err(|e| from_storage(op, e))?;
            journal.advance(op, WriteState::Failed, AdvanceFields::none())?;
            return Ok(WriteState::Failed);
        }
    }
    // Deleted BEFORE the terminal advance, as in row 2, so a crash in
    // between cannot orphan it under a row that is never revisited.
    delete_before_image(root, &note_id, op).map_err(|e| from_storage(op, e))?;
    // Task 6: the pointer rename never landed (this row's intended bytes
    // are not on disk, or there was no intended hash at all). This pipeline
    // commits the ciphertext (stage + commit_staged) strictly BEFORE the
    // pointer rename, so a row with a `pii_content_hash` can legitimately
    // have already made ITS OWN bytes the live `.pii` file at this point,
    // even though its pointer never landed — the
    // `AfterPiiCommittedBeforePointerRename` window. `roll_back_pii`
    // decides, from a verified hash, whether the live file is this
    // operation's own (restore its before-image, or remove it outright on
    // a first write with nothing to restore to) or belongs to an earlier,
    // already-resolved operation (left untouched) — this is the exact
    // multi-store consistency guard the crash-on-update test targets.
    roll_back_pii(journal, pii_store, &note_id, op, row)?;
    journal.advance(op, WriteState::Failed, AdvanceFields::none())?;
    Ok(WriteState::Failed)
}

/// Task 6: `true` iff `row` has no `pii_content_hash` (an ordinary,
/// non-sensitive write — always consistent from this check's point of view)
/// OR `pii_store` has a staged-or-committed ciphertext for `note_id` whose
/// decrypted plaintext hashes to `row.pii_content_hash`. Checks the LIVE
/// `.pii` file first (the common case once `PiiStore::commit_staged` has
/// already run), then the staging file for this exact `operation_id` (the
/// crash-between-stage-and-commit window `AfterPiiStagedBeforeCommit`
/// characterizes). `pii_store: None` with a row that DOES carry a
/// `pii_content_hash` is never consistent — there is no way to verify it
/// without a store to check against, so it fails closed rather than
/// silently rolling forward unverified.
fn pii_is_consistent(pii_store: Option<&PiiStore>, note_id: &NoteId, op: Uuid, row: &OperationRow) -> bool {
    let Some(expected) = row.pii_content_hash.as_deref() else {
        return true;
    };
    let Some(store) = pii_store else {
        return false;
    };
    if let Ok(plaintext) = store.get(note_id.as_str()) {
        if content_hash(&plaintext) == expected {
            return true;
        }
    }
    if let Ok(plaintext) = store.get_staged(note_id.as_str(), op) {
        if content_hash(&plaintext) == expected {
            return true;
        }
    }
    false
}

/// Task 6: discards whatever this operation left STAGED (never committed,
/// or committed but this row is being rolled back anyway) in `PiiStore`, as
/// part of rolling this operation back to `Failed`. Never touches the LIVE
/// `.pii` file — see [`roll_back_pii`] for that, which must also decide
/// between restoring a before-image and removing the live file outright,
/// never just discarding. A no-op when `pii_store` is `None`.
fn discard_pii(pii_store: Option<&PiiStore>, note_id: &NoteId, op: Uuid) {
    let Some(store) = pii_store else { return };
    let _ = store.discard_staged(note_id.as_str(), op);
}

/// Task 6 (A3 review round, CRITICAL fix): rolls back `PiiStore`'s state for
/// `operation_id` as part of failing this row, exactly mirroring
/// `storage::vault::roll_back_pii`'s logic (recovery cannot call that
/// function directly — it has no `&Vault` — so this is recovery's own copy
/// of the same decision):
///
/// - Always discards this operation's staged (never-promoted, or
///   promoted-then-rolled-back) file first.
/// - If this operation's own content is currently the live `.pii` file
///   (verified by hash against `row.pii_content_hash`, never assumed):
///   - If a PII before-image exists for `(note_id, operation_id)` (an
///     UPDATE — `PiiStore::commit_staged` preserved the prior,
///     already-committed operation's ciphertext before overwriting the
///     live file), RESTORE it — verified against the PRIOR operation's OWN
///     recorded `pii_content_hash` (looked up via
///     `Journal::last_committed_for_note`, exactly the hash the before-image
///     must decrypt to, since it IS that prior operation's committed
///     ciphertext — never this failing operation's own hash, which
///     describes different content), so the live file goes back to the
///     PRIOR operation's ciphertext rather than being left empty or
///     verified against the wrong expectation. The before-image is then
///     deleted (it has been consumed). If no prior committed row can be
///     found (should not happen once a before-image exists, since the
///     before-image is only ever created by overwriting a prior live file —
///     treated as `Inconsistent`, never silently guessed).
///   - If no before-image exists (a FIRST write — there was nothing to
///     restore the live file TO), remove the live file outright.
/// - If this operation's own content is NOT the live file (the live file,
///   if any, belongs to an earlier, already-resolved operation — this row
///   never got as far as `commit_staged`, or a later legitimate operation's
///   content is already there), nothing about the live file is touched.
///
/// A row with no `pii_content_hash` at all (an ordinary, non-sensitive
/// write) is a complete no-op beyond the staged-file discard, which is
/// itself already a no-op for such a row (nothing was ever staged for it).
fn roll_back_pii(
    journal: &Journal,
    pii_store: Option<&PiiStore>,
    note_id: &NoteId,
    op: Uuid,
    row: &OperationRow,
) -> Result<(), ReconcileError> {
    // A3 review round, IMPORTANT finding #5: a sensitive row with no
    // `PiiStore` configured for this reconcile pass must fail CLOSED
    // (`Inconsistent`, blocking `Vault::open` itself), never silently roll
    // back without ever touching `.pii` — the live ciphertext this row's
    // pointer may still legitimately describe (or may need cleaning up)
    // cannot be reasoned about at all without a store to check it against.
    if pii_store.is_none() && row.pii_content_hash.is_some() {
        return Err(inconsistent(
            op,
            "this operation's journal row carries a pii_content_hash but no PiiStore was supplied to reconcile; \
             cannot verify or roll back its PII state — open the vault with Vault::open_with_pii_store instead",
        ));
    }
    discard_pii(pii_store, note_id, op);
    let (Some(store), Some(expected)) = (pii_store, row.pii_content_hash.as_deref()) else {
        return Ok(());
    };
    let live_is_ours = matches!(store.get(note_id.as_str()), Ok(live_plaintext) if content_hash(&live_plaintext) == expected);
    if !live_is_ours {
        // A3 review round 2, NEW finding #1 ("not ours" branch): the live
        // file belongs to a different, already-resolved operation (an
        // earlier one this row never got to overwrite, or a LATER
        // legitimate operation that has since overwritten it again), so
        // the live file itself is correctly left untouched. But if THIS
        // failing operation's OWN `commit_staged` call ran before it was
        // superseded, it left its OWN before-image on disk
        // (`<note_id>-{op}.pii`, keyed by this op's own operation_id) —
        // an orphan now that this row is being failed, never reachable
        // through the live file again. Delete it unconditionally; it is
        // never legitimate content for ANY other operation (a
        // before-image is created only under the operation_id of the
        // write that is about to overwrite the live file, so no other
        // row's recovery could ever need THIS op's before-image).
        store
            .delete_before_image(note_id.as_str(), op)
            .map_err(|e| from_storage(op, StorageError::PiiStore(e.to_string())))?;
        return Ok(());
    }
    if !store.has_before_image(note_id.as_str(), op) {
        remove_live_pii(pii_store, note_id);
        return Ok(());
    }
    let prior_hash = journal
        .last_committed_for_note(note_id.as_str())?
        .and_then(|prior| prior.pii_content_hash)
        .ok_or_else(|| inconsistent(op, "a PII before-image exists but no prior committed operation's pii_content_hash could be found to verify it against"))?;
    let restored = store
        .restore_before_image(note_id.as_str(), op, &prior_hash)
        .map_err(|e| from_storage(op, StorageError::PiiStore(e.to_string())))?;
    if !restored {
        return Err(inconsistent(op, "PII before-image disappeared between the existence check and the restore attempt"));
    }
    store
        .delete_before_image(note_id.as_str(), op)
        .map_err(|e| from_storage(op, StorageError::PiiStore(e.to_string())))?;
    Ok(())
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
    pii_store: Option<&PiiStore>,
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
        // Task 6: the pointer's own bytes verify, but for a sensitive write
        // that is only half the pairing — the staged-or-committed
        // ciphertext this row names must ALSO verify before rolling
        // forward. Falling through to the rollback below (rather than
        // returning early here) reuses the exact same before-image-restore
        // machinery that already exists for an ordinary content mismatch:
        // from this row's point of view, "my ciphertext is missing or
        // wrong" is handled identically to "my pointer bytes are missing or
        // wrong" — both mean this operation never validly completed.
        if pii_is_consistent(pii_store, &note_id, op, row) {
            journal.advance(op, WriteState::JournalCommitted, AdvanceFields::none())?;
            return Ok(WriteState::JournalCommitted);
        }
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
        // Task 6: no Markdown before-image means a first write — `roll_back_pii`
        // correctly removes this operation's own live `.pii` file outright
        // (nothing to restore to) when it is this operation's own content,
        // and never touches it otherwise.
        roll_back_pii(journal, pii_store, &note_id, op, row)?;
        journal.advance(op, WriteState::Failed, AdvanceFields::none())?;
        return Ok(WriteState::Failed);
    };

    // A later operation on this note has already committed at or past this
    // row's version (evidence item 26: this row stalled live, and a later
    // write, accepted against the raw file, committed on top of it). The
    // record on disk belongs to that operation, so restoring this row's
    // before-image would destroy committed content. Skip the restore: the
    // before-image is superseded, and this row simply failed, as in row 1.
    // The later row and the record are left untouched; if the record does
    // not match that row's hash, `read_note` still reports `Inconsistent`.
    if let Some(later) = journal.last_committed_for_note(note_id.as_str())? {
        if matches!((later.new_version, row.new_version), (Some(l), Some(mine)) if l >= mine) {
            delete_before_image(root, &note_id, op).map_err(|e| from_storage(op, e))?;
            roll_back_pii(journal, pii_store, &note_id, op, row)?;
            journal.advance(op, WriteState::Failed, AdvanceFields::none())?;
            return Ok(WriteState::Failed);
        }
    }

    // The before-image is the never-committed content of a stalled sibling
    // (evidence item 26, second variant): an earlier operation on this note
    // renamed those bytes into place and never committed them, and this row
    // was accepted against them. No committed row carries that hash, while
    // a non-committed one does. Restoring it would put content nobody
    // committed back on disk, where the raw-version check would build on it.
    // Skip the restore, as above. The earlier sibling (resolved first, in
    // version order) either restored the last committed content via row 2,
    // or, if it was still IntentRecorded, row 1 left the record alone
    // without restoring anything. Either way this row's before-image is not
    // legitimate committed content. Decided from the journal alone, so a
    // pass that already deleted the before-image resolves the same way.
    if before_image_is_uncommitted_sibling_content(journal, &note_id, op, before_hash)? {
        delete_before_image(root, &note_id, op).map_err(|e| from_storage(op, e))?;
        roll_back_pii(journal, pii_store, &note_id, op, row)?;
        journal.advance(op, WriteState::Failed, AdvanceFields::none())?;
        return Ok(WriteState::Failed);
    }

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
    // Task 6 (A3 review round, CRITICAL fix): this is the rollback branch
    // the crash-on-update design targets — the pointer has just been
    // restored to its PREVIOUS version. `roll_back_pii` RESTORES the live
    // `.pii` file from this operation's own PII before-image (if this
    // operation's own content is currently live AND a before-image exists
    // — the UPDATE case: the live file goes back to the PRIOR operation's
    // ciphertext, never left empty) rather than merely removing it; it
    // removes outright only on a FIRST write (nothing to restore to), and
    // leaves the live file alone entirely if it belongs to a different
    // operation. Never unconditional, never a bare discard of the only
    // surviving copy of committed content.
    roll_back_pii(journal, pii_store, &note_id, op, row)?;
    journal.advance(op, WriteState::Failed, AdvanceFields::none())?;
    Ok(WriteState::Failed)
}

/// Task 6: removes the live `.pii` file for `note_id`, if `pii_store` is
/// configured. Used by the rollback paths above where this operation's own
/// ciphertext — committed or not — must not survive as an orphan once its
/// pointer has been rolled back.
fn remove_live_pii(pii_store: Option<&PiiStore>, note_id: &NoteId) {
    let Some(store) = pii_store else { return };
    let _ = store.remove_live(note_id.as_str());
}

/// Row 2's second guard. `true` when `before_hash` is the content of some
/// other operation on `note_id` that never committed (any state before
/// `JournalCommitted`, or `Failed`), and of no operation that did. A hash
/// that some committed row carries is legitimate content even if a failed
/// row also carries it (same bytes, same version). A hash that no row
/// carries at all (not written by any journaled operation) is not treated
/// as a sibling: the existing restore path, and its `Inconsistent` on a
/// missing or unverifiable before-image, are unchanged for it.
fn before_image_is_uncommitted_sibling_content(
    journal: &Journal,
    note_id: &NoteId,
    op: Uuid,
    before_hash: &str,
) -> Result<bool, ReconcileError> {
    let rows = journal.operations_with_content_hash(note_id.as_str(), before_hash)?;
    let committed = |state: Option<WriteState>| {
        matches!(state, Some(WriteState::JournalCommitted | WriteState::GitCommitted | WriteState::IndexCurrent))
    };
    // A row mid-recovery counts by the state it is being recovered from. Rows
    // are resolved in version order, so a sibling below this row's version
    // is normally resolved already; this only keeps the check conservative.
    if rows.iter().any(|r| {
        committed(Some(r.state)) || (r.state == WriteState::Recovering && committed(r.previous_state))
    }) {
        return Ok(false);
    }
    Ok(rows.iter().any(|r| r.operation_id != op))
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
    pii_store: Option<&PiiStore>,
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
    // A3 review round 2, NEW finding #1: the PII before-image must be
    // deleted on the EXACT SAME schedule as the Markdown one — once
    // GitCommitted is reached — never left behind. This is the only place
    // in `recovery.rs` a row resumed directly from `JournalCommitted` (a
    // crash after the linearization point but before the git commit)
    // reaches its point of no return; the live-write path's equivalent
    // moment is `Vault::commit_to_git`, which already does this. Before
    // this fix, a tombstone/deletion write's PII before-image (the REAL,
    // now-deleted content) could survive a successful roll-forward
    // recovery indefinitely, still decryptable by the live master key —
    // directly undermining spec contract C5.
    if let Some(store) = pii_store {
        store.delete_before_image(note_id.as_str(), op).map_err(|e| from_storage(op, StorageError::PiiStore(e.to_string())))?;
    }
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
        let report = reconcile(&root, &journal, &no_git_repo(), &NullIndexer, None, &FaultInjector::none()).unwrap();
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
        let report = reconcile(&root, &journal, &no_git_repo(), &NullIndexer, None, &FaultInjector::none()).unwrap();
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
        let report = reconcile(&root, &journal, &no_git_repo(), &NullIndexer, None, &FaultInjector::none()).unwrap();
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
        journal.advance(op, WriteState::ContentPersisted, AdvanceFields::content(1, content_hash(&bytes))).unwrap();
        let report = reconcile(&root, &journal, &no_git_repo(), &NullIndexer, None, &FaultInjector::none()).unwrap();
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
        journal.advance(op_prev, WriteState::ContentPersisted, AdvanceFields::content(1, content_hash(&before_bytes))).unwrap();
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
            2, "expected-hash-that-will-not-match", content_hash(&before_bytes),
        )).unwrap();

        let report = reconcile(&root, &journal, &no_git_repo(), &NullIndexer, None, &FaultInjector::none()).unwrap();

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
            2, "expected-hash-that-will-not-match", content_hash(&before_bytes),
        )).unwrap();

        let injector = FaultInjector::trigger_at(FaultPoint::AfterMarkedRecoveringBeforeFixup);
        let interrupted = reconcile(&root, &journal, &no_git_repo(), &NullIndexer, None, &injector);
        assert!(matches!(interrupted, Err(ReconcileError::Interrupted(_))), "row 2 must go through the same generic Recovering-marking step as every other row, not a special-cased shortcut");
        let mid_recovery_row = journal.get(op).unwrap().unwrap();
        assert_eq!(mid_recovery_row.state, WriteState::Recovering, "genuinely at Recovering, not fast-forwarded past it");
        assert_eq!(mid_recovery_row.previous_state, Some(WriteState::ContentPersisted));

        // A second, uninterrupted pass must still resolve to Failed, per the test above.
        let report = reconcile(&root, &journal, &no_git_repo(), &NullIndexer, None, &FaultInjector::none()).unwrap();
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
        let err = reconcile(&root, &journal, &no_git_repo(), &NullIndexer, None, &FaultInjector::none()).unwrap_err();
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
        journal.advance(op, WriteState::ContentPersisted, AdvanceFields::content(1, content_hash(&bytes))).unwrap();
        journal.advance(op, WriteState::JournalCommitted, AdvanceFields::none()).unwrap();
        let report = reconcile(&root, &journal, &repo_dir, &NullIndexer, None, &FaultInjector::none()).unwrap();
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
        journal.advance(op, WriteState::ContentPersisted, AdvanceFields::content(1, content_hash(&bytes))).unwrap();
        journal.advance(op, WriteState::JournalCommitted, AdvanceFields::none()).unwrap();
        reconcile(&root, &journal, &repo_dir, &NullIndexer, None, &FaultInjector::none()).unwrap();
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
        journal.advance(op, WriteState::ContentPersisted, AdvanceFields::content(1, content_hash(&bytes))).unwrap();
        journal.advance(op, WriteState::JournalCommitted, AdvanceFields::none()).unwrap();
        let report = reconcile(&root, &journal, &root, &NullIndexer, None, &FaultInjector::none()).unwrap();
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
        journal.advance(op, WriteState::ContentPersisted, AdvanceFields::content(1, content_hash(&bytes))).unwrap();
        journal.advance(op, WriteState::JournalCommitted, AdvanceFields::none()).unwrap();
        journal.advance(op, WriteState::GitCommitted, AdvanceFields::none()).unwrap();
        let report = reconcile(&root, &journal, &no_git_repo(), &NullIndexer, None, &FaultInjector::none()).unwrap();
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
        journal.advance(op, WriteState::ContentPersisted, AdvanceFields::content(1, content_hash(&bytes))).unwrap();
        journal.advance(op, WriteState::JournalCommitted, AdvanceFields::none()).unwrap();
        journal.advance(op, WriteState::GitCommitted, AdvanceFields::none()).unwrap();

        let injector = FaultInjector::trigger_at(FaultPoint::AfterMarkedRecoveringBeforeFixup);
        let interrupted = reconcile(&root, &journal, &no_git_repo(), &NullIndexer, None, &injector);
        assert!(matches!(interrupted, Err(ReconcileError::Interrupted(_))), "reconcile itself must be interruptible mid-fixup, not only the original write");
        let mid_recovery_row = journal.get(op).unwrap().unwrap();
        assert_eq!(mid_recovery_row.state, WriteState::Recovering, "genuinely left at Recovering by the interrupted reconcile, not inserted artificially by the test");
        assert_eq!(mid_recovery_row.previous_state, Some(WriteState::GitCommitted), "previous_state records exactly what row-4 dispatch was about to resume from");

        // A second, uninterrupted reconcile pass must re-dispatch from previous_state
        // and still reach IndexCurrent — proving the re-dispatch-from-Recovering path.
        let report = reconcile(&root, &journal, &no_git_repo(), &NullIndexer, None, &FaultInjector::none()).unwrap();
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
        let report = reconcile(&root, &journal, &no_git_repo(), &NullIndexer, None, &FaultInjector::none()).unwrap();
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
    fn reconcile_resolves_pending_operations_on_a_note_in_version_order_not_insertion_order() {
        // Final whole-branch review, M2 (a dependency of I1's fix): insert_intent
        // runs before the per-note lock, so a later-committing operation can
        // have the LOWER rowid. Here op_b (version 2) was inserted first. In
        // rowid order reconcile would commit op_b first and delete its
        // before-image, the only copy of op_a's bytes, stranding op_a.
        let (root, journal) = fresh_root_and_journal();
        let repo_dir = init_test_repo_at(&root);
        let id = NoteId::parse("note-a").unwrap();
        let a_bytes = serialize_record_for_test("content A", Version(1));
        let b_bytes = serialize_record_for_test("content B", Version(2));
        let (op_a, op_b) = (Uuid::new_v4(), Uuid::new_v4());
        journal.insert_intent(op_b, id.as_str(), 1).unwrap(); // lower rowid
        journal.insert_intent(op_a, id.as_str(), 0).unwrap();
        journal.advance(op_a, WriteState::ContentPersisted, AdvanceFields::content(1, content_hash(&a_bytes))).unwrap();
        journal.advance(op_a, WriteState::JournalCommitted, AdvanceFields::none()).unwrap();
        let before_b = persist_before_image(&root, &id, op_b, &a_bytes).unwrap();
        std::fs::write(root.join("note-a.record"), &b_bytes).unwrap();
        journal.advance(op_b, WriteState::ContentPersisted, AdvanceFields::content_with_before_image(2, content_hash(&b_bytes), before_b)).unwrap();
        journal.advance(op_b, WriteState::JournalCommitted, AdvanceFields::none()).unwrap();

        let report = reconcile(&root, &journal, &repo_dir, &NullIndexer, None, &FaultInjector::none()).unwrap();
        assert_eq!(report.recovered, vec![op_a, op_b], "resolved in version order");
        assert!(report.git_pending.is_empty(), "nothing stranded: {:?}", report.git_pending_reasons);
        assert_eq!(journal.get(op_a).unwrap().unwrap().state, WriteState::IndexCurrent);
        assert_eq!(journal.get(op_b).unwrap().unwrap().state, WriteState::IndexCurrent);
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
        let report = reconcile(&root, &journal, &repo_dir, &NullIndexer, None, &FaultInjector::none()).unwrap();
        assert_eq!(journal.get(op).unwrap().unwrap().state, WriteState::JournalCommitted);
        assert_eq!(report.git_pending, vec![op]);
        assert!(report.git_pending_reasons[0].1.contains("own content"));
        assert_eq!(find_commit_for_operation(&journal, &repo_dir, op, ScanDepth::Unbounded).unwrap(), None, "no commit under this marker");
    }

    // --- Evidence item 26: a stalled ContentPersisted row superseded live ---

    /// Builds the evidence item 26 state on a git-backed vault and drops it:
    /// v1 committed by a first write; op_a (v1 -> v2) stalls live after the
    /// rename and the ContentPersisted advance (the same `Vault` keeps being
    /// used, nothing is reopened); then op_b (v2 -> v3), accepted because the
    /// raw file says v2, commits all the way to IndexCurrent.
    ///
    /// Since the live write guard (`Journal::unresolved_operation_on_note`,
    /// evidence item 26 second variant) the write path refuses op_b, so this
    /// state can now only come from a journal written before that guard.
    /// The helper checks the refusal, then builds op_b's committed row and
    /// record by hand exactly as the write path used to leave them, so row
    /// 2's later-commit guard (commit 88b20f6) stays covered.
    fn stalled_a_then_committed_b(root: &std::path::Path, journal_path: &std::path::Path, op_a: Uuid, op_b: Uuid) {
        let id = NoteId::parse("note-a").unwrap();
        let vault = Vault::open_with_default_indexer(root.to_path_buf(), journal_path).unwrap();
        vault.write_record_unchecked(&id, "v1", Version(0), Uuid::new_v4()).unwrap();
        let injector = FaultInjector::trigger_at(FaultPoint::AfterRenameBeforeJournalCommitted);
        let stalled = vault.write_record_unchecked_with_injector(&id, "v2 from A", Version(1), op_a, &injector);
        assert!(matches!(stalled, Err(StorageError::Interrupted(_))));
        assert_eq!(vault.journal_row_for_test(op_a).unwrap().state, WriteState::ContentPersisted);
        let refused = vault.write_record_unchecked(&id, "v3 from B", Version(2), Uuid::new_v4());
        assert!(matches!(refused, Err(StorageError::UnresolvedOperation { operation_id, .. }) if operation_id == op_a), "got {refused:?}");

        let a_bytes = std::fs::read(root.join("note-a.record")).unwrap();
        let b_bytes = serialize_record_for_test("v3 from B", Version(3));
        {
            let journal = vault.journal_for_test();
            journal.insert_intent(op_b, id.as_str(), 2).unwrap();
            let before_b = persist_before_image(root, &id, op_b, &a_bytes).unwrap();
            let fields = AdvanceFields::content_with_before_image(3, content_hash(&b_bytes), before_b);
            journal.record_fields(op_b, WriteState::IntentRecorded, fields.clone()).unwrap();
            std::fs::write(root.join("note-a.record"), &b_bytes).unwrap();
            journal.advance(op_b, WriteState::ContentPersisted, fields).unwrap();
            journal.advance(op_b, WriteState::JournalCommitted, AdvanceFields::none()).unwrap();
            journal.advance(op_b, WriteState::GitCommitted, AdvanceFields::none()).unwrap();
            delete_before_image(root, &id, op_b).unwrap();
            journal.advance(op_b, WriteState::IndexCurrent, AdvanceFields::none()).unwrap();
        }
        assert_eq!(vault.read_note(&id).unwrap().content, "v3 from B", "live, B is readable before any reopen");
    }

    #[test]
    fn superseded_content_persisted_row_resolves_idempotently_across_interrupted_and_repeated_passes() {
        let root = crate::git::test_support::init_test_repo();
        let journal_path = unique_test_dir().join("journal.sqlite");
        let id = NoteId::parse("note-a").unwrap();
        let (op_a, op_b) = (Uuid::new_v4(), Uuid::new_v4());
        stalled_a_then_committed_b(&root, &journal_path, op_a, op_b);
        let b_bytes = serialize_record_for_test("v3 from B", Version(3));

        // A pass interrupted right after marking A Recovering.
        let injector = FaultInjector::trigger_at(FaultPoint::AfterMarkedRecoveringBeforeFixup);
        let interrupted = Vault::open_with_injector(root.clone(), &journal_path, &injector);
        assert!(matches!(interrupted, Err(StorageError::Interrupted(_))));

        // A pass that deleted A's before-image and stopped before the Failed
        // advance: the guard does not depend on the before-image existing.
        delete_before_image(&root, &id, op_a).unwrap();
        let journal = Journal::open(&journal_path).unwrap();
        let first = reconcile(&root, &journal, &root, &NullIndexer, None, &FaultInjector::none()).unwrap();
        assert_eq!(first.failed, vec![op_a]);
        let second = reconcile(&root, &journal, &root, &NullIndexer, None, &FaultInjector::none()).unwrap();
        assert_eq!(second, ReconcileReport::default(), "a second pass finds nothing left to do");
        drop(journal);

        let vault = Vault::open_with_default_indexer(root.clone(), &journal_path).unwrap();
        assert_eq!(vault.journal_row_for_test(op_a).unwrap().state, WriteState::Failed);
        assert_eq!(vault.journal_row_for_test(op_b).unwrap().state, WriteState::IndexCurrent);
        assert_eq!(std::fs::read(root.join("note-a.record")).unwrap(), b_bytes);
        assert_eq!(vault.read_note(&id).unwrap().content, "v3 from B");
    }

    #[test]
    fn superseded_row_over_a_corrupted_record_fails_closed_and_leaves_the_record_alone() {
        // The guard never "repairs" anything: if the record no longer
        // matches the later committed row, it is left as it is and read_note
        // still reports Inconsistent.
        let root = crate::git::test_support::init_test_repo();
        let journal_path = unique_test_dir().join("journal.sqlite");
        let id = NoteId::parse("note-a").unwrap();
        let (op_a, op_b) = (Uuid::new_v4(), Uuid::new_v4());
        stalled_a_then_committed_b(&root, &journal_path, op_a, op_b);
        std::fs::write(root.join("note-a.record"), b"CORRUPTED").unwrap();

        let vault = Vault::open_with_default_indexer(root.clone(), &journal_path).unwrap();
        assert_eq!(vault.journal_row_for_test(op_a).unwrap().state, WriteState::Failed);
        assert_eq!(std::fs::read(root.join("note-a.record")).unwrap(), b"CORRUPTED", "nothing restored over it");
        assert!(matches!(vault.read_note(&id), Err(StorageError::Inconsistent { .. })));
    }

    #[test]
    fn row_2_never_restores_a_before_image_over_a_later_operation_that_already_committed() {
        let root = crate::git::test_support::init_test_repo();
        let journal_path = unique_test_dir().join("journal.sqlite");
        let id = NoteId::parse("note-a").unwrap();
        let (op_a, op_b) = (Uuid::new_v4(), Uuid::new_v4());
        stalled_a_then_committed_b(&root, &journal_path, op_a, op_b);
        let b_row_before = Journal::open(&journal_path).unwrap().get(op_b).unwrap().unwrap();
        let b_bytes = serialize_record_for_test("v3 from B", Version(3));

        let vault = Vault::open_with_default_indexer(root.clone(), &journal_path).unwrap();

        // (b) A resolves to Failed, its before-image gone.
        assert_eq!(vault.journal_row_for_test(op_a).unwrap().state, WriteState::Failed);
        assert!(read_before_image(&root, &id, op_a).is_err(), "A's superseded before-image is deleted");
        // (a) B's content and B's row are untouched.
        assert_eq!(std::fs::read(root.join("note-a.record")).unwrap(), b_bytes, "B's committed bytes survive recovery");
        assert_eq!(vault.journal_row_for_test(op_b).unwrap(), b_row_before, "B's journal row is untouched");
        // (c) The note reads at B's version and content, not Inconsistent.
        let record = vault.read_note(&id).unwrap();
        assert_eq!((record.content.as_str(), record.version), ("v3 from B", Version(3)));
        // Step 7 of the item-26 sequence: a write at A's rolled-back version
        // (v1) must be refused, not accepted on top of diverged content.
        let err = vault.write_record_unchecked(&id, "diverging", Version(1), Uuid::new_v4()).unwrap_err();
        assert!(matches!(err, StorageError::Conflict { actual: Version(3), .. }), "got {err:?}");
        // The legitimate next write still works.
        assert_eq!(vault.write_record_unchecked(&id, "v4", Version(3), Uuid::new_v4()).unwrap(), Version(4));
    }

    // --- Evidence item 26, second variant: two stalled siblings, nothing committed past them ---

    /// Seeds, in the journal at `journal_path` and on disk at `root`, the
    /// second-variant state: `op_1` committed v1 (all the way to
    /// IndexCurrent); `op_a` (v1 -> v2) renamed its bytes into place and
    /// stalled at ContentPersisted; `op_b` (v2 -> v3), accepted against the
    /// raw file's v2, did the same. B's before-image is therefore A's v2
    /// bytes, which no operation ever committed. Nothing committed past A or
    /// B. Returns the serialized v1, v2 (A) and v3 (B) records.
    fn seed_two_stalled_siblings(root: &std::path::Path, journal: &Journal, op_1: Uuid, op_a: Uuid, op_b: Uuid) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        let id = NoteId::parse("note-a").unwrap();
        let v1 = serialize_record_for_test("v1", Version(1));
        let a = serialize_record_for_test("v2 from A", Version(2));
        let b = serialize_record_for_test("v3 from B", Version(3));
        journal.insert_intent(op_1, id.as_str(), 0).unwrap();
        journal.advance(op_1, WriteState::ContentPersisted, AdvanceFields::content(1, content_hash(&v1))).unwrap();
        journal.advance(op_1, WriteState::JournalCommitted, AdvanceFields::none()).unwrap();
        journal.advance(op_1, WriteState::GitCommitted, AdvanceFields::none()).unwrap();
        journal.advance(op_1, WriteState::IndexCurrent, AdvanceFields::none()).unwrap();
        let before_a = persist_before_image(root, &id, op_a, &v1).unwrap();
        journal.insert_intent(op_a, id.as_str(), 1).unwrap();
        journal.advance(op_a, WriteState::ContentPersisted, AdvanceFields::content_with_before_image(2, content_hash(&a), before_a)).unwrap();
        let before_b = persist_before_image(root, &id, op_b, &a).unwrap();
        journal.insert_intent(op_b, id.as_str(), 2).unwrap();
        journal.advance(op_b, WriteState::ContentPersisted, AdvanceFields::content_with_before_image(3, content_hash(&b), before_b)).unwrap();
        std::fs::write(root.join("note-a.record"), &b).unwrap();
        (v1, a, b)
    }

    #[test]
    fn row_2_never_restores_a_stalled_siblings_never_committed_content_as_a_before_image() {
        let root = crate::git::test_support::init_test_repo();
        let journal_path = unique_test_dir().join("journal.sqlite");
        let id = NoteId::parse("note-a").unwrap();
        let (op_1, op_a, op_b) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let (v1, _a, _b) = seed_two_stalled_siblings(&root, &Journal::open(&journal_path).unwrap(), op_1, op_a, op_b);

        let vault = Vault::open_with_default_indexer(root.clone(), &journal_path).unwrap();

        assert_eq!(vault.journal_row_for_test(op_a).unwrap().state, WriteState::Failed);
        assert_eq!(vault.journal_row_for_test(op_b).unwrap().state, WriteState::Failed);
        assert_eq!(std::fs::read(root.join("note-a.record")).unwrap(), v1, "the record holds the last committed content (v1), never A's never-committed v2");
        assert!(read_before_image(&root, &id, op_a).is_err() && read_before_image(&root, &id, op_b).is_err(), "no before-image is orphaned");
        assert_eq!(vault.journal_for_test().last_committed_for_note("note-a").unwrap().unwrap().operation_id, op_1);
        let record = vault.read_note(&id).unwrap();
        assert_eq!((record.content.as_str(), record.version), ("v1", Version(1)), "readable, not Inconsistent");
        // A write at A's never-committed version must not be accepted on top of it.
        let err = vault.write_record_unchecked(&id, "diverging", Version(2), Uuid::new_v4()).unwrap_err();
        assert!(matches!(err, StorageError::Conflict { actual: Version(1), .. }), "got {err:?}");
        assert_eq!(vault.write_record_unchecked(&id, "v2 for real", Version(1), Uuid::new_v4()).unwrap(), Version(2));
    }

    #[test]
    fn sibling_before_image_guard_is_idempotent_across_an_interrupted_and_a_repeated_pass() {
        // A pass that already resolved A, marked B Recovering and deleted B's
        // before-image, then stopped before B's Failed advance. The guard is
        // decided from the journal alone, so it does not need the file.
        let (root, journal) = fresh_root_and_journal();
        let id = NoteId::parse("note-a").unwrap();
        let (op_1, op_a, op_b) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let (v1, _a, _b) = seed_two_stalled_siblings(&root, &journal, op_1, op_a, op_b);
        let injector = FaultInjector::trigger_at(FaultPoint::AfterMarkedRecoveringBeforeFixup);
        assert!(matches!(reconcile(&root, &journal, &no_git_repo(), &NullIndexer, None, &injector), Err(ReconcileError::Interrupted(_))));
        // The interrupted pass left A at Recovering (first in version order).
        // Also put B where a later crash would leave it: Recovering, with its
        // before-image already deleted.
        journal.advance(op_b, WriteState::Recovering, AdvanceFields::previous_state(WriteState::ContentPersisted)).unwrap();
        delete_before_image(&root, &id, op_b).unwrap();

        let first = reconcile(&root, &journal, &no_git_repo(), &NullIndexer, None, &FaultInjector::none()).unwrap();
        assert_eq!(first.failed, vec![op_a, op_b], "B resolves to Failed, not Inconsistent over its missing before-image");
        let second = reconcile(&root, &journal, &no_git_repo(), &NullIndexer, None, &FaultInjector::none()).unwrap();
        assert_eq!(second, ReconcileReport::default(), "a second pass finds nothing left to do");
        assert_eq!(std::fs::read(root.join("note-a.record")).unwrap(), v1);
        assert_eq!(journal.get(op_b).unwrap().unwrap().state, WriteState::Failed);
    }

    #[test]
    fn a_before_image_matching_committed_content_is_still_restored_even_if_a_failed_row_has_the_same_hash() {
        // The guard only fires when the before-image is NOT committed content.
        // Identical bytes at the same version can belong both to a Failed row
        // and to a later committed one (A failed with "x" at v2, then C
        // committed "x" at v2): the committed row wins and the restore runs.
        let (root, journal) = fresh_root_and_journal();
        let id = NoteId::parse("note-a").unwrap();
        let x = serialize_record_for_test("x", Version(2));
        let (op_a, op_c, op_d) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        journal.insert_intent(op_a, id.as_str(), 1).unwrap();
        journal.advance(op_a, WriteState::ContentPersisted, AdvanceFields::content(2, content_hash(&x))).unwrap();
        journal.advance(op_a, WriteState::Recovering, AdvanceFields::previous_state(WriteState::ContentPersisted)).unwrap();
        journal.advance(op_a, WriteState::Failed, AdvanceFields::none()).unwrap();
        journal.insert_intent(op_c, id.as_str(), 1).unwrap();
        journal.advance(op_c, WriteState::ContentPersisted, AdvanceFields::content(2, content_hash(&x))).unwrap();
        journal.advance(op_c, WriteState::JournalCommitted, AdvanceFields::none()).unwrap();
        journal.advance(op_c, WriteState::GitCommitted, AdvanceFields::none()).unwrap();
        journal.advance(op_c, WriteState::IndexCurrent, AdvanceFields::none()).unwrap();
        let before_d = persist_before_image(&root, &id, op_d, &x).unwrap();
        std::fs::write(root.join("note-a.record"), b"CORRUPTED").unwrap();
        journal.insert_intent(op_d, id.as_str(), 2).unwrap();
        journal.advance(op_d, WriteState::ContentPersisted, AdvanceFields::content_with_before_image(3, "will-not-match", before_d)).unwrap();

        let report = reconcile(&root, &journal, &no_git_repo(), &NullIndexer, None, &FaultInjector::none()).unwrap();
        assert_eq!(report.failed, vec![op_d]);
        assert_eq!(std::fs::read(root.join("note-a.record")).unwrap(), x, "C's committed content is restored");
    }

    #[test]
    fn a_live_write_is_refused_before_any_filesystem_effect_while_a_stalled_content_persisted_row_exists() {
        // Protection 1, live window of the second variant: A stalls at
        // ContentPersisted (rename landed, the journal stopped). B, at the
        // raw file's version, and C, at the journal's version, are both
        // refused under the per-note lock before touching the record or
        // persisting a before-image. Reopening resolves A (its bytes are on
        // disk and verify, so C1 rolls it forward), after which the note
        // writes normally again.
        //
        // Supersedes 88b20f6's
        // `live_writes_after_a_stall_and_a_later_commit_stay_on_the_journal_and_survive_reopen`,
        // which pinned the old live behaviour this guard removes: a write at
        // the journal's version got Conflict(actual = raw v2) and B, at the
        // raw version, was accepted on top of A's never-committed bytes.
        let root = crate::git::test_support::init_test_repo();
        let journal_path = unique_test_dir().join("journal.sqlite");
        let id = NoteId::parse("note-a").unwrap();
        let (op_a, op_b, op_c) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        {
            let vault = Vault::open_with_default_indexer(root.clone(), &journal_path).unwrap();
            vault.write_record_unchecked(&id, "v1", Version(0), Uuid::new_v4()).unwrap();
            let injector = FaultInjector::trigger_at(FaultPoint::AfterRenameBeforeJournalCommitted);
            assert!(vault.write_record_unchecked_with_injector(&id, "v2 from A", Version(1), op_a, &injector).is_err());
            assert_eq!(vault.journal_row_for_test(op_a).unwrap().state, WriteState::ContentPersisted);
            assert!(matches!(vault.read_note(&id), Err(StorageError::Inconsistent { .. })), "reads still fail closed");
            let on_disk = std::fs::read(root.join("note-a.record")).unwrap();

            let b = vault.write_record_unchecked(&id, "v3 from B", Version(2), op_b);
            assert!(matches!(&b, Err(StorageError::UnresolvedOperation { note_id, operation_id }) if note_id == "note-a" && *operation_id == op_a), "a write at the raw file's version is refused, got {b:?}");
            let c = vault.write_record_unchecked(&id, "v2 from C", Version(1), op_c);
            assert!(matches!(&c, Err(StorageError::UnresolvedOperation { operation_id, .. }) if *operation_id == op_a), "a write at the journal's version is refused too, got {c:?}");
            // A retry of A itself still goes through the retry path, unchanged.
            let retry = vault.write_record_unchecked(&id, "v2 from A", Version(1), op_a);
            assert!(matches!(retry, Err(StorageError::OperationInProgress { operation_id }) if operation_id == op_a), "got {retry:?}");
            assert_eq!(std::fs::read(root.join("note-a.record")).unwrap(), on_disk, "no filesystem effect on the record");
            assert!(read_before_image(&root, &id, op_b).is_err() && read_before_image(&root, &id, op_c).is_err(), "no before-image persisted");
            assert_eq!(vault.journal_row_for_test(op_b).unwrap().state, WriteState::Failed, "the refused attempt is terminal");
            assert_eq!(vault.journal_row_for_test(op_c).unwrap().state, WriteState::Failed);
            // Other notes are unaffected.
            let other = NoteId::parse("note-b").unwrap();
            assert_eq!(vault.write_record_unchecked(&other, "b1", Version(0), Uuid::new_v4()).unwrap(), Version(1));
        }
        let vault = Vault::open_with_default_indexer(root.clone(), &journal_path).unwrap();
        assert_eq!(vault.journal_row_for_test(op_a).unwrap().state, WriteState::IndexCurrent, "A's verified bytes roll forward (C1)");
        let record = vault.read_note(&id).unwrap();
        assert_eq!((record.content.as_str(), record.version), ("v2 from A", Version(2)));
        assert_eq!(vault.write_record_unchecked(&id, "v3", Version(2), Uuid::new_v4()).unwrap(), Version(3));
    }

    #[test]
    fn a_live_write_is_refused_while_an_intent_recorded_row_whose_rename_may_have_landed_exists() {
        // The same stall one step earlier: the rename landed but the
        // ContentPersisted advance did not, so the row is IntentRecorded and
        // carries its intended hash (C1). Under the per-note lock such a row
        // can only be a stalled write, never one in flight.
        let root = crate::git::test_support::init_test_repo();
        let journal_path = unique_test_dir().join("journal.sqlite");
        let id = NoteId::parse("note-a").unwrap();
        let (op_a, op_b) = (Uuid::new_v4(), Uuid::new_v4());
        let vault = Vault::open_with_default_indexer(root.clone(), &journal_path).unwrap();
        vault.write_record_unchecked(&id, "v1", Version(0), Uuid::new_v4()).unwrap();
        let injector = FaultInjector::trigger_at(FaultPoint::AfterRenameBeforeContentPersisted);
        assert!(vault.write_record_unchecked_with_injector(&id, "v2 from A", Version(1), op_a, &injector).is_err());
        assert_eq!(vault.journal_row_for_test(op_a).unwrap().state, WriteState::IntentRecorded);
        let on_disk = std::fs::read(root.join("note-a.record")).unwrap();
        let b = vault.write_record_unchecked(&id, "v3 from B", Version(2), op_b);
        assert!(matches!(&b, Err(StorageError::UnresolvedOperation { operation_id, .. }) if *operation_id == op_a), "got {b:?}");
        assert_eq!(std::fs::read(root.join("note-a.record")).unwrap(), on_disk);
        assert!(read_before_image(&root, &id, op_b).is_err());
        assert_eq!(vault.journal_row_for_test(op_b).unwrap().state, WriteState::Failed);
    }

    #[test]
    fn a_git_pending_operation_does_not_block_later_writes_to_the_note() {
        // JournalCommitted is committed, readable state: only the git step is
        // pending. The live guard must not treat it as unresolved.
        let root = unique_test_dir(); // not a git repository: every write is git_pending
        let journal_path = unique_test_dir().join("journal.sqlite");
        let id = NoteId::parse("note-a").unwrap();
        let vault = Vault::open_with_default_indexer(root, &journal_path).unwrap();
        assert!(matches!(vault.write_record_unchecked(&id, "v1", Version(0), Uuid::new_v4()), Err(StorageError::GitPending { .. })));
        assert!(matches!(vault.write_record_unchecked(&id, "v2", Version(1), Uuid::new_v4()), Err(StorageError::GitPending { .. })));
        assert_eq!(vault.read_note(&id).unwrap().content, "v2");
        // The ordinary stale-version case is still Conflict.
        let err = vault.write_record_unchecked(&id, "stale", Version(1), Uuid::new_v4()).unwrap_err();
        assert!(matches!(err, StorageError::Conflict { expected: Version(1), actual: Version(2), .. }), "got {err:?}");
    }

    // --- A3 review round 2, item #4: fix_intent_recorded restores the
    // Markdown pointer from its before-image on a PII-inconsistent rollback ---

    #[test]
    fn fix_intent_recorded_restores_the_markdown_pointer_from_its_before_image_on_a_pii_inconsistent_rollback() {
        // Simulates the defensive, upstream-invariant-violation window
        // `fix_intent_recorded`'s pointer-landed branch guards: the v2
        // pointer rename landed and its content_hash matches, but
        // pii_content_hash names ciphertext that does not exist anywhere
        // (neither live nor staged) — never reachable via live traffic
        // (AfterPiiCommittedBeforePointerRename's own invariant prevents
        // it), but recovery must still resolve the row safely rather than
        // leaving the pointer stuck at the bad v2 version. Before the fix,
        // this branch called delete_before_image without ever restoring
        // the pointer from it, leaving the Markdown record directly at v2
        // although the row resolves to Failed.
        let root = crate::git::test_support::init_test_repo();
        let journal_path = unique_test_dir().join("journal.sqlite");
        let pii_root = unique_test_dir();
        let pii_store = PiiStore::open(pii_root.clone(), &pii_root.join("master.key")).unwrap();
        let id = NoteId::parse("note-a").unwrap();

        // v1 fully committed: a real pointer record plus real PiiStore ciphertext.
        let op1 = Uuid::new_v4();
        let v1_plaintext = b"v1 secret";
        let v1_pointer = serialize_record_for_test("pii://note-a", Version(1));
        pii_store.stage("note-a", v1_plaintext, op1).unwrap();
        pii_store.commit_staged("note-a", op1).unwrap();
        std::fs::write(root.join("note-a.record"), &v1_pointer).unwrap();
        let journal = Journal::open(&journal_path).unwrap();
        journal.insert_intent(op1, id.as_str(), 0).unwrap();
        let v1_fields = AdvanceFields::content(1, content_hash(&v1_pointer)).merge_pii_content_hash(content_hash(v1_plaintext));
        journal.advance(op1, WriteState::ContentPersisted, v1_fields).unwrap();
        journal.advance(op1, WriteState::JournalCommitted, AdvanceFields::none()).unwrap();
        journal.advance(op1, WriteState::GitCommitted, AdvanceFields::none()).unwrap();
        journal.advance(op1, WriteState::IndexCurrent, AdvanceFields::none()).unwrap();

        // v2's pointer rename landed (bytes on disk match its intended
        // content_hash), its Markdown before-image (v1's pointer) was
        // persisted under op2, but its pii_content_hash names ciphertext
        // that was never staged or committed anywhere.
        let op2 = Uuid::new_v4();
        let v2_pointer = serialize_record_for_test("pii://note-a", Version(2));
        let before_hash = persist_before_image(&root, &id, op2, &v1_pointer).unwrap();
        std::fs::write(root.join("note-a.record"), &v2_pointer).unwrap();
        journal.insert_intent(op2, id.as_str(), 1).unwrap();
        let v2_fields = AdvanceFields::content_with_before_image(2, content_hash(&v2_pointer), before_hash)
            .merge_pii_content_hash(content_hash(b"never actually staged or committed"));
        journal.record_fields(op2, WriteState::IntentRecorded, v2_fields).unwrap();

        let report = reconcile(&root, &journal, &root, &NullIndexer, Some(&pii_store), &FaultInjector::none()).unwrap();
        assert_eq!(report.failed, vec![op2]);
        assert_eq!(journal.get(op2).unwrap().unwrap().state, WriteState::Failed);

        // THE ASSERTION THAT MATTERS: the Markdown pointer must have been
        // restored to v1's, not left stuck at v2's bad pointer.
        assert_eq!(std::fs::read(root.join("note-a.record")).unwrap(), v1_pointer, "the pointer must be restored to v1's, not left at v2's inconsistent pointer");
        assert!(read_before_image(&root, &id, op2).is_err(), "the consumed before-image must be deleted, not orphaned");
        assert_eq!(pii_store.get("note-a").unwrap(), v1_plaintext, "v1's ciphertext, never touched by this rollback, must remain readable");
    }
}
