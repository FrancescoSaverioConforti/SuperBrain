//! Task 6: the single, crash-recoverable sensitive-write pipeline — the
//! PUBLIC-surface half of this task's test coverage.
//!
//! Exercises `PiiStore` (via `vault-security`) wired into `storage` through
//! `propose_and_commit`'s sensitive branch, using ONLY `propose_and_commit`
//! itself — the one and only public write entry point out of this crate
//! (see `integration.rs`'s `storage_exposes_exactly_one_public_write_function`,
//! which this file does not and must not try to work around).
//!
//! The crash-recovery tests that need to interrupt the pipeline at an exact
//! `FaultPoint` (`AfterPiiStagedBeforeCommit`,
//! `AfterPiiCommittedBeforePointerRename`) necessarily reach the
//! crate-private `Vault::write_sensitive_record_unchecked_with_injector`
//! and the crate-private `FaultPoint`/`FaultInjector` types, so — exactly
//! like every other fault-injection test in this crate — they live INSIDE
//! `crates/storage/src/vault.rs`'s own `#[cfg(test)] mod tests`, not in this
//! external integration-test file, which cannot see `pub(crate)` items.
//! See `vault.rs`'s `a3_sensitive_write_pipeline` test submodule for:
//! `crash_after_pii_staged_before_pointer_committed_recovers_without_a_readable_half_written_pointer`
//! and
//! `crash_mid_update_of_an_already_sensitive_note_never_exposes_a_pointer_whose_ciphertext_is_the_wrong_version`.

use std::collections::HashSet;
use std::path::PathBuf;

use uuid::Uuid;

use storage::{CommitOutcome, CommitResult, NoteId, ProposeError, StorageError, Vault, Version};
use vault_security::PiiStore;

fn unique_test_dir() -> PathBuf {
    static UNIQUE_DIR_COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "superbrain-a3-pii-test-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
        UNIQUE_DIR_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A git-backed `Vault` rooted at `root`, mirroring `integration.rs`'s
/// `open_vault` helper, plus the `PiiStore` sibling this file's tests need.
fn open_vault_and_pii_store(root: PathBuf) -> (Vault, PiiStore) {
    git2::Repository::init(&root).unwrap();
    {
        let repo = git2::Repository::open(&root).unwrap();
        let signature = git2::Signature::now("SuperBrain Test", "test@localhost").unwrap();
        let tree_oid = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        repo.commit(Some("HEAD"), &signature, &signature, "initial empty commit", &tree, &[]).unwrap();
    }
    let pii_store = open_pii_store(&root);
    let vault = Vault::open_with_pii_store(root, &unique_test_dir().join("journal.sqlite"), &pii_store).unwrap();
    (vault, pii_store)
}

fn open_pii_store(vault_root: &std::path::Path) -> PiiStore {
    let pii_root = vault_root.join(".pii-store");
    PiiStore::open(pii_root.clone(), &pii_root.join("master.key")).unwrap()
}

fn sensitive_yaml(id: &str) -> String {
    format!(
        r#"id: {id}
schema_version: 1
type: bug
status: draft
trust: internal
classification: sensitive
project: project-x
source: agent:coder@prompt-v3
created: 2026-10-23
updated: 2026-10-23
event_period: 2026-10-23
relations:
  supports: []
  contradicts: []
  derived_from: []
  supersedes: []
  applies_to: []
"#
    )
}

/// A full sensitive note: `---`-delimited frontmatter, then `body` as the
/// real plaintext content that must end up ONLY in `PiiStore`.
fn sensitive_note(id: &str, body: &str) -> String {
    format!("---\n{}---\n{body}", sensitive_yaml(id))
}

/// A sensitive-note deletion request: same frontmatter, body is the
/// reserved tombstone marker.
fn sensitive_tombstone(id: &str) -> String {
    sensitive_note(id, storage::SENSITIVE_TOMBSTONE_BODY)
}

fn gate() -> policy::Gate {
    policy::Gate::new(policy::SecretPolicy::new(1))
}

fn empty() -> (HashSet<NoteId>, HashSet<String>) {
    (HashSet::new(), HashSet::new())
}

// ---------------------------------------------------------------------
// Step 1/2: happy path
// ---------------------------------------------------------------------

#[test]
fn a_sensitive_note_is_written_to_the_pii_store_and_the_markdown_record_holds_only_a_pointer() {
    let root = unique_test_dir();
    let (vault, pii_store) = open_vault_and_pii_store(root);
    let (known, pending) = empty();
    let gate = gate();
    let note_text = sensitive_note("note-secret", "my actual sensitive content: SSN 123-45-6789");

    let outcome = storage::propose_and_commit(&vault, &note_text, Version(0), Uuid::new_v4(), &known, &pending, &gate, Some(&pii_store))
        .expect("sensitive write with an available store must succeed");
    assert!(matches!(outcome, CommitResult::Committed(CommitOutcome { version: Version(1), .. })), "got {outcome:?}");

    let id = NoteId::parse("note-secret").unwrap();
    let record = vault.read_note(&id).unwrap();
    assert!(record.content.contains("pii://note-secret"), "the Markdown record must hold the pointer, got {:?}", record.content);
    assert!(
        !record.content.contains("my actual sensitive content"),
        "the Markdown record must NEVER hold the plaintext, got {:?}",
        record.content
    );

    let plaintext = pii_store.get("note-secret").unwrap();
    assert_eq!(plaintext, b"my actual sensitive content: SSN 123-45-6789");
}

#[test]
fn reusing_an_operation_id_with_different_sensitive_content_is_refused_as_reused() {
    // A3 review round, IMPORTANT finding #3: for a Sensitive note, the
    // Markdown pointer bytes are IDENTICAL regardless of the real
    // plaintext (frontmatter plus the fixed pii://<id> URI), so a
    // pointer-hash-only retry check could never catch a reused
    // operation_id carrying genuinely different sensitive content. The
    // first call commits fully; a second call with the SAME operation_id
    // but a note whose real content differs must be refused as
    // `OperationIdReused`, not silently treated as a no-op retry of the
    // first call (which would under-report what actually happened) nor as
    // a second write (which would violate operation_id idempotency).
    let root = unique_test_dir();
    let (vault, pii_store) = open_vault_and_pii_store(root);
    let (known, pending) = empty();
    let gate = gate();
    let op = Uuid::new_v4();

    let note_text_a = sensitive_note("note-reuse", "first plaintext content");
    storage::propose_and_commit(&vault, &note_text_a, Version(0), op, &known, &pending, &gate, Some(&pii_store)).unwrap();

    // Reuse the SAME operation_id, same note, same expected_version, but
    // DIFFERENT real sensitive content. The pointer bytes propose.rs builds
    // are identical (same frontmatter, same fixed pii:// URI), so only a
    // pii_content_hash-aware check can catch this.
    let note_text_b = sensitive_note("note-reuse", "a completely different second plaintext");
    let result = storage::propose_and_commit(&vault, &note_text_b, Version(0), op, &known, &pending, &gate, Some(&pii_store));
    assert!(
        matches!(result, Err(ProposeError::Storage(StorageError::OperationIdReused { operation_id })) if operation_id == op),
        "got {result:?}"
    );

    // The original write's content must be completely unaffected.
    let id = NoteId::parse("note-reuse").unwrap();
    assert_eq!(pii_store.get("note-reuse").unwrap(), b"first plaintext content");
    let _ = vault.read_note(&id).unwrap();
}

#[test]
fn a_non_sensitive_note_write_is_completely_unaffected_by_the_pii_pipeline() {
    // Pins that Internal/Public notes go through the UNCHANGED
    // write_record_unchecked path: no pointer substitution, no PiiStore
    // involvement at all.
    let root = unique_test_dir();
    let (vault, pii_store) = open_vault_and_pii_store(root);
    let (known, pending) = empty();
    let gate = gate();
    let note_text =
        "---\nid: note-plain\nschema_version: 1\ntype: bug\nstatus: draft\ntrust: internal\nclassification: internal\nproject: p\nsource: agent:coder@prompt-v3\ncreated: 2026-10-23\nupdated: 2026-10-23\nevent_period: 2026-10-23\nrelations:\n  supports: []\n  contradicts: []\n  derived_from: []\n  supersedes: []\n  applies_to: []\n---\nplain body text\n"
            .to_string();
    storage::propose_and_commit(&vault, &note_text, Version(0), Uuid::new_v4(), &known, &pending, &gate, Some(&pii_store)).unwrap();
    let id = NoteId::parse("note-plain").unwrap();
    let record = vault.read_note(&id).unwrap();
    assert_eq!(record.content, note_text, "non-sensitive content is stored byte-for-byte, unchanged");
    assert!(!pii_store.has_live("note-plain"), "the PiiStore must never be touched for a non-sensitive note");
}

#[test]
fn a_non_sensitive_write_succeeds_with_no_pii_store_configured_at_all() {
    // A3 review round, IMPORTANT finding #2: a mandatory &PiiStore would
    // make even an Internal/Public write depend on a working encrypted
    // store. With `pii_store: None`, an ordinary write must succeed exactly
    // as it always has.
    let root = unique_test_dir();
    let (vault, _unused_pii_store) = open_vault_and_pii_store(root);
    let (known, pending) = empty();
    let gate = gate();
    let note_text = "---\nid: note-no-store\nschema_version: 1\ntype: bug\nstatus: draft\ntrust: internal\nclassification: internal\nproject: p\nsource: agent:coder@prompt-v3\ncreated: 2026-10-23\nupdated: 2026-10-23\nevent_period: 2026-10-23\nrelations:\n  supports: []\n  contradicts: []\n  derived_from: []\n  supersedes: []\n  applies_to: []\n---\nplain body\n";
    let result = storage::propose_and_commit(&vault, note_text, Version(0), Uuid::new_v4(), &known, &pending, &gate, None);
    assert!(matches!(result, Ok(CommitResult::Committed(_))), "a non-sensitive write with no PiiStore at all must still succeed, got {result:?}");
}

#[test]
fn a_sensitive_write_is_rejected_when_no_pii_store_is_configured() {
    // The flip side of the above: a Sensitive note with `pii_store: None`
    // must be rejected by the gate exactly as Task 4 designed — a broken
    // or unconfigured store blocks ONLY Sensitive writes, never ordinary
    // ones (see the test above).
    let root = unique_test_dir();
    let (vault, _unused_pii_store) = open_vault_and_pii_store(root);
    let (known, pending) = empty();
    let gate = gate();
    let note_text = sensitive_note("note-no-store-sensitive", "some sensitive content");
    let result = storage::propose_and_commit(&vault, &note_text, Version(0), Uuid::new_v4(), &known, &pending, &gate, None);
    assert!(
        matches!(result, Err(storage::ProposeError::Rejected(policy::GateRejection::SensitiveRequiresEncryptedStore { .. }))),
        "a Sensitive write with no PiiStore must be rejected by the gate, got {result:?}"
    );
}

// ---------------------------------------------------------------------
// Step 7/8: orphan detection
// ---------------------------------------------------------------------

#[test]
fn deleting_the_pointer_note_without_going_through_the_deletion_log_leaves_detectable_orphaned_ciphertext() {
    let root = unique_test_dir();
    let (vault, pii_store) = open_vault_and_pii_store(root);
    let (known, pending) = empty();
    let gate = gate();
    let note_text = sensitive_note("note-improper-delete", "secret to be improperly deleted");
    storage::propose_and_commit(&vault, &note_text, Version(0), Uuid::new_v4(), &known, &pending, &gate, Some(&pii_store)).unwrap();
    assert!(pii_store.has_live("note-improper-delete"));

    // Simulate an improper deletion: the pointer note is never retracted
    // through propose_and_commit's deletion path at all (e.g. an operator
    // manually deleted the record file, or a future bug skips the deletion
    // log). From PiiStore's point of view alone, no pointer exists anymore.
    let known_pointers: HashSet<String> = HashSet::new();
    let orphans = pii_store.find_orphans(&known_pointers);
    assert_eq!(orphans, vec!["note-improper-delete".to_string()], "an orphan must be detectable");
}

#[test]
fn a_properly_deleted_sensitive_note_goes_through_the_deletion_log_and_is_not_flagged_as_an_unexplained_orphan() {
    let root = unique_test_dir();
    let (vault, pii_store) = open_vault_and_pii_store(root);
    let (known, pending) = empty();
    let gate = gate();
    let note_text = sensitive_note("note-proper-delete", "secret to be properly deleted");
    storage::propose_and_commit(&vault, &note_text, Version(0), Uuid::new_v4(), &known, &pending, &gate, Some(&pii_store)).unwrap();
    let id = NoteId::parse("note-proper-delete").unwrap();
    let current = vault.current_version(&id).unwrap();

    let tombstone_text = sensitive_tombstone("note-proper-delete");
    let result = storage::propose_and_commit(&vault, &tombstone_text, current, Uuid::new_v4(), &known, &pending, &gate, Some(&pii_store))
        .expect("a tombstone write must succeed");
    assert!(matches!(result, CommitResult::Committed(_)), "got {result:?}");

    let deletion_log_path = pii_store.root().join("deletion-log.sqlite");
    let deletion_log = vault_security::DeletionLog::open(&deletion_log_path).unwrap();
    assert!(deletion_log.is_deleted("note-proper-delete").unwrap(), "the deletion log must record this deletion");

    // The pointer note STILL exists (now pointing at empty tombstone
    // content), so this note id is a "known pointer" from the vault's point
    // of view and must not be flagged as an orphan despite also being
    // deleted in the log — the two mechanisms are complementary, not
    // contradictory: an auditor cross-checks PiiStore orphans against the
    // SET OF POINTERS THE VAULT ACTUALLY HAS, which still includes this one.
    let mut known_pointers = HashSet::new();
    known_pointers.insert("note-proper-delete".to_string());
    let orphans = pii_store.find_orphans(&known_pointers);
    assert!(!orphans.contains(&"note-proper-delete".to_string()), "a deletion-logged note must not be an unexplained orphan");
}

/// Final whole-branch review, I2 (Option A: a deleted note id is retired).
/// Before the fix, new content could be written under a deleted id and
/// `get` returned it while `is_deleted` still said `true`.
#[test]
fn writing_new_sensitive_content_to_a_previously_deleted_note_id_is_rejected_as_note_deleted() {
    let root = unique_test_dir();
    let (vault, pii_store) = open_vault_and_pii_store(root);
    let (known, pending) = empty();
    let gate = gate();
    let id = NoteId::parse("note-retired").unwrap();

    storage::propose_and_commit(&vault, &sensitive_note("note-retired", "original secret"), Version(0), Uuid::new_v4(), &known, &pending, &gate, Some(&pii_store)).unwrap();
    let v1 = vault.current_version(&id).unwrap();
    storage::propose_and_commit(&vault, &sensitive_tombstone("note-retired"), v1, Uuid::new_v4(), &known, &pending, &gate, Some(&pii_store)).unwrap();
    let tombstone_version = vault.current_version(&id).unwrap();
    assert!(
        matches!(vault.read_note(&id), Err(StorageError::Deleted { .. })),
        "read_note withholds a deleted sensitive note's pointer record (restore-promotion TOCTOU fix, raw API boundary)"
    );

    let result = storage::propose_and_commit(
        &vault,
        &sensitive_note("note-retired", "resurrected content"),
        tombstone_version,
        Uuid::new_v4(),
        &known,
        &pending,
        &gate,
        Some(&pii_store),
    );
    assert!(
        matches!(&result, Err(ProposeError::Storage(StorageError::NoteDeleted { note_id })) if note_id == "note-retired"),
        "a write of new content to a deleted note id must be refused with NoteDeleted, got {result:?}"
    );
    assert_eq!(vault.current_version(&id).unwrap(), tombstone_version, "the tombstone record is untouched");
    assert_eq!(pii_store.get("note-retired").unwrap(), b"", "the live ciphertext is still the empty tombstone");
}

/// I2: the same retirement holds across a reclassification. A deleted
/// sensitive id cannot come back as an `Internal` note either.
#[test]
fn writing_internal_content_to_a_previously_deleted_sensitive_note_id_is_rejected_too() {
    let root = unique_test_dir();
    let (vault, pii_store) = open_vault_and_pii_store(root);
    let (known, pending) = empty();
    let gate = gate();
    let id = NoteId::parse("note-retired-internal").unwrap();

    storage::propose_and_commit(&vault, &sensitive_note("note-retired-internal", "original secret"), Version(0), Uuid::new_v4(), &known, &pending, &gate, Some(&pii_store)).unwrap();
    let v1 = vault.current_version(&id).unwrap();
    storage::propose_and_commit(&vault, &sensitive_tombstone("note-retired-internal"), v1, Uuid::new_v4(), &known, &pending, &gate, Some(&pii_store)).unwrap();
    let tombstone_version = vault.current_version(&id).unwrap();
    assert!(
        matches!(vault.read_note(&id), Err(StorageError::Deleted { .. })),
        "read_note withholds a deleted sensitive note's pointer record (restore-promotion TOCTOU fix, raw API boundary)"
    );

    let internal = sensitive_note("note-retired-internal", "now internal").replace("classification: sensitive", "classification: internal");
    let result = storage::propose_and_commit(&vault, &internal, tombstone_version, Uuid::new_v4(), &known, &pending, &gate, Some(&pii_store));
    assert!(
        matches!(&result, Err(ProposeError::Storage(StorageError::NoteDeleted { .. }))),
        "got {result:?}"
    );
    assert_eq!(vault.current_version(&id).unwrap(), tombstone_version);
}

/// I2: re-issuing the tombstone for an already-deleted id is accepted (it
/// is how an operator finishes a deletion whose tombstone step failed) and
/// does not append a second deletion-log entry.
#[test]
#[cfg(windows)]
fn a_repeated_tombstone_write_to_an_already_deleted_note_is_accepted_without_a_second_log_entry() {
    use vault_security::TrustedReferenceStore;
    let root = unique_test_dir();
    let (vault, pii_store) = open_vault_and_pii_store(root);
    let (known, pending) = empty();
    let gate = gate();
    let id = NoteId::parse("note-twice").unwrap();

    storage::propose_and_commit(&vault, &sensitive_note("note-twice", "secret"), Version(0), Uuid::new_v4(), &known, &pending, &gate, Some(&pii_store)).unwrap();
    let v1 = vault.current_version(&id).unwrap();
    storage::propose_and_commit(&vault, &sensitive_tombstone("note-twice"), v1, Uuid::new_v4(), &known, &pending, &gate, Some(&pii_store)).unwrap();
    let v2 = vault.current_version(&id).unwrap();
    let result = storage::propose_and_commit(&vault, &sensitive_tombstone("note-twice"), v2, Uuid::new_v4(), &known, &pending, &gate, Some(&pii_store));
    assert!(matches!(result, Ok(CommitResult::Committed(_))), "got {result:?}");

    let reference = vault_security::DpapiTrustedReferenceStore::new(pii_store.deletion_log_reference_path()).read().unwrap().unwrap();
    assert_eq!(reference.generation, 1, "the second tombstone must not record a second deletion");
}

/// I2, read side: the consumer-facing read refuses a deleted note.
#[test]
fn get_unless_deleted_refuses_a_deleted_note_and_serves_a_live_one() {
    let root = unique_test_dir();
    let (vault, pii_store) = open_vault_and_pii_store(root);
    let (known, pending) = empty();
    let gate = gate();
    let id = NoteId::parse("note-read-del").unwrap();

    storage::propose_and_commit(&vault, &sensitive_note("note-read-del", "secret"), Version(0), Uuid::new_v4(), &known, &pending, &gate, Some(&pii_store)).unwrap();
    assert_eq!(pii_store.get_unless_deleted("note-read-del").unwrap(), b"secret");
    let v1 = vault.current_version(&id).unwrap();
    storage::propose_and_commit(&vault, &sensitive_tombstone("note-read-del"), v1, Uuid::new_v4(), &known, &pending, &gate, Some(&pii_store)).unwrap();
    assert!(
        matches!(pii_store.get_unless_deleted("note-read-del"), Err(vault_security::PiiStoreError::Deleted { .. })),
        "a deleted note must read as Deleted"
    );
}

/// Writes the deletion-log row that a crash right after C5 step 1 leaves
/// behind (generation `generation`, `Pending`, chain-hashed exactly as
/// `DeletionLog::record_deletion` does), without steps 2 and 3. Goes
/// straight at the SQLite file: `vault-security`'s own fault injector is
/// crate-private, and this is the on-disk state such a crash produces.
fn write_pending_deletion_row(db_path: &std::path::Path, generation: u64, prev_chain_hash: &str, note_id: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(prev_chain_hash.as_bytes());
    hasher.update(generation.to_string().as_bytes());
    hasher.update(note_id.as_bytes());
    let chain_hash: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    drop(vault_security::DeletionLog::open(db_path).unwrap()); // creates the schema
    let conn = rusqlite::Connection::open(db_path).unwrap();
    conn.execute(
        "INSERT INTO deletions (generation, note_id, prev_chain_hash, chain_hash, state) VALUES (?1, ?2, ?3, ?4, 'pending')",
        rusqlite::params![generation as i64, note_id, prev_chain_hash, chain_hash],
    )
    .unwrap();
    chain_hash
}

fn deletion_row_state(db_path: &std::path::Path, generation: u64) -> String {
    let conn = rusqlite::Connection::open(db_path).unwrap();
    conn.query_row("SELECT state FROM deletions WHERE generation = ?1", [generation as i64], |r| r.get(0)).unwrap()
}

/// Final whole-branch review, I3: `DeletionLog::reconcile` must run on the
/// production startup path. A crash between C5 step 1 (row written,
/// `Pending`) and step 2 (trusted reference updated) must be rolled forward
/// by the next `Vault::open_with_pii_store`, not left `Pending` forever.
#[test]
#[cfg(windows)]
fn open_with_pii_store_rolls_a_crashed_pending_deletion_forward() {
    use vault_security::TrustedReferenceStore;
    let root = unique_test_dir();
    let pii_store = open_pii_store(&root);
    let db_path = pii_store.deletion_log_path();
    let chain_hash = write_pending_deletion_row(&db_path, 1, &"0".repeat(64), "note-crashed");
    let reference = vault_security::DpapiTrustedReferenceStore::new(pii_store.deletion_log_reference_path());
    assert!(reference.read().unwrap().is_none(), "precondition: step 2 never ran");

    let _vault = Vault::open_with_pii_store(root, &unique_test_dir().join("journal.sqlite"), &pii_store)
        .expect("a single pending first deletion is the one roll-forward case and must not block opening");

    assert_eq!(deletion_row_state(&db_path, 1), "completed", "reconcile completed the pending row");
    let rolled = reference.read().unwrap().expect("reconcile wrote the trusted reference");
    assert_eq!((rolled.generation, rolled.chain_hash), (1, chain_hash));
    assert!(matches!(pii_store.get_unless_deleted("note-crashed"), Err(vault_security::PiiStoreError::Deleted { .. })));
}

/// I3: any deletion-log/reference disagreement reconcile cannot explain
/// fails `Vault::open_with_pii_store` closed, like the A2 journal's own
/// reconcile does (here: a trusted reference claiming a generation the log
/// has never reached).
#[test]
#[cfg(windows)]
fn open_with_pii_store_fails_closed_on_an_unexplained_deletion_log_mismatch() {
    use vault_security::TrustedReferenceStore;
    let root = unique_test_dir();
    let pii_store = open_pii_store(&root);
    vault_security::DpapiTrustedReferenceStore::new(pii_store.deletion_log_reference_path())
        .write(&vault_security::GenerationRef { generation: 5, chain_hash: "f".repeat(64) })
        .unwrap();
    let result = Vault::open_with_pii_store(root, &unique_test_dir().join("journal.sqlite"), &pii_store);
    assert!(matches!(result, Err(StorageError::DeletionLog(_))), "got {:?}", result.err());
}

#[test]
fn a_tombstone_write_with_a_stale_expected_version_is_rejected_and_never_recorded_in_the_deletion_log() {
    // A3 review round, IMPORTANT finding #1 regression test: the deletion
    // log write must happen AFTER the version check inside the same
    // journaled operation, never before it. Proven here by attempting a
    // tombstone write at version 0 against a note that has already moved
    // on to version 2 (one real update after the initial sensitive write) —
    // the write must be rejected as a Conflict, and the note's actual
    // content must remain fully live and unaffected: no deletion-log entry,
    // no pointer change, no removed ciphertext.
    let root = unique_test_dir();
    let (vault, pii_store) = open_vault_and_pii_store(root);
    let (known, pending) = empty();
    let gate = gate();

    let note_text_v1 = sensitive_note("note-stale-tombstone", "v1 secret content");
    storage::propose_and_commit(&vault, &note_text_v1, Version(0), Uuid::new_v4(), &known, &pending, &gate, Some(&pii_store)).unwrap();
    let id = NoteId::parse("note-stale-tombstone").unwrap();

    let note_text_v2 = sensitive_note("note-stale-tombstone", "v2 secret content");
    let current = vault.current_version(&id).unwrap();
    storage::propose_and_commit(&vault, &note_text_v2, current, Uuid::new_v4(), &known, &pending, &gate, Some(&pii_store)).unwrap();
    let v2_record = vault.read_note(&id).unwrap();
    assert_eq!(v2_record.version, Version(2));

    // Attempt a stale tombstone at version 0 (the note's ORIGINAL version,
    // long since superseded).
    let stale_tombstone = sensitive_tombstone("note-stale-tombstone");
    let result = storage::propose_and_commit(&vault, &stale_tombstone, Version(0), Uuid::new_v4(), &known, &pending, &gate, Some(&pii_store));
    assert!(result.is_err(), "a stale-version tombstone must be rejected, got {result:?}");

    // The note must be completely unaffected: still at v2, pointer and
    // ciphertext both intact.
    let after = vault.read_note(&id).unwrap();
    assert_eq!(after, v2_record, "the note must be entirely unaffected by the rejected tombstone attempt");
    assert_eq!(pii_store.get("note-stale-tombstone").unwrap(), b"v2 secret content");

    // Critically: the deletion log must NOT have recorded this note as
    // deleted. Before the fix, DeletionLog::record_deletion ran BEFORE the
    // version check, so this exact call would have marked the note deleted
    // in the log while its real content stayed fully live.
    let deletion_log_path = pii_store.root().join("deletion-log.sqlite");
    let deletion_log = vault_security::DeletionLog::open(&deletion_log_path).unwrap();
    assert!(
        !deletion_log.is_deleted("note-stale-tombstone").unwrap(),
        "a rejected (stale-version) tombstone must NEVER be recorded in the deletion log"
    );
}
