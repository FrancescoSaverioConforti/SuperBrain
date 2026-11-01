use api::*;
use domain::{Classification, NoteStatus, TrustLevel};
use index::{
    indexer_impl::{rebuild_index, RealIndexer},
    search::{AppliedProjectScope, SearchOutcome},
    Index,
};
use std::{collections::HashSet, sync::Arc};
use storage::{NoteId, Vault, Version};
use uuid::Uuid;

struct Fixture {
    vault: Vault,
    pii: vault_security::PiiStore,
    indexer: Arc<RealIndexer>,
    root: std::path::PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("superbrain-api-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        {
            let repo = git2::Repository::init(&root).unwrap();
            let sig = git2::Signature::now("Test", "test@localhost").unwrap();
            let oid = repo.index().unwrap().write_tree().unwrap();
            let tree = repo.find_tree(oid).unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
                .unwrap();
        }
        let pii_root = root.join(".pii-store");
        let pii =
            vault_security::PiiStore::open(pii_root.clone(), &pii_root.join("master.key")).unwrap();
        // Rebuild after writes exercises the same public read-only seam as a
        // cold-start client; no direct access to raw record internals.
        let vault =
            Vault::open_with_pii_store(root.clone(), &root.join("journal.sqlite"), &pii).unwrap();
        Self {
            vault,
            pii,
            indexer: Arc::new(RealIndexer::new()),
            root,
        }
    }
    fn write(&self, id: &str, class: &str, body: &str, expected: u64) {
        let text = format!("---\nid: {id}\nschema_version: 1\ntype: bug\nstatus: reviewed\ntrust: official\nclassification: {class}\nproject: project-x\nsource: SOURCE-CANARY\ncreated: 2026-10-30\nupdated: 2026-10-30\nevent_period: 2026-08-23\nrelations:\n  supports: [RELATION-CANARY]\n---\n{body}");
        let result = storage::propose_and_commit(
            &self.vault,
            &text,
            Version(expected),
            Uuid::new_v4(),
            &HashSet::new(),
            &HashSet::from(["RELATION-CANARY".into()]),
            &policy::Gate::new(policy::SecretPolicy::new(1)),
            Some(&self.pii),
        )
        .unwrap();
        assert!(matches!(result, storage::CommitResult::Committed(_)));
        rebuild_index(&self.vault, &self.indexer).unwrap();
    }
    fn index(&self) -> Index<'_> {
        Index::new(
            &self.vault,
            &self.indexer,
            "project-x",
            chrono::NaiveDate::from_ymd_opt(2026, 10, 30).unwrap(),
        )
    }
    fn get(&self, id: &str) -> ApiResponse<RetrieveResponse> {
        retrieve(&self.vault, &allow(), context(), note(id))
    }
}
fn context() -> RequestContext {
    RequestContext {
        schema_version: SCHEMA_VERSION,
        correlation_id: Uuid::new_v4(),
    }
}
fn allow() -> policy::ReadPolicy {
    policy::ReadPolicy::new(true, true)
}
fn note(id: &str) -> RetrieveRequest {
    RetrieveRequest {
        id: RetrieveTarget::Note(NoteId::parse(id).unwrap()),
    }
}
fn query(text: &str) -> SearchRequest {
    SearchRequest {
        text: text.into(),
        filters: Default::default(),
        k: 10,
    }
}

#[test]
fn retrieve_preserves_committed_bytes_version_provenance_and_status() {
    let f = Fixture::new();
    f.write("note-a", "internal", "body v1", 0);
    f.write("note-a", "internal", "body v2", 1);
    let ctx = context();
    let r = retrieve(&f.vault, &allow(), ctx, note("note-a"));
    assert_eq!(r.schema_version, SCHEMA_VERSION);
    assert_eq!(r.correlation_id, ctx.correlation_id);
    let r = r.result.unwrap();
    let raw = f
        .vault
        .read_note(&NoteId::parse("note-a").unwrap())
        .unwrap();
    assert_eq!(r.content.as_bytes(), raw.content.as_bytes());
    assert_eq!(r.version, Version(2));
    assert_eq!(r.id.as_str(), "note-a");
    assert_eq!(r.status, NoteStatus::Reviewed);
    assert_eq!(r.provenance.trust, TrustLevel::Official);
    assert_eq!(r.provenance.project, "project-x");
    assert_eq!(r.provenance.classification, Classification::Internal);
    assert_eq!(r.provenance.source.as_deref(), Some("SOURCE-CANARY"));
    assert_eq!(r.metadata_schema_version, 1);
}
#[test]
fn sensitive_retrieve_is_pointer_only_and_retains_full_frontmatter() {
    let f = Fixture::new();
    f.write("note-sensitive", "sensitive", "PRIVATE-BODY-CANARY", 0);
    let r = f.get("note-sensitive").result.unwrap();
    assert!(r.content.contains("pii://note-sensitive"));
    assert!(!r.content.contains("PRIVATE-BODY-CANARY"));
    assert!(r.content.contains("RELATION-CANARY"));
    assert_eq!(r.provenance.source.as_deref(), Some("SOURCE-CANARY"));
    assert_eq!(r.provenance.classification, Classification::Sensitive);
    let raw = f
        .vault
        .read_note(&NoteId::parse("note-sensitive").unwrap())
        .unwrap();
    assert_eq!(r.content, raw.content);
}
#[test]
fn api_search_cites_actual_match_and_excludes_topical_nonmatch() {
    let f = Fixture::new();
    f.write(
        "note-match",
        "internal",
        "get_user_by_id retrieves a user",
        0,
    );
    f.write(
        "note-topic",
        "internal",
        "user retrieval authentication programming",
        0,
    );
    let ctx = context();
    let r = search(&f.index(), &allow(), ctx, query("get_user_by_id"));
    assert_eq!(r.schema_version, 1);
    assert_eq!(r.correlation_id, ctx.correlation_id);
    let r = r.result.unwrap();
    assert_eq!(
        r.context.project,
        AppliedProjectScope::ActiveOnly {
            project: "project-x".into()
        }
    );
    let SearchOutcome::Matches { first, rest } = r.outcome else {
        panic!("missing evidence")
    };
    assert_eq!(first.note_id.as_str(), "note-match");
    assert!(first.excerpt.contains("get_user_by_id"));
    assert!(rest.is_empty());
}
#[test]
fn wrapper_preserves_not_present_and_sensitive_search_redaction() {
    let f = Fixture::new();
    f.write("note-sensitive", "sensitive", "PRIVATE-CANARY", 0);
    for term in ["PRIVATE-CANARY", "SOURCE-CANARY", "RELATION-CANARY"] {
        let response = search(&f.index(), &allow(), context(), query(term))
            .result
            .unwrap();
        assert_eq!(response.outcome, SearchOutcome::NotPresent);
    }
    let response = search(&f.index(), &allow(), context(), query("note-sensitive"));
    assert!(!format!("{response:?}").contains("CANARY"));
}
#[test]
fn each_operation_checks_policy_and_preserves_error_correlation() {
    let f = Fixture::new();
    f.write("note-a", "internal", "needle", 0);
    let ctx = context();
    let deny = policy::ReadPolicy::default();
    let r = retrieve(&f.vault, &deny, ctx, note("note-a"));
    assert_eq!(r.result, Err(ApiError::PermissionDenied));
    assert_eq!(r.correlation_id, ctx.correlation_id);
    assert_eq!(
        search(&f.index(), &deny, ctx, query("needle")).result,
        Err(ApiError::PermissionDenied)
    );
    assert!(search(
        &f.index(),
        &policy::ReadPolicy::new(true, false),
        ctx,
        query("needle")
    )
    .result
    .is_ok());
    assert_eq!(
        retrieve(
            &f.vault,
            &policy::ReadPolicy::new(true, false),
            ctx,
            note("note-a")
        )
        .result,
        Err(ApiError::PermissionDenied)
    );
    assert!(retrieve(
        &f.vault,
        &policy::ReadPolicy::new(false, true),
        ctx,
        note("note-a")
    )
    .result
    .is_ok());
    assert_eq!(
        search(
            &f.index(),
            &policy::ReadPolicy::new(false, true),
            ctx,
            query("needle")
        )
        .result,
        Err(ApiError::PermissionDenied)
    );
}
#[test]
fn unsupported_schemas_chunks_and_missing_notes_are_typed() {
    let f = Fixture::new();
    let ctx = RequestContext {
        schema_version: 999,
        ..context()
    };
    assert_eq!(
        retrieve(&f.vault, &allow(), ctx, note("missing")).result,
        Err(ApiError::UnsupportedSchema)
    );
    assert_eq!(
        search(&f.index(), &allow(), ctx, query("needle")).result,
        Err(ApiError::UnsupportedSchema)
    );
    assert_eq!(f.get("missing").result, Err(ApiError::NotFound));
    assert_eq!(
        retrieve(
            &f.vault,
            &allow(),
            context(),
            RetrieveRequest {
                id: RetrieveTarget::Chunk("CANARY-PAYLOAD".into())
            }
        )
        .result,
        Err(ApiError::UnsupportedChunk)
    );
}
#[test]
fn retrieve_errors_are_sanitized_and_deleted_notes_are_unreadable() {
    let f = Fixture::new();
    f.write("note-a", "sensitive", "private", 0);
    f.write("note-a", "sensitive", storage::SENSITIVE_TOMBSTONE_BODY, 1);
    assert_eq!(f.get("note-a").result, Err(ApiError::Deleted));
    f.write("note-b", "internal", "needle", 0);
    std::fs::write(f.root.join("note-b.record"), "CORRUPT-CANARY").unwrap();
    let response = f.get("note-b");
    assert_eq!(response.result, Err(ApiError::Inconsistent));
    assert!(!format!("{response:?}").contains("CANARY"));
}
#[test]
fn operational_gate_failures_are_errors_in_both_wrappers() {
    let f = Fixture::new();
    f.write("note-a", "internal", "needle", 0);
    std::fs::write(f.pii.deletion_log_path(), "BROKEN-LOG-CANARY").unwrap();
    assert_eq!(f.get("note-a").result, Err(ApiError::ReadUnavailable));
    assert_eq!(
        search(&f.index(), &allow(), context(), query("needle")).result,
        Err(ApiError::Search(
            index::search::SearchError::ReadUnavailable
        ))
    );
}

#[test]
fn held_out_labels() {
    let f = Fixture::new();
    let fixture = include_str!("../../../eval/rag/test/notes/isolation-smoke.md");
    let labels = include_str!("../../../eval/rag/test/labels.jsonl");
    let outcome = storage::propose_and_commit(
        &f.vault,
        fixture,
        Version(0),
        Uuid::new_v4(),
        &HashSet::new(),
        &HashSet::new(),
        &policy::Gate::new(policy::SecretPolicy::new(1)),
        Some(&f.pii),
    )
    .unwrap();
    assert!(matches!(outcome, storage::CommitResult::Committed(_)));
    // Deliberately place a label copy under the vault to challenge accidental
    // recursive walks; ordinary runner operation rejects this layout.
    let adversarial = f.root.join("eval/rag/test");
    std::fs::create_dir_all(&adversarial).unwrap();
    std::fs::write(adversarial.join("labels.jsonl"), labels).unwrap();
    std::fs::write(adversarial.join("labels.record"), labels).unwrap();
    for _ in 0..2 {
        let fresh = RealIndexer::new();
        rebuild_index(&f.vault, &fresh).unwrap();
        let idx = Index::new(
            &f.vault,
            &fresh,
            "project-x",
            chrono::NaiveDate::from_ymd_opt(2026, 10, 30).unwrap(),
        );
        for marker in ["HELDOUT_QUESTION_CANARY_901", "HELDOUT_ANSWER_CANARY_902"] {
            assert!(labels.contains(marker));
            assert_eq!(
                search(&idx, &allow(), context(), query(marker))
                    .result
                    .unwrap()
                    .outcome,
                SearchOutcome::NotPresent
            );
        }
        assert!(matches!(
            search(&idx, &allow(), context(), query("rag-visible-needle"))
                .result
                .unwrap()
                .outcome,
            SearchOutcome::Matches { .. }
        ));
    }
}

#[test]
fn every_nonempty_relation_kind_is_reported_as_not_revalidated() {
    let f = Fixture::new();
    for kind in [
        "supports",
        "contradicts",
        "derived_from",
        "supersedes",
        "applies_to",
    ] {
        let id = format!("note-{kind}");
        let text = format!("---\nid: {id}\nschema_version: 1\ntype: bug\nstatus: draft\ntrust: internal\nclassification: internal\nproject: project-x\nsource: fixture\ncreated: 2026-10-30\nupdated: 2026-10-30\nevent_period: 2026-10-30\nrelations:\n  {kind}: [pending-target]\n---\nneedle");
        storage::propose_and_commit(
            &f.vault,
            &text,
            Version(0),
            Uuid::new_v4(),
            &HashSet::new(),
            &HashSet::from(["pending-target".into()]),
            &policy::Gate::new(policy::SecretPolicy::new(1)),
            Some(&f.pii),
        )
        .unwrap();
        assert_eq!(
            f.get(&id).result.unwrap().references,
            index::search::ReferenceState::NotRevalidated
        );
    }
}

#[test]
fn errors_have_nonempty_sanitized_diagnostics() {
    for error in [
        ApiError::UnsupportedSchema,
        ApiError::PermissionDenied,
        ApiError::UnsupportedChunk,
        ApiError::NotFound,
        ApiError::Deleted,
        ApiError::Inconsistent,
        ApiError::ReadUnavailable,
        ApiError::Search(index::search::SearchError::EmptyQuery),
    ] {
        assert!(!error.to_string().trim().is_empty());
        assert!(!error.to_string().contains("CANARY"));
    }
}
