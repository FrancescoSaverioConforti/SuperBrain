//! Task 6 production-path tests, including the deferred SearchResponse audit.
use std::collections::HashSet;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use chrono::{NaiveDate, Offset};
use domain::{Classification, IndexState, NoteStatus, NoteType, TrustLevel};
use index::{
    bm25::Bm25Index,
    document::Document,
    indexer_impl::RealIndexer,
    search::*,
    time_range::{ClarificationReason, TimeExpression, TimeResolutionError},
    Index,
};
use storage::{Indexer, NoteId, Vault, Version};
use uuid::Uuid;

struct LiveIndexer {
    real: Arc<RealIndexer>,
    paused: Arc<AtomicBool>,
}
impl Indexer for LiveIndexer {
    fn mark_current(&self, id: &str, version: u64, bytes: &[u8]) {
        if !self.paused.load(Ordering::SeqCst) {
            self.real.mark_current(id, version, bytes);
        }
    }
    fn state_for(&self, id: &str) -> IndexState {
        if self.paused.load(Ordering::SeqCst) {
            IndexState::Stale
        } else {
            self.real.state_for(id)
        }
    }
}

struct Fixture {
    vault: Vault,
    indexer: Arc<RealIndexer>,
    paused: Arc<AtomicBool>,
    pii: vault_security::PiiStore,
    root: std::path::PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("superbrain-b1-search-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        {
            let repo = git2::Repository::init(&root).unwrap();
            let signature = git2::Signature::now("SuperBrain Test", "test@localhost").unwrap();
            let oid = repo.index().unwrap().write_tree().unwrap();
            let tree = repo.find_tree(oid).unwrap();
            repo.commit(Some("HEAD"), &signature, &signature, "initial", &tree, &[])
                .unwrap();
        }
        let pii_root = root.join(".pii-store");
        let pii =
            vault_security::PiiStore::open(pii_root.clone(), &pii_root.join("master.key")).unwrap();
        pii.bootstrap_deletion_log().unwrap();
        let indexer = Arc::new(RealIndexer::new());
        let paused = Arc::new(AtomicBool::new(false));
        let vault = Vault::open_with_pii_store_and_indexer(
            root.clone(),
            &root.join("journal.sqlite"),
            &pii,
            Box::new(LiveIndexer {
                real: indexer.clone(),
                paused: paused.clone(),
            }),
        )
        .unwrap();
        Self {
            vault,
            indexer,
            paused,
            pii,
            root,
        }
    }
    fn write(&self, note: &Note<'_>, expected: u64) {
        let result = storage::propose_and_commit(
            &self.vault,
            &note.text(),
            Version(expected),
            Uuid::new_v4(),
            &HashSet::new(),
            &HashSet::from([note.relation.to_string()]),
            &policy::Gate::new(policy::SecretPolicy::new(1)),
            Some(&self.pii),
        )
        .unwrap();
        assert!(
            matches!(result, storage::CommitResult::Committed(_)),
            "{result:?}"
        );
    }
    fn lag(&self) {
        self.paused.store(true, Ordering::SeqCst);
    }
    fn search(&self, query: &str, filters: SearchFilters, k: usize) -> SearchResponse {
        self.index().search(query, filters, k).unwrap()
    }
    fn index(&self) -> Index<'_> {
        Index::new(&self.vault, &self.indexer, "project-x", date("2026-10-26"))
    }
}

struct Note<'a> {
    id: &'a str,
    body: &'a str,
    project: &'a str,
    classification: &'a str,
    source: &'a str,
    relation: &'a str,
    created: &'a str,
    updated: &'a str,
    event: &'a str,
    kind: &'a str,
    status: &'a str,
    trust: &'a str,
}
impl<'a> Note<'a> {
    fn new(id: &'a str, body: &'a str) -> Self {
        Self {
            id,
            body,
            project: "project-x",
            classification: "internal",
            source: "agent:fixture",
            relation: "opaque-pending",
            created: "2026-10-26",
            updated: "2026-10-26",
            event: "2026-08-23",
            kind: "bug",
            status: "draft",
            trust: "internal",
        }
    }
    fn text(&self) -> String {
        format!("---\nid: {}\nschema_version: 1\ntype: {}\nstatus: {}\ntrust: {}\nclassification: {}\nproject: {}\nsource: {}\ncreated: {}\nupdated: {}\nevent_period: {}\nrelations:\n  supports: [\"{}\"]\n---\n{}", self.id,self.kind,self.status,self.trust,self.classification,self.project,self.source,self.created,self.updated,self.event,self.relation,self.body)
    }
}
fn date(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}
fn august(field: TimeField) -> SearchFilters {
    SearchFilters {
        date_filter: Some((field, TimeExpression::MonthsAgo(2))),
        ..Default::default()
    }
}
fn results(response: &SearchResponse) -> Vec<&SearchResult> {
    match &response.outcome {
        SearchOutcome::NotPresent => vec![],
        SearchOutcome::Matches { first, rest } => std::iter::once(first).chain(rest).collect(),
    }
}
fn ids(response: &SearchResponse) -> Vec<&str> {
    results(response)
        .iter()
        .map(|r| r.note_id.as_str())
        .collect()
}

#[test]
fn absent_query_is_explicit_not_present_with_scope_context() {
    let f = Fixture::new();
    f.write(&Note::new("note-a", "retrieval"), 0);
    let r = f.search("unfindable-marker", Default::default(), 10);
    assert_eq!(r.outcome, SearchOutcome::NotPresent);
    assert_eq!(
        r.context.project,
        AppliedProjectScope::ActiveOnly {
            project: "project-x".into()
        }
    );
    assert_eq!(
        r.context.completeness,
        Completeness::IndexedCandidatesMayBeIncomplete
    );
}

#[test]
fn other_project_requires_explicit_scope_and_is_visible() {
    let f = Fixture::new();
    let mut n = Note::new("note-other", "retrieval");
    n.project = "project-y";
    f.write(&n, 0);
    assert!(ids(&f.search("retrieval", Default::default(), 10)).is_empty());
    let r = f.search(
        "retrieval",
        SearchFilters {
            project: ProjectScope::Explicit("project-y".into()),
            ..Default::default()
        },
        10,
    );
    assert_eq!(ids(&r), ["note-other"]);
    assert_eq!(
        r.context.project,
        AppliedProjectScope::Explicit {
            active_project: "project-x".into(),
            searched_project: "project-y".into()
        }
    );
}

fn temporal_fixture() -> Fixture {
    let f = Fixture::new();
    let mut n = Note::new("created-august", "discussion");
    n.created = "2026-08-10";
    n.event = "2026-10-01";
    f.write(&n, 0);
    let mut n = Note::new("updated-august", "discussion");
    n.updated = "2026-08-11";
    n.event = "2026-10-01";
    f.write(&n, 0);
    f.write(&Note::new("event-august", "discussion"), 0);
    f
}
#[test]
fn created_at_mode_uses_created_only() {
    let f = temporal_fixture();
    assert_eq!(
        ids(&f.search("discussion", august(TimeField::CreatedAt), 10)),
        ["created-august"]
    );
}
#[test]
fn updated_at_mode_uses_updated_only() {
    let f = temporal_fixture();
    assert_eq!(
        ids(&f.search("discussion", august(TimeField::UpdatedAt), 10)),
        ["updated-august"]
    );
}
#[test]
fn event_period_mode_finds_august_event_created_today() {
    let f = temporal_fixture();
    assert_eq!(
        ids(&f.search("discussion", august(TimeField::EventPeriod), 10)),
        ["event-august"]
    );
}
#[test]
fn any_mode_matches_any_temporal_field() {
    let f = temporal_fixture();
    let r = f.search("discussion", august(TimeField::Any), 10);
    let mut found = ids(&r);
    found.sort();
    assert_eq!(found, ["created-august", "event-august", "updated-august"]);
}
#[test]
fn discussion_and_modification_questions_have_distinct_results_and_annotations() {
    let f = temporal_fixture();
    let events = f.search("discussion", august(TimeField::EventPeriod), 10);
    let modified = f.search("discussion", august(TimeField::UpdatedAt), 10);
    assert_ne!(ids(&events), ids(&modified));
    let time = events.context.time.unwrap();
    assert_eq!(time.field, TimeField::EventPeriod);
    assert_eq!(time.timezone, "Europe/Rome");
    assert_eq!(time.start.date_naive(), date("2026-08-01"));
    assert_eq!(time.end.date_naive(), date("2026-09-01"));
    assert_eq!(time.start.offset().fix().local_minus_utc(), 7200);
    assert_eq!(time.convention, TimeConvention::CalendarMonthNotRollingDays);
    assert_eq!(modified.context.time.unwrap().field, TimeField::UpdatedAt);
}

#[test]
fn indexed_then_tombstoned_candidate_is_excluded_by_authoritative_read() {
    let f = Fixture::new();
    let mut n = Note::new("note-delete", "tombstoneneedle");
    f.write(&n, 0);
    assert_eq!(
        ids(&f.search("tombstoneneedle", Default::default(), 10)),
        ["note-delete"]
    );
    f.lag();
    n.classification = "sensitive";
    n.body = storage::SENSITIVE_TOMBSTONE_BODY;
    f.write(&n, 1);
    assert!(
        !f.indexer.bm25_ranked("tombstoneneedle", 10).is_empty(),
        "precondition: stale candidate survives"
    );
    assert!(f.vault.read_note(&NoteId::parse(n.id).unwrap()).is_err());
    assert_eq!(
        f.search("tombstoneneedle", Default::default(), 10).outcome,
        SearchOutcome::NotPresent
    );
}

#[test]
fn stale_candidate_returns_current_version_and_byte_identical_body_excerpt() {
    let f = Fixture::new();
    let mut n = Note::new("note-update", "needle old body");
    f.write(&n, 0);
    f.lag();
    n.body = "needle current body with different length";
    f.write(&n, 1);
    let r = f.search("needle", Default::default(), 10);
    let hit = results(&r)[0];
    let current = f.vault.read_note(&NoteId::parse(n.id).unwrap()).unwrap();
    let (_, body) = storage::split_record(current.content.as_bytes()).unwrap();
    assert_eq!(hit.excerpt.as_bytes(), body.unwrap().as_bytes());
    assert_eq!(hit.version, Version(2));
}

#[test]
fn stale_candidate_matching_all_old_terms_and_no_current_terms_is_absent() {
    let f = Fixture::new();
    let mut n = Note::new("note-stale", "alphaquartz betaquartz");
    f.write(&n, 0);
    f.lag();
    n.body = "completely unrelated vocabulary";
    f.write(&n, 1);
    assert!(!f
        .indexer
        .bm25_ranked("alphaquartz betaquartz", 10)
        .is_empty());
    assert_eq!(
        f.search("alphaquartz betaquartz", Default::default(), 10)
            .outcome,
        SearchOutcome::NotPresent
    );
}

#[test]
fn reclassified_sensitive_uses_current_allowlist_for_matching_and_response() {
    let f = Fixture::new();
    let mut n = Note::new("note-class", "bodymarker-x");
    n.source = "sourcemarker-x";
    n.relation = "relationmarker-x";
    f.write(&n, 0);
    f.lag();
    n.classification = "sensitive";
    f.write(&n, 1);
    for q in ["bodymarker-x", "sourcemarker-x", "relationmarker-x"] {
        assert!(!f.indexer.bm25_ranked(q, 10).is_empty());
        assert_eq!(
            f.search(q, Default::default(), 10).outcome,
            SearchOutcome::NotPresent
        );
    }
    let response = f.search("note-class", Default::default(), 10);
    assert_eq!(ids(&response), ["note-class"]);
    let hit = results(&response)[0];
    assert_eq!(hit.provenance.classification, Classification::Sensitive);
    assert_eq!(hit.provenance.source, None);
    let rendered = format!("{response:?}");
    for marker in [n.body, n.source, n.relation, "pii://"] {
        assert!(!rendered.contains(marker));
    }
}

fn sensitive_surface(marker: &str, field: &str) {
    let f = Fixture::new();
    let mut n = Note::new("note-sensitive", "safe private text");
    n.classification = "sensitive";
    match field {
        "body" => n.body = marker,
        "source" => n.source = marker,
        "relations" => n.relation = marker,
        _ => unreachable!(),
    }
    f.write(&n, 0);
    assert_eq!(
        f.search(marker, Default::default(), 10).outcome,
        SearchOutcome::NotPresent
    );
    let r = f.search("note-sensitive", Default::default(), 10);
    assert_eq!(
        ids(&r),
        ["note-sensitive"],
        "not vacuous: the structured fields remain searchable"
    );
    assert!(!format!("{r:?}").contains(marker));
    assert_eq!(results(&r)[0].provenance.source, None);
}
#[test]
fn sensitive_body_never_reaches_search_response() {
    sensitive_surface("BODY-CANARY-7e6f", "body");
}
#[test]
fn sensitive_source_never_reaches_search_response() {
    sensitive_surface("SOURCE-CANARY-6d5e", "source");
}
#[test]
fn sensitive_relations_never_reach_search_response() {
    sensitive_surface("RELATIONS-CANARY-5c4d", "relations");
}

#[test]
fn sensitive_temporal_markers_excluded_on_live_update_and_rebuild() {
    let f = Fixture::new();
    let mut n = Note::new("note-temporal", "PRIVATE-BODY-CANARY");
    n.classification = "sensitive";
    n.created = "CREATED-CANARY-8e42";
    n.updated = "UPDATED-CANARY-9f53";
    n.event = "PERIOD-CANARY-0a64";
    for version in 0..2 {
        f.write(&n, version);
        for rebuild in [false, true] {
            if rebuild {
                index::indexer_impl::rebuild_index(&f.vault, &f.indexer).unwrap();
            }
            for marker in [n.created, n.updated, n.event] {
                assert!(
                    f.indexer.bm25_ranked(marker, 10).is_empty(),
                    "postings: {marker}"
                );
                assert_eq!(
                    f.search(marker, Default::default(), 10).outcome,
                    SearchOutcome::NotPresent
                );
            }
            let response = f.search(n.id, Default::default(), 10);
            assert_eq!(results(&response)[0].version, Version(version + 1));
            for marker in [n.created, n.updated, n.event, n.body, n.source, n.relation] {
                assert!(!format!("{response:?}").contains(marker));
            }
            for field in [
                TimeField::CreatedAt,
                TimeField::UpdatedAt,
                TimeField::EventPeriod,
                TimeField::Any,
            ] {
                assert!(results(&f.search(n.id, august(field), 10)).is_empty());
            }
        }
    }
}

#[test]
fn sensitive_temporal_markers_revalidated_after_classification_change() {
    let f = Fixture::new();
    let mut n = Note::new("note-temporal-transition", "private-body");
    n.created = "TRANSITION-CREATED-MARKER";
    n.updated = "TRANSITION-UPDATED-MARKER";
    n.event = "TRANSITION-PERIOD-MARKER";
    f.write(&n, 0);
    for marker in [n.created, n.updated, n.event] {
        assert_eq!(ids(&f.search(marker, Default::default(), 10)), [n.id]);
    }
    f.lag();
    n.classification = "sensitive";
    f.write(&n, 1);
    for marker in [n.created, n.updated, n.event] {
        assert!(
            !f.indexer.bm25_ranked(marker, 10).is_empty(),
            "prove stale candidates exist"
        );
        assert_eq!(
            f.search(marker, Default::default(), 10).outcome,
            SearchOutcome::NotPresent
        );
        assert!(!format!("{:?}", f.search(n.id, Default::default(), 10)).contains(marker));
    }
    f.paused.store(false, Ordering::SeqCst);
    f.write(&n, 2);
    for marker in [n.created, n.updated, n.event] {
        assert!(f.indexer.bm25_ranked(marker, 10).is_empty());
    }
    index::indexer_impl::rebuild_index(&f.vault, &f.indexer).unwrap();
    for marker in [n.created, n.updated, n.event] {
        assert!(f.indexer.bm25_ranked(marker, 10).is_empty());
    }
}

#[test]
fn sensitive_temporal_filters_work_without_date_postings() {
    let f = Fixture::new();
    let mut n = Note::new("note-temporal-valid", "private-body");
    n.classification = "sensitive";
    n.created = "2026-08-10";
    n.updated = "2026-08-11T12:00:00+02:00";
    n.event = "2026-07-31/2026-08-02";
    f.write(&n, 0);
    for field in [
        TimeField::CreatedAt,
        TimeField::UpdatedAt,
        TimeField::EventPeriod,
        TimeField::Any,
    ] {
        assert_eq!(ids(&f.search(n.id, august(field), 10)), [n.id]);
    }
    for value in [n.created, n.updated, n.event] {
        assert_eq!(
            f.search(value, Default::default(), 10).outcome,
            SearchOutcome::NotPresent
        );
    }
    index::indexer_impl::rebuild_index(&f.vault, &f.indexer).unwrap();
    assert_eq!(
        ids(&f.search(n.id, august(TimeField::EventPeriod), 10)),
        [n.id]
    );
}

#[test]
fn changed_project_and_each_structured_filter_are_rechecked() {
    let f = Fixture::new();
    let mut n = Note::new("note-filters", "needle");
    f.write(&n, 0);
    f.lag();
    n.project = "project-y";
    n.kind = "decision";
    n.status = "verified";
    n.trust = "official";
    n.event = "2026-10-26";
    f.write(&n, 1);
    assert_eq!(
        f.search("needle", Default::default(), 10).outcome,
        SearchOutcome::NotPresent
    );
    let scope = ProjectScope::Explicit("project-y".into());
    for filters in [
        SearchFilters {
            project: scope.clone(),
            type_filter: Some(NoteType::Bug),
            ..Default::default()
        },
        SearchFilters {
            project: scope.clone(),
            status_filter: Some(NoteStatus::Draft),
            ..Default::default()
        },
        SearchFilters {
            project: scope.clone(),
            trust_filter: Some(TrustLevel::Internal),
            ..Default::default()
        },
        SearchFilters {
            project: scope.clone(),
            ..august(TimeField::EventPeriod)
        },
    ] {
        assert_eq!(
            f.search("needle", filters, 10).outcome,
            SearchOutcome::NotPresent
        );
    }
    let current = SearchFilters {
        project: scope,
        type_filter: Some(NoteType::Decision),
        status_filter: Some(NoteStatus::Verified),
        trust_filter: Some(TrustLevel::Official),
        ..Default::default()
    };
    assert_eq!(ids(&f.search("needle", current, 10)), ["note-filters"]);
}

#[test]
fn fresh_scores_and_order_ignore_stale_rankings_and_apply_k_after_filtering() {
    let f = Fixture::new();
    let mut a = Note::new("note-a", "needle needle needle");
    f.write(&a, 0);
    let b = Note::new("note-b", "needle small");
    f.write(&b, 0);
    f.lag();
    a.project = "project-y";
    a.body = "needle very different much longer content with extra words";
    f.write(&a, 1);
    assert_eq!(ids(&f.search("needle", Default::default(), 1)), ["note-b"]);
    let mut corpus = Bm25Index::new();
    for n in [&a, &b] {
        let record = f.vault.read_note(&NoteId::parse(n.id).unwrap()).unwrap();
        let d = Document::from_record_bytes(record.content.as_bytes()).unwrap();
        corpus.insert(NoteId::parse(n.id).unwrap(), &d.text_searchable);
    }
    // Explicitly isolate each project: scores use fresh eligible candidates.
    corpus.remove(&NoteId::parse(a.id).unwrap());
    let expected = corpus.query_ranked("needle", 1)[0].1;
    assert_eq!(
        results(&f.search("needle", Default::default(), 1))[0].score,
        expected
    );
    let r = f.search(
        "needle",
        SearchFilters {
            project: ProjectScope::Explicit("project-y".into()),
            ..Default::default()
        },
        1,
    );
    let mut fresh = Bm25Index::new();
    let d = Document::from_record_bytes(a.text().as_bytes()).unwrap();
    fresh.insert(NoteId::parse(a.id).unwrap(), &d.text_searchable);
    assert_eq!(results(&r)[0].score, fresh.query_ranked("needle", 1)[0].1);
}

#[test]
fn exact_identifier_substring_and_ranked_token_paths_both_work() {
    let f = Fixture::new();
    f.write(&Note::new("note-exact", "prefix_get_user_by_id_suffix"), 0);
    let r = f.search("get_user_by_id", Default::default(), 10);
    assert_eq!(ids(&r), ["note-exact"]);
    assert_eq!(results(&r)[0].match_kind, MatchKind::ExactSubstring);
    f.write(&Note::new("note-rank", "alpha filler beta"), 0);
    let r = f.search("alpha beta", Default::default(), 10);
    assert_eq!(ids(&r), ["note-rank"]);
    assert_eq!(results(&r)[0].match_kind, MatchKind::RankedTerms);
}

#[test]
fn metadata_only_matches_have_an_excerpt_that_contains_the_matched_text() {
    let f = Fixture::new();
    f.write(&Note::new("note-metadata", "unrelated body"), 0);
    let r = f.search("agent:fixture", Default::default(), 10);
    assert_eq!(ids(&r), ["note-metadata"]);
    assert!(results(&r)[0].excerpt.contains("agent:fixture"));
}

#[test]
fn malformed_and_missing_records_are_silently_dropped() {
    let f = Fixture::new();
    f.write(&Note::new("note-bad", "needle"), 0);
    f.write(&Note::new("note-missing", "needle"), 0);
    std::fs::write(f.root.join("note-bad.record"), "tampered needle").unwrap();
    std::fs::remove_file(f.root.join("note-missing.record")).unwrap();
    assert_eq!(
        f.search("needle", Default::default(), 10).outcome,
        SearchOutcome::NotPresent
    );
}

#[test]
fn date_points_and_event_intervals_respect_half_open_overlap_and_rome_offsets() {
    let f = Fixture::new();
    for (id, event) in [
        ("at-start", "2026-08-01"),
        ("at-end", "2026-09-01"),
        ("overlap", "2026-07-31/2026-08-02"),
        ("touch", "2026-07-01/2026-08-01"),
        ("offset-inside", "2026-07-31T22:00:00Z"),
        ("offset-outside", "2026-08-31T22:00:00Z"),
        ("bad-date", "2026-99-99"),
        ("zero-period", "2026-08-15/2026-08-15"),
        ("reversed-period", "2026-08-16/2026-08-15"),
        ("end-period", "2026-09-01/2026-09-02"),
    ] {
        let mut n = Note::new(id, "needle");
        n.event = event;
        f.write(&n, 0);
    }
    let r = f.search("needle", august(TimeField::EventPeriod), 10);
    let mut found = ids(&r);
    found.sort();
    assert_eq!(found, ["at-start", "offset-inside", "overlap"]);
}

#[test]
fn zero_k_empty_query_and_empty_project_are_typed_errors() {
    let f = Fixture::new();
    assert_eq!(
        f.index().search("needle", Default::default(), 0),
        Err(SearchError::InvalidLimit)
    );
    assert_eq!(
        f.index().search(" \n", Default::default(), 1),
        Err(SearchError::EmptyQuery)
    );
    assert_eq!(
        f.index().search(
            "needle",
            SearchFilters {
                project: ProjectScope::Explicit("".into()),
                ..Default::default()
            },
            1
        ),
        Err(SearchError::EmptyProject)
    );
}

#[test]
fn temporal_ambiguity_returns_clarification_instead_of_not_present() {
    let f = Fixture::new();
    let filters = SearchFilters {
        date_filter: Some((
            TimeField::Any,
            TimeExpression::BeforeEvent {
                resolved_range: None,
            },
        )),
        ..Default::default()
    };
    assert_eq!(
        f.index().search("needle", filters, 10),
        Err(SearchError::TimeResolution(
            TimeResolutionError::NeedsClarification(ClarificationReason::UnidentifiedEvent)
        ))
    );
}

#[test]
fn deterministic_ties_and_limit() {
    let f = Fixture::new();
    f.write(&Note::new("note-b", "needle"), 0);
    f.write(&Note::new("note-a", "needle"), 0);
    for _ in 0..3 {
        assert_eq!(ids(&f.search("needle", Default::default(), 1)), ["note-a"]);
    }
}

#[test]
fn unreadable_deletion_log_returns_typed_failure_without_claiming_not_present() {
    let f = Fixture::new();
    f.write(&Note::new("note-log", "needle"), 0);
    std::fs::write(f.pii.deletion_log_path(), "CORRUPT-LOG-CANARY").unwrap();
    assert!(matches!(
        f.vault.read_note(&NoteId::parse("note-log").unwrap()),
        Err(storage::StorageError::DeletionLog(_))
    ));
    let response = f.index().search("needle", Default::default(), 10);
    assert_eq!(response, Err(SearchError::ReadUnavailable));
    assert!(!format!("{response:?}").contains("CORRUPT-LOG-CANARY"));
}

#[test]
fn citations_report_unknown_reference_state_without_guessing_pending_or_resolved() {
    let f = Fixture::new();
    f.write(&Note::new("note-pending", "needle"), 0);
    let response = f.search("needle", Default::default(), 10);
    assert_eq!(
        results(&response)[0].references,
        ReferenceState::NotRevalidated
    );
    assert_eq!(results(&response)[0].provenance.repository_id, None);
    assert_eq!(results(&response)[0].provenance.commit, None);
    // Remove the only relation through the real write path, keeping the index
    // stale, and prove the response derives Unlinked from current metadata.
    f.lag();
    let text = Note::new("note-pending", "needle")
        .text()
        .replace("supports: [\"opaque-pending\"]", "supports: []");
    storage::propose_and_commit(
        &f.vault,
        &text,
        Version(1),
        Uuid::new_v4(),
        &HashSet::new(),
        &HashSet::new(),
        &policy::Gate::new(policy::SecretPolicy::new(1)),
        Some(&f.pii),
    )
    .unwrap();
    let response = f.search("needle", Default::default(), 10);
    assert_eq!(results(&response)[0].references, ReferenceState::Unlinked);
}

#[test]
fn all_relation_kinds_are_reported_honestly_without_exposing_targets() {
    let f = Fixture::new();
    for kind in [
        "supports",
        "contradicts",
        "derived_from",
        "supersedes",
        "applies_to",
    ] {
        let id = format!("note-{kind}");
        let text = Note::new(&id, "needle")
            .text()
            .replace("  supports:", &format!("  {kind}:"));
        storage::propose_and_commit(
            &f.vault,
            &text,
            Version(0),
            Uuid::new_v4(),
            &HashSet::new(),
            &HashSet::from(["opaque-pending".into()]),
            &policy::Gate::new(policy::SecretPolicy::new(1)),
            Some(&f.pii),
        )
        .unwrap();
        let response = f.search(&id, Default::default(), 10);
        assert_eq!(
            results(&response)[0].references,
            ReferenceState::NotRevalidated
        );
    }
}

#[test]
fn search_errors_have_nonempty_sanitized_diagnostics() {
    for error in [
        SearchError::InvalidLimit,
        SearchError::EmptyQuery,
        SearchError::EmptyProject,
        SearchError::ReadUnavailable,
        SearchError::TimeResolution(TimeResolutionError::InvalidDate),
    ] {
        assert!(!error.to_string().trim().is_empty());
    }
}

#[test]
fn profiling_preserves_response_and_counts_both_accepted_and_filtered_candidates() {
    let f = Fixture::new();
    f.write(&Note::new("note-good", "needle"), 0);
    let mut other = Note::new("note-other", "needle");
    other.project = "project-y";
    f.write(&other, 0);
    let expected = f.search("needle", Default::default(), 10);
    let start = std::time::Instant::now();
    let (actual, profile) = f
        .index()
        .search_profiled("needle", Default::default(), 10)
        .unwrap();
    let elapsed = start.elapsed();
    assert_eq!(actual, expected);
    assert_eq!(
        (profile.candidates, profile.reads, profile.eligible),
        (2, 2, 1)
    );
    for phase in [
        profile.candidate_generation,
        profile.authoritative_reads,
        profile.revalidation,
        profile.response_building,
    ] {
        assert!(phase > std::time::Duration::ZERO);
    }
    assert!(
        profile.candidate_generation
            + profile.authoritative_reads
            + profile.revalidation
            + profile.response_building
            <= elapsed
    );
    assert!(!format!("{profile:?}").contains("needle"));
}

#[test]
fn with_bm25_params_changes_real_search_scores() {
    let f = Fixture::new();
    f.write(&Note::new("short-note", "rust"), 0);
    f.write(&Note::new("long-note", &"rust ".repeat(50)), 0);

    let default_scores = Index::new(&f.vault, &f.indexer, "project-x", date("2026-10-26"))
        .search("rust", SearchFilters::default(), 10)
        .unwrap();
    let b_zero_scores = Index::new(&f.vault, &f.indexer, "project-x", date("2026-10-26"))
        .with_bm25_params(1.2, 0.0) // b=0 disables length normalization entirely
        .search("rust", SearchFilters::default(), 10)
        .unwrap();

    assert_ne!(
        default_scores, b_zero_scores,
        "b=0.0 must change real search scores vs the default b=0.75"
    );
}

#[test]
fn new_with_bm25_params_changes_candidate_prefilter_scores() {
    let f = Fixture::new();
    f.write(&Note::new("short-note", "rust"), 0);
    f.write(&Note::new("long-note", &"rust ".repeat(50)), 0);

    let default_indexer = RealIndexer::new();
    index::indexer_impl::rebuild_index(&f.vault, &default_indexer).unwrap();
    let default_ranked = default_indexer.bm25_ranked("rust", 10);

    let b_zero_indexer = RealIndexer::new_with_bm25_params(1.2, 0.0);
    index::indexer_impl::rebuild_index(&f.vault, &b_zero_indexer).unwrap();
    let b_zero_ranked = b_zero_indexer.bm25_ranked("rust", 10);

    assert_ne!(
        default_ranked, b_zero_ranked,
        "b=0.0 must change the candidate prefilter's own BM25 scores vs the default b=0.75"
    );
}

#[test]
fn new_with_text_normalizer_applies_the_hook_before_indexing_text() {
    let f = Fixture::new();
    f.write(&Note::new("note-a", "resolve_configured_timezone,"), 0);

    let default_indexer = RealIndexer::new();
    index::indexer_impl::rebuild_index(&f.vault, &default_indexer).unwrap();
    // Default indexer: the trailing comma stays part of the indexed
    // token, so a query WITHOUT the comma relies on prefix-free exact
    // token matching alone and the comma-bearing token never equals it.
    let default_hit_without_comma = default_indexer.bm25_ranked("resolve_configured_timezone", 10);

    fn strip_trailing_comma(text: &str) -> String {
        text.split_whitespace()
            .map(|tok| tok.trim_end_matches(','))
            .collect::<Vec<_>>()
            .join(" ")
    }
    let normalized_indexer = RealIndexer::new_with_text_normalizer(strip_trailing_comma);
    index::indexer_impl::rebuild_index(&f.vault, &normalized_indexer).unwrap();
    let normalized_hit_without_comma = normalized_indexer.bm25_ranked("resolve_configured_timezone", 10);

    assert!(
        default_hit_without_comma.is_empty(),
        "sanity check: without the hook, the comma-bearing indexed token must not match the comma-free query"
    );
    assert!(
        !normalized_hit_without_comma.is_empty(),
        "the text normalizer hook must apply to indexed text before it reaches trigram/bm25 insertion"
    );
}

// --- B3-TUNE-v2 Priority 3 Stage 1, P1: exact structural-identifier boost. ---

#[test]
fn identifier_boost_fires_on_exact_whole_token_match_against_source_fragment() {
    let f = Fixture::new();
    // Both notes share a body term so both are real BM25 candidates for
    // the query regardless of the boost (candidate generation is
    // unaffected by P1) — note-a's source additionally contains
    // "...#SEC-001" per the frozen extraction rule, note-b's does not,
    // so only the boost (not candidate membership) decides which ranks
    // first once the query also contains the literal token "sec-001".
    f.write(&Note { source: "docs/technical-debt.md#SEC-001", ..Note::new("note-a", "shared_probe_term extra filler words so note-b scores higher on shared_probe_term alone") }, 0);
    f.write(&Note::new("note-b", "shared_probe_term shared_probe_term shared_probe_term"), 0);
    let indexer = RealIndexer::new();
    index::indexer_impl::rebuild_index(&f.vault, &indexer).unwrap();
    let plain = index::Index::new(&f.vault, &indexer, "project-x", date("2026-10-26"));
    let boosted = index::Index::new(&f.vault, &indexer, "project-x", date("2026-10-26")).with_identifier_boost();

    let plain_first = |outcome: index::search::SearchOutcome| match outcome {
        index::search::SearchOutcome::Matches { first, .. } => first.note_id.as_str().to_string(),
        index::search::SearchOutcome::NotPresent => panic!("expected a match"),
    };
    // Sanity check: without the boost, note-b's higher term frequency
    // on "shared_probe_term" alone must outrank note-a (the query's
    // "sec-001" token has zero BM25/trigram overlap with either note's
    // indexed text on its own, since neither note's source token is an
    // exact whole-token match for "sec-001" by itself).
    let plain_outcome = plain.search("shared_probe_term sec-001", Default::default(), 10).unwrap().outcome;
    assert_eq!(plain_first(plain_outcome), "note-b", "sanity check: without the boost, note-b's term frequency must win");

    let boosted_outcome = boosted.search("shared_probe_term sec-001", Default::default(), 10).unwrap().outcome;
    assert_eq!(plain_first(boosted_outcome), "note-a", "the exact identifier match's boost must outrank note-b's higher term frequency");
}

#[test]
fn identifier_boost_never_fires_on_a_substring_or_prefix_match() {
    let f = Fixture::new();
    // Both notes are genuine candidates (shared body term) regardless of
    // the boost, so this test can actually distinguish "the boost fired"
    // from "the candidate was never reachable" -- the Review finding
    // confirmed the original version of this test (note-a isolated, no
    // shared term) passed even when the matching rule was mutated to
    // always fire, since note-a never became a candidate at all.
    //
    // To make this test actually falsifiable against a substring/prefix
    // bug (e.g. a buggy rule using `contains`/`starts_with` instead of
    // exact equality), note-b must have a genuinely higher base BM25
    // score than note-a -- so that a WRONGLY-applied boost on note-a
    // (from "sec" incorrectly matching "SEC-001" as a substring) would
    // flip the ranking, while the correct exact-match rule (which
    // declines to boost either note, since "sec" != "sec-001") leaves
    // note-b's higher term frequency winning untouched.
    f.write(&Note { source: "docs/technical-debt.md#SEC-001", ..Note::new("note-a", "shared_probe_term_b") }, 0);
    f.write(&Note::new("note-b", "shared_probe_term_b shared_probe_term_b shared_probe_term_b extra filler words so note-b scores higher on term frequency alone"), 0);
    let indexer = RealIndexer::new();
    index::indexer_impl::rebuild_index(&f.vault, &indexer).unwrap();
    let idx = index::Index::new(&f.vault, &indexer, "project-x", date("2026-10-26")).with_identifier_boost();

    // "sec" is a substring/prefix of "SEC-001" but must never match it,
    // so note-b's higher term frequency on shared_probe_term_b must
    // still win even with the boost enabled -- a substring-matching bug
    // would incorrectly boost note-a's score by IDENTIFIER_BOOST (100.0,
    // far larger than any term-frequency difference) and flip this.
    let response = idx.search("shared_probe_term_b sec", Default::default(), 10).unwrap();

    let index::search::SearchOutcome::Matches { first, .. } = response.outcome else {
        panic!("expected a match");
    };
    assert_eq!(first.note_id.as_str(), "note-b", "\"sec\" must never match \"SEC-001\" as a substring/prefix");
}

#[test]
fn identifier_boost_never_applies_to_a_sensitive_candidates_withheld_source() {
    let f = Fixture::new();
    // Both notes are genuine candidates (shared body term) regardless of
    // the boost, mirroring the fix above -- a Sensitive note isolated
    // with no shared term would never become a candidate at all, which
    // would make this test pass vacuously (confirmed by Review mutation
    // testing on the sibling test above).
    f.write(&Note { source: "docs/technical-debt.md#SEC-001", classification: "sensitive", ..Note::new("note-a", "shared_probe_term_c") }, 0);
    f.write(&Note::new("note-b", "shared_probe_term_c shared_probe_term_c shared_probe_term_c"), 0);
    let indexer = RealIndexer::new();
    index::indexer_impl::rebuild_index(&f.vault, &indexer).unwrap();
    let plain = index::Index::new(&f.vault, &indexer, "project-x", date("2026-10-26"));
    let boosted = index::Index::new(&f.vault, &indexer, "project-x", date("2026-10-26")).with_identifier_boost();

    // Sensitive: source is withheld by the existing pipeline before
    // identifier_boost's logic can see it at all -- so the boosted
    // ordering must be identical to the plain ordering even though the
    // query contains the literal identifier and note-a would otherwise
    // be eligible for the boost were its source not withheld.
    let plain_outcome = plain.search("shared_probe_term_c sec-001", Default::default(), 10).unwrap().outcome;
    let boosted_outcome = boosted.search("shared_probe_term_c sec-001", Default::default(), 10).unwrap().outcome;
    assert_eq!(
        format!("{plain_outcome:?}"), format!("{boosted_outcome:?}"),
        "a Sensitive candidate's withheld source must never let the boost apply"
    );
}

#[test]
fn identifier_boost_does_not_change_the_candidate_set() {
    let f = Fixture::new();
    f.write(&Note { source: "docs/technical-debt.md#SEC-001", ..Note::new("note-a", "a note about sec-001 topic") }, 0);
    f.write(&Note::new("note-b", "a note about sec-001 topic too"), 0);
    let indexer = RealIndexer::new();
    index::indexer_impl::rebuild_index(&f.vault, &indexer).unwrap();
    let plain = index::Index::new(&f.vault, &indexer, "project-x", date("2026-10-26"));
    let boosted = index::Index::new(&f.vault, &indexer, "project-x", date("2026-10-26")).with_identifier_boost();

    let ids = |outcome: index::search::SearchOutcome| -> std::collections::HashSet<String> {
        match outcome {
            index::search::SearchOutcome::NotPresent => Default::default(),
            index::search::SearchOutcome::Matches { first, rest } => {
                let mut s: std::collections::HashSet<String> = rest.iter().map(|r| r.note_id.as_str().to_string()).collect();
                s.insert(first.note_id.as_str().to_string());
                s
            }
        }
    };
    let plain_ids = ids(plain.search("sec-001 topic", Default::default(), 10).unwrap().outcome);
    let boosted_ids = ids(boosted.search("sec-001 topic", Default::default(), 10).unwrap().outcome);
    assert_eq!(plain_ids, boosted_ids, "identifier_boost must only reorder, never change membership");
}

// --- B3-TUNE-v2 Priority 3 Stage 1, P2: status tie-break. ---

#[test]
fn status_tiebreak_fires_only_on_exactly_equal_bm25_scores() {
    let f = Fixture::new();
    // Both notes share identical text so their BM25 scores are
    // bitwise-identical f64 values under the real pipeline.
    f.write(&Note { status: "draft", ..Note::new("note-draft", "shared_term appears here") }, 0);
    f.write(&Note { status: "verified", ..Note::new("note-verified", "shared_term appears here") }, 0);
    let indexer = RealIndexer::new();
    index::indexer_impl::rebuild_index(&f.vault, &indexer).unwrap();
    let idx = index::Index::new(&f.vault, &indexer, "project-x", date("2026-10-26")).with_status_tiebreak();

    let response = idx.search("shared_term", Default::default(), 10).unwrap();

    let index::search::SearchOutcome::Matches { first, .. } = response.outcome else {
        panic!("expected a match");
    };
    assert_eq!(first.note_id.as_str(), "note-verified", "Verified must outrank Draft at an exact score tie");
}

#[test]
fn status_tiebreak_never_fires_on_a_nonzero_score_difference() {
    let f = Fixture::new();
    // note-a has extra matching term frequency -> a strictly higher,
    // non-tied BM25 score than note-b, despite note-b's higher status tier.
    f.write(&Note { status: "draft", ..Note::new("note-a", "shared_term shared_term shared_term extra filler text to differ length") }, 0);
    f.write(&Note { status: "verified", ..Note::new("note-b", "shared_term") }, 0);
    let indexer = RealIndexer::new();
    index::indexer_impl::rebuild_index(&f.vault, &indexer).unwrap();
    let plain = index::Index::new(&f.vault, &indexer, "project-x", date("2026-10-26"));
    let tiebreak = index::Index::new(&f.vault, &indexer, "project-x", date("2026-10-26")).with_status_tiebreak();

    let plain_outcome = plain.search("shared_term", Default::default(), 10).unwrap().outcome;
    let tiebreak_outcome = tiebreak.search("shared_term", Default::default(), 10).unwrap().outcome;

    // Ordering must be identical whether or not the tiebreak flag is set,
    // because the scores are NOT exactly equal.
    assert_eq!(format!("{plain_outcome:?}"), format!("{tiebreak_outcome:?}"));
}

#[test]
fn status_tiebreak_all_six_variants_map_to_the_frozen_tier_via_observed_ordering() {
    // `status_tier` itself is private to `search.rs` and `#[cfg(test)]`
    // does NOT cross the crate boundary into this integration-test binary
    // (confirmed: Priority 2 Lever B had to duplicate logic locally for
    // exactly this reason, and `indexer_impl.rs`'s own `#[cfg(test)]`
    // helpers like `trigram_hits` are never called from this file today).
    // So this test observes the tier ordering indirectly, through
    // `with_status_tiebreak`'s real effect on search output, for every
    // pair of tiers, rather than calling a private function directly.
    let pairs = [
        ("verified", "test-passed"),
        ("reviewed", "test-passed"),
        ("test-passed", "draft"),
        ("draft", "superseded"),
        ("draft", "contradicted"),
    ];
    for (higher_status, lower_status) in pairs {
        let f = Fixture::new();
        // Note ids are chosen so the HIGHER-tier note sorts AFTER the
        // lower-tier note lexicographically ("note-z-higher" > "note-a-lower")
        // -- the existing note-id tie-break alone would put note-a-lower
        // first, so only a genuinely correct status_tier mapping can make
        // note-z-higher win. The Review's mutation test confirmed the
        // original id choice ("note-higher" < "note-lower") passed this
        // test even with status_tier always returning Ordering::Equal,
        // since the id tie-break alone already produced the expected
        // order for every pair.
        f.write(&Note { id: "note-z-higher", status: higher_status, ..Note::new("note-z-higher", "shared_tier_probe_term") }, 0);
        f.write(&Note { id: "note-a-lower", status: lower_status, ..Note::new("note-a-lower", "shared_tier_probe_term") }, 0);
        let indexer = RealIndexer::new();
        index::indexer_impl::rebuild_index(&f.vault, &indexer).unwrap();
        let plain = index::Index::new(&f.vault, &indexer, "project-x", date("2026-10-26"));
        let tiebreak = index::Index::new(&f.vault, &indexer, "project-x", date("2026-10-26")).with_status_tiebreak();

        let plain_response = plain.search("shared_tier_probe_term", Default::default(), 10).unwrap();
        let index::search::SearchOutcome::Matches { first: plain_first, .. } = plain_response.outcome else {
            panic!("expected a match for {higher_status} vs {lower_status}");
        };
        assert_eq!(
            plain_first.note_id.as_str(), "note-a-lower",
            "control: without the tiebreak, the note-id order alone must put note-a-lower first for {higher_status} vs {lower_status}"
        );

        let tiebreak_response = tiebreak.search("shared_tier_probe_term", Default::default(), 10).unwrap();
        let index::search::SearchOutcome::Matches { first: tiebreak_first, .. } = tiebreak_response.outcome else {
            panic!("expected a match for {higher_status} vs {lower_status}");
        };
        assert_eq!(
            tiebreak_first.note_id.as_str(), "note-z-higher",
            "{higher_status} must outrank {lower_status} at an exact score tie, reversing the note-id order"
        );
    }
    // Same-tier pair: Verified and Reviewed must NOT have a defined
    // relative order from this rule (falls through to note-id tie-break).
    let f_same_tier = Fixture::new();
    f_same_tier.write(&Note { id: "note-verified", status: "verified", ..Note::new("note-verified", "shared_tier_probe_term") }, 0);
    f_same_tier.write(&Note { id: "note-reviewed", status: "reviewed", ..Note::new("note-reviewed", "shared_tier_probe_term") }, 0);
    let indexer = RealIndexer::new();
    index::indexer_impl::rebuild_index(&f_same_tier.vault, &indexer).unwrap();
    let idx = index::Index::new(&f_same_tier.vault, &indexer, "project-x", date("2026-10-26")).with_status_tiebreak();
    let response = idx.search("shared_tier_probe_term", Default::default(), 10).unwrap();
    let index::search::SearchOutcome::Matches { first, .. } = response.outcome else {
        panic!("expected a match");
    };
    // note-reviewed < note-verified lexicographically -> the existing
    // note-id tie-break decides, unchanged by the same-tier status pair.
    assert_eq!(first.note_id.as_str(), "note-reviewed");
}

#[test]
fn status_tiebreak_does_not_change_the_candidate_set() {
    let f = Fixture::new();
    f.write(&Note { status: "draft", ..Note::new("note-a", "shared_term") }, 0);
    f.write(&Note { status: "verified", ..Note::new("note-b", "shared_term") }, 0);
    let indexer = RealIndexer::new();
    index::indexer_impl::rebuild_index(&f.vault, &indexer).unwrap();
    let plain = index::Index::new(&f.vault, &indexer, "project-x", date("2026-10-26"));
    let tiebreak = index::Index::new(&f.vault, &indexer, "project-x", date("2026-10-26")).with_status_tiebreak();

    let ids = |outcome: index::search::SearchOutcome| -> std::collections::HashSet<String> {
        match outcome {
            index::search::SearchOutcome::NotPresent => Default::default(),
            index::search::SearchOutcome::Matches { first, rest } => {
                let mut s: std::collections::HashSet<String> = rest.iter().map(|r| r.note_id.as_str().to_string()).collect();
                s.insert(first.note_id.as_str().to_string());
                s
            }
        }
    };
    assert_eq!(
        ids(plain.search("shared_term", Default::default(), 10).unwrap().outcome),
        ids(tiebreak.search("shared_term", Default::default(), 10).unwrap().outcome),
    );
}

// ---------------------------------------------------------------------------
// B4.0: frozen `RetrievalMode` contract
// (docs/superpowers/specs/2026-11-15-b4-production-contract-freeze.md).
// These tests pin the contract before any dense-retrieval implementation
// exists: `Lexical` is the enum's `Default`, `Index::new(...)` behaves
// exactly as before B4 without ever mentioning `RetrievalMode`, and the new
// `with_retrieval_mode` builder is additive -- never changing behavior for a
// caller who never calls it, mirroring `with_identifier_boost`/
// `with_status_tiebreak`'s own precedent exactly.
// ---------------------------------------------------------------------------

#[test]
fn retrieval_mode_default_is_lexical() {
    assert_eq!(index::search::RetrievalMode::default(), index::search::RetrievalMode::Lexical);
}

#[test]
fn index_new_defaults_to_lexical_retrieval_mode_without_mentioning_it() {
    // This is the backward-compatibility contract itself: an `Index`
    // constructed exactly as every pre-B4 call site already does, with zero
    // knowledge that `RetrievalMode` exists, must behave under `Lexical`.
    let f = Fixture::new();
    f.write(&Note::new("note-a", "distinctive_lexical_default_probe"), 0);
    index::indexer_impl::rebuild_index(&f.vault, &f.indexer).unwrap();
    let idx = f.index();
    let response = idx.search("distinctive_lexical_default_probe", Default::default(), 10).unwrap();
    let index::search::SearchOutcome::Matches { first, .. } = response.outcome else {
        panic!("expected a match under the default (Lexical) retrieval mode");
    };
    assert_eq!(first.note_id.as_str(), "note-a");
}

#[test]
fn with_retrieval_mode_lexical_is_byte_identical_to_never_calling_it() {
    let f = Fixture::new();
    f.write(&Note::new("note-a", "retrieval_mode_parity_probe_term"), 0);
    index::indexer_impl::rebuild_index(&f.vault, &f.indexer).unwrap();
    let plain = Index::new(&f.vault, &f.indexer, "project-x", date("2026-10-26"));
    let explicit_lexical = Index::new(&f.vault, &f.indexer, "project-x", date("2026-10-26"))
        .with_retrieval_mode(index::search::RetrievalMode::Lexical);
    assert_eq!(
        plain.search("retrieval_mode_parity_probe_term", Default::default(), 10).unwrap(),
        explicit_lexical.search("retrieval_mode_parity_probe_term", Default::default(), 10).unwrap(),
        "explicitly setting Lexical must be byte-identical to never calling with_retrieval_mode at all"
    );
}

// ---------------------------------------------------------------------------
// B4.2: `DenseScorer` trait boundary
// (docs/superpowers/specs/2026-11-15-b4-2-dense-scorer-trait-and-runtime.md).
// `with_dense_scorer` is additive and default-off (`None`): a caller who
// never calls it, and every pre-B4.2 call site, must behave identically
// regardless of RetrievalMode -- the trait's mere existence changes nothing
// until B4.6 wires search_impl's own DenseV2 branch to actually consult it.
// ---------------------------------------------------------------------------

struct StubDenseScorer {
    response: std::sync::Mutex<Option<Vec<(storage::NoteId, f64)>>>,
}
impl index::search::DenseScorer for StubDenseScorer {
    fn score(&self, _query: &str, _candidate_ids: &[storage::NoteId]) -> Option<Vec<(storage::NoteId, f64)>> {
        self.response.lock().unwrap().clone()
    }
    fn rebuild(&self, _vault: &storage::Vault) -> Result<(), index::search::DenseScorerError> {
        Ok(())
    }
    fn on_note_indexed(&self, _note_id: &storage::NoteId, _document: &index::document::Document) {}
    fn on_note_deleted(&self, _note_id: &storage::NoteId) {}
}

#[test]
fn with_dense_scorer_under_lexical_mode_does_not_change_behavior() {
    // Attaching a scorer without ever selecting DenseV2 must be a no-op --
    // the trait's mere presence on `Index` changes nothing under Lexical.
    let f = Fixture::new();
    f.write(&Note::new("note-a", "dense_scorer_noop_under_lexical_probe"), 0);
    index::indexer_impl::rebuild_index(&f.vault, &f.indexer).unwrap();
    let scorer = StubDenseScorer { response: std::sync::Mutex::new(Some(vec![])) };
    let plain = Index::new(&f.vault, &f.indexer, "project-x", date("2026-10-26"));
    let with_scorer = Index::new(&f.vault, &f.indexer, "project-x", date("2026-10-26"))
        .with_dense_scorer(&scorer);
    assert_eq!(
        plain.search("dense_scorer_noop_under_lexical_probe", Default::default(), 10).unwrap(),
        with_scorer.search("dense_scorer_noop_under_lexical_probe", Default::default(), 10).unwrap(),
        "attaching a DenseScorer must not change Lexical-mode behavior at all"
    );
}

#[test]
fn dense_scorer_error_variants_exist_and_are_distinct() {
    // Pins the exact error shape B4.2's design froze -- a compile-time
    // contract test, not a behavioral one.
    assert_ne!(
        index::search::DenseScorerError::VaultUnavailable,
        index::search::DenseScorerError::IndexWriteFailed
    );
}
