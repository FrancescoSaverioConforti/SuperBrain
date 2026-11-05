use vault_security::{DeletionLog, DpapiTrustedReferenceStore, PiiStore};
fn main() {
    let root = std::env::current_dir()
        .unwrap()
        .join(".superpowers")
        .join(format!(
            "security-review-missing-log-{}",
            std::process::id()
        ));
    std::fs::create_dir_all(&root).unwrap();
    let store = PiiStore::open(root.join("pii"), &root.join("master.key")).unwrap();
    let op = uuid::Uuid::nil();
    store.stage("note-a", b"SYNTHETIC-CONTENT", op).unwrap();
    store.commit_staged("note-a", op).unwrap();
    store
        .stage("note-b", b"OTHER-SYNTHETIC-CONTENT", op)
        .unwrap();
    store.commit_staged("note-b", op).unwrap();
    let log = DeletionLog::open(&store.deletion_log_path()).unwrap();
    let reference = DpapiTrustedReferenceStore::new(store.deletion_log_reference_path());
    log.record_deletion("note-a", &reference).unwrap();
    println!(
        "before_missing_log_rejected={}",
        store.get_unless_deleted("note-a").is_err()
    );
    drop(log);
    std::fs::rename(
        store.deletion_log_path(),
        root.join("deletion-log.saved.sqlite"),
    )
    .unwrap();
    println!(
        "authentic_reference_still_exists={}",
        store.deletion_log_reference_path().is_file()
    );
    let result = store.get_unless_deleted("note-a");
    println!(
        "after_missing_log_returns_plaintext={}",
        result
            .as_ref()
            .is_ok_and(|bytes| bytes == b"SYNTHETIC-CONTENT")
    );
    std::fs::rename(
        store.deletion_log_path(),
        root.join("empty-log.saved.sqlite"),
    )
    .unwrap();
    std::fs::rename(
        root.join("deletion-log.saved.sqlite"),
        store.deletion_log_path(),
    )
    .unwrap();
    std::fs::rename(
        store.deletion_log_reference_path(),
        root.join("trusted-reference.saved.dpapi"),
    )
    .unwrap();
    println!(
        "missing_reference_other_note_returns_plaintext={}",
        store.get_unless_deleted("note-b").is_ok()
    );
    std::fs::rename(
        root.join("trusted-reference.saved.dpapi"),
        store.deletion_log_reference_path(),
    )
    .unwrap();
    let connection = rusqlite::Connection::open(store.deletion_log_path()).unwrap();
    connection
        .execute(
            "UPDATE deletions SET note_id = 'note-other' WHERE note_id = 'note-a'",
            [],
        )
        .unwrap();
    drop(connection);
    let log = DeletionLog::open(&store.deletion_log_path()).unwrap();
    println!("tampered_chain_detectable={}", log.verify_chain().is_err());
    drop(log);
    println!(
        "tampered_chain_deleted_note_returns_plaintext={}",
        store.get_unless_deleted("note-a").is_ok()
    );
}
