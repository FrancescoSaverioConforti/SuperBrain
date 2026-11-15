# B3-TUNE-v2 Priority 3 — Ranking Pipeline Audit

**Date:** 2026-11-14.
**Purpose:** ground-truth audit of the current ranking pipeline (real
code, not assumption), required before any Priority 3 (deterministic
reranking) design. Every claim below cites the exact file/line read in
this session. No signal is asserted absent without having read the
code path that would carry it.

**Baseline in force:** official B1 (`eval/results/B3/frozen-config.json`,
commit `52d42ea`), unmodified — Priority 1 and Priority 2 both closed
without a KEEP (see `docs/research/B3-TUNE-v2-PRIORITY1-CLOSURE.md` and
`eval/results/B3-TUNE-v2/priority2-stage1-summary.md`).

---

## 1. Candidate generation

`RealIndexer::candidate_ids(&self, query: &str) -> Vec<NoteId>`
(`crates/index/src/indexer_impl.rs:267-279`):

```rust
pub(crate) fn candidate_ids(&self, query: &str) -> Vec<NoteId> {
    let data = self.data();
    let mut ids: HashSet<_> = data.trigram.query_exact(query).into_iter().collect();
    ids.extend(
        data.bm25
            .query_ranked(query, usize::MAX)
            .into_iter()
            .map(|(id, _)| id),
    );
    let mut ids: Vec<_> = ids.into_iter().collect();
    ids.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    ids
}
```

The candidate set is the **union** of:
- `TrigramIndex::query_exact(query)` — the *whole query string* run as
  one substring query against the trigram index (not per-token). This
  index is case-insensitive, Unicode-`char`-based, and verifies a real
  literal substring via `contains_term` after the trigram-intersection
  prefilter (`crates/index/src/trigram.rs:123-150`). A query under 3
  characters has no trigrams and contributes nothing here
  (`trigrams_of`, `trigram.rs:166-174`).
- `Bm25Index::query_ranked(query, usize::MAX)` — every note with a
  strictly positive BM25 score for the query, i.e. at least one
  whitespace-delimited, case-folded query token also appears in the
  note's indexed text (`bm25.rs:168-208`). No smoothing floor exists
  (`bm25.rs:45-57`, `200`): zero term overlap is always zero score and
  never a candidate via this path.

No other source feeds the candidate set. There is no project/type/
status/trust/time narrowing at this stage — that happens later, per
result, inside `search_impl` (Section 3). This means a query can
surface candidates from every project and every classification before
filtering; nothing here limits candidate generation to the active
project.

## 2. Final BM25 scoring (`fresh_scores`)

Inside `Index::search_impl` (`crates/index/src/search.rs:238-420`):

1. A **fresh, per-request** `Bm25Index` (`fresh_scores`, line 295) is
   built from scratch, seeded only with the documents that survive the
   per-candidate authoritative-read + filter loop (line 362:
   `fresh_scores.insert(id.clone(), &document.text_searchable)` — only
   reached after the project/type/status/trust/time checks at lines
   354-361 pass).
2. After the loop, `fresh_scores.query_ranked(query, usize::MAX)`
   (line 392) scores exactly that filtered candidate pool — IDF and
   `avgdl` are therefore scoped to **this query's eligible candidates**,
   never the whole corpus and never the persistent `RealIndexer`-level
   `Bm25Index` used for candidate generation. This is the real-pipeline
   distinction that Priority 2 Lever B's standalone-scorer measurement
   got wrong (it used whole-corpus IDF/avgdl instead) — confirmed
   directly from this code, not inferred.
3. Each `SearchResult.score` is the resulting BM25 score, defaulting to
   `0.0` if the fresh index didn't rank it at all (line 396) — this can
   happen for a result that reached the loop via `query_match`'s
   exact-substring branch (Section 4) without any BM25 token overlap.
4. `k1`/`k2`→(`b`) for this final scoring stage come from
   `Index::bm25_k1`/`bm25_b`, defaulted to `(1.2, 0.75)` in `new()` and
   overridable only via the additive, ablation-only
   `with_bm25_params` builder (lines 184-214) — production call sites
   never call it, so production always scores at the conventional
   defaults.

**The persistent, `RealIndexer`-level `Bm25Index` (and `TrigramIndex`)
are used only for candidate membership, never for final scores.**
Final score always comes from the freshly built, per-query,
candidate-scoped index.

## 3. Filtering

All filtering happens inside the per-candidate loop in `search_impl`,
after an authoritative re-read of the note (`self.vault.read_note`,
line 314) and after re-parsing it fresh (`storage::split_record`, line
327) — the response never trusts anything cached at index time. In
order:

1. **Storage-level exclusion** (lines 316-325): `Deleted`,
   `NoteDeleted`, `NotFound`, `Inconsistent` → silently skipped (not an
   error, not a candidate). Any other storage error → the whole search
   fails closed (`SearchError::ReadUnavailable`), never silently
   degrading to "not present."
2. **ID consistency check** (line 330): the record's own `metadata.id`
   must equal the candidate id being iterated, or it's skipped.
3. **`query_match`** (line 351, defined 425-435) — re-validates the
   match against the FRESH text, not the candidate index's stale
   postings (Section 4).
4. **Structured filters** (lines 354-361), all exact-match, all
   AND-combined, any one failing drops the candidate entirely before it
   is scored or returned:
   - `document.project != project` (active or explicitly requested
     project — `ProjectScope`)
   - `filters.type_filter` vs `document.r#type` (`NoteType`)
   - `filters.status_filter` vs `document.status` (`NoteStatus`)
   - `filters.trust_filter` vs `document.trust` (`TrustLevel`)
   - `filters.date_filter` resolved via `matches_time` (lines 437-456)
     against `created`/`updated`/`event_period`, per `TimeField`.

   These are **caller-supplied, optional** filters (`SearchFilters`,
   lines 46-53) — none is applied automatically by default; a caller
   that passes `SearchFilters::default()` gets only the project scope
   (defaults to `ActiveOnly`) with no type/status/trust/time narrowing
   at all.
5. **Classification-based redaction**, not exclusion: a `Sensitive`
   note is never dropped from candidates by classification alone — its
   `source` is withheld (`None` at line 345-349) and its excerpt never
   uses the body (`excerpt` logic, lines 369-372, guards on `!sensitive`).

No trust level, note type, status, recency, or project/source metadata
is currently used as a **scoring** signal — all five are exact-match,
opt-in **filters** only, applied after candidate generation and before
final scoring. None of them influences score magnitude or tie-break
(Section 5).

## 4. `MatchKind` and trigram/substring role

`MatchKind` (`search.rs:109-113`) has exactly two variants, assigned by
`query_match` (lines 425-435):

```rust
fn query_match(text: &str, query: &str) -> Option<MatchKind> {
    let needle = query.to_lowercase();
    if needle.chars().count() >= 3 && text.to_lowercase().contains(&needle) {
        return Some(MatchKind::ExactSubstring);
    }
    let terms: HashSet<_> = tokenize(text).into_iter().collect();
    tokenize(query)
        .iter()
        .any(|term| terms.contains(term))
        .then_some(MatchKind::RankedTerms)
}
```

- `ExactSubstring` — the **whole query string** (≥3 chars) is a literal
  case-insensitive substring of the candidate's fresh `text_searchable`.
  Checked first; if it matches, `RankedTerms` is never evaluated for
  that candidate.
- `RankedTerms` — otherwise, at least one BM25-tokenized query term
  overlaps the candidate's tokenized text. Uses the same `tokenize`
  function BM25 itself uses (`crate::bm25::tokenize`, imported at line
  25), by design (module doc comment, lines 1-11): "the tokenizer is
  shared with BM25 so re-validation cannot drift from it."
- Neither condition matching → the candidate is dropped entirely (`let
  else { continue }`, line 353) — it never reaches scoring or the
  response, **even if it was in `candidate_ids`'s union.** This is
  possible because `candidate_ids` runs `TrigramIndex::query_exact` on
  the *persisted, possibly stale* text, while `query_match` re-checks
  against the *freshly re-read* text; a note that changed between
  indexing and this request can fail re-validation and silently drop
  out.

`MatchKind` is stored on the result (`SearchResult.match_kind`) and
returned to the caller, but **is not used by `search_impl` itself for
scoring or tie-break** — a `MatchKind::ExactSubstring` candidate with
BM25 score 0.0 sorts no differently than any other 0.0-score candidate;
`MatchKind` is informational/explanatory metadata only at this layer.
(Whether a caller elsewhere in the `api` crate uses `MatchKind` to
influence ordering was not checked in this session — out of scope for
B1's own `index` crate, which is what Priority 2/3 operate on; flagged
below as an open item if Priority 3 needs it.)

## 5. Tie-breaking and final ordering

`search_impl`, lines 398-403:

```rust
fresh_results.sort_by(|a, b| {
    b.score
        .total_cmp(&a.score)
        .then_with(|| a.note_id.as_str().cmp(b.note_id.as_str()))
});
fresh_results.truncate(k);
```

Ordering is: **BM25 score descending** (primary, via `total_cmp` — a
total order over `f64`, including NaN-safety), then **note id
lexicographic ascending** (tie-break, purely for deterministic output,
not a meaningful relevance signal). This is the **only** ordering
signal in the entire pipeline. No other field — trust, type, status,
recency, project, source, `MatchKind`, exact-identifier-match boost —
participates in sort order today.

`Bm25Index::query_ranked`'s own internal sort (`bm25.rs:200-207`) uses
the identical tie-break pattern (score, then note id), but that
internal order is irrelevant to the final response: `search_impl`
discards `fresh_scores.query_ranked`'s own order and only consumes its
score-per-id as a lookup (`HashMap`, line 391-394), re-sorting
`fresh_results` independently at lines 398-403.

## 6. Signals confirmed present elsewhere but NOT used for ranking today

Confirmed by reading `document.rs` (lines 96-111, 153-198) and
`search.rs` (46-53, 354-361): every one of these fields exists, is
carried on `Document`/`SearchFilters`, and is already read by the
pipeline — but only as an optional exact-match **filter**, never as a
score or tie-break input:

- **Trust level** (`TrustLevel`: Web/Internal/Official) — filter only.
- **Note type** (`NoteType`: Code/Bug/Decision/Pattern/Research/
  Session/Summary) — filter only.
- **Status** (`NoteStatus`: Draft/TestPassed/Reviewed/Verified/
  Superseded/Contradicted) — filter only.
- **Recency/temporal metadata** (`created`, `updated`, `event_period`)
  — filter only (`date_filter`); never a recency boost or decay term.
- **Project/source metadata** — `project` is filter only (and also
  folded into `text_searchable`'s structured-fields block, so it can
  coincidentally affect BM25 score as ordinary token overlap, never as
  a deliberate signal); `source` is appended to `text_searchable` for
  Internal/Public notes only (never for Sensitive — `document.rs:189-
  190`) and is otherwise not read by `search_impl` except for the
  redaction check at line 345.
- **Exact identifier match** — there is no dedicated "query equals a
  note id" or "query equals a frontmatter identifier" boost. The
  closest existing behavior is `MatchKind::ExactSubstring`, which is
  purely text-substring, not identifier-aware, and (per Section 4) does
  not currently affect score or order.

**No signal was assumed absent without verification.** Every item above
was checked against the field's actual appearance (or non-appearance)
in the scoring (`bm25.rs`) and ordering (`search.rs:398-403`) code
paths, not inferred from naming or expectation.

## 7. Summary table

| Stage | Input | Code |
|---|---|---|
| Candidate generation | trigram substring (whole query) ∪ BM25 term overlap (persistent index) | `indexer_impl.rs:267-279` |
| Re-validation | fresh literal substring OR fresh BM25 token overlap (`MatchKind`) | `search.rs:425-435` |
| Filtering (opt-in) | project, type, status, trust, time — exact match, AND-combined | `search.rs:354-361` |
| Redaction (automatic) | `Sensitive` → source withheld, body excluded from excerpt | `search.rs:345-349, 369-372` |
| Final scoring | fresh, per-query, candidate-scoped BM25 (`k1=1.2, b=0.75` in production) | `search.rs:295, 391-397` |
| Ordering | score desc, then note id asc (tie-break only) | `search.rs:398-403` |

This is the complete, verified picture of B1's current ranking
pipeline as of commit `d7f182f` (branch `b3-retrieval-eval`). Priority
3's design (deterministic reranking) must build strictly on top of
this — it changes nothing documented above by itself; this document is
audit only, no code change.
