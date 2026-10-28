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

use storage::{CommitOutcome, CommitResult, NoteId, Vault, Version};
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

    let outcome = storage::propose_and_commit(&vault, &note_text, Version(0), Uuid::new_v4(), &known, &pending, &gate, &pii_store)
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
    storage::propose_and_commit(&vault, &note_text, Version(0), Uuid::new_v4(), &known, &pending, &gate, &pii_store).unwrap();
    let id = NoteId::parse("note-plain").unwrap();
    let record = vault.read_note(&id).unwrap();
    assert_eq!(record.content, note_text, "non-sensitive content is stored byte-for-byte, unchanged");
    assert!(!pii_store.has_live("note-plain"), "the PiiStore must never be touched for a non-sensitive note");
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
    storage::propose_and_commit(&vault, &note_text, Version(0), Uuid::new_v4(), &known, &pending, &gate, &pii_store).unwrap();
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
    storage::propose_and_commit(&vault, &note_text, Version(0), Uuid::new_v4(), &known, &pending, &gate, &pii_store).unwrap();
    let id = NoteId::parse("note-proper-delete").unwrap();
    let current = vault.current_version(&id).unwrap();

    let tombstone_text = sensitive_tombstone("note-proper-delete");
    let result = storage::propose_and_commit(&vault, &tombstone_text, current, Uuid::new_v4(), &known, &pending, &gate, &pii_store)
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
