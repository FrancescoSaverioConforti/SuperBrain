use std::collections::HashSet;
use std::path::PathBuf;

use uuid::Uuid;

use storage::{NoteId, ProposeError, StorageError, Vault, Version};

fn unique_test_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "superbrain-propose-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
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
    let vault = Vault::new(unique_test_dir());
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
    assert_eq!(v, Version(1));
}

#[test]
fn secret_classified_note_is_rejected_by_propose_and_commit_and_the_record_file_is_never_created() {
    let vault = Vault::new(unique_test_dir());
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
    let vault = Vault::new(unique_test_dir());
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
    let vault = Vault::new(unique_test_dir());
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
