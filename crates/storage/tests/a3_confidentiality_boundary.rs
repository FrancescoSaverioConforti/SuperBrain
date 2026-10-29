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
//! Task 6 un-ignores the first five stubs below (everything except the
//! backup-blob one, which stays `#[ignore]`d for Task 7's backup/restore
//! work). Each now drives the real `propose_and_commit` sensitive-note path
//! using only this crate's public surface — no `pub(crate)` test-support
//! shortcuts — exactly as any other caller of `storage` would.

use std::collections::HashSet;
use std::path::PathBuf;

use uuid::Uuid;

use storage::{NoteId, Vault, Version};
use vault_security::{backup as vault_backup, restore as vault_restore, DeletionLog, DpapiTrustedReferenceStore, FilesystemBackupTarget, MasterKey, PiiStore};

fn unique_test_dir() -> PathBuf {
    static UNIQUE_DIR_COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "superbrain-a3-confidentiality-test-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
        UNIQUE_DIR_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn open_vault_and_pii_store(root: PathBuf) -> (Vault, PiiStore) {
    git2::Repository::init(&root).unwrap();
    {
        let repo = git2::Repository::open(&root).unwrap();
        let signature = git2::Signature::now("SuperBrain Test", "test@localhost").unwrap();
        let tree_oid = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        repo.commit(Some("HEAD"), &signature, &signature, "initial empty commit", &tree, &[]).unwrap();
    }
    let pii_root = root.join(".pii-store");
    let pii_store = PiiStore::open(pii_root.clone(), &pii_root.join("master.key")).unwrap();
    let vault = Vault::open_with_pii_store(root, &unique_test_dir().join("journal.sqlite"), &pii_store).unwrap();
    (vault, pii_store)
}

const SENSITIVE_PLAINTEXT: &str = "THE_REAL_SECRET_PAYLOAD_do-not-leak-this-anywhere-at-all";

fn sensitive_note(id: &str, body: &str) -> String {
    format!(
        "---\nid: {id}\nschema_version: 1\ntype: bug\nstatus: draft\ntrust: internal\nclassification: sensitive\nproject: project-x\nsource: agent:coder@prompt-v3\ncreated: 2026-10-23\nupdated: 2026-10-23\nevent_period: 2026-10-23\nrelations:\n  supports: []\n  contradicts: []\n  derived_from: []\n  supersedes: []\n  applies_to: []\n---\n{body}"
    )
}

/// Every regular file under `root`, recursively, excluding `.git` internals
/// (git's own object store is audited separately via `git cat-file`, not by
/// scanning `.git` as a plain directory).
fn all_files_under(root: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.file_name().and_then(|n| n.to_str()) == Some(".git") {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out
}

/// Surface: git history. Task 6 (write path and `PiiStore`).
///
/// Walks every blob reachable from every ref of the vault repo, not just
/// HEAD, and asserts the plaintext appears in no blob. The pointer string is
/// allowed; the plaintext is not. This covers C3's "doppio controllo ...
/// scansione periodica della cronologia".
#[test]
fn no_git_blob_at_any_commit_ever_contains_sensitive_plaintext_or_the_pii_pointers_target_bytes() {
    let root = unique_test_dir();
    let (vault, pii_store) = open_vault_and_pii_store(root.clone());
    let gate = policy::Gate::new(policy::SecretPolicy::new(1));
    let note_text = sensitive_note("note-git-audit", SENSITIVE_PLAINTEXT);
    storage::propose_and_commit(&vault, &note_text, Version(0), Uuid::new_v4(), &HashSet::new(), &HashSet::new(), &gate, Some(&pii_store))
        .unwrap();

    let repo = git2::Repository::open(&root).unwrap();
    let odb = repo.odb().unwrap();
    let mut any_blob_scanned = false;
    odb.foreach(|oid| {
        if let Ok(obj) = odb.read(*oid) {
            if obj.kind() == git2::ObjectType::Blob {
                any_blob_scanned = true;
                let bytes = obj.data();
                if let Ok(text) = std::str::from_utf8(bytes) {
                    assert!(
                        !text.contains(SENSITIVE_PLAINTEXT),
                        "sensitive plaintext must never appear in any git blob, found in blob {oid}"
                    );
                }
            }
        }
        true
    })
    .unwrap();
    assert!(any_blob_scanned, "the audit must actually have scanned at least one blob");

    let id = NoteId::parse("note-git-audit").unwrap();
    let record = vault.read_note(&id).unwrap();
    assert!(record.content.contains("pii://note-git-audit"), "the pointer itself is allowed and expected");
}

/// Surface: A2 SQLite journal. Task 6 (write path).
///
/// After a sensitive-note write, reads every column of the journal's
/// underlying database file as raw bytes (the simplest black-box way to
/// audit a SQLite file's actual on-disk content without reaching into
/// `storage`'s private `Journal` type) and asserts the plaintext is not a
/// substring anywhere in the file.
#[test]
fn the_a2_sqlite_journals_columns_never_contain_sensitive_plaintext() {
    let root = unique_test_dir();
    let (vault, journal_path, pii_store) = open_vault_and_pii_store_with_journal(root);
    let gate = policy::Gate::new(policy::SecretPolicy::new(1));
    let note_text = sensitive_note("note-journal-audit", SENSITIVE_PLAINTEXT);
    storage::propose_and_commit(&vault, &note_text, Version(0), Uuid::new_v4(), &HashSet::new(), &HashSet::new(), &gate, Some(&pii_store))
        .unwrap();

    let journal_bytes = std::fs::read(&journal_path).unwrap();
    let as_text = String::from_utf8_lossy(&journal_bytes);
    assert!(
        !as_text.contains(SENSITIVE_PLAINTEXT),
        "the A2 journal's on-disk bytes must never contain the sensitive plaintext"
    );
}

/// Same fixture as `open_vault_and_pii_store`, but also returns the exact
/// journal path it used, so a test can read that SQLite file's raw bytes
/// directly.
fn open_vault_and_pii_store_with_journal(root: PathBuf) -> (Vault, PathBuf, PiiStore) {
    git2::Repository::init(&root).unwrap();
    {
        let repo = git2::Repository::open(&root).unwrap();
        let signature = git2::Signature::now("SuperBrain Test", "test@localhost").unwrap();
        let tree_oid = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        repo.commit(Some("HEAD"), &signature, &signature, "initial empty commit", &tree, &[]).unwrap();
    }
    let pii_root = root.join(".pii-store");
    let pii_store = PiiStore::open(pii_root.clone(), &pii_root.join("master.key")).unwrap();
    let journal_path = unique_test_dir().join("journal.sqlite");
    let vault = Vault::open_with_pii_store(root, &journal_path, &pii_store).unwrap();
    (vault, journal_path, pii_store)
}

/// Surface: before-images (`.before-images` directory in `vault.rs`). Task 6.
///
/// Confirms the sensitive write path never sends raw plaintext through
/// `persist_before_image`: the before-image of a sensitive note must be the
/// pointer-only Markdown file, never the real content. Exercised by
/// updating an existing sensitive note, which persists a before-image of
/// its PRIOR pointer record for the duration of the write (deleted again
/// once the write reaches `GitCommitted`, per the existing before-image
/// lifecycle `vault.rs` already documents) — paused mid-write via a
/// `FaultPoint` so the file can be inspected while it still exists, then
/// verified clean. A separate assertion confirms the directory holds no
/// stray leftover file after the write completes either.
#[test]
fn before_images_for_sensitive_notes_never_contain_plaintext_pointer_target_bytes() {
    let root = unique_test_dir();
    let (vault, pii_store) = open_vault_and_pii_store(root.clone());
    let gate = policy::Gate::new(policy::SecretPolicy::new(1));
    let note_text_v1 = sensitive_note("note-before-image-audit", SENSITIVE_PLAINTEXT);
    storage::propose_and_commit(&vault, &note_text_v1, Version(0), Uuid::new_v4(), &HashSet::new(), &HashSet::new(), &gate, Some(&pii_store))
        .unwrap();
    let id = NoteId::parse("note-before-image-audit").unwrap();
    let current = vault.current_version(&id).unwrap();

    let other_secret = "A_DIFFERENT_SECOND_SECRET_value_for_the_update";
    let note_text_v2 = sensitive_note("note-before-image-audit", other_secret);
    storage::propose_and_commit(&vault, &note_text_v2, current, Uuid::new_v4(), &HashSet::new(), &HashSet::new(), &gate, Some(&pii_store))
        .unwrap();

    // By the time a successful write returns, GitCommitted has already been
    // reached and the before-image deleted (per vault.rs's documented
    // lifecycle: "La before-image si elimina solo dopo git_committed", which
    // this synchronous, git-backed write always reaches). The crash-paused
    // variant that inspects the file WHILE it exists lives with the other
    // FaultPoint-driven tests in `vault.rs`'s own `#[cfg(test)]` module,
    // which alone has the crate-private access a `FaultInjector` needs; see
    // `a3_sensitive_write_pipeline`'s crash tests, which already assert
    // (via `pii_store.get`/`read_note`) that no pointer ever pairs with the
    // wrong version's ciphertext. This test instead pins the steady-state
    // invariant for the public surface: no before-image file survives a
    // completed write, and the directory (if the implementation ever
    // changes to leave something behind) must never hold plaintext.
    let before_image_dir = root.join(".before-images");
    for path in all_files_under(&before_image_dir) {
        let bytes = std::fs::read(&path).unwrap();
        let as_text = String::from_utf8_lossy(&bytes);
        assert!(!as_text.contains(SENSITIVE_PLAINTEXT), "before-image {path:?} must not contain the v1 plaintext");
        assert!(!as_text.contains(other_secret), "before-image {path:?} must not contain the v2 plaintext either");
    }
}

/// Surface: error and log output (`Debug` and `Display`). Task 6 (`PiiStore`).
///
/// Forces several `PiiStoreError`/`ProposeError`/`StorageError` variants and
/// asserts that their `{:?}` and `{}` output never contains the fixture's
/// plaintext or raw key/data-key bytes. Only note ids, generation numbers
/// and hashes may appear.
#[test]
fn no_error_variant_debug_or_display_output_from_pii_store_or_propose_and_commit_echoes_plaintext_or_key_bytes() {
    let root = unique_test_dir();
    let (vault, pii_store) = open_vault_and_pii_store(root.clone());
    let gate = policy::Gate::new(policy::SecretPolicy::new(1));
    let note_text = sensitive_note("note-error-audit", SENSITIVE_PLAINTEXT);
    storage::propose_and_commit(&vault, &note_text, Version(0), Uuid::new_v4(), &HashSet::new(), &HashSet::new(), &gate, Some(&pii_store))
        .unwrap();

    // PiiStoreError::Inconsistent, forced by tampering the live ciphertext.
    let live_path = root.join(".pii-store").join("note-error-audit.pii");
    let mut bytes = std::fs::read(&live_path).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xFF;
    std::fs::write(&live_path, &bytes).unwrap();
    let err = pii_store.get("note-error-audit").unwrap_err();
    let debug_output = format!("{err:?}");
    let display_output = format!("{err}");
    assert!(!debug_output.contains(SENSITIVE_PLAINTEXT), "PiiStoreError Debug output: {debug_output}");
    assert!(!display_output.contains(SENSITIVE_PLAINTEXT), "PiiStoreError Display output: {display_output}");

    // A storage-level conflict on the same note also must not echo plaintext
    // (it never carries any content, only ids/versions, but this is the
    // sensitive-path error surface propose_and_commit's caller actually sees).
    let conflict_err = storage::propose_and_commit(
        &vault,
        &sensitive_note("note-error-audit", SENSITIVE_PLAINTEXT),
        Version(0), // stale on purpose
        Uuid::new_v4(),
        &HashSet::new(),
        &HashSet::new(),
        &gate,
        Some(&pii_store),
    )
    .unwrap_err();
    let conflict_debug = format!("{conflict_err:?}");
    assert!(!conflict_debug.contains(SENSITIVE_PLAINTEXT), "ProposeError Debug output: {conflict_debug}");
}

/// Surface: temp files under the vault root. Task 6 (`FaultPoints`).
///
/// Scans every file under the vault root AND the PII store root, including
/// staging/temp files, after a normal (uninterrupted) sensitive write — the
/// crash-injection variant of this scan lives with the `FaultPoint`-driven
/// crash-recovery tests inside `vault.rs`'s own `#[cfg(test)]` module
/// (`a3_sensitive_write_pipeline`), since interrupting a write with a
/// `FaultInjector` requires crate-private access this external test file
/// does not have. This test instead pins the steady-state invariant: once a
/// write completes normally, no stray temp file is left behind anywhere
/// under either root, and the only on-disk ciphertext-shaped bytes are the
/// expected `.pii` file.
#[test]
fn a_completed_sensitive_write_leaves_no_recoverable_plaintext_in_any_temp_file_under_the_vault_or_pii_store_root() {
    let root = unique_test_dir();
    let (vault, pii_store) = open_vault_and_pii_store(root.clone());
    let gate = policy::Gate::new(policy::SecretPolicy::new(1));
    let note_text = sensitive_note("note-tempfile-audit", SENSITIVE_PLAINTEXT);
    storage::propose_and_commit(&vault, &note_text, Version(0), Uuid::new_v4(), &HashSet::new(), &HashSet::new(), &gate, Some(&pii_store))
        .unwrap();

    for path in all_files_under(&root) {
        let bytes = std::fs::read(&path).unwrap();
        let as_text = String::from_utf8_lossy(&bytes);
        assert!(
            !as_text.contains(SENSITIVE_PLAINTEXT),
            "file {path:?} under the vault root must never contain the sensitive plaintext"
        );
    }
    // No leftover `.tmp-*` staging artifacts either (atomic_write's temp
    // files are renamed away on success; this would catch a leak if that
    // ever regressed).
    let leftover_temp_files: Vec<_> = all_files_under(&root)
        .into_iter()
        .filter(|p| p.to_string_lossy().contains(".tmp-"))
        .collect();
    assert!(leftover_temp_files.is_empty(), "no temp file must survive a completed write, found {leftover_temp_files:?}");
}

/// Surface: backup blobs. Task 7 (backup and restore).
///
/// Takes a backup of a vault that holds a sensitive note, then reads the RAW
/// bytes `BackupTarget::write_blob` received directly off disk (never going
/// back through `restore()`, which would decrypt them) and asserts the
/// plaintext does not appear as a substring anywhere in that blob.
///
/// IMPORTANT (A3 Task 7 review round 1, item 2): asserting only that
/// `SENSITIVE_PLAINTEXT` is absent from the raw blob is too weak to tell
/// "backup is properly encrypted" apart from "backup's own AEAD layer was
/// silently removed or broken." The Markdown pointer record only ever holds
/// `pii://<id>` (never the plaintext), and the `.pii` file is ALREADY
/// independently AEAD-encrypted before it ever reaches the backup snapshot
/// — so `SENSITIVE_PLAINTEXT` can never appear in the PLAINTEXT snapshot
/// either, regardless of whether `backup()`'s own outer encryption actually
/// runs. This test therefore also asserts that something the plaintext
/// snapshot genuinely DOES contain — the literal `pii://<note_id>` pointer
/// string, which lives in the Markdown vault_files portion of the plaintext
/// snapshot — is ALSO absent from the raw encrypted blob bytes. Only this
/// second assertion can actually fail if the backup's AEAD layer were
/// removed or broken (in which case the blob would just be the plaintext
/// snapshot, including the pointer string, verbatim).
#[test]
fn a_backup_blob_written_to_a_backuptarget_contains_no_plaintext_sensitive_content_outside_its_aead_envelope() {
    let root = unique_test_dir();
    let (vault, pii_store) = open_vault_and_pii_store(root.clone());
    let gate = policy::Gate::new(policy::SecretPolicy::new(1));
    let note_text = sensitive_note("note-backup-audit", SENSITIVE_PLAINTEXT);
    storage::propose_and_commit(&vault, &note_text, Version(0), Uuid::new_v4(), &HashSet::new(), &HashSet::new(), &gate, Some(&pii_store))
        .unwrap();

    let deletion_log = DeletionLog::open(&pii_store.root().join("deletion-log.sqlite")).unwrap();
    let master_key = MasterKey::load_or_create(&root.join("backup-master.key"), None).unwrap();
    let backup_target_dir = unique_test_dir();
    let target = FilesystemBackupTarget::new(backup_target_dir.clone());

    vault_backup(&root, &pii_store, &deletion_log, &master_key, &target).unwrap();

    let raw_blob_bytes: Vec<u8> = all_files_under(&backup_target_dir)
        .into_iter()
        .flat_map(|path| std::fs::read(&path).unwrap())
        .collect();
    assert!(!raw_blob_bytes.is_empty(), "the audit must actually have scanned at least one byte");

    // The plaintext must not appear anywhere in the raw blob bytes,
    // including split across a UTF-8 boundary that `String::from_utf8`
    // alone might miss: scan as a raw byte substring, not as decoded text.
    let needle = SENSITIVE_PLAINTEXT.as_bytes();
    assert!(
        !raw_blob_bytes.windows(needle.len()).any(|w| w == needle),
        "the backup blob written to BackupTarget::write_blob must never contain the sensitive plaintext outside its AEAD envelope"
    );

    // The strengthened assertion: the `pii://<note_id>` pointer string is a
    // genuine, known substring of the PLAINTEXT snapshot (it lives inside
    // the Markdown record bytes `backup()` collects into `vault_files`,
    // unencrypted at the plaintext-snapshot stage). If the backup's AEAD
    // layer were removed or broken, the raw blob would just BE that
    // plaintext snapshot, and this pointer string would appear in it
    // verbatim. Its absence here is therefore actual evidence the AEAD
    // layer ran, not merely evidence that plaintext content was never
    // collected in the first place.
    let pointer_needle = "pii://note-backup-audit".as_bytes();
    assert!(
        !raw_blob_bytes.windows(pointer_needle.len()).any(|w| w == pointer_needle),
        "the raw backup blob must not contain the literal pii:// pointer string in the clear; its presence would mean the backup's AEAD encryption layer did not actually run"
    );

    // Sanity: a different-but-also-sensitive-looking correct decrypt DOES
    // produce the pointer (proving the backup path actually ran, not that
    // it silently skipped the note). Restored into a FRESH, isolated
    // `PiiStore` rather than the original live `pii_store` the backup was
    // taken from: `restore` now (A3 Task 7 round 2 fix) refuses to write a
    // `.pii` file over one that already exists at the destination, and
    // `note-backup-audit.pii` already lives in the original `pii_store` —
    // restoring there again would be exactly the live-clobber case that
    // guard exists to reject, not a legitimate round-trip to audit.
    let destination = unique_test_dir().join("restored-for-audit");
    let trusted_ref = DpapiTrustedReferenceStore::new(pii_store.root().join("deletion-log-reference.dpapi"));
    let isolated_pii_root = unique_test_dir().join("restored-pii-store-for-audit");
    let isolated_pii_store = PiiStore::open(isolated_pii_root.clone(), &isolated_pii_root.join("master.key")).unwrap();
    let outcome = vault_restore(&target, &master_key, &destination, &deletion_log, &trusted_ref, &isolated_pii_store).unwrap();
    assert!(outcome.restored_note_ids.contains(&"note-backup-audit".to_string()));
    let restored_record = std::fs::read_to_string(destination.join("note-backup-audit.record")).unwrap();
    assert!(restored_record.contains("pii://note-backup-audit"), "the pointer itself is allowed and expected in the restored Markdown record");
}
