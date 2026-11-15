# B4.2 — Dense Model Runtime: Trait Boundary and Design

**Status: design frozen before implementation begins.** Resolves
B4.1's own flagged consequence: `index::search::RetrievalMode::DenseV2`
exists (B4.0) but `index` has zero dependency on any embedding
runtime (B4.1 decision: the runtime lives in `api`) — so `Index` needs
a way to USE a dense scorer without depending on it.

## 1. The trait, in `index`

```rust
// crates/index/src/search.rs (or a new crates/index/src/dense.rs module)

/// Injected capability (mirrors `storage::Indexer`'s own cross-crate
/// trait-boundary pattern exactly): `index` depends on no embedding
/// runtime. A caller that wants `RetrievalMode::DenseV2` to do
/// anything supplies an implementation of this trait at `Index`
/// construction time; `index` never knows or cares what's behind it.
pub trait DenseScorer: Send + Sync {
    /// Returns a ranked list of (note_id, cosine_similarity) pairs for
    /// `query`, restricted to `candidate_ids` -- NEVER a wider set.
    /// `index` still re-reads, re-validates and re-filters every
    /// returned id through the existing authoritative per-candidate
    /// loop (search.rs:379-464) exactly as it does for the lexical
    /// path; this method only proposes which ids to consider and a
    /// score to rank them by.
    ///
    /// Returns `None` if the dense path is structurally unavailable
    /// (model/index not loaded, hash-mismatched, runtime failed to
    /// init -- B4.0's frozen fallback conditions) -- `search_impl`
    /// falls back to the Lexical candidate set for that request. A
    /// transient error (as opposed to a structural unavailability)
    /// is the implementation's own responsibility to resolve into one
    /// of these two outcomes before returning; this trait has no
    /// third "error" variant, by design, so a `DenseScorer`
    /// implementation cannot accidentally propagate a transient IO
    /// hiccup as a permanent-looking `None` without choosing to.
    fn score(&self, query: &str, candidate_ids: &[storage::NoteId]) -> Option<Vec<(storage::NoteId, f64)>>;

    /// Builds (or confirms up to date) the dense index for every live
    /// note `vault` holds, mirroring `index::indexer_impl::rebuild_index`'s
    /// own cold-start contract exactly (same caller discipline: outside
    /// any writer's lock, called once at startup or on-demand).
    fn rebuild(&self, vault: &storage::Vault) -> Result<(), DenseScorerError>;

    /// Called once per note write (mirroring `storage::Indexer::mark_current`'s
    /// own per-write hook), to keep the dense index incrementally
    /// current without a full rebuild. `document` is the SAME
    /// `index::document::Document` the lexical path already built for
    /// this note -- this method embeds `document.text_searchable`,
    /// never a raw body, enforcing privacy parity (B4.5) at the type
    /// level: there is no raw-body parameter for an implementation to
    /// reach for even by mistake.
    fn on_note_indexed(&self, note_id: &storage::NoteId, document: &crate::document::Document);

    /// Called when a note is deleted, mirroring the same deletion-log-
    /// driven removal B1's own lexical indexes already go through on
    /// rebuild (a deleted note is absent from `Vault::list_note_ids`,
    /// so `rebuild` never re-adds it) -- this is the INCREMENTAL path's
    /// removal call, for a delete that happens between rebuilds.
    fn on_note_deleted(&self, note_id: &storage::NoteId);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DenseScorerError {
    /// The vault could not be read during rebuild (mirrors
    /// `storage::StorageError` at the boundary without `index` needing
    /// to depend on knowing every storage error variant).
    VaultUnavailable,
    /// The dense index could not be written (storage layer, B4.3/B4.4).
    IndexWriteFailed,
}
```

**Why `Option<Vec<...>>` and not `Result<Vec<...>, Error>` for
`score`:** per B4.0's frozen fallback contract, "dense path
unavailable" is never an error condition from `Index::search`'s own
point of view — it is a signal to silently use `Lexical` candidates
instead. Forcing this through a `Result` would tempt `search_impl` to
propagate an `Err` as a `SearchError`, which B4.0 explicitly forbids
for this exact case. `None` is the type-level enforcement of "this is
a fallback trigger, never a `SearchError`" — the same reasoning
`RetrievalMode`'s own two-variant (not three) shape used in B4.0.

**Why `rebuild`/`on_note_indexed`/`on_note_deleted` instead of a
single "sync everything" method:** mirrors `storage::Indexer`'s own
real split between `mark_current` (one call per write) and
`index::indexer_impl::rebuild_index` (one cold-start function,
separate from the trait) — keeping the same two-path shape (cold-start
rebuild vs. incremental per-write update) this project already uses
for the lexical indexes, rather than inventing a different lifecycle
model for the dense one.

## 2. Where this trait is held on `Index`

```rust
// crates/index/src/search.rs, Index struct

pub struct Index<'a> {
    // ...existing fields...
    dense_scorer: Option<&'a dyn DenseScorer>,
}
```

New builder: `Index::with_dense_scorer(mut self, scorer: &'a dyn DenseScorer) -> Self`.
Default: `None` — every existing call site (which never calls this
builder) behaves exactly as before B4.2, identical to every other
additive builder in this file.

**`search_impl`'s own branch** (B4.6's actual wiring job, not done in
this task): when `self.retrieval_mode == RetrievalMode::DenseV2`:
- If `self.dense_scorer` is `None`, OR the scorer's `score(...)`
  returns `None` → use the existing `self.indexer.candidate_ids(query)`
  + BM25-scoring path unchanged (the `Lexical` fallback, per B4.0).
- If `Some(scores)` → candidate ids come from `scores`' own keys
  (restricted exactly to what the scorer returned, per the trait's own
  "NEVER a wider set" contract above) instead of
  `self.indexer.candidate_ids(query)`; the per-candidate authoritative-
  read/filter/redaction loop (search.rs:379-464) runs **unchanged** on
  those ids; final `result.score` comes from the scorer's returned
  cosine value instead of BM25's `fresh_scores.query_ranked(...)`
  lookup.

This is designed now, in B4.2, so the trait's shape is right before
any implementation is written against it — the actual edit to
`search_impl` is B4.6's own atomic commit, not bundled into B4.2.

## 3. The real implementation — `api::dense`

New module `crates/api/src/dense.rs`, compiled only under the
`dense-retrieval` feature (`#[cfg(feature = "dense-retrieval")]` on
the module declaration in `lib.rs`), implementing `index::search::DenseScorer`.

**Reuses, does not reinvent, the evaluation harness's own frozen
representation-semantics logic** (`crates/api/examples/priority4_common/embedding.rs`):
tokenization, prefix (empty for MiniLM — confirmed:
`ModelSpec { name: "all-MiniLM-L6-v2", query_prefix: "", document_prefix: "", ... }`),
truncation to 256 tokens, mean pooling over the attention mask, L2
normalization, cosine similarity. The evaluation harness's own code is
NOT imported directly (it is `include!`d example-binary code, not a
library — per the design review's own "what must not enter the
production path" list) — `api::dense` is a fresh implementation
against the SAME frozen parameters, verified against
`eval/results/B3-TUNE-v2/frozen-config-v2.json` and the shortlist
audit's own `MODEL_SPECS` entry for `all-MiniLM-L6-v2` specifically
(query/document prefix both empty, max_seq_len 256, mean pooling, L2
normalization) — not re-derived from memory.

**One real gap the evaluation harness never had to close, closed
here:** the evaluation harness fetches and loads the ONNX/tokenizer
artifacts without ever verifying their SHA-256 against
`frozen-config-v2.json`'s own recorded
`onnx_artifact_sha256`/`tokenizer_artifact_sha256` fields — it trusted
whatever HTTPS delivered. **Production code verifies both hashes
before loading**, per B4.1's own "Pinning/hash verification" decision
and the design doc's own Section 6 requirement — a mismatch is a hard
load failure (not a silent continue), which becomes one of B4.0's
frozen "structurally unavailable" conditions (triggering the
`Lexical` fallback at the `DenseScorer::score` level, never a panic
reaching a caller).

## 4. Golden parity test

A fixed, small set of (text, expected-embedding-vector) pairs, checked
within an explicit numerical tolerance (not bitwise-exact, since ONNX
Runtime floating-point results can have tiny last-bit differences
across runs/platforms) — **tolerance decision: `1e-5` absolute
per-component**, chosen as comfortably tighter than any meaningful
retrieval-ranking difference (cosine similarities differing by `1e-5`
cannot plausibly reorder a top-10 ranking on real queries) while
loose enough not to flag ordinary floating-point noise. The fixed
input set reuses literal query/document strings from the already-
committed evaluation reports (e.g. a DEV-split query whose embedding
the evaluation harness already computed and whose resulting
Recall@10/nDCG@10 are already in `priority4-dev-all-MiniLM-L6-v2.json`)
so the "expected" vector in this test is not invented — it is
recomputed by this production implementation and checked for
agreement with what the evaluation's own run already produced for
that exact input (requires instrumenting one offline comparison run;
the fixed expected values are then hardcoded into the test, same
discipline golden tests always use).

## 5. STOP — design frozen, implementation follows in dependent commits

This document does not implement anything. B4.2's own implementation
commits (the trait in `index`, then `api::dense`, each TDD'd
separately) follow as the next atomic steps.
