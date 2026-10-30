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

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use domain::IndexState;
use storage::{Indexer, NoteId, Vault};

use crate::bm25::Bm25Index;
use crate::document::Document;
use crate::trigram::TrigramIndex;

/// The separator between the version header line and the raw note content
/// in the on-disk record format `storage::Indexer::mark_current`'s
/// `record_bytes` carries. Mirrors `storage::vault::RECORD_SEPARATOR`
/// exactly (that constant is crate-private to `storage`, and the trait's own
/// doc comment on `mark_current` directs implementors to locate this
/// separator themselves rather than treating it as something `storage`
/// should expose — unlike frontmatter/body splitting, which IS exposed via
/// `storage::split_record` precisely to avoid this kind of duplication; see
/// `document.rs`'s module doc comment for that distinction).
const RECORD_SEPARATOR: &str = "\n---\n";

/// Strips the version header + separator off `record_bytes` (the exact
/// shape `Indexer::mark_current` receives — see `vault.rs`'s
/// `serialize_record`), returning the raw content bytes `Document::from_record_bytes`
/// expects. Returns `None` if `record_bytes` is not valid UTF-8 or has no
/// separator at all (both already-defensive cases: every real on-disk
/// record always has one, written by `serialize_record`, so this should
/// only fail on bytes a test or a future caller constructs by hand).
fn strip_version_header(record_bytes: &[u8]) -> Option<&[u8]> {
    let text = std::str::from_utf8(record_bytes).ok()?;
    let (_version_part, content) = text.split_once(RECORD_SEPARATOR)?;
    Some(content.as_bytes())
}

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
// B1 Task 3: `RealIndexer` — the first real `storage::Indexer` implementor.
// ---------------------------------------------------------------------------

/// The real [`Indexer`] implementation: a [`TrigramIndex`] and a
/// [`Bm25Index`], both behind their own `Mutex` (so a read-heavy future
/// `search` call and the single write-path `mark_current` call never need
/// the same lock held across both structures at once — each structure
/// guards its own invariants independently), plus a per-note [`IndexState`]
/// map.
///
/// `mark_current`'s contract (pinned by Task 1, closed for real here):
///
/// - Builds a [`Document`] from `record_bytes` via
///   [`strip_version_header`] + [`Document::from_record_bytes`].
/// - On success: removes any prior postings for this note id from BOTH
///   `TrigramIndex` and `Bm25Index` (`insert` already does this internally,
///   so a plain `insert` call is the correct "this might be an update"
///   handling — no separate removal step is needed), inserts the new
///   `text_searchable`, and marks `state_for` `Current`.
/// - On failure (version header missing, invalid UTF-8, or a schema error
///   from `domain::parse_frontmatter`, OR the test-only injected fault
///   below): nothing is inserted into either structure, any PRIOR postings
///   for this note id are left exactly as they were (a failed re-index must
///   not silently erase a previously-successful index entry), and
///   `state_for` is left/set `Stale`. `mark_current`'s return type is `()`,
///   so there is nothing to propagate to the caller regardless — the
///   discipline here is purely "don't panic, don't partially insert."
///
/// # Lock-scope guarantee (Task 1's seam, closed against real code)
///
/// `mark_current` never calls `Vault::read_note` or any other `Vault`
/// method — it only ever touches its own three `Mutex`-protected fields.
/// This is the property the real wiring had to preserve: every call site in
/// `storage::vault` invokes `mark_current` while already holding the
/// writing note's per-note lock, so a callback into `Vault` here would
/// self-deadlock. Nothing in this impl reaches outside `self`.
pub struct RealIndexer {
    trigram: Mutex<TrigramIndex>,
    bm25: Mutex<Bm25Index>,
    states: Mutex<HashMap<String, IndexState>>,
    /// Test-only injected-fault hook (Task 3, Step 2): the set of note ids
    /// for which the NEXT `mark_current` call must behave as though its
    /// internal indexing logic hit an invariant violation, without
    /// `storage`-layer I/O being involved at all (`mark_current` is pure
    /// in-memory logic: tokenize + insert into postings, no filesystem, no
    /// git, no journal — there is no I/O fault to inject here, unlike
    /// `storage::fault::FaultPoint`, which exists for crash-injection on
    /// disk/journal/git operations). A note id is removed from this set the
    /// moment its poisoned call runs, so the fault fires exactly once per
    /// `poison` call, mirroring how a real internal bug would affect one
    /// specific write, not every future write to that note id forever.
    #[cfg(test)]
    poisoned: Mutex<HashSet<String>>,
}

impl RealIndexer {
    pub fn new() -> Self {
        Self {
            trigram: Mutex::new(TrigramIndex::new()),
            bm25: Mutex::new(Bm25Index::new()),
            states: Mutex::new(HashMap::new()),
            #[cfg(test)]
            poisoned: Mutex::new(HashSet::new()),
        }
    }

    /// Test-only (Step 2): arms the fault hook so the NEXT `mark_current`
    /// call for `note_id` fails its internal indexing step — simulating an
    /// invariant violation inside `mark_current`'s own logic (not a
    /// malformed-input parse failure: see the module-level test for why a
    /// real `propose_and_commit` call can never actually produce
    /// `record_bytes` that fail `Document::from_record_bytes`, since
    /// `propose_and_commit` already parsed the identical content
    /// successfully earlier in the same call). This is the simplest
    /// mechanism that lets a test deterministically trigger the "indexing
    /// failed, write must still succeed" path end-to-end through a real
    /// `propose_and_commit` call, without inventing a new
    /// `storage::fault::FaultPoint` variant for in-memory indexer logic
    /// that has no I/O to fault-inject in the first place.
    #[cfg(test)]
    pub fn poison_next_mark_current(&self, note_id: &str) {
        self.poisoned.lock().unwrap().insert(note_id.to_string());
    }

    /// Returns every note id currently indexed in the `TrigramIndex`
    /// (test/introspection helper for the cold-start rebuild test).
    #[cfg(test)]
    pub fn trigram_hits(&self, term: &str) -> Vec<NoteId> {
        self.trigram.lock().unwrap().query_exact(term)
    }

    /// Returns the top `k` BM25 ranked hits for `query` (test/introspection
    /// helper, and the shape a future `search` implementation will also
    /// need).
    pub fn bm25_ranked(&self, query: &str, k: usize) -> Vec<(NoteId, f64)> {
        self.bm25.lock().unwrap().query_ranked(query, k)
    }

    /// Indexes an already-built [`Document`] (shared by `mark_current` and
    /// the cold-start rebuild function below, so both paths use IDENTICAL
    /// insertion logic — no drift between "index on write" and "index on
    /// rebuild").
    fn index_document(&self, note_id: NoteId, document: &Document) {
        self.trigram.lock().unwrap().insert(note_id.clone(), &document.text_searchable);
        self.bm25.lock().unwrap().insert(note_id.clone(), &document.text_searchable);
        self.states.lock().unwrap().insert(note_id.as_str().to_string(), IndexState::Current);
    }
}

impl Default for RealIndexer {
    fn default() -> Self {
        Self::new()
    }
}

impl Indexer for RealIndexer {
    fn mark_current(&self, note_id: &str, _version: u64, record_bytes: &[u8]) {
        // Test-only fault injection (Step 2): checked first, before any real
        // parsing, so a poisoned call fails deterministically regardless of
        // whether `record_bytes` would otherwise have parsed fine.
        #[cfg(test)]
        {
            let mut poisoned = self.poisoned.lock().unwrap();
            if poisoned.remove(note_id) {
                // Simulates an internal invariant violation inside the
                // indexing step itself. Absorbed right here: nothing is
                // inserted into either postings structure, `state_for`
                // stays whatever it already was (Stale, for a note never
                // successfully indexed before), and — critically — this
                // function does not panic and has no error to propagate
                // (its return type is `()`), so `propose_and_commit`'s own
                // call is completely unaffected.
                return;
            }
        }

        let Some(content_bytes) = strip_version_header(record_bytes) else {
            // No version-header separator, or not valid UTF-8: cannot even
            // locate the content half of the buffer. Leave this note id's
            // state exactly as it was (Stale if never indexed before);
            // don't panic, don't insert anything.
            return;
        };

        match Document::from_record_bytes(content_bytes) {
            Ok(document) => {
                let Ok(id) = NoteId::parse(note_id) else {
                    // `note_id` always comes from a real `storage::NoteId`
                    // on every production call site, so this cannot
                    // actually happen on the write path; defensive-only.
                    return;
                };
                self.index_document(id, &document);
            }
            Err(_) => {
                // `record_bytes`' content half failed to parse as a note
                // (bad frontmatter / schema error). Absorbed here, exactly
                // like the poisoned-fault branch above: no insertion into
                // either postings structure, no panic, no error
                // propagated. `state_for` is left as it was.
            }
        }
    }

    fn state_for(&self, note_id: &str) -> IndexState {
        self.states.lock().unwrap().get(note_id).copied().unwrap_or(IndexState::Stale)
    }
}

/// Cold-start rebuild (Task 1's Step 4 design, closed for real here):
/// populates `indexer` from every currently-live note `vault` holds, using
/// `Vault::list_note_ids` (Task 1's chosen enumeration mechanism — excludes
/// deleted notes via the deletion log when present, excludes temp/
/// before-image/pii-store paths) followed by `Vault::read_note` for each id.
///
/// # Why this uses `Document::new`, not `Document::from_record_bytes`
///
/// `Vault::read_note` returns an already-parsed [`storage::VaultRecord`]
/// whose `content` field is the frontmatter+body text directly (no version
/// header to strip — that header is an internal on-disk encoding detail
/// `read_note` already peels off, unlike the raw `record_bytes`
/// `mark_current` receives). Splitting THAT content still needs
/// `storage::split_record` (to get a parsed `NoteMetadata` plus the already
/// -separated body), so this calls `storage::split_record` directly and
/// then `Document::new` with the result — equivalent to
/// `Document::from_record_bytes`, but avoiding a redundant round trip
/// through the version-header-stripping logic that only applies to the
/// OTHER raw-bytes shape.
///
/// # Lock-scope note
///
/// This function must be called OUTSIDE any writer's lock. It calls
/// `Vault::list_note_ids` (which itself takes no per-note lock) followed by
/// `Vault::read_note` once per returned id (each call taking and releasing
/// its own per-note lock in turn) — the exact calling pattern
/// `list_note_ids`' own doc comment describes as introducing no new
/// deadlock risk for a cold-start rebuild. Nothing here holds any lock
/// across more than one of these calls, and nothing here is called from
/// inside `storage`'s write pipeline (that pipeline only ever calls
/// `Indexer::mark_current`, never this function).
///
/// A note that fails to parse (its content fails `split_record`, e.g. a
/// pointer record whose frontmatter happens to be malformed — should not
/// occur in practice, since every committed note already passed the gate's
/// schema check at write time) is skipped rather than aborting the whole
/// rebuild, on the same "don't let one bad note break indexing" principle
/// `mark_current` itself follows.
pub fn rebuild_index(vault: &Vault, indexer: &RealIndexer) -> Result<(), storage::StorageError> {
    for note_id in vault.list_note_ids()? {
        let record = match vault.read_note(&note_id) {
            Ok(record) => record,
            // A note that vanished (deleted) or became unreadable between
            // `list_note_ids` and this call is simply skipped: the same
            // "approximately current" guarantee `list_note_ids`'s own doc
            // comment already documents for a cold-start rebuild.
            Err(_) => continue,
        };
        let Ok((metadata, body)) = storage::split_record(record.content.as_bytes()) else {
            continue;
        };
        let document = Document::new(metadata, body.as_deref());
        indexer.index_document(note_id, &document);
    }
    Ok(())
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

    // -----------------------------------------------------------------
    // B1 Task 3: RealIndexer
    // -----------------------------------------------------------------

    /// Shared test fixture: a real git repo (with an initial commit) at a
    /// fresh temp dir, ready to open a `Vault` against. Mirrors the
    /// `SkeletonIndexer` seam test's own setup exactly.
    fn init_git_repo(dir: &std::path::Path) {
        std::fs::create_dir_all(dir).unwrap();
        let repo = git2::Repository::init(dir).unwrap();
        let signature = git2::Signature::now("SuperBrain Test", "test@localhost").unwrap();
        let tree_oid = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        repo.commit(Some("HEAD"), &signature, &signature, "initial empty commit", &tree, &[]).unwrap();
    }

    fn unique_test_dir(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("superbrain-index-{label}-{}", uuid::Uuid::new_v4()))
    }

    fn note_text(id: &str, classification: &str, body: &str) -> String {
        format!(
            "---\nid: {id}\nschema_version: 1\ntype: bug\nstatus: draft\ntrust: internal\nclassification: {classification}\nproject: project-x\nsource: agent:coder@prompt-v3\ncreated: 2026-10-30\nupdated: 2026-10-30\nevent_period: 2026-10-30\nrelations:\n  supports: []\n  contradicts: []\n  derived_from: []\n  supersedes: []\n  applies_to: []\n---\n{body}"
        )
    }

    // --- Step 5 (unit-level): RealIndexer's mark_current/state_for happy
    // path, and that insert into both postings structures actually happens.

    #[test]
    fn real_indexer_mark_current_indexes_into_both_trigram_and_bm25_and_reports_current() {
        let indexer = RealIndexer::new();
        assert_eq!(indexer.state_for("note-a"), IndexState::Stale, "never marked yet");

        let text = note_text("note-a", "internal", "a distinctive_marker_term appears here");
        let record_bytes = format!("1\n---\n{text}");
        indexer.mark_current("note-a", 1, record_bytes.as_bytes());

        assert_eq!(indexer.state_for("note-a"), IndexState::Current);
        assert_eq!(indexer.trigram_hits("distinctive_marker_term"), vec![NoteId::parse("note-a").unwrap()]);
        assert!(indexer.bm25_ranked("distinctive_marker_term", 10).iter().any(|(id, _)| id.as_str() == "note-a"));
    }

    #[test]
    fn real_indexer_reindexing_an_existing_note_replaces_its_postings_not_duplicates_them() {
        let indexer = RealIndexer::new();
        let old_text = note_text("note-a", "internal", "old_distinctive_term here");
        indexer.mark_current("note-a", 1, format!("1\n---\n{old_text}").as_bytes());
        assert_eq!(indexer.trigram_hits("old_distinctive_term"), vec![NoteId::parse("note-a").unwrap()]);

        let new_text = note_text("note-a", "internal", "new_distinctive_term here");
        indexer.mark_current("note-a", 2, format!("2\n---\n{new_text}").as_bytes());

        assert!(indexer.trigram_hits("old_distinctive_term").is_empty(), "stale postings must be gone after re-index");
        assert_eq!(indexer.trigram_hits("new_distinctive_term"), vec![NoteId::parse("note-a").unwrap()]);
        assert_eq!(indexer.state_for("note-a"), IndexState::Current);
    }

    // --- Step 2 (RED->GREEN): an indexing failure inside mark_current
    // leaves state_for Stale and does not propagate an error back through
    // propose_and_commit. This is the single most important test in this
    // task (see the module's `RealIndexer::poison_next_mark_current` doc
    // comment for why a real malformed-parse trigger cannot be produced
    // through an actual `propose_and_commit` call, and why the injected
    // fault hook is the honest substitute).

    #[test]
    fn mark_current_poisoned_fault_leaves_note_stale_without_panicking() {
        let indexer = RealIndexer::new();
        indexer.poison_next_mark_current("note-a");

        let text = note_text("note-a", "internal", "some content");
        let record_bytes = format!("1\n---\n{text}");
        // Must not panic.
        indexer.mark_current("note-a", 1, record_bytes.as_bytes());

        assert_eq!(indexer.state_for("note-a"), IndexState::Stale, "a poisoned mark_current call must leave the note Stale");
        assert!(indexer.trigram_hits("content").is_empty(), "nothing must have been inserted into the trigram index either");

        // The fault fires exactly once: a subsequent, unpoisoned call
        // succeeds normally.
        indexer.mark_current("note-a", 1, record_bytes.as_bytes());
        assert_eq!(indexer.state_for("note-a"), IndexState::Current, "the fault must not persist beyond the one poisoned call");
    }

    #[test]
    fn an_indexing_failure_during_mark_current_does_not_fail_the_write_through_propose_and_commit() {
        // The full end-to-end version of the test above: drives the fault
        // through a REAL production write pipeline (propose_and_commit),
        // confirming (a) the commit itself still succeeds, and (b)
        // state_for honestly reports Stale afterward, since the note was
        // never actually indexed.
        let dir = unique_test_dir("fault-injection");
        init_git_repo(&dir);

        let indexer = std::sync::Arc::new(RealIndexer::new());
        struct ArcIndexer(std::sync::Arc<RealIndexer>);
        impl Indexer for ArcIndexer {
            fn mark_current(&self, note_id: &str, version: u64, record_bytes: &[u8]) {
                self.0.mark_current(note_id, version, record_bytes);
            }
            fn state_for(&self, note_id: &str) -> IndexState {
                self.0.state_for(note_id)
            }
        }
        indexer.poison_next_mark_current("note-a");
        let vault = storage::Vault::open_with_indexer(dir.clone(), &dir.join("journal.sqlite"), Box::new(ArcIndexer(indexer.clone()))).unwrap();

        let text = note_text("note-a", "internal", "some content, never actually indexed");
        let gate = policy::Gate::new(policy::SecretPolicy::new(1));
        let known: HashSet<storage::NoteId> = HashSet::new();
        let pending: HashSet<String> = HashSet::new();
        let result = storage::propose_and_commit(&vault, &text, storage::Version(0), uuid::Uuid::new_v4(), &known, &pending, &gate, None);

        assert!(
            matches!(result, Ok(storage::CommitResult::Committed(_))),
            "the write itself must succeed even though indexing failed internally: {result:?}"
        );
        assert_eq!(
            indexer.state_for("note-a"),
            IndexState::Stale,
            "state_for must honestly report Stale: the note was never actually indexed"
        );
    }

    // --- Step 3: a Sensitive note's body text is never inserted into the
    // BM25 index, verified through RealIndexer's own mark_current (the
    // real end-to-end path), not just Bm25Index::insert directly. This is
    // the second-index-structure sibling of Task 2's TrigramIndex test,
    // through the SAME production code path real writes use.

    #[test]
    fn real_indexer_never_indexes_a_sensitive_notes_body_into_bm25() {
        let indexer = RealIndexer::new();
        let sensitive_text = note_text("note-sensitive", "sensitive", "SECRET-MARKER-BODY-CONTENT");
        let record_bytes = format!("1\n---\n{sensitive_text}");
        indexer.mark_current("note-sensitive", 1, record_bytes.as_bytes());

        // A filler note so the corpus has more than one document (otherwise
        // a lone zero-score document and an empty-result query look the
        // same and the test would not distinguish "excluded" from "never
        // queried meaningfully").
        let filler_text = note_text("note-filler", "internal", "some other unrelated content entirely");
        indexer.mark_current("note-filler", 1, format!("1\n---\n{filler_text}").as_bytes());

        assert_eq!(indexer.state_for("note-sensitive"), IndexState::Current, "the note itself was indexed (structured fields only)");

        let hits = indexer.bm25_ranked("SECRET-MARKER-BODY-CONTENT", 10);
        assert!(hits.is_empty(), "a Sensitive note's body marker must never be findable via BM25: {hits:?}");
    }

    // --- Step 4: cold-start rebuild. Populate a vault with N notes
    // (including one deleted Sensitive note) via propose_and_commit
    // directly, BYPASSING the indexer entirely (SkeletonIndexer, which does
    // no real indexing), then run rebuild_index and confirm every
    // non-deleted note is queryable and the deleted one is not.

    #[test]
    fn cold_start_rebuild_indexes_every_live_note_and_skips_a_deleted_sensitive_one() {
        let dir = unique_test_dir("cold-start-rebuild");
        init_git_repo(&dir);
        let journal_path = dir.join("journal.sqlite");
        let pii_root = dir.join(".pii-store");
        let pii_store = vault_security::PiiStore::open(pii_root.clone(), &pii_root.join("master.key")).unwrap();

        let gate = policy::Gate::new(policy::SecretPolicy::new(1));
        let known: HashSet<storage::NoteId> = HashSet::new();
        let pending: HashSet<String> = HashSet::new();

        // Write every note through a vault opened with SkeletonIndexer (no
        // real indexing at all) — simulating "an index that never ran",
        // per the brief's own framing.
        {
            let vault = storage::Vault::open_with_pii_store_and_indexer(
                dir.clone(),
                &journal_path,
                &pii_store,
                Box::new(SkeletonIndexer::new()),
            )
            .unwrap();

            let live_internal = note_text("note-live-internal", "internal", "a findable_live_term appears here");
            let result = storage::propose_and_commit(&vault, &live_internal, storage::Version(0), uuid::Uuid::new_v4(), &known, &pending, &gate, Some(&pii_store));
            assert!(matches!(result, Ok(storage::CommitResult::Committed(_))), "{result:?}");

            // A second, live Sensitive note (not deleted): its structured
            // fields must still be queryable after rebuild, but never its
            // body (it has none meaningful here, by construction of the
            // pointer-record scheme, but this also exercises Document's own
            // Sensitive-classification branch through the rebuild path).
            let live_sensitive = note_text("note-live-sensitive", "sensitive", "the real plaintext body, never written to the markdown vault");
            let result = storage::propose_and_commit(&vault, &live_sensitive, storage::Version(0), uuid::Uuid::new_v4(), &known, &pending, &gate, Some(&pii_store));
            assert!(matches!(result, Ok(storage::CommitResult::Committed(_))), "{result:?}");

            // A Sensitive note that is then DELETED (tombstoned).
            let to_delete = note_text("note-deleted-sensitive", "sensitive", "SECRET-MARKER-DELETED-CONTENT");
            let result = storage::propose_and_commit(&vault, &to_delete, storage::Version(0), uuid::Uuid::new_v4(), &known, &pending, &gate, Some(&pii_store));
            let storage::CommitResult::Committed(outcome) = result.unwrap() else { panic!("expected Committed") };

            let tombstone = note_text("note-deleted-sensitive", "sensitive", storage::SENSITIVE_TOMBSTONE_BODY);
            let result = storage::propose_and_commit(&vault, &tombstone, outcome.version, uuid::Uuid::new_v4(), &known, &pending, &gate, Some(&pii_store));
            assert!(matches!(result, Ok(storage::CommitResult::Committed(_))), "tombstone write must succeed: {result:?}");
        }

        // Now rebuild a FRESH RealIndexer from scratch, reading the vault
        // cold (a brand-new Vault handle, mirroring an actual process
        // restart), outside any writer's lock.
        let vault = storage::Vault::open_with_pii_store_and_indexer(
            dir.clone(),
            &journal_path,
            &pii_store,
            Box::new(SkeletonIndexer::new()),
        )
        .unwrap();
        let real_indexer = RealIndexer::new();
        rebuild_index(&vault, &real_indexer).unwrap();

        // Every live note is queryable.
        assert_eq!(real_indexer.state_for("note-live-internal"), IndexState::Current);
        assert!(real_indexer.trigram_hits("findable_live_term").iter().any(|id| id.as_str() == "note-live-internal"));
        assert_eq!(real_indexer.state_for("note-live-sensitive"), IndexState::Current);

        // The deleted note is NOT in the rebuilt index at all: list_note_ids
        // excludes it (the deletion log), so rebuild_index never even calls
        // mark_current-equivalent logic for it.
        assert_eq!(real_indexer.state_for("note-deleted-sensitive"), IndexState::Stale, "a deleted note must not be reported as indexed");
        assert!(
            real_indexer.trigram_hits("SECRET-MARKER-DELETED-CONTENT").is_empty(),
            "a deleted sensitive note's content must never surface after rebuild"
        );
        assert!(
            real_indexer.bm25_ranked("SECRET-MARKER-DELETED-CONTENT", 10).is_empty(),
            "a deleted sensitive note's content must never surface via BM25 after rebuild either"
        );
    }

    // --- Lock-scope regression guard: rebuild_index must be callable
    // immediately after a batch of writes complete (i.e. entirely outside
    // any writer's lock) without hanging or deadlocking. A real deadlock
    // would hang this test rather than fail it cleanly, which is exactly
    // why Step 6 also runs the full `storage` test suite separately.

    #[test]
    fn rebuild_index_does_not_hang_when_called_right_after_writes_complete() {
        let dir = unique_test_dir("rebuild-no-deadlock");
        init_git_repo(&dir);
        let vault = storage::Vault::open_with_default_indexer(dir.clone(), &dir.join("journal.sqlite")).unwrap();

        let gate = policy::Gate::new(policy::SecretPolicy::new(1));
        let known: HashSet<storage::NoteId> = HashSet::new();
        let pending: HashSet<String> = HashSet::new();
        for i in 0..5 {
            let text = note_text(&format!("note-{i}"), "internal", "some content");
            let result = storage::propose_and_commit(&vault, &text, storage::Version(0), uuid::Uuid::new_v4(), &known, &pending, &gate, None);
            assert!(matches!(result, Ok(storage::CommitResult::Committed(_))), "{result:?}");
        }

        let real_indexer = RealIndexer::new();
        rebuild_index(&vault, &real_indexer).unwrap();

        for i in 0..5 {
            assert_eq!(real_indexer.state_for(&format!("note-{i}")), IndexState::Current);
        }
    }
}
