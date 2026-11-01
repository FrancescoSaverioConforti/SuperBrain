//! Deterministic interleavings: barriers/hooks only, no sleeps or timing races.
use super::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Barrier,
};
use storage::{CommitResult, Version};
use uuid::Uuid;

struct Shared(Arc<RealIndexer>);
impl Indexer for Shared {
    fn mark_current(&self, id: &str, version: u64, bytes: &[u8]) {
        self.0.mark_current(id, version, bytes);
    }
    fn state_for(&self, id: &str) -> IndexState {
        self.0.state_for(id)
    }
}
struct Fixture {
    vault: Arc<Vault>,
    indexer: Arc<RealIndexer>,
    pii: Arc<vault_security::PiiStore>,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("superbrain-b1-atomic-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        {
            let repo = git2::Repository::init(&root).unwrap();
            let signature = git2::Signature::now("Test", "test@localhost").unwrap();
            let oid = repo.index().unwrap().write_tree().unwrap();
            let tree = repo.find_tree(oid).unwrap();
            repo.commit(Some("HEAD"), &signature, &signature, "initial", &tree, &[])
                .unwrap();
        }
        let pii_root = root.join(".pii-store");
        let pii = Arc::new(
            vault_security::PiiStore::open(pii_root.clone(), &pii_root.join("master.key")).unwrap(),
        );
        let indexer = Arc::new(RealIndexer::new());
        let vault = Arc::new(
            Vault::open_with_pii_store_and_indexer(
                root.clone(),
                &root.join("journal.sqlite"),
                &pii,
                Box::new(Shared(indexer.clone())),
            )
            .unwrap(),
        );
        Self {
            vault,
            indexer,
            pii,
        }
    }
    fn write(&self, version: u64, class: &str, body: &str) -> CommitResult {
        commit(&self.vault, &self.pii, version, class, body)
    }
    fn query(&self, term: &str) -> crate::search::SearchResponse {
        crate::Index::new(
            &self.vault,
            &self.indexer,
            "project-x",
            "2026-10-30".parse().unwrap(),
        )
        .search(term, Default::default(), 10)
        .unwrap()
    }
}
fn text(class: &str, body: &str) -> String {
    format!("---\nid: note-race\nschema_version: 1\ntype: bug\nstatus: draft\ntrust: internal\nclassification: {class}\nproject: project-x\nsource: sourcecanary\ncreated: 2026-10-30\nupdated: 2026-10-30\nevent_period: 2026-10-30\nrelations: {{}}\n---\n{body}")
}
fn commit(
    vault: &Vault,
    pii: &vault_security::PiiStore,
    expected: u64,
    class: &str,
    body: &str,
) -> CommitResult {
    storage::propose_and_commit(
        vault,
        &text(class, body),
        Version(expected),
        Uuid::new_v4(),
        &HashSet::new(),
        &HashSet::new(),
        &policy::Gate::new(policy::SecretPolicy::new(1)),
        Some(pii),
    )
    .unwrap()
}
struct Reached(std::sync::mpsc::Receiver<()>);
impl Reached {
    fn wait(&self) {
        // Watchdog only: never determines the interleaving. A mutant removing
        // the callback must fail promptly rather than hang on an arrival barrier.
        self.0
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("index test hook was not reached");
    }
}

fn pause(indexer: &RealIndexer, point: TestPoint, version: u64) -> (Reached, Arc<Barrier>) {
    let (arrived, reached) = std::sync::mpsc::channel();
    let resume = Arc::new(Barrier::new(2));
    let r = resume.clone();
    let fired = AtomicBool::new(false);
    *indexer.hook.lock().unwrap() = Some(Arc::new(move |p, id, v| {
        if p == point && id == "note-race" && v == version && !fired.swap(true, Ordering::SeqCst) {
            let _ = arrived.send(());
            r.wait();
        }
    }));
    (Reached(reached), resume)
}
fn rebuild(f: &Fixture) -> std::thread::JoinHandle<()> {
    let vault = f.vault.clone();
    let indexer = f.indexer.clone();
    std::thread::spawn(move || rebuild_index(&vault, &indexer).unwrap())
}
fn both(indexer: &RealIndexer, term: &str, expected: usize) {
    assert_eq!(indexer.trigram_hits(term).len(), expected, "trigram {term}");
    assert_eq!(indexer.bm25_ranked(term, 10).len(), expected, "BM25 {term}");
}

#[test]
fn rebuild_snapshot_v1_cannot_replace_committed_live_v2() {
    let f = Fixture::new();
    f.write(0, "internal", "oldonlyneedle");
    let (arrived, resume) = pause(&f.indexer, TestPoint::RebuildRead, 1);
    let worker = rebuild(&f);
    arrived.wait();
    let committed = f.write(1, "internal", "newonlyneedle");
    resume.wait();
    worker.join().unwrap();
    assert!(matches!(committed, CommitResult::Committed(_)));
    both(&f.indexer, "newonlyneedle", 1);
    both(&f.indexer, "oldonlyneedle", 0);
    assert_eq!(f.indexer.represented_version("note-race"), Some(2));
    assert_eq!(f.indexer.state_for("note-race"), IndexState::Current);
    assert!(matches!(
        f.query("newonlyneedle").outcome,
        crate::search::SearchOutcome::Matches { .. }
    ));
}

#[test]
fn sensitive_temporal_markers_cannot_return_from_late_internal_rebuild() {
    let f = Fixture::new();
    let marker = "TEMPORAL-REBUILD-CANARY";
    let write = |expected, class| {
        storage::propose_and_commit(
            &f.vault,
            &text(class, "private-body").replace("2026-10-30", marker),
            Version(expected),
            Uuid::new_v4(),
            &HashSet::new(),
            &HashSet::new(),
            &policy::Gate::new(policy::SecretPolicy::new(1)),
            Some(&f.pii),
        )
        .unwrap()
    };
    assert!(matches!(write(0, "internal"), CommitResult::Committed(_)));
    both(&f.indexer, marker, 1);
    let (arrived, resume) = pause(&f.indexer, TestPoint::RebuildRead, 1);
    let worker = rebuild(&f);
    arrived.wait();
    let committed = write(1, "sensitive");
    resume.wait();
    worker.join().unwrap();
    assert!(matches!(committed, CommitResult::Committed(_)));
    both(&f.indexer, marker, 0);
    assert_eq!(f.indexer.represented_version("note-race"), Some(2));
    assert_eq!(f.indexer.state_for("note-race"), IndexState::Current);
    rebuild_index(&f.vault, &f.indexer).unwrap();
    both(&f.indexer, marker, 0);
    assert_eq!(
        f.query(marker).outcome,
        crate::search::SearchOutcome::NotPresent
    );
}

#[test]
fn observed_live_v2_stays_stale_until_atomic_publication() {
    let f = Fixture::new();
    f.write(0, "internal", "oldonlyneedle");
    let (arrived, resume) = pause(&f.indexer, TestPoint::AfterObserved, 2);
    let v = f.vault.clone();
    let pii = f.pii.clone();
    let writer = std::thread::spawn(move || commit(&v, &pii, 1, "internal", "newonlyneedle"));
    arrived.wait();
    let during = f.indexer.state_for("note-race");
    let represented = f.indexer.represented_version("note-race");
    resume.wait();
    let committed = writer.join().unwrap();
    assert_eq!(during, IndexState::Stale);
    assert_eq!(represented, Some(1));
    assert!(matches!(committed, CommitResult::Committed(_)));
    assert_eq!(f.indexer.state_for("note-race"), IndexState::Current);
    both(&f.indexer, "newonlyneedle", 1);
}

#[test]
fn failed_live_v2_and_late_rebuild_v1_remain_stale_until_v2_rebuild() {
    let f = Fixture::new();
    f.write(0, "internal", "oldonlyneedle");
    let (arrived, resume) = pause(&f.indexer, TestPoint::RebuildRead, 1);
    let worker = rebuild(&f);
    arrived.wait();
    f.indexer.poison_next_mark_current("note-race");
    let committed = f.write(1, "internal", "newonlyneedle");
    resume.wait();
    worker.join().unwrap();
    assert!(
        matches!(committed, CommitResult::Committed(_)),
        "index failure must not fail commit"
    );
    assert_eq!(f.indexer.state_for("note-race"), IndexState::Stale);
    assert_eq!(f.indexer.represented_version("note-race"), Some(1));
    assert_eq!(
        f.query("oldonlyneedle").outcome,
        crate::search::SearchOutcome::NotPresent
    );
    rebuild_index(&f.vault, &f.indexer).unwrap();
    both(&f.indexer, "newonlyneedle", 1);
    both(&f.indexer, "oldonlyneedle", 0);
    assert_eq!(f.indexer.state_for("note-race"), IndexState::Current);
    assert_eq!(f.indexer.represented_version("note-race"), Some(2));
}

#[test]
fn deletion_during_rebuild_does_not_restore_old_content_or_bypass_gate() {
    let f = Fixture::new();
    f.write(0, "internal", "oldonlyneedle");
    let (arrived, resume) = pause(&f.indexer, TestPoint::RebuildRead, 1);
    let worker = rebuild(&f);
    arrived.wait();
    let committed = f.write(1, "sensitive", storage::SENSITIVE_TOMBSTONE_BODY);
    resume.wait();
    worker.join().unwrap();
    assert!(matches!(committed, CommitResult::Committed(_)));
    both(&f.indexer, "oldonlyneedle", 0);
    assert!(matches!(
        f.vault.read_note(&NoteId::parse("note-race").unwrap()),
        Err(storage::StorageError::Deleted { .. } | storage::StorageError::NoteDeleted { .. })
    ));
    assert_eq!(
        f.query("note-race").outcome,
        crate::search::SearchOutcome::NotPresent
    );
    assert_eq!(f.indexer.represented_version("note-race"), Some(2));
}

#[test]
fn reclassification_during_rebuild_cannot_republish_internal_text() {
    let f = Fixture::new();
    f.write(0, "internal", "bodycanary");
    let (arrived, resume) = pause(&f.indexer, TestPoint::RebuildRead, 1);
    let worker = rebuild(&f);
    arrived.wait();
    f.write(1, "sensitive", "bodycanary");
    resume.wait();
    worker.join().unwrap();
    both(&f.indexer, "bodycanary", 0);
    both(&f.indexer, "sourcecanary", 0);
    let response = f.query("note-race");
    assert!(!format!("{response:?}").contains("canary"));
    assert_eq!(f.indexer.represented_version("note-race"), Some(2));
}

#[test]
fn readers_cannot_observe_half_published_postings_or_current_state() {
    let f = Fixture::new();
    f.write(0, "internal", "oldonlyneedle");
    let (arrived, resume) = pause(&f.indexer, TestPoint::AfterTrigram, 2);
    let v = f.vault.clone();
    let pii = f.pii.clone();
    let writer = std::thread::spawn(move || commit(&v, &pii, 1, "internal", "newonlyneedle"));
    arrived.wait();
    let locked = f.indexer.publication_is_locked();
    resume.wait();
    let committed = writer.join().unwrap();
    assert!(
        locked,
        "all index observers must share the publication lock"
    );
    assert!(matches!(committed, CommitResult::Committed(_)));
    both(&f.indexer, "newonlyneedle", 1);
    both(&f.indexer, "oldonlyneedle", 0);
    assert_eq!(f.indexer.represented_version("note-race"), Some(2));
}

#[test]
fn failure_between_postings_rolls_back_atomically_and_commit_succeeds() {
    let f = Fixture::new();
    f.write(0, "internal", "oldonlyneedle");
    *f.indexer.hook.lock().unwrap() = Some(Arc::new(|point, _, version| {
        if point == TestPoint::AfterTrigram && version == 2 {
            panic!("injected index publication fault");
        }
    }));
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        f.write(1, "internal", "newonlyneedle")
    }));
    assert!(matches!(outcome, Ok(CommitResult::Committed(_))));
    assert_eq!(
        f.vault
            .current_version(&NoteId::parse("note-race").unwrap())
            .unwrap(),
        Version(2)
    );
    both(&f.indexer, "oldonlyneedle", 1);
    both(&f.indexer, "newonlyneedle", 0);
    assert_eq!(f.indexer.state_for("note-race"), IndexState::Stale);
    assert_eq!(f.indexer.represented_version("note-race"), Some(1));
    *f.indexer.hook.lock().unwrap() = None;
    rebuild_index(&f.vault, &f.indexer).unwrap();
    assert_eq!(f.indexer.state_for("note-race"), IndexState::Current);
    both(&f.indexer, "newonlyneedle", 1);
}

#[test]
fn old_or_malformed_delivery_never_decreases_high_water_version() {
    let f = Fixture::new();
    f.write(0, "internal", "oldonlyneedle");
    f.write(1, "internal", "newonlyneedle");
    f.indexer.mark_current(
        "note-race",
        1,
        format!("1\n---\n{}", text("internal", "oldonlyneedle")).as_bytes(),
    );
    f.indexer
        .mark_current("note-race", 1, b"invalid old payload");
    both(&f.indexer, "newonlyneedle", 1);
    both(&f.indexer, "oldonlyneedle", 0);
    assert_eq!(f.indexer.state_for("note-race"), IndexState::Current);
    assert_eq!(f.indexer.represented_version("note-race"), Some(2));
    f.indexer
        .mark_current("note-race", 3, b"malformed newer payload");
    f.indexer.mark_current(
        "note-race",
        2,
        format!("2\n---\n{}", text("internal", "newonlyneedle")).as_bytes(),
    );
    assert_eq!(f.indexer.state_for("note-race"), IndexState::Stale);
    assert_eq!(f.indexer.represented_version("note-race"), Some(2));
}

#[test]
fn metadata_and_version_publish_together_and_old_rebuild_cannot_restore_them() {
    let f = Fixture::new();
    f.write(0, "internal", "oldonlyneedle");
    let (arrived, resume) = pause(&f.indexer, TestPoint::RebuildRead, 1);
    let worker = rebuild(&f);
    arrived.wait();
    let updated = text("sensitive", "privatecanary")
        .replace("project: project-x", "project: project-y")
        .replace("status: draft", "status: reviewed");
    let committed = storage::propose_and_commit(
        &f.vault,
        &updated,
        Version(1),
        Uuid::new_v4(),
        &HashSet::new(),
        &HashSet::new(),
        &policy::Gate::new(policy::SecretPolicy::new(1)),
        Some(&f.pii),
    );
    resume.wait();
    worker.join().unwrap();
    assert!(matches!(committed, Ok(CommitResult::Committed(_))));
    let data = f.indexer.data();
    let note = &data.notes["note-race"];
    let published = note.published.as_ref().unwrap();
    assert_eq!((note.expected_version, published.version), (2, 2));
    assert_eq!(published.document.project, "project-y");
    assert_eq!(published.document.status, domain::NoteStatus::Reviewed);
    assert_eq!(
        published.document.classification,
        domain::Classification::Sensitive
    );
    assert!(!published.document.text_searchable.contains("canary"));
    assert!(data.trigram.query_exact("sourcecanary").is_empty());
    assert!(data.bm25.query_ranked("sourcecanary", 10).is_empty());
}

#[test]
fn contradictory_record_version_or_id_cannot_be_published_and_same_version_retry_repairs() {
    let f = Fixture::new();
    f.write(0, "internal", "oldonlyneedle");
    f.indexer.poison_next_mark_current("note-race");
    f.write(1, "internal", "newonlyneedle");
    let record = f
        .vault
        .read_note(&NoteId::parse("note-race").unwrap())
        .unwrap();
    let wrong_header = format!("1\n---\n{}", record.content);
    let wrong_id = format!(
        "2\n---\n{}",
        record
            .content
            .replace("id: note-race", "id: different-note")
    );
    for bytes in [wrong_header.as_bytes(), wrong_id.as_bytes()] {
        f.indexer.mark_current("note-race", 2, bytes);
        assert_eq!(f.indexer.state_for("note-race"), IndexState::Stale);
        assert_eq!(f.indexer.represented_version("note-race"), Some(1));
        both(&f.indexer, "newonlyneedle", 0);
        both(&f.indexer, "oldonlyneedle", 1);
    }
    let correct = format!("2\n---\n{}", record.content);
    f.indexer.mark_current("note-race", 2, correct.as_bytes());
    assert_eq!(f.indexer.state_for("note-race"), IndexState::Current);
    assert_eq!(f.indexer.represented_version("note-race"), Some(2));
    both(&f.indexer, "newonlyneedle", 1);
    both(&f.indexer, "oldonlyneedle", 0);
}

#[test]
fn identical_redelivery_of_current_version_is_idempotent() {
    let f = Fixture::new();
    f.write(0, "internal", "onlyneedle");
    let record = f
        .vault
        .read_note(&NoteId::parse("note-race").unwrap())
        .unwrap();
    let bytes = format!("1\n---\n{}", record.content);
    for _ in 0..3 {
        f.indexer.mark_current("note-race", 1, bytes.as_bytes());
        assert_eq!(f.indexer.state_for("note-race"), IndexState::Current);
        assert_eq!(f.indexer.represented_version("note-race"), Some(1));
        both(&f.indexer, "onlyneedle", 1);
    }
}

#[test]
fn poisoned_publication_lock_recovers_coherently_and_does_not_fail_next_commit() {
    let f = Fixture::new();
    f.write(0, "internal", "oldonlyneedle");
    let indexer = f.indexer.clone();
    let poisoned = std::thread::spawn(move || {
        let mut data = indexer.data.lock().unwrap();
        data.trigram
            .insert(NoteId::parse("note-race").unwrap(), "inconsistentneedle");
        panic!("injected mutex poison without content");
    })
    .join();
    assert!(poisoned.is_err());
    assert_eq!(f.indexer.state_for("note-race"), IndexState::Stale);
    assert_eq!(f.indexer.represented_version("note-race"), None);
    both(&f.indexer, "oldonlyneedle", 0);
    both(&f.indexer, "inconsistentneedle", 0);
    assert!(matches!(
        f.write(1, "internal", "newonlyneedle"),
        CommitResult::Committed(_)
    ));
    assert_eq!(f.indexer.state_for("note-race"), IndexState::Current);
    assert_eq!(f.indexer.represented_version("note-race"), Some(2));
    both(&f.indexer, "newonlyneedle", 1);
}
