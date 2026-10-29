//! Restore a backup into a vault that is usable through the normal storage
//! API (final whole-branch review, I1).
//!
//! `vault_security::restore` writes byte-identical `.record` and `.pii`
//! files, but `Vault::read_note` resolves every read through the A2 journal
//! (`Journal::last_committed_for_note`), and the journal is a separate
//! SQLite file the backup does not contain. A directory restored by
//! `vault_security::restore` alone therefore reads as `NotFound` for every
//! note. [`restore_vault_from_backup`] closes that gap.
//!
//! # Why the journal is rebuilt, not backed up
//!
//! The journal lives at a caller-chosen path outside the vault root, which
//! `vault_security::backup` never sees. Copying it into the backup would
//! also be wrong on its own terms: a live SQLite file copied mid-write can
//! be internally inconsistent; its rows describe before-images and git
//! commits the backup does not carry (dot-directories and `.git` are not
//! backed up); it holds rows for notes the restore now excludes as deleted
//! (C1); and any non-terminal row would make the restored vault's first
//! `Vault::open` try to roll back against before-images that do not exist.
//!
//! So a fresh journal is seeded from what was actually restored, which the
//! backup's AEAD envelope and plaintext hash have already authenticated:
//! one row per restored record, carrying that record's own version and
//! content hash (and, for a `Sensitive` note, the hash of the plaintext its
//! restored ciphertext decrypts to). Each row is driven through the normal
//! `IntentRecorded -> ContentPersisted -> JournalCommitted` transitions and
//! left at `JournalCommitted`, the journal's documented resting state for
//! "durable and readable, not (yet) in git": the restored root has no git
//! history, so claiming `GitCommitted` would be false. The final
//! `Vault::open_with_pii_store` then runs the ordinary reconcile pass over
//! those rows (git unavailable: they rest as `git_pending`; a git repository
//! with a first commit at the root: they are committed).
//!
//! # Not a second write entry point
//!
//! `propose_and_commit` stays the only way to write a note into a vault.
//! This function cannot add to an existing vault: it refuses a journal path
//! that already exists, and `vault_security::restore` refuses a non-empty
//! destination. It only bootstraps a brand-new vault from a backup blob
//! authenticated under the backup master key, whose every note already
//! passed the gate when it was first written.

use std::path::Path;

use domain::Classification;
use uuid::Uuid;
use vault_security::{BackupError, BackupTarget, DeletionLog, MasterKey, PiiPointer, PiiStore, RestoreOutcome, TrustedReferenceStore};

use crate::journal::{AdvanceFields, Journal};
use crate::note_id::NoteId;
use crate::propose::{body_of, frontmatter_of};
use crate::vault::{content_hash, parse_record, record_path_checked, StorageError, Vault};

/// Why [`restore_vault_from_backup`] failed.
#[derive(Debug, thiserror::Error)]
pub enum RestoreError {
    /// `vault_security::restore` refused or failed (empty target, hash
    /// mismatch, deletion-log rejection, non-empty destination, live PII
    /// clobber, ...). Nothing was journaled.
    #[error("backup restore failed: {0}")]
    Backup(BackupError),
    /// `journal_path` already exists. Restore only ever creates a brand-new
    /// vault, never adds records to an existing journal.
    #[error("refusing to restore: a journal already exists at the given path")]
    JournalAlreadyExists,
    /// The restored files could not be journaled or the restored vault could
    /// not be opened. The restored files stay where they are; the
    /// partially-seeded journal is removed, so the destination is not left
    /// looking like a usable vault.
    #[error("restored files could not be turned into a usable vault: {0}")]
    Storage(StorageError),
}

/// Restores the backup in `target` into `destination_root` (and its PII
/// ciphertext into `pii_store`'s root), exactly as `vault_security::restore`
/// does, then seeds a fresh A2 journal at `journal_path` from the restored
/// records and opens the result as a [`Vault`], so every restored note is
/// readable through [`Vault::read_note`] (and a `Sensitive` one's content
/// through `PiiStore::get_unless_deleted`). See the module docs for why the
/// journal is rebuilt rather than backed up.
///
/// Fails closed: `journal_path` must not exist yet; a restored record whose
/// note id, version header or frontmatter does not parse, or a `Sensitive`
/// record whose body is not its own `pii://` pointer or whose ciphertext
/// does not decrypt, is [`RestoreError::Storage`] (`Inconsistent`).
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
    match std::fs::metadata(journal_path) {
        Ok(_) => return Err(RestoreError::JournalAlreadyExists),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(RestoreError::Storage(StorageError::Io(e))),
    }

    let outcome = vault_security::restore(target, master_key, destination_root, live_deletion_log, live_trusted_ref, pii_store)
        .map_err(RestoreError::Backup)?;

    let seeded = seed_journal(destination_root, journal_path, pii_store, &outcome.restored_note_ids)
        .and_then(|()| Vault::open_with_pii_store(destination_root.to_path_buf(), journal_path, pii_store));
    match seeded {
        Ok(vault) => Ok((vault, outcome)),
        Err(e) => {
            // We created this file (it did not exist on entry); a
            // half-seeded journal must not be left behind looking usable.
            let _ = std::fs::remove_file(journal_path);
            Err(RestoreError::Storage(e))
        }
    }
}

/// Seeds one `JournalCommitted` row per restored record. See the module docs.
fn seed_journal(root: &Path, journal_path: &Path, pii_store: &PiiStore, note_ids: &[String]) -> Result<(), StorageError> {
    let journal = Journal::open(journal_path).map_err(|e| StorageError::Journal(e.to_string()))?;
    let journal_err = |e: crate::journal::JournalError| StorageError::Journal(e.to_string());
    for raw_id in note_ids {
        let inconsistent = || StorageError::Inconsistent { note_id: raw_id.clone() };
        let note_id = NoteId::parse(raw_id).map_err(|_| inconsistent())?;
        let path = record_path_checked(root, &note_id)?;
        let bytes = std::fs::read(&path).map_err(StorageError::Io)?;
        let record = parse_record(&bytes, &note_id)?;
        if record.version.0 == 0 {
            return Err(inconsistent());
        }
        let note = domain::parse_frontmatter(frontmatter_of(&record.content)).map_err(|_| inconsistent())?;
        if note.id != note_id.as_str() {
            return Err(inconsistent());
        }

        let mut fields = AdvanceFields::content(record.version.0, content_hash(&bytes));
        if matches!(note.classification, Classification::Sensitive) {
            if body_of(&record.content).trim() != PiiPointer::for_note(note_id.as_str()).uri {
                return Err(inconsistent());
            }
            let plaintext = pii_store.get(note_id.as_str()).map_err(|_| inconsistent())?;
            fields = fields.merge_pii_content_hash(content_hash(&plaintext));
        }

        let operation_id = Uuid::new_v4();
        journal.insert_intent(operation_id, note_id.as_str(), record.version.0 - 1).map_err(journal_err)?;
        journal.advance(operation_id, domain::WriteState::ContentPersisted, fields).map_err(journal_err)?;
        journal.advance(operation_id, domain::WriteState::JournalCommitted, AdvanceFields::none()).map_err(journal_err)?;
    }
    Ok(())
}
