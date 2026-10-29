//! Final whole-branch review fix wave: end-to-end backup/restore through the
//! PUBLIC surface only (`propose_and_commit` for every write, including the
//! tombstone deletion, `vault_security::backup`/`restore` for the backup
//! cycle), against the REAL DPAPI-authenticated trusted reference the
//! sensitive-write pipeline itself uses.
//!
//! - C1: a vault that has ever deleted a sensitive note must still be able
//!   to back up and restore; the deleted note is excluded and reported,
//!   never exposed and never a reason to reject the whole restore.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use uuid::Uuid;

use storage::{CommitResult, NoteId, Vault, Version};
use vault_security::{DeletionLog, DpapiTrustedReferenceStore, FilesystemBackupTarget, MasterKey, PiiStore};

fn unique_test_dir() -> PathBuf {
    static UNIQUE_DIR_COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "superbrain-a3-backup-test-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
        UNIQUE_DIR_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn init_git_repo(root: &Path) {
    git2::Repository::init(root).unwrap();
    let repo = git2::Repository::open(root).unwrap();
    let signature = git2::Signature::now("SuperBrain Test", "test@localhost").unwrap();
    let tree_oid = repo.index().unwrap().write_tree().unwrap();
    let tree = repo.find_tree(tree_oid).unwrap();
    repo.commit(Some("HEAD"), &signature, &signature, "initial empty commit", &tree, &[]).unwrap();
}

fn pii_key_path(vault_root: &Path) -> PathBuf {
    vault_root.join(".pii-store").join("master.key")
}

fn open_pii_store(vault_root: &Path) -> PiiStore {
    PiiStore::open(vault_root.join(".pii-store"), &pii_key_path(vault_root)).unwrap()
}

fn yaml(id: &str, classification: &str) -> String {
    format!(
        "id: {id}\nschema_version: 1\ntype: bug\nstatus: draft\ntrust: internal\nclassification: {classification}\nproject: project-x\nsource: agent:coder@prompt-v3\ncreated: 2026-10-23\nupdated: 2026-10-23\nevent_period: 2026-10-23\nrelations:\n  supports: []\n  contradicts: []\n  derived_from: []\n  supersedes: []\n  applies_to: []\n"
    )
}

fn note(id: &str, classification: &str, body: &str) -> String {
    format!("---\n{}---\n{body}", yaml(id, classification))
}

fn gate() -> policy::Gate {
    policy::Gate::new(policy::SecretPolicy::new(1))
}

fn write(vault: &Vault, pii_store: &PiiStore, text: &str, expected: Version) -> CommitResult {
    storage::propose_and_commit(vault, text, expected, Uuid::new_v4(), &HashSet::new(), &HashSet::new(), &gate(), Some(pii_store))
        .expect("write through propose_and_commit must succeed")
}

fn live_log_and_ref(pii_store: &PiiStore) -> (DeletionLog, DpapiTrustedReferenceStore) {
    let log = DeletionLog::open(&pii_store.root().join("deletion-log.sqlite")).unwrap();
    let reference = DpapiTrustedReferenceStore::new(pii_store.root().join("deletion-log-reference.dpapi"));
    (log, reference)
}

fn all_files_under(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out
}

/// CRITICAL C1, the reviewer's scenario end to end: write a sensitive note,
/// delete it through the full tombstone path, back up AFTER the deletion,
/// write a second, different sensitive note, back up AGAIN, and restore from
/// EITHER backup. Both restores must succeed (never `DeletedSinceBackup` for
/// the whole restore) and the deleted note's content must never be exposed.
#[test]
#[cfg(windows)]
fn a_vault_that_has_deleted_a_sensitive_note_can_still_back_up_and_restore_from_either_later_backup() {
    let root = unique_test_dir();
    init_git_repo(&root);
    let pii_store = open_pii_store(&root);
    let vault = Vault::open_with_pii_store(root.clone(), &unique_test_dir().join("journal.sqlite"), &pii_store).unwrap();

    const DELETED_SECRET: &str = "DELETED-SECRET-CONTENT-MUST-NEVER-REAPPEAR";
    write(&vault, &pii_store, &note("note-del", "sensitive", DELETED_SECRET), Version(0));
    write(&vault, &pii_store, &note("note-plain", "internal", "plain body\n"), Version(0));
    let del_id = NoteId::parse("note-del").unwrap();
    let current = vault.current_version(&del_id).unwrap();
    write(&vault, &pii_store, &note("note-del", "sensitive", storage::SENSITIVE_TOMBSTONE_BODY), current);

    let (log, reference) = live_log_and_ref(&pii_store);
    assert!(log.is_deleted("note-del").unwrap(), "precondition: the tombstone write recorded the deletion");

    let master_key = MasterKey::load_or_create(&unique_test_dir().join("backup-master.key"), None).unwrap();
    let target_1 = FilesystemBackupTarget::new(unique_test_dir().join("target-1"));
    vault_security::backup(&root, &pii_store, &log, &master_key, &target_1).expect("backup after the deletion");

    write(&vault, &pii_store, &note("note-keep", "sensitive", "a second, different sensitive note"), Version(0));
    let target_2 = FilesystemBackupTarget::new(unique_test_dir().join("target-2"));
    vault_security::backup(&root, &pii_store, &log, &master_key, &target_2).expect("second backup");

    for (target, expected_restored) in [
        (&target_1, vec!["note-plain".to_string()]),
        (&target_2, vec!["note-keep".to_string(), "note-plain".to_string()]),
    ] {
        let destination = unique_test_dir().join("restored");
        let restored_pii_store = PiiStore::open(unique_test_dir().join("restored-pii"), &pii_key_path(&root)).unwrap();
        let outcome = vault_security::restore(target, &master_key, &destination, &log, &reference, &restored_pii_store)
            .expect("a backup taken after the deletion must restore successfully, not be rejected wholesale");

        let mut restored = outcome.restored_note_ids.clone();
        restored.sort();
        assert_eq!(restored, expected_restored);
        assert_eq!(outcome.excluded_deleted_note_ids, vec!["note-del".to_string()], "the deleted note is reported as excluded");

        // The deleted note is not exposed anywhere in the restored output.
        assert!(!destination.join("note-del.record").exists(), "no record for the deleted note");
        assert!(
            matches!(restored_pii_store.get("note-del"), Err(vault_security::PiiStoreError::NotFound { .. })),
            "no ciphertext for the deleted note"
        );
        for path in all_files_under(&destination).into_iter().chain(all_files_under(restored_pii_store.root())) {
            assert!(!path.to_string_lossy().contains("note-del"), "no restored file is named after the deleted note: {path:?}");
            let bytes = std::fs::read(&path).unwrap();
            assert!(
                !bytes.windows(DELETED_SECRET.len()).any(|w| w == DELETED_SECRET.as_bytes()),
                "the deleted plaintext must not appear in any restored file: {path:?}"
            );
        }
        if expected_restored.contains(&"note-keep".to_string()) {
            assert_eq!(restored_pii_store.get("note-keep").unwrap(), b"a second, different sensitive note");
        }
    }
}

/// Final whole-branch review, I1: a restored vault must be usable through
/// the normal storage API, not just byte-identical on disk. Restores a plain
/// and a sensitive note (plus an update, so versions above 1 are covered),
/// then drops the returned `Vault` and reopens the restored root with the
/// ordinary `Vault::open_with_pii_store` and the restored journal, and reads
/// both notes back through `read_note` (and the sensitive one's content
/// through `get_unless_deleted`).
#[test]
#[cfg(windows)]
fn a_restored_vault_is_readable_through_vault_open_and_read_note_for_plain_and_sensitive_notes() {
    let root = unique_test_dir();
    init_git_repo(&root);
    let pii_store = open_pii_store(&root);
    let vault = Vault::open_with_pii_store(root.clone(), &unique_test_dir().join("journal.sqlite"), &pii_store).unwrap();
    let plain_text = note("note-plain", "internal", "plain body v1\n");
    write(&vault, &pii_store, &plain_text, Version(0));
    let plain_v2 = note("note-plain", "internal", "plain body v2\n");
    write(&vault, &pii_store, &plain_v2, Version(1));
    write(&vault, &pii_store, &note("note-secret", "sensitive", "restored sensitive content"), Version(0));
    let plain_id = NoteId::parse("note-plain").unwrap();
    let secret_id = NoteId::parse("note-secret").unwrap();
    let original_plain = vault.read_note(&plain_id).unwrap();
    let original_secret = vault.read_note(&secret_id).unwrap();

    let (log, reference) = live_log_and_ref(&pii_store);
    let master_key = MasterKey::load_or_create(&unique_test_dir().join("backup-master.key"), None).unwrap();
    let target = FilesystemBackupTarget::new(unique_test_dir().join("target"));
    vault_security::backup(&root, &pii_store, &log, &master_key, &target).unwrap();

    let destination = unique_test_dir().join("restored");
    let journal_path = unique_test_dir().join("restored-journal.sqlite");
    let restored_pii_store = PiiStore::open(unique_test_dir().join("restored-pii"), &pii_key_path(&root)).unwrap();
    let (restored_vault, outcome) =
        storage::restore_vault_from_backup(&target, &master_key, &destination, &journal_path, &log, &reference, &restored_pii_store)
            .expect("restore into a usable vault");
    let mut ids = outcome.restored_note_ids.clone();
    ids.sort();
    assert_eq!(ids, vec!["note-plain".to_string(), "note-secret".to_string()]);
    assert_eq!(restored_vault.read_note(&plain_id).unwrap(), original_plain);
    drop(restored_vault);

    // The ordinary constructor, on the restored root and restored journal.
    let reopened = Vault::open_with_pii_store(destination.clone(), &journal_path, &restored_pii_store).expect("reopen the restored vault");
    let plain = reopened.read_note(&plain_id).expect("plain note readable after restore");
    assert_eq!(plain, original_plain);
    assert_eq!(plain.version, Version(2));
    assert_eq!(plain.content, plain_v2);
    let secret = reopened.read_note(&secret_id).expect("sensitive note's pointer record readable after restore");
    assert_eq!(secret, original_secret);
    assert!(secret.content.ends_with("pii://note-secret"));
    assert_eq!(restored_pii_store.get_unless_deleted("note-secret").unwrap(), b"restored sensitive content");

    // And it is a working vault: an ordinary update on top of the restore.
    let plain_v3 = note("note-plain", "internal", "plain body v3\n");
    write(&reopened, &restored_pii_store, &plain_v3, Version(2));
    assert_eq!(reopened.read_note(&plain_id).unwrap().content, plain_v3);
}

/// I1: restore only bootstraps a brand-new vault; it never seeds rows into
/// an existing journal.
#[test]
fn restore_vault_from_backup_refuses_an_existing_journal_path() {
    let root = unique_test_dir();
    let pii_store = open_pii_store(&root);
    let log = DeletionLog::open(&pii_store.deletion_log_path()).unwrap();
    let reference = DpapiTrustedReferenceStore::new(pii_store.deletion_log_reference_path());
    let master_key = MasterKey::load_or_create(&unique_test_dir().join("backup-master.key"), None).unwrap();
    let target = FilesystemBackupTarget::new(unique_test_dir().join("target"));
    let journal_path = unique_test_dir().join("existing-journal.sqlite");
    std::fs::write(&journal_path, b"").unwrap();
    let destination = unique_test_dir().join("restored");
    let result = storage::restore_vault_from_backup(&target, &master_key, &destination, &journal_path, &log, &reference, &pii_store);
    assert!(matches!(result, Err(storage::RestoreError::JournalAlreadyExists)), "got {:?}", result.err());
    assert!(!destination.exists(), "nothing restored");
}
