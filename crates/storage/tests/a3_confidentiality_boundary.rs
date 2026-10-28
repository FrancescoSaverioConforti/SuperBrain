//! A3 confidentiality-boundary audit: the enumerated surfaces that must never
//! see plaintext or key material.
//!
//! Contract C3 (`docs/Spec_mvp.md`, line 364) names TEN surfaces: file,
//! frontmatter e metadati, allegati, file generati, cache, log, indici,
//! embedding, riassunti, backup. The stubs in this file map onto them as
//! follows.
//!
//! COVERED by the stubs below:
//! - file content and frontmatter/metadata: the git-history stub (every
//!   reachable blob, via the pointer scheme) and the A2 journal stub.
//! - log and error output: the `Debug`/`Display` stub (Task 6).
//! - file generati (generated files): the temp-file stub. Temp files written
//!   during an interrupted sensitive write ARE generated files, so this surface
//!   is covered by that stub, not deferred separately.
//! - backup: the backup-blob stub (Task 7).
//!
//! DEFERRED, deliberately, because the subsystem does not exist yet and so
//! there is nothing to audit:
//! - `indici` (search index), `embedding`, `riassunti` (summaries): Fase B and
//!   Fase C work has not started.
//! - `allegati` (attachments) and `cache`: no attachment-handling or caching
//!   subsystem exists anywhere in this codebase today.
//!
//! This is a deliberate deferral, not an oversight. B1/B2, C2, and any plan
//! that introduces attachments or a cache must re-audit the
//! sensitive-classification boundary against those surfaces once they exist.
//!
//! Every test below is a `#[ignore]`d stub. Its body is a `todo!()` placeholder
//! that compiles today but must not be run until the owning task lands. Each
//! stub's comment says what must be implemented to un-ignore it.

/// Surface: git history. Task 6 (write path and `PiiStore`).
///
/// Un-ignore by: calling `propose_and_commit` with a sensitive note, then
/// walking every blob reachable from every ref of the vault repo (for example
/// `git cat-file --batch-all-objects`, not just HEAD). Assert that the
/// plaintext appears in no blob. The pointer string is allowed; the plaintext
/// is not. This covers C3's "doppio controllo ... scansione periodica della
/// cronologia".
#[test]
#[ignore = "A3 Task 6: needs propose_and_commit sensitive path and PiiStore"]
fn no_git_blob_at_any_commit_ever_contains_sensitive_plaintext_or_the_pii_pointers_target_bytes() {
    todo!("implement in Task 6: scan every reachable blob for the sensitive plaintext")
}

/// Surface: A2 SQLite journal. Task 6 (write path).
///
/// Un-ignore by: after a sensitive-note write, reading every column of the
/// journal row for that `operation_id`. `content_hash` is a hash and is fine.
/// Nothing in the row may be the plaintext itself or the unwrapped data key.
#[test]
#[ignore = "A3 Task 6: needs the sensitive-note write path to produce a journal row"]
fn the_a2_sqlite_journals_columns_never_contain_sensitive_plaintext() {
    todo!("implement in Task 6: inspect every journal column for plaintext and key bytes")
}

/// Surface: before-images (`.before-images` directory in `vault.rs`). Task 6.
///
/// Un-ignore by: confirming the Task 6 write path never sends raw plaintext
/// through `persist_before_image`. The before-image of a sensitive note must be
/// the pointer-only Markdown file. This guards against an update path that
/// snapshots "the content" generically before it checks classification.
#[test]
#[ignore = "A3 Task 6: needs the sensitive update path that snapshots before-images"]
fn before_images_for_sensitive_notes_never_contain_plaintext_pointer_target_bytes() {
    todo!("implement in Task 6: assert before-images hold only the pointer-only file")
}

/// Surface: error and log output (`Debug` and `Display`). Task 6 (`PiiStore`).
///
/// Un-ignore by: forcing every `PiiStoreError` variant and the sensitive-path
/// `ProposeError` variants. Assert that their `{:?}` and `{}` output never
/// contains the fixture's plaintext or raw key or data-key bytes. Only note
/// ids, generation numbers and hashes may appear.
#[test]
#[ignore = "A3 Task 6: needs PiiStoreError and the sensitive-path ProposeError variants"]
fn no_error_variant_debug_or_display_output_from_pii_store_or_propose_and_commit_echoes_plaintext_or_key_bytes() {
    todo!("implement in Task 6: exercise each error variant and check Debug/Display output")
}

/// Surface: temp files under the vault root. Task 6 (`FaultPoints`).
///
/// Un-ignore by: interrupting a sensitive write partway through, using the
/// `FaultPoints` added in Task 6. Then scan every file under the vault root,
/// not only the expected ones, for the plaintext. Only ciphertext-shaped bytes,
/// or nothing if the step had not run yet, may be present.
#[test]
#[ignore = "A3 Task 6: needs FaultPoints to interrupt a sensitive write"]
fn an_interrupted_sensitive_write_leaves_no_recoverable_plaintext_in_any_temp_file_under_the_vault_root() {
    todo!("implement in Task 6: interrupt via FaultPoints and scan the whole vault root")
}

/// Surface: backup blobs. Task 7 (backup and restore).
///
/// Un-ignore by: taking a backup of a vault that holds a sensitive note. Read
/// the raw bytes that `BackupTarget::write_blob` received, not the bytes that
/// `restore()` returns. Assert that the plaintext does not appear as a
/// substring anywhere in that blob.
#[test]
#[ignore = "A3 Task 7: needs backup and restore with BackupTarget::write_blob"]
fn a_backup_blob_written_to_a_backuptarget_contains_no_plaintext_sensitive_content_outside_its_aead_envelope() {
    todo!("implement in Task 7: capture the raw bytes passed to write_blob and scan them")
}
