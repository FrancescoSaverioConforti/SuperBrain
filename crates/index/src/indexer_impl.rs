//! B1 Task 1 skeleton: a concrete, `storage::Indexer`-implementing struct
//! that proves the integration seam Task 1 designed is actually usable from
//! *outside* `storage`, with a `Vault` constructible against it.
//!
//! This file deliberately contains **no indexing logic**. There is no
//! trigram index, no BM25 index, and `mark_current`/`state_for` below store
//! nothing beyond what is needed to prove the trait is implementable and the
//! seam compiles end-to-end. Real tokenization, postings and scoring are
//! Tasks 2 and 3; `search` does not exist yet. Anything written here that
//! looks like a design decision for THOSE tasks (field shapes, storage
//! layout, etc.) should be treated as provisional scaffolding, not as a
//! commitment Task 2/3 must honor.
//!
//! # Why this struct exists at all (recap of the Task 1 design note in
//! `storage::vault`'s `Indexer` trait doc comment)
//!
//! `storage::Indexer` is `pub` (B1 Task 1's Step 2 decision). `index` is the
//! one real implementor that decision was made for. This type —
//! [`SkeletonIndexer`] — is that implementor, for now holding only enough
//! state to answer `state_for` honestly (nothing is ever `Current`, because
//! nothing is indexed yet) and to let `mark_current` run without touching
//! any real index structure. A `storage::Vault` can already be opened with
//! `Box::new(SkeletonIndexer::new())` via `Vault::open_with_indexer` /
//! `Vault::open_with_pii_store_and_indexer` (both added in Task 1) — that
//! end-to-end construction path is exactly what this skeleton exists to
//! prove compiles and runs, ahead of Task 2/3 filling in real logic.

use std::collections::HashSet;
use std::sync::Mutex;

use domain::IndexState;
use storage::Indexer;

/// A placeholder [`Indexer`] implementation with no real index behind it.
///
/// `mark_current` records only THAT `note_id` was marked current (for
/// `state_for` to answer honestly — see below), never `record_bytes` itself
/// and never any derived structure (no tokens, no postings, no parsed
/// `domain::NoteMetadata`). `state_for` reports `IndexState::Current` for
/// any note id this skeleton has seen via `mark_current` at least once,
/// and `IndexState::Stale` for everything else — a deliberately weaker
/// claim than a real indexer will make (a real `state_for` must also know
/// whether the note has since moved to a NEWER version than what it
/// indexed; this skeleton has no versioned storage at all, so it cannot and
/// does not attempt that check). This is intentionally the simplest
/// behavior that (a) is not just `NullIndexer` renamed — it genuinely
/// distinguishes "seen" from "never seen" — while (b) adding zero of Task
/// 2/3's real logic.
pub struct SkeletonIndexer {
    seen: Mutex<HashSet<String>>,
}

impl SkeletonIndexer {
    pub fn new() -> Self {
        Self { seen: Mutex::new(HashSet::new()) }
    }
}

impl Default for SkeletonIndexer {
    fn default() -> Self {
        Self::new()
    }
}

impl Indexer for SkeletonIndexer {
    fn mark_current(&self, note_id: &str, _version: u64, _record_bytes: &[u8]) {
        // Task 2/3 will parse `_record_bytes` (via `domain::parse_frontmatter`
        // over its content half, after locating the version-header
        // separator) and feed the result into the real trigram/BM25
        // structures. Nothing is parsed here; the skeleton only needs to
        // prove this call site is reachable from outside `storage` with the
        // new signature.
        self.seen.lock().unwrap().insert(note_id.to_string());
    }

    fn state_for(&self, note_id: &str) -> IndexState {
        if self.seen.lock().unwrap().contains(note_id) {
            IndexState::Current
        } else {
            IndexState::Stale
        }
    }
}

// ---------------------------------------------------------------------------
// B1 Task 1, Step 5: scalability benchmark PROTOCOL (definition only).
// ---------------------------------------------------------------------------
//
// This section is the deliverable the brief's Step 5 asks for: a written
// protocol, not benchmark code and not benchmark results. `TrigramIndex`,
// `Bm25Index` and `search` do not exist at Task 1 — there is nothing real to
// measure yet. Task 9 executes this protocol once Tasks 2/3 (the real
// indexes) and the query layer exist. No pass/fail threshold is specified
// anywhere below, by design (per the brief): the protocol's job is to
// produce honest numbers and a sustainability assessment at each scale, not
// to gate implementation on an arbitrary cutoff the user's real vault size
// (not yet known, and expected to grow over time) cannot yet justify.
//
// ## Scales
//
// Four synthetic vault sizes, each a fresh, isolated vault populated with
// exactly N synthetic notes before any measurement begins:
//
//   N = 100 | 1,000 | 10,000 | 100,000
//
// Synthetic notes at each scale: realistic-shaped Markdown records (valid
// `domain::NoteMetadata` frontmatter + a body of prose-like text, not
// Lorem-Ipsum-uniform filler — see "Query set" below for why body content
// must vary realistically), generated with a fixed seed so the same corpus
// is reproducible across benchmark runs and across whichever machine Task 9
// runs on. Note bodies should vary in length (a mix of short, ~1-paragraph
// notes and longer, multi-section notes) so P50/P95 query latency reflects a
// realistic mix of postings-list sizes, not N copies of one length.
//
// ## Metrics (per N, four numbers per scale)
//
// (a) Full rebuild wall-clock time: time to go from an empty in-memory
//     index to a fully populated `TrigramIndex` + `Bm25Index` covering every
//     note `Vault::list_note_ids` returns, reading each via `Vault::read_note`
//     (the cold-start path Task 1's Step 4 designed). Measured end-to-end,
//     single-threaded (matching how a real cold start runs), wall clock from
//     immediately before the first `list_note_ids` call to immediately after
//     the last note's postings are committed into both structures.
//
// (b) Peak/resident memory for the fully-populated in-memory indexes: the
//     process's peak resident set size (RSS) attributable to the two index
//     structures, measured by sampling RSS immediately before rebuild starts
//     (baseline) and immediately after it completes (peak), reporting the
//     difference. On a platform where sampling peak RSS mid-run is available
//     (e.g. `/proc/self/status` VmHWM on Linux, `GetProcessMemoryInfo`
//     `PeakWorkingSetSize` on Windows) Task 9 should prefer that over the
//     before/after delta, since the delta can miss a transient peak during
//     rebuild that was later freed.
//
// (c) Query latency, P50 and P95, over the fixed, named query set below, run
//     against each scale's fully-populated index (i.e. after metric (a)'s
//     rebuild has completed, not interleaved with it). Each query is timed
//     individually, wall clock, from call to `search` to the return of its
//     final result; P50/P95 are computed over all queries in the set at that
//     scale (not per sub-category) unless Task 9 finds category-level
//     numbers more informative, in which case it reports both the overall
//     and the per-category P50/P95.
//
// ## Query set (fixed composition, so Task 9 does not have to invent one
// ## ad hoc)
//
// For a given scale of N notes, the query set has size N/10 + N/10 + N/20
// (rounded to the nearest integer, minimum 1 query per category so even
// N = 100 exercises all three):
//
// - N/10 exact-identifier lookups: queries that are themselves a valid
//   `NoteId` string drawn from the corpus (uniformly sampled, with
//   replacement, from the N generated note ids) — the cheapest, most
//   direct lookup shape, exercising whatever fast path an exact-id query
//   takes (if the eventual `search` API distinguishes one) or the trigram
//   index's behavior on a short, highly specific needle.
//
// - N/10 multi-term BM25 queries, each built from 2-4 terms drawn from the
//   corpus's actual note bodies (sampled from a frequency-weighted
//   vocabulary built from the same generated corpus, not a fixed hardcoded
//   term list), so query selectivity spans both common and rare terms in
//   roughly the proportion real content would produce. This is the
//   category that most directly exercises BM25 ranking over realistic
//   postings-list sizes.
//
// - N/20 filtered queries: a multi-term BM25 query (built the same way as
//   above) combined with BOTH a `type_filter` (one of `domain::NoteType`'s
//   variants, chosen to match the proportions the synthetic corpus was
//   generated with) AND a `date_filter` (a bounded range over the
//   synthetic corpus's `created`/`event_period` spread, wide enough to
//   match a non-trivial fraction of the corpus, e.g. covering roughly a
//   third of the generated date range) — exercising the combined
//   filter+rank path specifically, which is usually the one most at risk of
//   not scaling linearly if filtering happens after full-corpus ranking
//   instead of before or alongside it.
//
// Every query in every category is drawn once, with a fixed seed, before
// benchmarking starts at that scale, so the exact same query set is re-used
// across repeated runs (for variance checking) and is reportable verbatim
// alongside the numbers (Task 9 should save the generated query set per
// scale, not just regenerate it silently on each run).
//
// ## What Task 9 reports
//
// For each of the four scales: the three metrics above, plus a short
// sustainability note — whether rebuild time and memory appear to be
// growing linearly, worse-than-linearly, or sub-linearly with N (a rough
// read from four data points, not a rigorous curve fit), and whether
// anything observed (e.g. P95 latency, memory) looks likely to become a
// real-world problem before the next order of magnitude. No cutoff is
// imposed; Task 9's job ends at reporting honestly, not at passing or
// failing a threshold.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skeleton_indexer_reports_current_only_after_mark_current_and_stale_before() {
        let indexer = SkeletonIndexer::new();
        assert_eq!(indexer.state_for("note-a"), IndexState::Stale, "never marked yet");
        indexer.mark_current("note-a", 1, b"1\n---\nsome content");
        assert_eq!(indexer.state_for("note-a"), IndexState::Current, "now marked");
        assert_eq!(indexer.state_for("note-b"), IndexState::Stale, "a different note id is unaffected");
    }

    #[test]
    fn skeleton_indexer_is_usable_as_a_boxed_trait_object_from_outside_storage() {
        // The actual seam being proven: this type, built in `index`, must be
        // constructible as a `Box<dyn storage::Indexer>` and usable
        // wherever `storage::Vault` expects one — exactly what
        // `Vault::open_with_indexer` takes.
        let boxed: Box<dyn Indexer> = Box::new(SkeletonIndexer::new());
        assert_eq!(boxed.state_for("note-a"), IndexState::Stale);
    }

    #[test]
    fn vault_opens_and_writes_end_to_end_with_the_skeleton_indexer_from_outside_storage() {
        // The full seam, exercised once: open a real Vault from the `index`
        // crate's own test, with `index`'s own Indexer implementation, no
        // access to anything storage-internal, and drive an actual
        // production write through `propose_and_commit` (the one sanctioned
        // write path) to confirm `mark_current` is really invoked at the
        // end of a real write pipeline, from outside `storage`. This is
        // Task 1's actual deliverable made concrete — if this test did not
        // compile before Task 1's changes (`Indexer` was `pub(crate)`, no
        // `open_with_indexer` existed), it compiles and passes now.
        let dir = std::env::temp_dir().join(format!("superbrain-index-seam-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        // A real git repo (with an initial commit, so HEAD exists) at the
        // vault root: without one the write rests at GitPending rather than
        // reaching GitCommitted/IndexCurrent, and mark_current would never
        // be called — this test specifically wants to observe that it is.
        {
            let repo = git2::Repository::init(&dir).unwrap();
            let signature = git2::Signature::now("SuperBrain Test", "test@localhost").unwrap();
            let tree_oid = repo.index().unwrap().write_tree().unwrap();
            let tree = repo.find_tree(tree_oid).unwrap();
            repo.commit(Some("HEAD"), &signature, &signature, "initial empty commit", &tree, &[]).unwrap();
        }
        let indexer = std::sync::Arc::new(SkeletonIndexer::new());
        // `Vault::open_with_indexer` takes ownership of the `Box<dyn
        // Indexer>`, so this test keeps its own `Arc` to the same
        // underlying state for the post-write assertion below.
        struct ArcIndexer(std::sync::Arc<SkeletonIndexer>);
        impl Indexer for ArcIndexer {
            fn mark_current(&self, note_id: &str, version: u64, record_bytes: &[u8]) {
                self.0.mark_current(note_id, version, record_bytes);
            }
            fn state_for(&self, note_id: &str) -> IndexState {
                self.0.state_for(note_id)
            }
        }
        let vault = storage::Vault::open_with_indexer(dir.clone(), &dir.join("journal.sqlite"), Box::new(ArcIndexer(indexer.clone()))).unwrap();

        let note_text = "---\nid: note-a\nschema_version: 1\ntype: bug\nstatus: draft\ntrust: internal\nclassification: internal\nproject: project-x\nsource: agent:coder@prompt-v3\ncreated: 2026-10-30\nupdated: 2026-10-30\nevent_period: 2026-10-30\nrelations:\n  supports: []\n  contradicts: []\n  derived_from: []\n  supersedes: []\n  applies_to: []\n---\nbody text";
        let gate = policy::Gate::new(policy::SecretPolicy::new(1));
        let known: std::collections::HashSet<storage::NoteId> = std::collections::HashSet::new();
        let pending: std::collections::HashSet<String> = std::collections::HashSet::new();
        let result = storage::propose_and_commit(
            &vault,
            note_text,
            storage::Version(0),
            uuid::Uuid::new_v4(),
            &known,
            &pending,
            &gate,
            None,
        );
        assert!(matches!(result, Ok(storage::CommitResult::Committed(_))), "expected a successful commit, got {result:?}");
        assert_eq!(indexer.state_for("note-a"), IndexState::Current, "the real write pipeline must have called mark_current on this exact indexer instance");
    }
}
