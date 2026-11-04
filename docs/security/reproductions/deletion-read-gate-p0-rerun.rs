// P0 re-run of docs/security/reproductions/deletion-read-gate.rs against the
// fixed fail-closed DeletionLog lifecycle (commit e92dc78 and follow-ups).
// The original probe no longer even COMPILES: DeletionLog::open (the
// unauthenticated open-or-create used to construct the vulnerable scenario)
// was made pub(crate)/#[cfg(test)] by the fix, so it is not part of the
// public API any external caller (like this probe) can reach. This rerun
// uses the same scenario shape through the ACTUAL public entry points
// (PiiStore::bootstrap_deletion_log / open_deletion_log) to show the
// equivalent attack now fails closed rather than silently succeeding.
use vault_security::PiiStore;
fn main() {
    let root = std::env::current_dir()
        .unwrap()
        .join(".superpowers")
        .join(format!(
            "p0-rerun-missing-log-{}",
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

    // The P0 lifecycle requires an EXPLICIT bootstrap; there is no implicit
    // "open(create)" left for any consumer to call.
    let log = store.bootstrap_deletion_log().unwrap();
    let reference = vault_security::DpapiTrustedReferenceStore::new(store.deletion_log_reference_path());
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
    // ORIGINAL FINDING: this used to return Ok(plaintext) because
    // get_unless_deleted's own log-open fell back to create-on-open, and an
    // empty freshly-created log reads as "nothing ever deleted." Now:
    // get_unless_deleted must open_existing (no implicit create) and fail
    // closed when the DB file is genuinely missing.
    let result = store.get_unless_deleted("note-a");
    println!(
        "after_missing_log_result={result:?}"
    );
    println!(
        "after_missing_log_returns_plaintext={}",
        result
            .as_ref()
            .is_ok_and(|bytes| bytes == b"SYNTHETIC-CONTENT")
    );
    // Original probe's next step moved the live (now DeletionLog::open
    // implicitly-recreated empty) log aside and restored the REAL saved
    // log. Under the P0 fix there is nothing to move aside: open_existing
    // never created anything at the live path, confirmed by its absence.
    assert!(!store.deletion_log_path().is_file(), "P0: open_existing must never implicitly recreate the missing DB file");
    println!("no_implicit_recreation_confirmed=true");
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
    let other_result = store.get_unless_deleted("note-b");
    println!("missing_reference_other_note_result={other_result:?}");
    println!(
        "missing_reference_other_note_returns_plaintext={}",
        other_result.is_ok()
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
    // DeletionLog::open is no longer reachable from outside the crate; use
    // open_deletion_log (which performs the same verified open_existing the
    // real consumer path uses) instead.
    let reopen_result = store.open_deletion_log();
    println!("tampered_chain_reopen_err={:?}", reopen_result.as_ref().err());
    println!("tampered_chain_detectable_at_reopen={}", reopen_result.is_err());
    let tampered_result = store.get_unless_deleted("note-a");
    println!("tampered_chain_deleted_note_result={tampered_result:?}");
    println!(
        "tampered_chain_deleted_note_returns_plaintext={}",
        tampered_result.is_ok()
    );
}
