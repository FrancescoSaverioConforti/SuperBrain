//! Restore a backup into a vault that is usable through the normal storage
//! API (final whole-branch review, I1), promoting each restored note through
//! the A2 journal under a final, locked deletion-log re-check
//! (restore-promotion TOCTOU fix).
//!
//! # Division of responsibility
//!
//! `vault_security::restore` decrypts the backup, runs its two deletion-log
//! checks (the backup's reference against the live one; a per-note
//! `is_deleted` exclusion) and returns the surviving notes STAGED: record
//! bytes and PII plaintext, in memory, nothing written. This module owns
//! every write: [`restore_vault_from_backup`] promotes each staged note into
//! a fresh vault as its own A2-style operation, with its own journal row.
//!
//! # Why promotion re-checks the deletion log under a lock
//!
//! Before this fix `vault_security::restore` wrote the files itself right
//! after its last `is_deleted` read, with no lock and no further check, and
//! this module then seeded journal rows from whatever was on disk. A
//! deletion completing in between was ignored: the deleted note's record and
//! ciphertext were written anyway and reported as restored (an independent
//! review reproduced this deterministically). Now, for each note, the last
//! step before the pointer rename — the point after which the note is
//! readable — is: take [`DeletionLog::acquire_write_lock`] (the same
//! cross-process lock `DeletionLog::record_deletion` holds for its whole
//! span), call `is_deleted` once more, and only if it is still `false`
//! rename the pointer and advance the row to `JournalCommitted` before
//! releasing the lock. A deletion therefore lands either entirely before
//! that re-check (the note is excluded: its row is failed, its staged or
//! committed ciphertext rolled back exactly as recovery would, and it is
//! reported in `excluded_deleted_note_ids`) or entirely after the note's
//! commit (the note was legitimately restored before it was deleted, and the
//! read gates — `Vault::read_note`, `PiiStore::get_unless_deleted` — then
//! refuse it like any other deleted note). Any error reading the log fails
//! the whole restore closed.
//!
//! Lock ordering: promotion runs against the FRESH vault this function
//! opens, before it is returned to anyone, and takes none of its per-note
//! locks, so it acquires the deletion-log write lock with nothing held above
//! it; under that lock it only takes this vault's journal mutex and the
//! deletion log's own connection mutex (innermost). It never takes a `Vault`
//! per-note lock while holding the write lock.
//!
//! The re-check applies to every note, not only `Sensitive` ones. In this
//! codebase deletions are only ever RECORDED through a `Sensitive` tombstone
//! write, but `is_deleted` keys on the note id alone, and a backup can hold
//! an older, non-sensitive version of a note that was later reclassified and
//! deleted; a deleted id is retired for every classification (I2).
//!
//! # Per-note promotion reuses the A2 machinery
//!
//! Each note is one operation with the same row shape the live sensitive
//! write path produces (`Vault::write_sensitive_record_unchecked`):
//! `insert_intent`; its intended `content_hash` (and, for a `Sensitive`
//! note, `pii_content_hash` via `AdvanceFields::merge_pii_content_hash`)
//! recorded on the still-`IntentRecorded` row; `PiiStore::stage` and
//! `commit_staged`; the locked re-check; `atomic_write` of the backed-up
//! record bytes; `ContentPersisted`; on-disk verification; `JournalCommitted`
//! (the documented resting state for "durable and readable, not yet in
//! git": the restored root has no git history). No new journal column and no
//! new state machine: a crash at any point leaves a row `recovery::reconcile`
//! already knows how to resolve (rolled back if its pointer never landed or
//! its ciphertext does not verify, rolled forward otherwise — and, since this
//! fix, never forwarded for a sensitive row whose note the deletion log
//! records as deleted).
//!
//! # Resuming an interrupted restore
//!
//! Operation ids are deterministic: a version-8 (custom) UUID derived from
//! the backup's plaintext hash, the note id and an attempt counter
//! ([`restore_operation_id`]). Live writes always use random v4 ids, so a
//! journal whose every row is a v8 id is recognizably a restore journal, and
//! one whose every row is one of THIS backup's ids is an interrupted restore
//! of this backup. Calling [`restore_vault_from_backup`] again with the same
//! arguments then resumes instead of refusing: `Vault::open` first reconciles
//! the crashed rows; a note whose last committed row already carries exactly
//! this backup's bytes (and plaintext hash) is counted as restored without
//! being written again (never double-applied); a note with no committed row
//! is promoted under its next unused attempt id (an attempt whose row ended
//! `Failed` is skipped, never reused). Any journal that is not such a restore
//! journal is still refused ([`RestoreError::JournalAlreadyExists`]).
//! Deterministic ids, rather than fresh ones per attempt, are what make this
//! recognizable at all: with random ids a crashed restore's journal would be
//! indistinguishable from an unrelated vault's, and could only be refused.
//!
//! # Not a second write entry point
//!
//! `propose_and_commit` stays the only way to write new content into a
//! vault. This function only bootstraps a brand-new vault (or finishes
//! bootstrapping one it started) from a backup blob authenticated under the
//! backup master key, whose every note passed the gate when it was first
//! written.
//!
//! # The deletion log the restored vault consults
//!
//! The restored vault's read gates and its crash recovery consult the
//! deletion log beside the `PiiStore` it is opened with
//! (`PiiStore::deletion_log_path`). In production that store is the live one
//! (a restore writes its ciphertext back into the live store), so this is the
//! same log as `live_deletion_log`. A caller restoring into an isolated PII
//! root gets a vault whose gates consult that root's own log instead.

use std::collections::BTreeSet;
use std::path::Path;

use domain::{Classification, WriteState};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use vault_security::{
    BackupError, BackupTarget, DeletionLog, MasterKey, PiiPointer, PiiStore, RestoreOutcome, StagedNote, TrustedReferenceStore,
};

use crate::atomic::{atomic_write, AtomicWriteError};
use crate::fault::{FaultInjector, FaultPoint};
use crate::journal::{AdvanceFields, Journal};
use crate::note_id::NoteId;
use crate::propose::{body_of, frontmatter_of};
use crate::vault::{content_hash, parse_record, reconcile_err, record_path_checked, StorageError, Vault};

/// Why [`restore_vault_from_backup`] failed.
#[derive(Debug, thiserror::Error)]
pub enum RestoreError {
    /// `vault_security::restore` refused or failed (empty target, hash
    /// mismatch, deletion-log rejection, undecryptable PII), or this
    /// module's own pre-write guards refused (non-empty destination, live
    /// PII clobber). Nothing was promoted.
    #[error("backup restore failed: {0}")]
    Backup(BackupError),
    /// `journal_path` already exists and is not an interrupted restore of
    /// this same backup (see the module docs, "Resuming"). Restore never adds
    /// records to an unrelated journal.
    #[error("refusing to restore: a journal already exists at the given path")]
    JournalAlreadyExists,
    /// A staged note could not be turned into a usable vault record, a
    /// promotion step failed (including any failure to consult the deletion
    /// log, which is never read as "not deleted"), or the restored vault
    /// could not be opened. Notes promoted before the failure stay
    /// committed; the journal is kept so the same call can resume.
    #[error("restored files could not be turned into a usable vault: {0}")]
    Storage(StorageError),
}

/// Restores the backup in `target` into a brand-new vault at
/// `destination_root` (journal at `journal_path`, PII ciphertext re-staged
/// into `pii_store`), promoting each surviving note under a final, locked
/// deletion-log re-check, then opens the result as a [`Vault`] so every
/// restored note is readable through [`Vault::read_note`] (and a `Sensitive`
/// one's content through `PiiStore::get_unless_deleted`). See the module
/// docs for the promotion protocol and for resuming an interrupted call.
///
/// Fails closed: a staged record whose note id, version header or
/// frontmatter does not parse, or a `Sensitive` record whose body is not its
/// own `pii://` pointer or whose backup held no PII for it, is
/// [`RestoreError::Storage`] (`Inconsistent`) before anything is written.
///
/// `RestoreOutcome::restored_note_ids` lists only notes that reached
/// `JournalCommitted` (in this call or an earlier, interrupted one);
/// `excluded_deleted_note_ids` lists every note excluded as deleted, by
/// `vault_security::restore`'s checks or by the final re-check.
#[allow(clippy::too_many_arguments)]
pub fn restore_vault_from_backup(
    target: &dyn BackupTarget,
    master_key: &MasterKey,
    destination_root: &Path,
    journal_path: &Path,
    live_deletion_log: &DeletionLog,
    live_trusted_ref: &dyn TrustedReferenceStore,
    pii_store: &PiiStore,
) -> Result<(Vault, RestoreOutcome), RestoreError> {
    restore_vault_from_backup_with_injector(
        target,
        master_key,
        destination_root,
        journal_path,
        live_deletion_log,
        live_trusted_ref,
        pii_store,
        &FaultInjector::none(),
    )
}

/// The deterministic operation id of `note_id`'s `attempt`-th promotion in a
/// restore of the backup whose plaintext snapshot hashes to `backup_hash`: a
/// version-8 UUID over `sha256("superbrain-restore-op-v1" || backup_hash ||
/// note_id || attempt)`. See the module docs, "Resuming".
pub(crate) fn restore_operation_id(backup_hash: &str, note_id: &str, attempt: u32) -> Uuid {
    let mut hasher = Sha256::new();
    hasher.update(b"superbrain-restore-op-v1\0");
    hasher.update(backup_hash.as_bytes());
    hasher.update(b"\0");
    hasher.update(note_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(attempt.to_le_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    uuid::Builder::from_custom_bytes(bytes).into_uuid()
}

/// Upper bound on promotion attempts per note across resumed calls. Each
/// attempt that ends `Failed` costs one; reaching this is an anomaly, failed
/// closed as `Inconsistent`.
const MAX_ATTEMPTS: u32 = 64;

fn is_restore_operation_id(id: Uuid) -> bool {
    id.get_version_num() == 8
}

/// A staged note, validated and hashed, ready to promote.
struct Prepared<'a> {
    staged: &'a StagedNote,
    note_id: NoteId,
    version: u64,
    record_hash: String,
    /// `Some` iff the note is `Sensitive`: the hash of its PII plaintext.
    pii_hash: Option<String>,
}

/// How one note's promotion ended.
enum Promotion {
    Committed,
    ExcludedDeleted,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn restore_vault_from_backup_with_injector(
    target: &dyn BackupTarget,
    master_key: &MasterKey,
    destination_root: &Path,
    journal_path: &Path,
    live_deletion_log: &DeletionLog,
    live_trusted_ref: &dyn TrustedReferenceStore,
    pii_store: &PiiStore,
    injector: &FaultInjector,
) -> Result<(Vault, RestoreOutcome), RestoreError> {
    let storage = RestoreError::Storage;
    let journal_err = |e: crate::journal::JournalError| RestoreError::Storage(StorageError::Journal(e.to_string()));

    // An existing journal is only ever resumed into if every row in it is a
    // restore operation id (checked against THIS backup once it is
    // decrypted, below). Anything else is refused before the backup is even
    // read.
    let resuming = match std::fs::metadata(journal_path) {
        Ok(_) => {
            let rows = Journal::open(journal_path).map_err(journal_err)?.all_operations().map_err(journal_err)?;
            if rows.is_empty() || !rows.iter().all(|row| is_restore_operation_id(row.operation_id)) {
                return Err(RestoreError::JournalAlreadyExists);
            }
            true
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => return Err(storage(StorageError::Io(e))),
    };

    if !resuming {
        match std::fs::read_dir(destination_root) {
            Ok(mut entries) => {
                if entries.next().is_some() {
                    return Err(RestoreError::Backup(BackupError::AlreadyExists));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(storage(StorageError::Io(e))),
        }
    }

    let staged = vault_security::restore(target, master_key, live_deletion_log, live_trusted_ref, pii_store)
        .map_err(RestoreError::Backup)?;

    if resuming {
        let journal = Journal::open(journal_path).map_err(journal_err)?;
        let known: BTreeSet<&str> = staged
            .notes
            .iter()
            .map(|n| n.note_id())
            .chain(staged.excluded_deleted_note_ids.iter().map(String::as_str))
            .collect();
        for row in journal.all_operations().map_err(journal_err)? {
            let ours = known.contains(row.note_id.as_str())
                && (0..MAX_ATTEMPTS).any(|k| restore_operation_id(&staged.hash, &row.note_id, k) == row.operation_id);
            if !ours {
                return Err(RestoreError::JournalAlreadyExists);
            }
        }
    }

    // Validate every staged note before anything is written.
    let prepared = staged.notes.iter().map(prepare).collect::<Result<Vec<_>, _>>().map_err(storage)?;

    std::fs::create_dir_all(destination_root).map_err(|e| storage(StorageError::Io(e)))?;
    let vault = match Vault::open_with_pii_store(destination_root.to_path_buf(), journal_path, pii_store) {
        Ok(vault) => vault,
        Err(e) => {
            if !resuming {
                // We created this file (it did not exist on entry) and no row
                // was ever written to it.
                let _ = std::fs::remove_file(journal_path);
            }
            return Err(storage(e));
        }
    };

    let result = already_restored_notes(&vault, &staged.hash, &prepared).and_then(|already_restored| {
        let already_live = live_pii_in_the_way(pii_store, &prepared, &already_restored)?;
        if !already_live.is_empty() {
            return Ok(Err(already_live));
        }
        promote_all(&vault, &staged, &prepared, &already_restored, pii_store, live_deletion_log, injector).map(Ok)
    });
    let (restored, excluded_by_recheck) = match result {
        Ok(Ok(lists)) => lists,
        Ok(Err(already_live)) => {
            drop(vault);
            if !resuming {
                let _ = std::fs::remove_file(journal_path);
            }
            return Err(RestoreError::Backup(BackupError::PiiAlreadyExists { note_ids: already_live }));
        }
        Err(e) => {
            let no_rows = vault.journal().lock().unwrap().all_operations().map(|rows| rows.is_empty()).unwrap_or(false);
            drop(vault);
            if !resuming && no_rows {
                let _ = std::fs::remove_file(journal_path);
            }
            return Err(storage(e));
        }
    };
    drop(vault);

    // Reopen through the ordinary constructor, so the ordinary reconcile
    // pass runs over the freshly promoted rows (git unavailable: they rest as
    // git_pending; a git repository with a first commit: they are committed).
    let vault = Vault::open_with_pii_store(destination_root.to_path_buf(), journal_path, pii_store).map_err(storage)?;

    let mut excluded: Vec<String> = staged.excluded_deleted_note_ids.clone();
    excluded.extend(excluded_by_recheck);
    excluded.sort();
    excluded.dedup();
    Ok((
        vault,
        RestoreOutcome {
            hash: staged.hash.clone(),
            generation: staged.generation,
            restored_note_ids: restored,
            excluded_deleted_note_ids: excluded,
        },
    ))
}

/// Validates one staged note (the checks the old `seed_journal` made against
/// the restored files, now made before anything is written).
fn prepare(staged: &StagedNote) -> Result<Prepared<'_>, StorageError> {
    let inconsistent = || StorageError::Inconsistent { note_id: staged.note_id().to_string() };
    let note_id = NoteId::parse(staged.note_id()).map_err(|_| inconsistent())?;
    let record = parse_record(staged.record_bytes(), &note_id)?;
    if record.version.0 == 0 {
        return Err(inconsistent());
    }
    let note = domain::parse_frontmatter(frontmatter_of(&record.content)).map_err(|_| inconsistent())?;
    if note.id != note_id.as_str() {
        return Err(inconsistent());
    }
    // A non-sensitive note's backed-up `.pii` file (e.g. one left over from
    // before a reclassification) is not referenced by its record and is
    // never promoted.
    let pii_hash = if matches!(note.classification, Classification::Sensitive) {
        if body_of(&record.content).trim() != PiiPointer::for_note(note_id.as_str()).uri {
            return Err(inconsistent());
        }
        let plaintext = staged.pii_plaintext().ok_or_else(inconsistent)?;
        Some(content_hash(plaintext))
    } else {
        None
    };
    Ok(Prepared { staged, note_id, version: record.version.0, record_hash: content_hash(staged.record_bytes()), pii_hash })
}

/// Which notes an earlier, interrupted call of this same restore already
/// committed (idempotent resume: these are counted, never written again).
/// Such a row must carry exactly this backup's record bytes and plaintext
/// hash, under one of this backup's operation ids; a committed row that does
/// not is not ours to count, and fails closed.
fn already_restored_notes(vault: &Vault, backup_hash: &str, prepared: &[Prepared<'_>]) -> Result<BTreeSet<String>, StorageError> {
    let mut already_restored = BTreeSet::new();
    for note in prepared {
        let committed = vault
            .journal()
            .lock()
            .unwrap()
            .last_committed_for_note(note.note_id.as_str())
            .map_err(|e| StorageError::Journal(e.to_string()))?;
        if let Some(row) = committed {
            let matches = row.content_hash.as_deref() == Some(note.record_hash.as_str())
                && row.pii_content_hash == note.pii_hash
                && (0..MAX_ATTEMPTS).any(|k| restore_operation_id(backup_hash, note.note_id.as_str(), k) == row.operation_id);
            if !matches {
                return Err(StorageError::Inconsistent { note_id: note.note_id.as_str().to_string() });
            }
            already_restored.insert(note.note_id.as_str().to_string());
        }
    }
    Ok(already_restored)
}

/// Never silently overwrite live PII ciphertext (Task 7 round 2 review
/// finding, previously enforced inside `vault_security::restore`): every
/// sensitive note still to be promoted whose live `.pii` file already exists
/// in `pii_store` (typically the LIVE store, which can hold newer content
/// than the backup). Checked for all of them before any is promoted. Only
/// `NotFound` counts as absent.
fn live_pii_in_the_way(pii_store: &PiiStore, prepared: &[Prepared<'_>], already_restored: &BTreeSet<String>) -> Result<Vec<String>, StorageError> {
    let mut already_live = Vec::new();
    for note in prepared.iter().filter(|n| n.pii_hash.is_some() && !already_restored.contains(n.note_id.as_str())) {
        match std::fs::metadata(pii_store.root().join(format!("{}.pii", note.note_id.as_str()))) {
            Ok(_) => already_live.push(note.note_id.as_str().to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(StorageError::Io(e)),
        }
    }
    Ok(already_live)
}

/// Promotes every prepared note not already restored. Returns
/// `(restored_note_ids, excluded_by_final_recheck)`; the first includes the
/// already-restored ones and is sorted.
fn promote_all(
    vault: &Vault,
    staged: &vault_security::StagedRestore,
    prepared: &[Prepared<'_>],
    already_restored: &BTreeSet<String>,
    pii_store: &PiiStore,
    live_deletion_log: &DeletionLog,
    injector: &FaultInjector,
) -> Result<(Vec<String>, Vec<String>), StorageError> {
    for (name, bytes) in &staged.other_files {
        let path = vault.root().join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(StorageError::Io)?;
        }
        atomic_write(&path, bytes).map_err(|AtomicWriteError::Io(e)| StorageError::Io(e))?;
    }

    let mut restored: Vec<String> = already_restored.iter().cloned().collect();
    let mut excluded = Vec::new();
    for note in prepared.iter().filter(|n| !already_restored.contains(n.note_id.as_str())) {
        let operation_id = next_attempt_id(vault, &staged.hash, note)?;
        match promote_note(vault, note, operation_id, pii_store, live_deletion_log, injector)? {
            Promotion::Committed => restored.push(note.note_id.as_str().to_string()),
            Promotion::ExcludedDeleted => excluded.push(note.note_id.as_str().to_string()),
        }
    }
    restored.sort();
    Ok((restored, excluded))
}

/// The first attempt id for `note` with no journal row yet. Attempts whose
/// rows ended `Failed` are skipped; any other existing row (one still in
/// flight, which `Vault::open`'s reconcile pass should already have
/// resolved) refuses.
fn next_attempt_id(vault: &Vault, backup_hash: &str, note: &Prepared<'_>) -> Result<Uuid, StorageError> {
    for attempt in 0..MAX_ATTEMPTS {
        let id = restore_operation_id(backup_hash, note.note_id.as_str(), attempt);
        match vault.journal().lock().unwrap().get(id).map_err(|e| StorageError::Journal(e.to_string()))? {
            None => return Ok(id),
            Some(row) if row.state == WriteState::Failed => continue,
            Some(_) => return Err(StorageError::OperationInProgress { operation_id: id }),
        }
    }
    Err(StorageError::Inconsistent { note_id: note.note_id.as_str().to_string() })
}

/// Promotes one note as its own A2 operation. See the module docs.
fn promote_note(
    vault: &Vault,
    note: &Prepared<'_>,
    operation_id: Uuid,
    pii_store: &PiiStore,
    live_deletion_log: &DeletionLog,
    injector: &FaultInjector,
) -> Result<Promotion, StorageError> {
    let interrupted = |crate::fault::Interrupted::At(point)| StorageError::Interrupted(point);
    let journal_err = |e: crate::journal::JournalError| StorageError::Journal(e.to_string());
    let deletion_log_err = |e: vault_security::DeletionLogError| StorageError::DeletionLog(e.to_string());
    let pii_err = |e: vault_security::PiiStoreError| StorageError::PiiStore(e.to_string());
    let note_id = &note.note_id;
    let journal = vault.journal();
    let record_path = record_path_checked(vault.root(), note_id)?;

    journal.lock().unwrap().insert_intent(operation_id, note_id.as_str(), note.version - 1).map_err(journal_err)?;
    injector.check(FaultPoint::AfterIntentRecorded).map_err(interrupted)?;

    // Intended hashes on the still-IntentRecorded row first, exactly as the
    // live sensitive path does, so a staged file never exists under a row
    // recovery does not know is sensitive.
    let mut fields = AdvanceFields::content(note.version, note.record_hash.clone());
    if let Some(pii_hash) = &note.pii_hash {
        fields = AdvanceFields::merge_pii_content_hash(fields, pii_hash.clone());
    }
    journal.lock().unwrap().record_fields(operation_id, WriteState::IntentRecorded, fields.clone()).map_err(journal_err)?;

    if let Some(pii_hash) = &note.pii_hash {
        let plaintext = note.staged.pii_plaintext().ok_or_else(|| StorageError::Inconsistent { note_id: note_id.as_str().to_string() })?;
        let staged = pii_store.stage(note_id.as_str(), plaintext, operation_id).map_err(pii_err)?;
        if staged.content_hash != *pii_hash {
            return Err(StorageError::Inconsistent { note_id: note_id.as_str().to_string() });
        }
        injector.check(FaultPoint::AfterPiiStagedBeforeCommit).map_err(interrupted)?;
        pii_store.commit_staged(note_id.as_str(), operation_id).map_err(pii_err)?;
        injector.check(FaultPoint::AfterPiiCommittedBeforePointerRename).map_err(interrupted)?;
    }

    // The final re-check, under the cross-process write lock, immediately
    // before the point of no return. The guard is held until this note's row
    // is JournalCommitted (or failed), and is released on every return path,
    // including an injected crash (as process death would release it).
    injector.check(FaultPoint::RestoreBeforeDeletionRecheck).map_err(interrupted)?;
    let write_lock = live_deletion_log.acquire_write_lock().map_err(deletion_log_err)?;
    injector.check(FaultPoint::RestoreWriteLockHeld).map_err(interrupted)?;
    if live_deletion_log.is_deleted(note_id.as_str()).map_err(deletion_log_err)? {
        // Deleted after `vault_security::restore`'s own checks: exclude it.
        // The pointer was never renamed, so the note was never readable;
        // undo its committed (or staged) ciphertext with the same
        // fail-closed rollback recovery uses, then fail the row.
        let journal = journal.lock().unwrap();
        let row = journal
            .get(operation_id)
            .map_err(journal_err)?
            .ok_or_else(|| StorageError::Inconsistent { note_id: note_id.as_str().to_string() })?;
        crate::recovery::roll_back_pii(&journal, Some(pii_store), note_id, operation_id, &row).map_err(|e| reconcile_err(note_id, e))?;
        journal.advance(operation_id, WriteState::Failed, AdvanceFields::none()).map_err(journal_err)?;
        drop(write_lock);
        return Ok(Promotion::ExcludedDeleted);
    }

    atomic_write(&record_path, note.staged.record_bytes()).map_err(|AtomicWriteError::Io(e)| StorageError::Io(e))?;
    injector.check(FaultPoint::AfterRenameBeforeContentPersisted).map_err(interrupted)?;
    journal.lock().unwrap().advance(operation_id, WriteState::ContentPersisted, fields).map_err(journal_err)?;
    injector.check(FaultPoint::AfterRenameBeforeJournalCommitted).map_err(interrupted)?;

    // Verify what actually landed. A mismatch leaves the row at
    // ContentPersisted for recovery's row 2 (which rolls it back).
    let written = std::fs::read(&record_path).map_err(StorageError::Io)?;
    if content_hash(&written) != note.record_hash {
        return Err(StorageError::Inconsistent { note_id: note_id.as_str().to_string() });
    }
    journal.lock().unwrap().advance(operation_id, WriteState::JournalCommitted, AdvanceFields::none()).map_err(journal_err)?;
    drop(write_lock);
    Ok(Promotion::Committed)
}

/// Restore-promotion TOCTOU fix: the tests for the promotion protocol, its
/// locked final re-check, its crash recovery and its idempotent resume.
/// Windows-only because they drive the real DPAPI-authenticated trusted
/// reference, exactly as production does.
#[cfg(all(test, windows))]
mod tests {
    use std::collections::HashSet;
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::time::Duration;

    use vault_security::{DpapiTrustedReferenceStore, FilesystemBackupTarget, PiiStoreError};

    use super::*;

    fn unique_dir() -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "superbrain-restore-promotion-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn note_text(id: &str, classification: &str, body: &str) -> String {
        format!(
            "---\nid: {id}\nschema_version: 1\ntype: bug\nstatus: draft\ntrust: internal\nclassification: {classification}\nproject: project-x\nsource: agent:coder@prompt-v3\ncreated: 2026-10-23\nupdated: 2026-10-23\nevent_period: 2026-10-23\nrelations:\n  supports: []\n  contradicts: []\n  derived_from: []\n  supersedes: []\n  applies_to: []\n---\n{body}"
        )
    }

    /// A backup of a source vault, plus a restore destination whose
    /// `PiiStore` shares the source store's master key and whose deletion
    /// log (beside that store) IS the live deletion log the restore checks
    /// against: the production shape, where a restore writes back into the
    /// live store, so the restored vault's read gates and crash recovery
    /// consult the same log the promotion re-checks.
    struct Fixture {
        target: FilesystemBackupTarget,
        master_key: MasterKey,
        destination: PathBuf,
        journal_path: PathBuf,
        pii_store: PiiStore,
        live_log: DeletionLog,
        live_ref: DpapiTrustedReferenceStore,
    }

    /// `notes`: `(id, classification, body)`, written through
    /// `propose_and_commit` and backed up.
    fn fixture(notes: &[(&str, &str, &str)]) -> Fixture {
        let source_root = unique_dir();
        let key_path = source_root.join(".pii-store").join("master.key");
        let source_pii = PiiStore::open(source_root.join(".pii-store"), &key_path).unwrap();
        let source_vault = Vault::open_with_pii_store(source_root.clone(), &unique_dir().join("journal.sqlite"), &source_pii).unwrap();
        let gate = policy::Gate::new(policy::SecretPolicy::new(1));
        for (id, classification, body) in notes {
            crate::propose_and_commit(
                &source_vault,
                &note_text(id, classification, body),
                crate::Version(0),
                Uuid::new_v4(),
                &HashSet::new(),
                &HashSet::new(),
                &gate,
                Some(&source_pii),
            )
            .unwrap();
        }
        let source_log = DeletionLog::open(&source_pii.deletion_log_path()).unwrap();
        let master_key = MasterKey::load_or_create(&unique_dir().join("backup-master.key"), None).unwrap();
        let target = FilesystemBackupTarget::new(unique_dir().join("target"));
        vault_security::backup(&source_root, &source_pii, &source_log, &master_key, &target).unwrap();

        let pii_store = PiiStore::open(unique_dir().join("restored-pii"), &key_path).unwrap();
        let live_log = DeletionLog::open(&pii_store.deletion_log_path()).unwrap();
        let live_ref = DpapiTrustedReferenceStore::new(pii_store.deletion_log_reference_path());
        Fixture {
            target,
            master_key,
            destination: unique_dir().join("restored"),
            journal_path: unique_dir().join("restored-journal.sqlite"),
            pii_store,
            live_log,
            live_ref,
        }
    }

    impl Fixture {
        fn restore(&self, injector: &FaultInjector) -> Result<(Vault, RestoreOutcome), RestoreError> {
            restore_vault_from_backup_with_injector(
                &self.target,
                &self.master_key,
                &self.destination,
                &self.journal_path,
                &self.live_log,
                &self.live_ref,
                &self.pii_store,
                injector,
            )
        }

        fn reopen(&self) -> Vault {
            Vault::open_with_pii_store(self.destination.clone(), &self.journal_path, &self.pii_store).unwrap()
        }

        fn delete(&self, note_id: &str) {
            self.live_log.record_deletion(note_id, &self.live_ref).unwrap();
        }

        fn rows_for(&self, note_id: &str) -> Vec<crate::journal::OperationRow> {
            Journal::open(&self.journal_path)
                .unwrap()
                .all_operations()
                .unwrap()
                .into_iter()
                .filter(|row| row.note_id == note_id)
                .collect()
        }

        /// Nothing of the note survives on disk: no record, no live, staged
        /// or before-image `.pii` file.
        fn assert_nothing_of(&self, note_id: &str) {
            assert!(!self.destination.join(format!("{note_id}.record")).exists(), "{note_id}: no record may be on disk");
            assert!(!self.pii_store.has_live(note_id), "{note_id}: no live ciphertext may be on disk");
            for sub in [".staging", ".before-images"] {
                if let Ok(entries) = std::fs::read_dir(self.pii_store.root().join(sub)) {
                    for entry in entries.flatten() {
                        assert!(!entry.file_name().to_string_lossy().starts_with(note_id), "{note_id}: leftover {:?}", entry.path());
                    }
                }
            }
        }
    }

    fn id(s: &str) -> NoteId {
        NoteId::parse(s).unwrap()
    }

    /// Required test 1, the security review's repro shape, aimed at the
    /// promotion window: a deletion that completes AFTER
    /// `vault_security::restore`'s last check (check 2, part B) but BEFORE
    /// the note is written. Pre-fix, the note was written right there and
    /// reported restored. Now the final, locked re-check catches it.
    #[test]
    fn a_deletion_completing_after_vault_securitys_last_check_but_before_the_write_is_excluded_not_resurrected() {
        let f = fixture(&[("note-a", "sensitive", "RACED SECRET A"), ("note-b", "internal", "plain b\n")]);
        let (paused_tx, paused_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel::<()>();
        // AfterIntentRecorded is hit first for note-a, strictly after every
        // check `vault_security::restore` makes.
        let injector = FaultInjector::pause_at(FaultPoint::AfterIntentRecorded, paused_tx, resume_rx);
        let outcome = std::thread::scope(|scope| {
            let restore = {
                let f = &f;
                scope.spawn(move || f.restore(&injector))
            };
            paused_rx.recv().unwrap();
            f.delete("note-a");
            drop(resume_tx); // let note-a, and every later hit, continue
            restore.join().unwrap()
        });
        let (vault, outcome) = outcome.expect("the restore itself succeeds for note-b");
        assert_eq!(outcome.excluded_deleted_note_ids, vec!["note-a".to_string()], "the raced note is excluded and reported");
        assert_eq!(outcome.restored_note_ids, vec!["note-b".to_string()], "restored_note_ids only lists what was committed");
        f.assert_nothing_of("note-a");
        assert!(matches!(vault.read_note(&id("note-a")), Err(StorageError::NotFound { .. })));
        assert!(matches!(f.pii_store.get_unless_deleted("note-a"), Err(PiiStoreError::Deleted { .. })));
        assert_eq!(f.rows_for("note-a").iter().map(|r| r.state).collect::<Vec<_>>(), vec![WriteState::Failed]);
        assert!(vault.read_note(&id("note-b")).is_ok());
    }

    /// Required tests 2 and 4. While the promotion holds the deletion-log
    /// write lock for note-a, a deletion of note-b (through the SAME log
    /// instance, or through a second, independently-opened handle on the
    /// same file, the proxy for a second process) must block without
    /// writing anything; once note-a's promotion releases the lock it
    /// completes, and note-b's own final re-check then catches it.
    fn deletion_blocked_by_the_promotion_lock_is_caught_by_the_next_recheck(separate_handle: bool) {
        let f = fixture(&[("note-a", "sensitive", "secret a"), ("note-b", "sensitive", "SECRET B MUST NOT RESURFACE")]);
        let (paused_tx, paused_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        let injector =
            FaultInjector::pause_at_each(vec![FaultPoint::RestoreBeforeDeletionRecheck, FaultPoint::RestoreWriteLockHeld], paused_tx, resume_rx);
        let second_handle = DeletionLog::open(f.live_log.db_path()).unwrap();
        let deleter_log = if separate_handle { &second_handle } else { &f.live_log };
        let observer = DeletionLog::open(f.live_log.db_path()).unwrap();

        let (outcome, blocked) = std::thread::scope(|scope| {
            let restore = {
                let f = &f;
                scope.spawn(move || f.restore(&injector))
            };
            assert_eq!(paused_rx.recv().unwrap(), FaultPoint::RestoreBeforeDeletionRecheck); // note-a
            resume_tx.send(()).unwrap();
            assert_eq!(paused_rx.recv().unwrap(), FaultPoint::RestoreWriteLockHeld); // note-a, lock held
            let deleter = scope.spawn(|| deleter_log.record_deletion("note-b", &f.live_ref).unwrap());
            std::thread::sleep(Duration::from_millis(300));
            let blocked = !deleter.is_finished() && !observer.is_deleted("note-b").unwrap();
            resume_tx.send(()).unwrap();
            assert_eq!(paused_rx.recv().unwrap(), FaultPoint::RestoreBeforeDeletionRecheck); // note-b
            deleter.join().unwrap(); // note-a's promotion released the lock
            assert!(observer.is_deleted("note-b").unwrap());
            drop(resume_tx);
            (restore.join().unwrap(), blocked)
        });
        assert!(blocked, "the deletion must wait for the promotion's write lock, writing nothing meanwhile");
        let (vault, outcome) = outcome.unwrap();
        assert_eq!(outcome.restored_note_ids, vec!["note-a".to_string()]);
        assert_eq!(outcome.excluded_deleted_note_ids, vec!["note-b".to_string()]);
        f.assert_nothing_of("note-b");
        assert!(matches!(vault.read_note(&id("note-b")), Err(StorageError::NotFound { .. })));
        assert_eq!(f.pii_store.get_unless_deleted("note-a").unwrap(), b"secret a");
    }

    #[test]
    fn a_deletion_racing_the_promotion_lock_through_the_same_log_instance_blocks_then_is_caught() {
        deletion_blocked_by_the_promotion_lock_is_caught_by_the_next_recheck(false);
    }

    #[test]
    fn a_deletion_racing_the_promotion_lock_through_a_second_independent_handle_blocks_then_is_caught() {
        deletion_blocked_by_the_promotion_lock_is_caught_by_the_next_recheck(true);
    }

    /// Required test 4, unscripted: real threads, two independent handles,
    /// no pauses, repeated. Whichever side wins each race, the deleted note
    /// is never consultable afterwards: either it was excluded (and nothing
    /// of it is on disk), or it was committed strictly before the deletion
    /// and the read gates refuse it.
    #[test]
    fn concurrent_restore_and_deletion_through_independent_handles_never_expose_the_deleted_note() {
        for _ in 0..8 {
            let f = fixture(&[("note-a", "sensitive", "a"), ("note-b", "sensitive", "B SECRET"), ("note-c", "internal", "c\n")]);
            let deleter_handle = DeletionLog::open(f.live_log.db_path()).unwrap();
            let barrier = std::sync::Barrier::new(2);
            let outcome = std::thread::scope(|scope| {
                let restore = scope.spawn(|| {
                    barrier.wait();
                    f.restore(&FaultInjector::none())
                });
                barrier.wait();
                deleter_handle.record_deletion("note-b", &f.live_ref).unwrap();
                restore.join().unwrap()
            });
            let (vault, outcome) = outcome.unwrap();
            let restored = outcome.restored_note_ids.contains(&"note-b".to_string());
            let excluded = outcome.excluded_deleted_note_ids.contains(&"note-b".to_string());
            assert!(restored != excluded, "note-b is exactly one of restored or excluded: {outcome:?}");
            drop(vault);
            let vault = f.reopen();
            if excluded {
                f.assert_nothing_of("note-b");
            }
            assert!(matches!(vault.read_note(&id("note-b")), Err(StorageError::Deleted { .. } | StorageError::NotFound { .. })));
            assert!(matches!(f.pii_store.get_unless_deleted("note-b"), Err(PiiStoreError::Deleted { .. })));
        }
    }

    /// Required test 3: a crash exactly between `ContentPersisted` and
    /// `JournalCommitted` (while the lock was held, so no deletion could
    /// land before it), and a deletion landing after the crash, before the
    /// next recovery pass. Promotion's own re-check passed; recovery's new
    /// fail-closed condition must discard the row rather than forward it.
    #[test]
    fn a_deletion_landing_in_the_content_persisted_to_journal_committed_gap_after_a_crash_is_discarded_by_reconcile() {
        let f = fixture(&[("note-a", "sensitive", "SECRET A")]);
        let result = f.restore(&FaultInjector::trigger_at(FaultPoint::AfterRenameBeforeJournalCommitted));
        assert!(matches!(result, Err(RestoreError::Storage(StorageError::Interrupted(FaultPoint::AfterRenameBeforeJournalCommitted)))));
        assert_eq!(f.rows_for("note-a")[0].state, WriteState::ContentPersisted, "precondition: pointer and ciphertext are in place");

        f.delete("note-a");
        let vault = f.reopen();
        assert_eq!(f.rows_for("note-a")[0].state, WriteState::Failed, "reconcile must never forward a deleted note's sensitive row");
        f.assert_nothing_of("note-a");
        assert!(matches!(vault.read_note(&id("note-a")), Err(StorageError::NotFound { .. })));
    }

    /// The same window without a deletion rolls forward exactly as a live
    /// write's would: the new condition only ever discards deleted notes.
    #[test]
    fn the_same_crash_without_a_deletion_still_rolls_forward_on_reconcile() {
        let f = fixture(&[("note-a", "sensitive", "SECRET A")]);
        let _ = f.restore(&FaultInjector::trigger_at(FaultPoint::AfterRenameBeforeJournalCommitted));
        let vault = f.reopen();
        assert!(vault.read_note(&id("note-a")).unwrap().content.ends_with("pii://note-a"));
        assert_eq!(f.pii_store.get_unless_deleted("note-a").unwrap(), b"SECRET A");
    }

    /// Required test 8: a simulated crash at every boundary of the
    /// promotion path, each resolved by the ordinary `reconcile` exactly as
    /// it resolves a live write's crash at the same boundary (rolled back
    /// before the pointer rename, rolled forward after it), with the write
    /// lock released by the "crash", and each then completed by simply
    /// calling the same restore again.
    #[test]
    fn a_crash_at_every_promotion_boundary_is_resolved_by_reconcile_and_completed_by_a_retry() {
        let before_rename = [
            FaultPoint::AfterIntentRecorded,
            FaultPoint::AfterPiiStagedBeforeCommit,
            FaultPoint::AfterPiiCommittedBeforePointerRename,
            FaultPoint::RestoreBeforeDeletionRecheck,
            FaultPoint::RestoreWriteLockHeld,
        ];
        let after_rename = [FaultPoint::AfterRenameBeforeContentPersisted, FaultPoint::AfterRenameBeforeJournalCommitted];
        for point in before_rename.into_iter().chain(after_rename) {
            let f = fixture(&[("note-a", "sensitive", "SECRET A")]);
            let result = f.restore(&FaultInjector::trigger_at(point));
            assert!(
                matches!(&result, Err(RestoreError::Storage(StorageError::Interrupted(p))) if *p == point),
                "{point:?}: got {:?}",
                result.err()
            );
            // The "crash" released the write lock (a held lock would make
            // this wait out the full timeout and fail).
            drop(f.live_log.acquire_write_lock().expect("the write lock must be released by the crash"));

            let vault = f.reopen();
            let rows = f.rows_for("note-a");
            if before_rename.contains(&point) {
                assert_eq!(rows[0].state, WriteState::Failed, "{point:?}");
                f.assert_nothing_of("note-a");
                assert!(matches!(vault.read_note(&id("note-a")), Err(StorageError::NotFound { .. })), "{point:?}");
            } else {
                assert!(matches!(rows[0].state, WriteState::JournalCommitted | WriteState::IndexCurrent), "{point:?}: {:?}", rows[0].state);
                assert_eq!(f.pii_store.get_unless_deleted("note-a").unwrap(), b"SECRET A", "{point:?}");
            }
            drop(vault);

            let (vault, outcome) = f.restore(&FaultInjector::none()).unwrap_or_else(|e| panic!("{point:?}: retry failed: {e}"));
            assert_eq!(outcome.restored_note_ids, vec!["note-a".to_string()], "{point:?}");
            assert_eq!(f.pii_store.get_unless_deleted("note-a").unwrap(), b"SECRET A", "{point:?}");
            assert!(vault.read_note(&id("note-a")).is_ok(), "{point:?}");
            let committed = f.rows_for("note-a").iter().filter(|r| r.state != WriteState::Failed).count();
            assert_eq!(committed, 1, "{point:?}: exactly one committed row, never a double application");
        }
    }

    /// Required test 7 (and point 6's multi-note crash): a crash after the
    /// first two of three notes reach `JournalCommitted`. The committed ones
    /// are readable; retrying the exact same call resumes, promoting only
    /// what is missing and never re-applying what is done; and calling it
    /// yet again is a no-op with the same outcome.
    #[test]
    fn retrying_the_same_restore_after_a_mid_restore_crash_resumes_idempotently() {
        let f = fixture(&[("note-a", "sensitive", "A"), ("note-b", "internal", "b\n"), ("note-c", "sensitive", "C")]);
        // The third hit of this point is note-c (sorted order), before its
        // pointer rename.
        let result = f.restore(&FaultInjector::trigger_at_nth(FaultPoint::RestoreWriteLockHeld, 3));
        assert!(matches!(result, Err(RestoreError::Storage(StorageError::Interrupted(FaultPoint::RestoreWriteLockHeld)))));
        {
            let vault = f.reopen();
            assert!(vault.read_note(&id("note-a")).is_ok() && vault.read_note(&id("note-b")).is_ok(), "committed notes are valid");
            assert_eq!(f.pii_store.get_unless_deleted("note-a").unwrap(), b"A");
            assert!(matches!(vault.read_note(&id("note-c")), Err(StorageError::NotFound { .. })));
        }
        let rows_a_before = f.rows_for("note-a");

        for _ in 0..2 {
            let (vault, outcome) = f.restore(&FaultInjector::none()).expect("the same call resumes");
            assert_eq!(outcome.restored_note_ids, vec!["note-a".to_string(), "note-b".to_string(), "note-c".to_string()]);
            assert!(outcome.excluded_deleted_note_ids.is_empty());
            assert_eq!(f.pii_store.get_unless_deleted("note-c").unwrap(), b"C");
            assert!(vault.read_note(&id("note-c")).is_ok());
            drop(vault);
            assert_eq!(f.rows_for("note-a").len(), rows_a_before.len(), "note-a is never promoted again");
            assert_eq!(f.rows_for("note-b").len(), 1);
            let c_states: Vec<_> = f.rows_for("note-c").iter().map(|r| r.state).collect();
            assert_eq!(c_states.len(), 2, "note-c: the failed first attempt plus one new attempt, never reused: {c_states:?}");
            assert_eq!(c_states[0], WriteState::Failed);
        }
    }

    /// A journal that is not an interrupted restore of THIS backup is never
    /// resumed into (here: another backup's completed restore).
    #[test]
    fn an_existing_journal_that_is_not_this_backups_interrupted_restore_is_refused() {
        let f = fixture(&[("note-a", "internal", "a\n")]);
        let other = fixture(&[("note-a", "internal", "different\n")]);
        let (vault, _) = other.restore(&FaultInjector::none()).unwrap();
        drop(vault);
        let foreign = Fixture { journal_path: other.journal_path.clone(), destination: other.destination.clone(), ..f };
        assert!(matches!(foreign.restore(&FaultInjector::none()), Err(RestoreError::JournalAlreadyExists)));
    }

    /// Moved from `vault_security::backup`'s tests (the guard moved with
    /// the writes): restoring into a non-empty destination is refused.
    #[test]
    fn restore_refuses_a_non_empty_destination() {
        let f = fixture(&[("note-a", "internal", "a\n")]);
        std::fs::create_dir_all(&f.destination).unwrap();
        std::fs::write(f.destination.join("pre-existing.txt"), b"already here").unwrap();
        assert!(matches!(f.restore(&FaultInjector::none()), Err(RestoreError::Backup(BackupError::AlreadyExists))));
        assert_eq!(std::fs::read(f.destination.join("pre-existing.txt")).unwrap(), b"already here");
    }

    /// Moved from `vault_security::backup`'s tests (Task 7 round 2 repro):
    /// restore must never silently overwrite live, newer PII ciphertext.
    #[test]
    fn restore_never_silently_overwrites_live_pii_ciphertext() {
        let f = fixture(&[("note-a", "sensitive", "OLD")]);
        let op = Uuid::new_v4();
        f.pii_store.stage("note-a", b"NEWER", op).unwrap();
        f.pii_store.commit_staged("note-a", op).unwrap();
        let result = f.restore(&FaultInjector::none());
        assert!(
            matches!(&result, Err(RestoreError::Backup(BackupError::PiiAlreadyExists { note_ids })) if note_ids == &["note-a".to_string()]),
            "got {:?}",
            result.err()
        );
        assert_eq!(f.pii_store.get("note-a").unwrap(), b"NEWER", "the live content is untouched");
        assert!(!f.journal_path.exists(), "nothing was promoted, so no journal is left behind");
    }

    /// Required test 5: for a note restored and THEN deleted, no read path
    /// consults it: `get_unless_deleted` refuses its content, and
    /// `read_note` withholds even the pointer record
    /// (`StorageError::Deleted`).
    #[test]
    fn no_read_api_consults_a_note_deleted_after_it_was_restored() {
        let f = fixture(&[("note-a", "sensitive", "SECRET A"), ("note-b", "internal", "b\n")]);
        let (vault, outcome) = f.restore(&FaultInjector::none()).unwrap();
        assert_eq!(outcome.restored_note_ids.len(), 2);
        assert!(vault.read_note(&id("note-a")).is_ok(), "precondition: readable before the deletion");
        f.delete("note-a");
        assert!(matches!(vault.read_note(&id("note-a")), Err(StorageError::Deleted { note_id }) if note_id == "note-a"));
        assert!(matches!(f.pii_store.get_unless_deleted("note-a"), Err(PiiStoreError::Deleted { .. })));
        assert!(vault.read_note(&id("note-b")).is_ok(), "other notes are unaffected");
        assert!(vault.current_version(&id("note-a")).is_ok(), "the version stays available to writers (a re-issued tombstone needs it)");
    }

    /// Required test 5, the raw-API boundary, pinned by inspection of the
    /// source: neither the restore promotion nor `Vault::read_note` ever
    /// calls the ungated `PiiStore::get` (restore stages plaintext from the
    /// backup itself, after its deletion checks; `read_note` never decrypts
    /// PII at all).
    #[test]
    fn the_raw_pii_get_is_not_reachable_from_the_restore_or_read_paths() {
        let restore_src = include_str!("restore.rs");
        let production = &restore_src[..restore_src.find("#[cfg(all(test, windows))]").unwrap()];
        for forbidden in ["pii_store.get(", "store.get(", ".get_staged(", ".get_before_image("] {
            assert!(!production.contains(forbidden), "restore.rs must not call {forbidden}");
        }
        let vault_src = include_str!("vault.rs");
        let read_path = &vault_src[vault_src.find("pub fn read_note(").unwrap()..vault_src.find("fn note_lock(").unwrap()];
        for forbidden in ["pii_store", "PiiStore", ".get_unless_deleted("] {
            assert!(!read_path.contains(forbidden), "read_note must not touch PII content ({forbidden})");
        }
    }
}
