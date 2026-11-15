# B4.6 — `search_impl` Wiring for `RetrievalMode::DenseV2`

**Status: implemented** (`crates/index/src/search.rs`). This document
records the design as built, including the one correction made to
B4.2's own trait shape, discovered while implementing this task.

## 1. The flagged question, resolved

B4.4's design doc (Section 6) flagged: does `DenseV2` rank the
*lexical* candidate set, or does it use the dense index's *own*
candidate set? **Resolved: the dense index's own set.** Confirmed
directly against the real evaluation harness
(`crates/api/examples/b3_tune_v2_priority4_dense_calibration/main.rs`):
`note_vectors.iter()` scores every note the dense index holds, with
zero lexical pre-filtering. Restricting `DenseV2` to lexical
candidates would make it structurally unable to surface anything
lexical search didn't already find — defeating dense retrieval's
entire purpose and silently diverging from every number in
`frozen-config-v2.json`.

## 2. Correction to B4.2's `DenseScorer` trait

B4.2's original trait only had `score(query, candidate_ids)` — it
assumed the CALLER already knew the candidate set. Resolving
Section 1 required knowing what to ask `score` to rank in the first
place, so a new method was added: `candidate_ids(&self) -> Option<Vec<NoteId>>`,
returning everything the dense index currently holds (or `None` under
the same structurally-unavailable conditions `score` already uses).
Both `ProductionDenseScorer` and the test-only `StubDenseScorer` were
updated to implement it.

## 3. `search_impl` changes

- **Candidate generation:** `RetrievalMode::DenseV2` with an available
  scorer uses `dense_scorer.candidate_ids()`; `Lexical`, or `DenseV2`
  with no scorer attached or a scorer reporting `None`, uses the
  existing `self.indexer.candidate_ids(query)` — byte-identical
  fallback, confirmed by dedicated tests.
- **`query_match` bypass:** a new `MatchKind::SemanticMatch` variant
  lets a dense-sourced candidate skip the literal-substring/BM25-
  token-overlap gate entirely — a semantically related note is
  expected to share zero literal tokens with the query, so applying
  that gate would wrongly reject exactly the candidates dense
  retrieval exists to surface.
- **Final scoring:** under `using_dense`, scores come from
  `dense_scorer.score(query, &eligible_ids)` (restricted to the
  post-filter candidate set), never BM25's `fresh_scores`. The
  identifier-boost lever (Priority 3 P1, BM25-specific) is skipped
  entirely under dense scoring — it has no meaning for a cosine score.
- **Unchanged:** structured filters (project/type/status/trust/time),
  `Sensitive`-redaction, the authoritative per-candidate vault re-read,
  and the final note-id lexicographic tie-break all run identically
  regardless of `retrieval_mode` — `RetrievalMode` only changes
  candidate generation and scoring, exactly as frozen in B4.0.

## 4. Tests (all passing)

1. `densev2_with_an_unavailable_scorer_falls_back_to_lexical_byte_identically`
2. `densev2_with_no_scorer_attached_falls_back_to_lexical`
3. `densev2_with_an_available_scorer_uses_dense_scores_not_bm25`
4. `densev2_candidate_set_is_the_dense_indexs_own_not_the_lexical_generators`
   (the test that would have failed under the rejected
   lexical-candidates design)
5. `densev2_still_applies_existing_structured_filters_and_privacy_redaction`
6. `densev2_deterministic_tie_break_on_equal_dense_scores_matches_lexicals_own_convention`

## 5. What remains for later tasks

- B4.7: broader integration tests (concurrency, crash/recovery under
  `DenseV2` specifically).
- `fresh_scores.insert(...)` still runs for every eligible candidate
  even under `DenseV2` (the BM25 index is built but never consulted)
  — harmless but wasteful; not fixed here, flagged as a minor,
  named inefficiency for a future cleanup pass, not a correctness
  issue blocking B4.
