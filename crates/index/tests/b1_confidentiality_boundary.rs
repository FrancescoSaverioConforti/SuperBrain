//! B1 Task 4: confidentiality-boundary audit for the index surface.
//!
//! Closes [n2io] **partially** — see the module-level summary at the bottom
//! of this file for exactly what is and is not closed by this file.
//!
//! # Step 1: every `domain::NoteMetadata` field, enumerated, with disposition
//!
//! `NoteMetadata` has exactly twelve fields. Per `crates/index/src/document.rs`'s
//! `build_text_searchable` (read directly for this enumeration, not taken on
//! faith from any planning document): **ten** of the twelve are the
//! "structured fields" — kept as real typed values on `Document` AND
//! included, as their string form, in `text_searchable` for every
//! classification, Sensitive included. The other **two** (`source`,
//! `relations`) are excluded from `text_searchable` specifically for
//! Sensitive notes.
//!
//! | # | Field            | Structured on `Document`? | In Sensitive `text_searchable`? | In Internal/Public `text_searchable`? |
//! |---|------------------|----------------------------|----------------------------------|------------------------------------------|
//! | 1 | `id`             | yes (typed `String`)       | yes                              | yes                                      |
//! | 2 | `schema_version` | yes (typed `u32`)          | yes                              | yes                                      |
//! | 3 | `r#type`         | yes (typed `NoteType`)     | yes                              | yes                                      |
//! | 4 | `status`         | yes (typed `NoteStatus`)   | yes                              | yes                                      |
//! | 5 | `trust`          | yes (typed `TrustLevel`)   | yes                              | yes                                      |
//! | 6 | `classification` | yes (typed `Classification`) | yes                           | yes                                      |
//! | 7 | `project`        | yes (typed `String`)       | yes                              | yes                                      |
//! | 8 | `source`         | **no** (not on `Document` at all) | **NO — excluded**         | yes                                      |
//! | 9 | `created`        | yes (typed `String`)       | yes                              | yes                                      |
//! | 10| `updated`        | yes (typed `String`)       | yes                              | yes                                      |
//! | 11| `event_period`   | yes (typed `String`)       | yes                              | yes                                      |
//! | 12| `relations`      | **no** (not on `Document` at all) | **NO — excluded**         | yes (every `relations.*` target)         |
//!
//! Rows 8 and 12 (`source`, `relations`) are the ones this file adds
//! dedicated tests for (brief Steps 2b/2c) — Tasks 2/3's own unit tests
//! (`trigram.rs::a_distinctive_marker_never_passed_to_insert_is_never_findable`,
//! `bm25.rs::a_distinctive_marker_never_passed_to_insert_is_never_findable`,
//! `indexer_impl.rs::real_indexer_never_indexes_a_sensitive_notes_body_into_bm25`)
//! already cover the BODY-leakage case for both postings structures and for
//! `RealIndexer`, so this file does not repeat that; it specifically targets
//! `source` and `relations`, which those tests did NOT cover. One exception:
//! `trigram.rs` already has `a_distinctive_source_marker_never_passed_to_insert_is_never_findable`
//! (a `TrigramIndex`-level test using a simulated `text_searchable` string
//! that never included the source marker in the first place). This file does
//! not duplicate that unit test; instead it adds the two things that test
//! did NOT do: (a) drive the exact same property through `Bm25Index` and
//! through the real `Document`/`RealIndexer` construction path (not a
//! hand-simulated `text_searchable` string), and (b) cover `relations`,
//! which had no existing test anywhere in the codebase (confirmed by reading
//! every `#[test]` name in `trigram.rs`, `bm25.rs`, and `indexer_impl.rs`
//! before writing this file).
//!
//! # Scope correction (controller-ruled; see B1 Task 4 brief Steps 1/2b/2c)
//!
//! The brief's Steps 1/2b/2c also name `SearchResponse` as a surface to
//! audit. `SearchResponse` is Task 6's output type and **does not exist in
//! this codebase yet** — there is nothing to construct, nothing to query,
//! and a test referencing it would not compile. The user, asked directly by
//! the controller how to handle this, decided: this audit covers only
//! surfaces that actually exist today — the in-memory `TrigramIndex` /
//! `Bm25Index` postings structures (via direct construction and via the real
//! `Document::from_record_bytes` → `RealIndexer::mark_current` path), and
//! log/error output around `RealIndexer::mark_current`'s failure path. No
//! test below references `SearchResponse`. [n2io]'s closure for the index
//! surface is therefore **partial**: the `SearchResponse`/search-API-response
//! surface remains open for Task 6 to close once that type exists.
//!
//! # A note on the log/error-output surface and `poison_next_mark_current`
//!
//! `RealIndexer::poison_next_mark_current` and the `trigram_hits` test helper
//! are both `#[cfg(test)]`-gated inside `crates/index/src/indexer_impl.rs`.
//! `#[cfg(test)]` applies only when the `index` crate's OWN unit-test binary
//! is built — it does not apply when `index` is linked as an ordinary
//! dependency for an external integration-test crate (this file, under
//! `crates/index/tests/`), which is confirmed directly: a probe call to
//! `poison_next_mark_current` from this file fails to compile with "no
//! method named `poison_next_mark_current` found for struct `RealIndexer`".
//! So the fault-injection path the brief names cannot be driven from here.
//! To still close that part of the audit honestly, Step 2 below is split:
//! `indexer_impl.rs`'s own `#[cfg(test)] mod tests` gained one additional
//! unit test, `poisoned_mark_current_debug_and_display_output_never_echoes_a_distinctive_marker`,
//! which drives `poison_next_mark_current` directly (it has the
//! crate-internal access this file does not) and is cross-referenced from
//! this file's own log/error-output test below so a reviewer has exactly one
//! place to look ("is the index surface's confidentiality story complete")
//! for the full picture across both files. This file's OWN log/error-output
//! test instead exercises the externally-reachable error surface:
//! `Document::from_record_bytes`'s `Err` path and `storage::SplitRecordError`'s
//! `Debug`/`Display` impls, plus `RealIndexer::mark_current`'s silent
//! absorption of a malformed-content failure (the same "absorbed, no panic,
//! no propagated error" design the poisoned-fault path uses, reachable here
//! without `#[cfg(test)]` access because a genuinely malformed record
//! produces it too).

use std::collections::HashSet;

use domain::{Classification, NoteMetadata, NoteStatus, NoteType, Relations, TrustLevel};
use index::document::Document;
use index::indexer_impl::RealIndexer;
use index::{bm25::Bm25Index, trigram::TrigramIndex};
use storage::{Indexer, NoteId};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Shared fixtures
// ---------------------------------------------------------------------------

fn unique_test_dir(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "superbrain-b1-confidentiality-{label}-{}-{}",
        std::process::id(),
        Uuid::new_v4()
    ))
}

fn init_git_repo(dir: &std::path::Path) {
    std::fs::create_dir_all(dir).unwrap();
    let repo = git2::Repository::init(dir).unwrap();
    let signature = git2::Signature::now("SuperBrain Test", "test@localhost").unwrap();
    let tree_oid = repo.index().unwrap().write_tree().unwrap();
    let tree = repo.find_tree(tree_oid).unwrap();
    repo.commit(Some("HEAD"), &signature, &signature, "initial empty commit", &tree, &[]).unwrap();
}

/// A Sensitive note's raw record text, with a distinctive `source` field and
/// a distinctive `relations.supports` target, both chosen to be maximally
/// adversarial (not just a plausible-looking value) so a leak cannot hide
/// behind "it happened to look like a normal field".
fn sensitive_note_with_source_and_relations_markers(
    id: &str,
    source_marker: &str,
    relations_marker: &str,
) -> String {
    format!(
        "---\nid: {id}\nschema_version: 1\ntype: bug\nstatus: draft\ntrust: internal\nclassification: sensitive\nproject: project-x\nsource: {source_marker}\ncreated: 2026-10-30\nupdated: 2026-10-30\nevent_period: 2026-10-30\nrelations:\n  supports: [\"{relations_marker}\"]\n  contradicts: []\n  derived_from: []\n  supersedes: []\n  applies_to: []\n---\nordinary body text, not under test here\n"
    )
}

fn internal_filler_note(id: &str) -> String {
    format!(
        "---\nid: {id}\nschema_version: 1\ntype: bug\nstatus: draft\ntrust: internal\nclassification: internal\nproject: project-x\nsource: agent:coder@prompt-v3\ncreated: 2026-10-30\nupdated: 2026-10-30\nevent_period: 2026-10-30\nrelations:\n  supports: []\n  contradicts: []\n  derived_from: []\n  supersedes: []\n  applies_to: []\n---\nsome unrelated filler content entirely\n"
    )
}

fn base_metadata(classification: Classification, source: &str, relations: Relations) -> NoteMetadata {
    NoteMetadata {
        id: "note-marker-fixture".to_string(),
        schema_version: 1,
        r#type: NoteType::Bug,
        status: NoteStatus::Draft,
        trust: TrustLevel::Internal,
        classification,
        project: "project-x".to_string(),
        source: source.to_string(),
        created: "2026-10-30".to_string(),
        updated: "2026-10-30".to_string(),
        event_period: "2026-10-30".to_string(),
        relations,
    }
}

// ---------------------------------------------------------------------------
// Step 2b: a Sensitive note's `source` field never reaches TrigramIndex or
// Bm25Index's postings, through the real Document construction path (not a
// hand-simulated text_searchable string — the gap trigram.rs's own existing
// source test left open).
// ---------------------------------------------------------------------------

const SOURCE_MARKER: &str = "agent:coder@prompt-v3-DISTINCTIVE-SOURCE-MARKER-9f3c2";

#[test]
fn sensitive_notes_source_field_never_reaches_trigram_or_bm25_via_real_document_construction() {
    let relations = Relations { supports: vec!["pattern-foo".to_string()], ..Relations::default() };
    let metadata = base_metadata(Classification::Sensitive, SOURCE_MARKER, relations);
    let document = Document::new(metadata, Some("ordinary body text"));

    // First, the allowlist invariant itself: the marker must not even be
    // present in text_searchable (this is what Tasks 2/3 already guarantee
    // at the Document level; restated here as the premise this test's
    // postings-level assertions depend on).
    assert!(
        !document.text_searchable.contains(SOURCE_MARKER),
        "precondition failed: the source marker must not even be in text_searchable"
    );

    let mut trigram = TrigramIndex::new();
    let mut bm25 = Bm25Index::new();
    let id = NoteId::parse("note-marker-fixture").unwrap();
    trigram.insert(id.clone(), &document.text_searchable);
    bm25.insert(id, &document.text_searchable);

    let trigram_hits = trigram.query_exact(SOURCE_MARKER);
    assert!(trigram_hits.is_empty(), "TrigramIndex must never find a Sensitive note's source marker: {trigram_hits:?}");

    let bm25_hits = bm25.query_ranked(SOURCE_MARKER, 10);
    assert!(bm25_hits.is_empty(), "Bm25Index must never find a Sensitive note's source marker: {bm25_hits:?}");
}

/// Same property, driven through the full production write path
/// (`propose_and_commit` -> `Vault` -> `RealIndexer::mark_current`), not
/// just direct `Document`/postings construction. This is the strongest form
/// of the test: it proves the real wiring, not a simulation of it.
#[test]
fn sensitive_notes_source_field_never_reaches_the_real_indexer_via_propose_and_commit() {
    let dir = unique_test_dir("source-marker-e2e");
    init_git_repo(&dir);

    let indexer = std::sync::Arc::new(RealIndexer::new());
    struct ArcIndexer(std::sync::Arc<RealIndexer>);
    impl Indexer for ArcIndexer {
        fn mark_current(&self, note_id: &str, version: u64, record_bytes: &[u8]) {
            self.0.mark_current(note_id, version, record_bytes);
        }
        fn state_for(&self, note_id: &str) -> domain::IndexState {
            self.0.state_for(note_id)
        }
    }
    let vault = storage::Vault::open_with_indexer(dir.clone(), &dir.join("journal.sqlite"), Box::new(ArcIndexer(indexer.clone()))).unwrap();

    let pii_root = dir.join(".pii-store");
    let pii_store = vault_security::PiiStore::open(pii_root.clone(), &pii_root.join("master.key")).unwrap();
    let gate = policy::Gate::new(policy::SecretPolicy::new(1));

    // The relations target is not under test in THIS function (the
    // relations marker test below handles that field). propose_and_commit
    // validates every non-pending relation target as a NoteId-shaped string
    // and rejects a BrokenLink for one that does not resolve to a known or
    // pending note, so "pattern-foo" is declared pending here purely to
    // keep this write focused on the source field.
    let note_text = sensitive_note_with_source_and_relations_markers("note-source-e2e", SOURCE_MARKER, "pattern-foo");
    let known: HashSet<NoteId> = HashSet::new();
    let mut pending: HashSet<String> = HashSet::new();
    pending.insert("pattern-foo".to_string());
    let result = storage::propose_and_commit(&vault, &note_text, storage::Version(0), Uuid::new_v4(), &known, &pending, &gate, Some(&pii_store));
    assert!(matches!(result, Ok(storage::CommitResult::Committed(_))), "expected a successful commit, got {result:?}");

    // A filler note so an empty-corpus/degenerate-query-result ambiguity
    // cannot masquerade as "the marker was excluded".
    let filler_text = internal_filler_note("note-filler-source-e2e");
    storage::propose_and_commit(&vault, &filler_text, storage::Version(0), Uuid::new_v4(), &known, &pending, &gate, Some(&pii_store)).unwrap();

    assert_eq!(indexer.state_for("note-source-e2e"), domain::IndexState::Current, "the note itself was indexed (structured fields only)");

    let hits = indexer.bm25_ranked(SOURCE_MARKER, 10);
    assert!(hits.is_empty(), "a Sensitive note's source marker must never be findable via the real indexer's BM25 postings: {hits:?}");
}

// ---------------------------------------------------------------------------
// Step 2c: a Sensitive note's `relations` targets never reach TrigramIndex
// or Bm25Index's postings. No existing test anywhere in the codebase covers
// this field (confirmed by reading every #[test] name in trigram.rs,
// bm25.rs, and indexer_impl.rs before writing this file) — this is the
// primary gap this task closes.
// ---------------------------------------------------------------------------

const RELATIONS_MARKER_DIRECT: &str = "DISTINCTIVE-RELATIONS-TARGET-MARKER-7b1e4";

#[test]
fn sensitive_notes_relations_targets_never_reach_trigram_or_bm25_via_real_document_construction() {
    // domain::Relations' fields are plain Vec<String> with no NoteId-shaped
    // validation at the parse_frontmatter level (confirmed by reading
    // crates/domain/src/note.rs: `Relations` derives Deserialize over
    // Vec<String> fields directly, and the "only known keys" check inspects
    // field NAMES, not target VALUES) — so a fully adversarial marker string
    // is a valid `Relations.supports` entry at the Document-construction
    // layer used by this test. (`propose_and_commit`'s OWN extra validation
    // layer, which does require NoteId-shaped targets unless the caller
    // declares them `pending`, is exercised separately below.)
    let relations = Relations { supports: vec![RELATIONS_MARKER_DIRECT.to_string()], ..Relations::default() };
    let metadata = base_metadata(Classification::Sensitive, "agent:coder@prompt-v3", relations);
    let document = Document::new(metadata, Some("ordinary body text"));

    assert!(
        !document.text_searchable.contains(RELATIONS_MARKER_DIRECT),
        "precondition failed: the relations marker must not even be in text_searchable"
    );

    let mut trigram = TrigramIndex::new();
    let mut bm25 = Bm25Index::new();
    let id = NoteId::parse("note-marker-fixture").unwrap();
    trigram.insert(id.clone(), &document.text_searchable);
    bm25.insert(id, &document.text_searchable);

    let trigram_hits = trigram.query_exact(RELATIONS_MARKER_DIRECT);
    assert!(trigram_hits.is_empty(), "TrigramIndex must never find a Sensitive note's relations marker: {trigram_hits:?}");

    let bm25_hits = bm25.query_ranked(RELATIONS_MARKER_DIRECT, 10);
    assert!(bm25_hits.is_empty(), "Bm25Index must never find a Sensitive note's relations marker: {bm25_hits:?}");
}

/// Same property, driven through the full production write path. Because
/// `storage::propose_and_commit` validates every non-`pending` relation
/// target as a `NoteId` (`[a-z0-9._-]+`, lowercase, <=128 bytes — see
/// `crates/storage/src/note_id.rs` and the validation loop in
/// `crates/storage/src/propose.rs`), a target string containing uppercase
/// letters, `@`, or other characters outside that charset is REJECTED
/// before it would ever reach the indexer, unless the caller declares it
/// `pending` (an opaque reference the production code path explicitly
/// supports for "URL, DOI, code symbol, ..." targets that are not meant to
/// resolve to a real NoteId). This test uses `pending_ids` precisely to
/// keep the marker fully adversarial (mixed case, unusual punctuation)
/// while still exercising the real, unmodified `propose_and_commit` path —
/// no NoteId-charset limitation was needed to make this test meaningful.
#[test]
fn sensitive_notes_relations_targets_never_reach_the_real_indexer_via_propose_and_commit() {
    const RELATIONS_MARKER_E2E: &str = "DISTINCTIVE-RELATIONS-TARGET-MARKER-Pending-Opaque-5c9a1";

    let dir = unique_test_dir("relations-marker-e2e");
    init_git_repo(&dir);

    let indexer = std::sync::Arc::new(RealIndexer::new());
    struct ArcIndexer(std::sync::Arc<RealIndexer>);
    impl Indexer for ArcIndexer {
        fn mark_current(&self, note_id: &str, version: u64, record_bytes: &[u8]) {
            self.0.mark_current(note_id, version, record_bytes);
        }
        fn state_for(&self, note_id: &str) -> domain::IndexState {
            self.0.state_for(note_id)
        }
    }
    let vault = storage::Vault::open_with_indexer(dir.clone(), &dir.join("journal.sqlite"), Box::new(ArcIndexer(indexer.clone()))).unwrap();

    let pii_root = dir.join(".pii-store");
    let pii_store = vault_security::PiiStore::open(pii_root.clone(), &pii_root.join("master.key")).unwrap();
    let gate = policy::Gate::new(policy::SecretPolicy::new(1));

    let note_text = sensitive_note_with_source_and_relations_markers("note-relations-e2e", "agent:coder@prompt-v3", RELATIONS_MARKER_E2E);
    let known: HashSet<NoteId> = HashSet::new();
    // Declare the marker `pending` so propose_and_commit's NoteId-shaped
    // validation treats it as an opaque external reference rather than
    // rejecting the write for an invalid-charset relation target.
    let mut pending: HashSet<String> = HashSet::new();
    pending.insert(RELATIONS_MARKER_E2E.to_string());

    let result = storage::propose_and_commit(&vault, &note_text, storage::Version(0), Uuid::new_v4(), &known, &pending, &gate, Some(&pii_store));
    assert!(matches!(result, Ok(storage::CommitResult::Committed(_))), "expected a successful commit, got {result:?}");

    let filler_text = internal_filler_note("note-filler-relations-e2e");
    storage::propose_and_commit(&vault, &filler_text, storage::Version(0), Uuid::new_v4(), &known, &pending, &gate, Some(&pii_store)).unwrap();

    assert_eq!(indexer.state_for("note-relations-e2e"), domain::IndexState::Current, "the note itself was indexed (structured fields only)");

    let hits = indexer.bm25_ranked(RELATIONS_MARKER_E2E, 10);
    assert!(hits.is_empty(), "a Sensitive note's relations marker must never be findable via the real indexer's BM25 postings: {hits:?}");
}

// ---------------------------------------------------------------------------
// Log/error-output surface: Document::from_record_bytes's Err path and
// storage::SplitRecordError's Debug/Display, plus RealIndexer::mark_current's
// silent absorption of a malformed-content failure. See this file's module
// doc comment for why this is the externally-reachable half of the
// fault-injection story, and indexer_impl.rs's own
// poisoned_mark_current_debug_and_display_output_never_echoes_a_distinctive_marker
// unit test for the `poison_next_mark_current`-driven half this file cannot
// reach (cfg(test)-gated, invisible from an external integration test).
// ---------------------------------------------------------------------------

const POINTER_RECORD_MARKER: &str = "DISTINCTIVE-POINTER-RECORD-MARKER-should-never-appear-in-any-log";

#[test]
fn malformed_record_bytes_error_debug_and_display_output_never_echoes_note_content() {
    // A deliberately malformed record (frontmatter present but a required
    // field missing) that still happens to carry a distinctive marker
    // string inside it, so a test failure here would be visible evidence of
    // a real leak, not just "the function returned an error".
    let malformed = format!(
        "---\nid: note-malformed\nproject: project-x\nsource: {POINTER_RECORD_MARKER}\n---\nbody with {POINTER_RECORD_MARKER} inside\n"
    );

    let result = Document::from_record_bytes(malformed.as_bytes());
    let err = result.expect_err("missing required fields must fail to parse");
    let debug_output = format!("{err:?}");
    let display_output = format!("{err}");

    assert!(!debug_output.contains(POINTER_RECORD_MARKER), "SplitRecordError Debug output must not echo note content: {debug_output}");
    assert!(!display_output.contains(POINTER_RECORD_MARKER), "SplitRecordError Display output must not echo note content: {display_output}");
}

/// `RealIndexer::mark_current` absorbs a malformed-content failure exactly
/// like a poisoned fault: no panic, no error propagated (the trait method's
/// return type is `()`), state_for left as it was. This test confirms that
/// absorption path — reachable here via genuinely malformed record_bytes,
/// the externally-triggerable sibling of the `poison_next_mark_current`
/// path indexer_impl.rs's own unit test covers directly — leaves no trace of
/// the marker anywhere observable (postings empty, state Stale, and no
/// panic message/output could have echoed it either, since mark_current
/// never panics on this path).
#[test]
fn real_indexer_mark_current_absorbs_a_malformed_record_without_echoing_its_content_anywhere() {
    let indexer = RealIndexer::new();
    let malformed_with_marker = format!(
        "1\n---\nid: note-malformed\nproject: project-x\nsource: {POINTER_RECORD_MARKER}\n---\nbody with {POINTER_RECORD_MARKER} inside\n"
    );

    // Must not panic — this is the core property under test.
    indexer.mark_current("note-malformed", 1, malformed_with_marker.as_bytes());

    assert_eq!(indexer.state_for("note-malformed"), domain::IndexState::Stale, "a malformed record must never advance state_for to Current");
    let hits = indexer.bm25_ranked(POINTER_RECORD_MARKER, 10);
    assert!(hits.is_empty(), "a malformed record's content must never reach the postings structures either: {hits:?}");
}

// ---------------------------------------------------------------------------
// Summary: what [n2io] closure this file represents.
// ---------------------------------------------------------------------------
//
// CLOSED by this file, for the index surface:
// - TrigramIndex / Bm25Index postings: a Sensitive note's `source` field and
//   `relations` targets never appear in either structure's postings, proven
//   both via direct Document/postings construction and via the full
//   propose_and_commit -> Vault -> RealIndexer::mark_current production path.
// - Log/error output around RealIndexer::mark_current's failure path: no
//   Debug/Display output from Document::from_record_bytes's error path, or
//   from mark_current's own absorption of a malformed/poisoned record,
//   echoes note content. (The poison_next_mark_current-specific half of this
//   lives in indexer_impl.rs's own unit test, cross-referenced above, since
//   that hook is #[cfg(test)]-gated and unreachable from this external file.)
//
// NOT closed by this file (remains open, per the controller-ruled scope
// correction):
// - SearchResponse / the search-API-response shape: Task 6's output type,
//   does not exist yet. [n2io]'s closure for THIS surface is Task 6's
//   responsibility once that type exists.
//
// Unrelated to this task, unaddressed as before (not reopened or newly
// deferred here, just restated for completeness):
// - Embedding and cache surfaces: no such subsystems exist anywhere in this
//   codebase yet.
