use std::collections::HashSet;
use std::path::PathBuf;

use uuid::Uuid;

use storage::{CommitOutcome, CommitResult, NoteId, NoteIdError, ProposeError, StorageError, Vault, Version};

fn unique_test_dir() -> PathBuf {
    // A per-process counter on top of the timestamp: parallel tests can read
    // the same clock value, which made two tests share one directory.
    static UNIQUE_DIR_COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "superbrain-propose-test-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        UNIQUE_DIR_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// `Vault::open` with `root` and a fresh, unique journal path alongside it —
/// the mechanical A2 replacement for A1's `Vault::new(root)` at every call
/// site in this file. This integration test continues to exercise only
/// `propose_and_commit`'s genuinely public surface; this is its only change.
///
/// Task 5 note: `root` is ALSO git-initialized (with an initial empty
/// commit), so a successful `propose_and_commit` call reaches the new
/// terminus, `WriteState::GitCommitted`, and reports
/// `Ok(CommitResult::Committed(_))` rather than `Ok(CommitResult::GitPending { .. })`.
/// Every call site in this file exercises the ordinary, git-backed success
/// path; `propose_and_commit_surfaces_git_pending_as_ok_not_err` below is
/// the one test that deliberately builds its own non-git vault instead.
fn open_vault(root: PathBuf) -> Vault {
    git2::Repository::init(&root).unwrap();
    {
        let repo = git2::Repository::open(&root).unwrap();
        let signature = git2::Signature::now("SuperBrain Test", "test@localhost").unwrap();
        let tree_oid = {
            let mut index = repo.index().unwrap();
            index.write_tree().unwrap()
        };
        let tree = repo.find_tree(tree_oid).unwrap();
        repo.commit(Some("HEAD"), &signature, &signature, "initial empty commit", &tree, &[])
            .unwrap();
    }
    Vault::open_with_default_indexer(root, &unique_test_dir().join("journal.sqlite")).unwrap()
}

fn valid_yaml(id: &str) -> String {
    format!(
        r#"
id: {id}
schema_version: 1
type: bug
status: draft
trust: internal
classification: internal
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

fn secret_classified_yaml(id: &str) -> String {
    format!(
        r#"
id: {id}
schema_version: 1
type: bug
status: draft
trust: internal
classification: secret
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

fn yaml_with_broken_link(id: &str) -> String {
    format!(
        r#"
id: {id}
schema_version: 1
type: bug
status: draft
trust: internal
classification: internal
project: project-x
source: agent:coder@prompt-v3
created: 2026-10-23
updated: 2026-10-23
event_period: 2026-10-23
relations:
  supports: ["missing-id"]
  contradicts: []
  derived_from: []
  supersedes: []
  applies_to: []
"#
    )
}

#[test]
fn storage_exposes_exactly_one_public_write_function() {
    // The "no bypass" guarantee for A1 is: both raw write primitives —
    // Vault::write_record_unchecked and atomic::atomic_write — are pub(crate),
    // unreachable from outside this crate, neither is re-exported from lib.rs,
    // and propose_and_commit is the only public function whose body can reach
    // them. The compiler-level proof is the set of compile_fail doctests in
    // lib.rs; this textual test is a belt-and-braces regression guard over the
    // same shape, so a future change adding a second public write path (in any
    // of vault.rs, propose.rs, atomic.rs, or lib.rs's re-exports) fails here
    // and forces a conscious review, rather than landing silently.
    let lib_source = include_str!("../src/lib.rs");
    let atomic_source = include_str!("../src/atomic.rs");
    let vault_source = include_str!("../src/vault.rs");
    let propose_source = include_str!("../src/propose.rs");

    let forbidden_reexports: Vec<&str> = lib_source
        .lines()
        .filter(|l| l.trim_start().starts_with("pub use"))
        .filter(|l| {
            l.contains("atomic")
                || l.contains("AtomicWriteError")
                || l.contains("write_record_unchecked")
        })
        .collect();
    assert!(
        forbidden_reexports.is_empty(),
        "lib.rs must not re-export any raw write primitive, found: {forbidden_reexports:?}"
    );

    let bare_pub_items_in_atomic: Vec<&str> = atomic_source
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            t.starts_with("pub ") && !t.starts_with("pub(crate)")
        })
        .collect();
    assert!(
        bare_pub_items_in_atomic.is_empty(),
        "atomic.rs must expose no bare-pub item, found: {bare_pub_items_in_atomic:?}"
    );

    let pub_write_fns_in_vault: Vec<&str> = vault_source
        .lines()
        .filter(|l| l.trim_start().starts_with("pub fn") && l.contains("write"))
        .collect();
    assert!(
        pub_write_fns_in_vault.is_empty(),
        "vault.rs must expose no public write function, found: {pub_write_fns_in_vault:?}"
    );
    let pub_write_fns_in_propose: Vec<&str> = propose_source
        .lines()
        .filter(|l| l.trim_start().starts_with("pub fn"))
        .collect();
    assert_eq!(
        pub_write_fns_in_propose.len(),
        1,
        "propose.rs must expose exactly one public function, found: {pub_write_fns_in_propose:?}"
    );
    assert!(pub_write_fns_in_propose[0].contains("propose_and_commit"));
}

#[test]
fn valid_note_passes_gate_and_is_committed_through_the_controlled_entry_point() {
    let vault = open_vault(unique_test_dir());
    let gate = policy::Gate::new(policy::SecretPolicy::new(1));
    let yaml = valid_yaml("note-e2e");
    let v = storage::propose_and_commit(
        &vault,
        &yaml,
        Version(0),
        Uuid::new_v4(),
        &HashSet::new(),
        &HashSet::new(),
        &gate,
    )
    .unwrap();
    assert_eq!(
        v,
        CommitResult::Committed(CommitOutcome { version: Version(1), link_state: domain::LinkState::Unlinked })
    );
}

#[test]
fn secret_classified_note_is_rejected_by_propose_and_commit_and_the_record_file_is_never_created() {
    let vault = open_vault(unique_test_dir());
    let gate = policy::Gate::new(policy::SecretPolicy::new(1));
    let yaml = secret_classified_yaml("note-secret");
    let err = storage::propose_and_commit(
        &vault,
        &yaml,
        Version(0),
        Uuid::new_v4(),
        &HashSet::new(),
        &HashSet::new(),
        &gate,
    )
    .unwrap_err();
    assert!(matches!(
        err,
        ProposeError::Rejected(policy::GateRejection::SecretClassification)
    ));
    // positive proof the write never happened, not just that we didn't call write ourselves:
    let id = NoteId::parse("note-secret").unwrap();
    assert!(matches!(
        vault.read_note(&id).unwrap_err(),
        StorageError::NotFound { .. }
    ));
}

#[test]
fn note_with_broken_link_is_rejected_by_propose_and_commit_and_the_record_file_is_never_created() {
    let vault = open_vault(unique_test_dir());
    let gate = policy::Gate::new(policy::SecretPolicy::new(1));
    let yaml = yaml_with_broken_link("note-broken-link");
    let err = storage::propose_and_commit(
        &vault,
        &yaml,
        Version(0),
        Uuid::new_v4(),
        &HashSet::new(),
        &HashSet::new(),
        &gate,
    )
    .unwrap_err();
    assert!(matches!(err, ProposeError::Link(domain::LinkError::BrokenLink { .. })));
    let id = NoteId::parse("note-broken-link").unwrap();
    assert!(matches!(
        vault.read_note(&id).unwrap_err(),
        StorageError::NotFound { .. }
    ));
}

#[test]
fn version_conflict_surfaces_end_to_end_through_propose_and_commit() {
    let vault = open_vault(unique_test_dir());
    let gate = policy::Gate::new(policy::SecretPolicy::new(1));
    let yaml = valid_yaml("note-e2e-conflict");
    storage::propose_and_commit(
        &vault,
        &yaml,
        Version(0),
        Uuid::new_v4(),
        &HashSet::new(),
        &HashSet::new(),
        &gate,
    )
    .unwrap();
    let err = storage::propose_and_commit(
        &vault,
        &yaml,
        Version(0),
        Uuid::new_v4(),
        &HashSet::new(),
        &HashSet::new(),
        &gate,
    )
    .unwrap_err();
    assert!(matches!(err, ProposeError::Storage(StorageError::Conflict { .. })));
}

// --- A1 final review, Finding I1: typo'd relation key must not commit ---

#[test]
fn typoed_relation_key_is_rejected_end_to_end_and_the_record_file_is_never_created() {
    // Reviewer's reproduction: `supprots: ["missing-id"]` used to parse and
    // COMMIT, the broken link silently vanishing with the ignored key.
    let root = unique_test_dir();
    let vault = open_vault(root.clone());
    let gate = policy::Gate::new(policy::SecretPolicy::new(1));
    let yaml = valid_yaml("note-typo").replace("supports: []", "supprots: [\"missing-id\"]");
    let err = storage::propose_and_commit(
        &vault,
        &yaml,
        Version(0),
        Uuid::new_v4(),
        &HashSet::new(),
        &HashSet::new(),
        &gate,
    )
    .unwrap_err();
    match err {
        ProposeError::Rejected(policy::GateRejection::Schema(domain::ValidationError::UnknownField(f))) => {
            assert_eq!(f, "relations.supprots")
        }
        other => panic!("expected Rejected(Schema(UnknownField)), got {other:?}"),
    }
    assert!(files_in(&root).is_empty(), "nothing may be written");
}

#[test]
fn unknown_top_level_frontmatter_key_is_rejected_end_to_end() {
    let root = unique_test_dir();
    let vault = open_vault(root.clone());
    let gate = policy::Gate::new(policy::SecretPolicy::new(1));
    let yaml = format!("{}totally_unknown: 1\n", valid_yaml("note-extra"));
    let err = storage::propose_and_commit(
        &vault,
        &yaml,
        Version(0),
        Uuid::new_v4(),
        &HashSet::new(),
        &HashSet::new(),
        &gate,
    )
    .unwrap_err();
    assert!(
        matches!(
            &err,
            ProposeError::Rejected(policy::GateRejection::Schema(domain::ValidationError::UnknownField(f)))
                if f == "totally_unknown"
        ),
        "got {err:?}"
    );
    assert!(files_in(&root).is_empty(), "nothing may be written");
}

// --- A1 final review, Finding C2: case-insensitive filesystem collision ---

/// Entries directly inside `dir`, excluding `.git` (Task 5: `open_vault`
/// now git-initializes its root as a side effect of making the vault
/// git-backed, so `.git` is a vault-setup artifact, not note content — the
/// "nothing may be written" assertions in this file are about note content).
fn files_in(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name != ".git")
        .collect();
    names.sort();
    names
}

#[test]
fn case_variant_of_an_existing_note_id_is_rejected_before_the_vault_and_original_is_unchanged() {
    // Reproduction of the reviewer's data-loss scenario: write note-a, then
    // follow the documented "read current version, retry" flow with id
    // NOTE-A at note-a's current version. On a case-insensitive filesystem
    // NOTE-A.record IS note-a.record, so before the fix this silently
    // overwrote note-a. It must now be a validation error naming the id
    // field, raised before the vault is touched.
    let root = unique_test_dir();
    let vault = open_vault(root.clone());
    let gate = policy::Gate::new(policy::SecretPolicy::new(1));
    let original_yaml = valid_yaml("note-a");
    storage::propose_and_commit(
        &vault,
        &original_yaml,
        Version(0),
        Uuid::new_v4(),
        &HashSet::new(),
        &HashSet::new(),
        &gate,
    )
    .unwrap();
    let id = NoteId::parse("note-a").unwrap();
    let original = vault.read_note(&id).unwrap();
    let files_before = files_in(&root);

    for alias in ["NOTE-A", "Note-A", "note-A"] {
        let current = vault.current_version(&id).unwrap();
        let err = storage::propose_and_commit(
            &vault,
            &valid_yaml(alias).replace("project-x", "clobbered"),
            current,
            Uuid::new_v4(),
            &HashSet::new(),
            &HashSet::new(),
            &gate,
        )
        .unwrap_err();
        match err {
            ProposeError::InvalidNoteId { field, value, error } => {
                assert_eq!(field, "id");
                assert_eq!(value, alias);
                assert!(
                    matches!(error, NoteIdError::MustBeLowercase { .. }),
                    "alias {alias:?}: expected MustBeLowercase, got {error:?}"
                );
            }
            other => panic!("alias {alias:?}: expected ProposeError::InvalidNoteId, got {other:?}"),
        }
        assert_eq!(
            vault.read_note(&id).unwrap(),
            original,
            "alias {alias:?} must not modify note-a's record"
        );
        assert_eq!(files_in(&root), files_before, "alias {alias:?} must not create or touch any file");
    }
}

#[test]
fn relation_target_that_is_not_a_canonical_note_id_is_rejected_as_invalid_note_id() {
    // Link lookups must compare canonical NoteIds too: an uppercase relation
    // target must not be silently treated as a different (broken or
    // resolved) id than its lowercase form.
    let root = unique_test_dir();
    let vault = open_vault(root.clone());
    let gate = policy::Gate::new(policy::SecretPolicy::new(1));
    let mut known = HashSet::new();
    known.insert(NoteId::parse("note-b").unwrap());
    let yaml = valid_yaml("note-c").replace("supports: []", "supports: [\"NOTE-B\"]");
    let err = storage::propose_and_commit(
        &vault,
        &yaml,
        Version(0),
        Uuid::new_v4(),
        &known,
        &HashSet::new(),
        &gate,
    )
    .unwrap_err();
    match err {
        ProposeError::InvalidNoteId { field, value, error } => {
            assert_eq!(field, "relations.supports");
            assert_eq!(value, "NOTE-B");
            assert!(matches!(error, NoteIdError::MustBeLowercase { .. }));
        }
        other => panic!("expected ProposeError::InvalidNoteId, got {other:?}"),
    }
    assert!(files_in(&root).is_empty(), "nothing may be written");
}

#[test]
fn canonical_relation_target_resolves_against_known_note_ids() {
    let vault = open_vault(unique_test_dir());
    let gate = policy::Gate::new(policy::SecretPolicy::new(1));
    let mut known = HashSet::new();
    known.insert(NoteId::parse("note-b").unwrap());
    let yaml = valid_yaml("note-c").replace("supports: []", "supports: [\"note-b\"]");
    let v = storage::propose_and_commit(
        &vault,
        &yaml,
        Version(0),
        Uuid::new_v4(),
        &known,
        &HashSet::new(),
        &gate,
    )
    .unwrap();
    assert_eq!(
        v,
        CommitResult::Committed(CommitOutcome { version: Version(1), link_state: domain::LinkState::Resolved })
    );
}

// --- A1 second whole-branch review, Finding I-A: pending references are opaque ---
//
// Spec C6 / Fase A: a "pending" reference (external source, not-yet-imported
// note, unresolved code symbol) is accepted and flagged. Such references are
// not note ids and must not be forced through NoteId syntax.

fn pending(refs: &[&str]) -> HashSet<String> {
    refs.iter().map(|r| r.to_string()).collect()
}

fn assert_pending_ref_commits(relation: &str, target: &str) {
    let vault = open_vault(unique_test_dir());
    let gate = policy::Gate::new(policy::SecretPolicy::new(1));
    let yaml = valid_yaml("note-p").replace(&format!("{relation}: []"), &format!("{relation}: [\"{target}\"]"));
    assert!(yaml.contains(target), "fixture must actually contain the target");
    let v = storage::propose_and_commit(
        &vault,
        &yaml,
        Version(0),
        Uuid::new_v4(),
        &HashSet::new(),
        &pending(&[target]),
        &gate,
    )
    .unwrap_or_else(|e| panic!("pending {relation} target {target:?} must commit, got {e:?}"));
    assert_eq!(
        v,
        CommitResult::Committed(CommitOutcome { version: Version(1), link_state: domain::LinkState::Pending })
    );
    let id = NoteId::parse("note-p").unwrap();
    assert_eq!(vault.read_note(&id).unwrap().content, yaml);
}

#[test]
fn pending_url_reference_in_derived_from_is_accepted() {
    assert_pending_ref_commits("derived_from", "https://example.com/some-paper");
}

#[test]
fn pending_doi_reference_in_supports_is_accepted() {
    assert_pending_ref_commits("supports", "doi:10.1000/xyz");
}

#[test]
fn pending_unresolved_code_symbol_in_applies_to_is_accepted() {
    assert_pending_ref_commits("applies_to", "src/main.rs::CorsMiddleware");
}

#[test]
fn mixed_resolved_and_opaque_pending_targets_commit_and_classify_as_pending() {
    let vault = open_vault(unique_test_dir());
    let gate = policy::Gate::new(policy::SecretPolicy::new(1));
    let mut known = HashSet::new();
    known.insert(NoteId::parse("note-b").unwrap());
    let pending_refs = pending(&["https://fastapi.tiangolo.com/tutorial/cors/"]);
    let yaml = valid_yaml("note-mixed")
        .replace("supports: []", "supports: [\"note-b\"]")
        .replace(
            "derived_from: []",
            "derived_from: [\"https://fastapi.tiangolo.com/tutorial/cors/\"]",
        );
    let outcome = storage::propose_and_commit(
        &vault,
        &yaml,
        Version(0),
        Uuid::new_v4(),
        &known,
        &pending_refs,
        &gate,
    )
    .unwrap();
    // Spec C6: a pending reference is "accettato e segnalato". The commit
    // result itself flags it; the caller does not have to re-classify.
    assert_eq!(
        outcome,
        CommitResult::Committed(CommitOutcome { version: Version(1), link_state: domain::LinkState::Pending })
    );
}

#[test]
fn non_note_id_target_not_declared_pending_is_still_rejected_as_invalid_note_id() {
    // Only a caller-declared pending reference may bypass NoteId syntax. A
    // URL that is NOT in pending_ids is malformed input, reported precisely
    // as InvalidNoteId (not confused with a BrokenLink to a missing note).
    let root = unique_test_dir();
    let vault = open_vault(root.clone());
    let gate = policy::Gate::new(policy::SecretPolicy::new(1));
    let yaml = valid_yaml("note-q").replace("derived_from: []", "derived_from: [\"https://example.com/x\"]");
    let err = storage::propose_and_commit(
        &vault,
        &yaml,
        Version(0),
        Uuid::new_v4(),
        &HashSet::new(),
        &pending(&["https://example.com/other"]),
        &gate,
    )
    .unwrap_err();
    match err {
        ProposeError::InvalidNoteId { field, value, .. } => {
            assert_eq!(field, "relations.derived_from");
            assert_eq!(value, "https://example.com/x");
        }
        other => panic!("expected ProposeError::InvalidNoteId, got {other:?}"),
    }
    assert!(files_in(&root).is_empty(), "nothing may be written");
}

#[test]
fn valid_note_id_target_neither_known_nor_pending_is_still_a_broken_link() {
    let root = unique_test_dir();
    let vault = open_vault(root.clone());
    let gate = policy::Gate::new(policy::SecretPolicy::new(1));
    let yaml = valid_yaml("note-r").replace("supports: []", "supports: [\"missing-id\"]");
    let err = storage::propose_and_commit(
        &vault,
        &yaml,
        Version(0),
        Uuid::new_v4(),
        &HashSet::new(),
        &pending(&["https://example.com/unrelated"]),
        &gate,
    )
    .unwrap_err();
    assert!(
        matches!(&err, ProposeError::Link(domain::LinkError::BrokenLink { target_id, .. }) if target_id == "missing-id"),
        "got {err:?}"
    );
    assert!(files_in(&root).is_empty(), "nothing may be written");
}

// --- A1 second whole-branch review, Finding I-B: full notes (frontmatter + body) ---

/// A full note: `---`-delimited YAML frontmatter followed by an opaque body.
fn full_note(id: &str, body: &str) -> String {
    format!("---\n{}---\n{body}", valid_yaml(id).trim_start())
}

/// Thin wrapper over `propose_and_commit` for this section's tests, which
/// only care about the ordinary `CommitOutcome` (every call site here uses
/// `open_vault`, which is git-backed, so a successful commit always reaches
/// `CommitResult::Committed`). `GitPending` has its own dedicated test
/// (`propose_and_commit_surfaces_git_pending_as_ok_not_err`), so unwrapping
/// it away here would hide a real regression rather than paper over an
/// irrelevant case.
fn commit(vault: &Vault, text: &str, expected: Version) -> Result<CommitOutcome, ProposeError> {
    let gate = policy::Gate::new(policy::SecretPolicy::new(1));
    storage::propose_and_commit(
        vault,
        text,
        expected,
        Uuid::new_v4(),
        &HashSet::new(),
        &HashSet::new(),
        &gate,
    )
    .map(|result| match result {
        CommitResult::Committed(outcome) => outcome,
        CommitResult::GitPending { note_id, operation_id } => panic!(
            "commit() test helper expects a git-backed vault to reach CommitResult::Committed, got GitPending for note {note_id:?} operation {operation_id}"
        ),
    })
}

#[test]
fn note_with_frontmatter_and_body_round_trips_byte_for_byte() {
    let vault = open_vault(unique_test_dir());
    let text = full_note(
        "note-body",
        "# CORS bug\n\nSome prose.\n\n---\n\nA thematic break above, and a fake key below:\nid: not-the-real-id\n",
    );
    assert_eq!(commit(&vault, &text, Version(0)).unwrap().version, Version(1));
    let record = vault.read_note(&NoteId::parse("note-body").unwrap()).unwrap();
    assert_eq!(record.content, text, "stored content must equal the input exactly");
    assert_eq!(record.version, Version(1));
}

#[test]
fn updating_a_note_replaces_its_body_atomically() {
    let vault = open_vault(unique_test_dir());
    let id = NoteId::parse("note-upd").unwrap();
    let v1 = full_note("note-upd", "old body: first draft\n");
    let v2 = full_note("note-upd", "new body: rewritten\n");
    commit(&vault, &v1, Version(0)).unwrap();
    assert_eq!(commit(&vault, &v2, Version(1)).unwrap().version, Version(2));
    let record = vault.read_note(&id).unwrap();
    assert_eq!(record.content, v2);
    assert!(!record.content.contains("first draft"), "old body must be gone");
    assert_eq!(record.version, Version(2));
}

#[test]
fn version_conflict_on_a_note_with_a_body_is_the_ordinary_storage_conflict() {
    let vault = open_vault(unique_test_dir());
    let id = NoteId::parse("note-conf").unwrap();
    let original = full_note("note-conf", "body A\n");
    commit(&vault, &original, Version(0)).unwrap();
    let err = commit(&vault, &full_note("note-conf", "body B\n"), Version(0)).unwrap_err();
    match err {
        ProposeError::Storage(StorageError::Conflict { note_id, expected, actual }) => {
            assert_eq!(note_id, "note-conf");
            assert_eq!(expected, Version(0));
            assert_eq!(actual, Version(1));
        }
        other => panic!("expected Storage(Conflict), got {other:?}"),
    }
    assert_eq!(vault.read_note(&id).unwrap().content, original, "loser must not modify the note");
}

#[test]
fn secret_only_in_the_body_is_rejected_by_the_gate_and_the_record_file_is_never_created() {
    let root = unique_test_dir();
    let vault = open_vault(root.clone());
    let text = full_note("note-leak", "Debug notes.\naws_key = AKIAABCDEFGHIJKLMNOP\n");
    let frontmatter_end = text.find("---\nDebug").unwrap();
    assert!(!text[..frontmatter_end].contains("AKIA"), "secret must be in the body only");
    let err = commit(&vault, &text, Version(0)).unwrap_err();
    assert!(
        matches!(
            &err,
            ProposeError::Rejected(policy::GateRejection::SecretContentDetected { note_id, pattern_id })
                if note_id == "note-leak" && pattern_id == "aws-access-key"
        ),
        "got {err:?}"
    );
    assert!(matches!(
        vault.read_note(&NoteId::parse("note-leak").unwrap()).unwrap_err(),
        StorageError::NotFound { .. }
    ));
    assert!(files_in(&root).is_empty(), "nothing may be written");
}

// --- A1 third whole-branch review, Finding I-C: symlink at the real record path ---

#[test]
fn propose_and_commit_rejects_a_record_path_symlinked_outside_the_vault_root() {
    // `root/note-a.record` (the exact path every I/O call uses) is a symlink
    // to a well-formed record at version 7 outside the root. Before the fix,
    // committing note-a at Version(7) succeeded because the version check
    // read the outside file through the link.
    let root = unique_test_dir();
    let outside_dir = unique_test_dir();
    let outside_file = outside_dir.join("victim.record");
    let outside_bytes = "7\n---\nOUTSIDE CONTENT".as_bytes();
    std::fs::write(&outside_file, outside_bytes).unwrap();
    let link = root.join("note-a.record");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside_file, &link).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_file(&outside_file, &link).unwrap();
    let vault = open_vault(root.clone());

    for expected in [Version(7), Version(0)] {
        let err = commit(&vault, &valid_yaml("note-a"), expected).unwrap_err();
        assert!(
            matches!(
                &err,
                ProposeError::Storage(StorageError::PathEscapesVaultRoot { note_id }) if note_id == "note-a"
            ),
            "expected Version {expected:?}: got {err:?}"
        );
    }
    assert!(link.symlink_metadata().unwrap().file_type().is_symlink(), "link left untouched");
    assert_eq!(std::fs::read(&outside_file).unwrap(), outside_bytes, "outside file untouched");
}

#[test]
fn frontmatter_only_note_without_delimiters_still_commits_unchanged() {
    // Backward compatibility: bare YAML (no `---` delimiters, no body) is
    // still accepted and stored exactly as given.
    let vault = open_vault(unique_test_dir());
    let yaml = valid_yaml("note-legacy");
    assert!(!yaml.contains("---"));
    assert_eq!(commit(&vault, &yaml, Version(0)).unwrap().version, Version(1));
    assert_eq!(vault.read_note(&NoteId::parse("note-legacy").unwrap()).unwrap().content, yaml);
}

// --- Task 5: git_pending as a non-error CommitResult ---

/// Alias for `valid_yaml`, under the name the Task 5 brief's own test uses.
fn valid_note_yaml(id: &str) -> String {
    valid_yaml(id)
}

/// A vault whose root is deliberately NOT a git repository, plus the other
/// fixtures `propose_and_commit` needs (a gate, empty known/pending sets),
/// for the one test in this file that exercises the `GitPending` response
/// code rather than the ordinary git-backed success path every other test
/// here uses via `open_vault`.
fn test_fixtures_no_git_repo() -> (Vault, policy::Gate, HashSet<NoteId>, HashSet<String>) {
    let vault = Vault::open_with_default_indexer(unique_test_dir(), &unique_test_dir().join("journal.sqlite")).unwrap();
    let gate = policy::Gate::new(policy::SecretPolicy::new(1));
    (vault, gate, HashSet::new(), HashSet::new())
}

#[test]
fn propose_and_commit_surfaces_git_pending_as_ok_not_err() {
    let (vault, gate, known, pending) = test_fixtures_no_git_repo();
    let note_text = valid_note_yaml("note-a");
    let result = storage::propose_and_commit(&vault, &note_text, Version(0), Uuid::new_v4(), &known, &pending, &gate);
    assert!(matches!(result, Ok(CommitResult::GitPending { .. })), "git unavailability is a non-error outcome, got {result:?}");
}
