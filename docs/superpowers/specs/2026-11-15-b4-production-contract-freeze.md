# B4.0 — Frozen Production Contract

**Status: FROZEN before any B4 code is written.** This document is
the single source of truth for `RetrievalMode`'s exact shape and
integration contract. No later B4 task may change what is frozen here
without stopping and documenting why, per the user's explicit
instruction.

**Source documents verified against the real codebase (not assumed)
before writing this freeze:**
- `docs/research/B3-TUNE-v2-PRIORITY4-PRODUCTION-INTEGRATION-DESIGN.md`
  Section 3 (the design's own `RetrievalMode` sketch, post-review).
- `docs/research/B3-TUNE-v2-PRIORITY4-DESIGN-REVIEW.md` (commit
  `13441d6`, B4.0-B4.8 breakdown).
- `docs/research/B3-TUNE-v2-CLOSURE-SUMMARY.md`.
- `eval/results/B3-TUNE-v2/frozen-config-v2.json`.
- `crates/index/src/search.rs` (real `Index::search` signature, read in
  full).
- `crates/api/src/search.rs`, `crates/api/src/lib.rs` (real
  `api::search::search` wrapper, read in full).

## 1. Divergence found between plan and codebase — documented per instruction, architecture unchanged

The design doc's Section 3 discusses `Index::search` only. Reading the
real codebase surfaces one additional integration point the design did
not name: **`api::search::search`** (`crates/api/src/search.rs:14-26`)
is a thin wrapper around `Index::search` that takes a flat
`SearchRequest { text, filters, k }` struct and performs a policy
check (`check(context, policy, ReadOperation::Search)`) before calling
`index.search(&request.text, request.filters, request.k)`.

**If `Index::search` grows a `retrieval_mode` parameter and
`SearchRequest`/`api::search::search` are not also updated, the `api`
layer can never request `DenseV2` at all** — every caller going
through `api::search::search` (which is the documented, policy-checked
entrypoint `RequestContext`/`ApiResponse` wraps) would be permanently
stuck on the enum's default (`Lexical`), regardless of what
`Index::search` itself supports. This would make the whole feature
unreachable through the one real production call path that exists
today, while `Index::search` itself technically "supports" it — a gap
the design doc, written before this file was read, could not have
caught.

**Resolution, consistent with the frozen architectural principle (one
additive, opt-in, default-`Lexical` parameter, never a parallel API):**
`SearchRequest` gains one new field, `retrieval_mode: RetrievalMode`,
and `api::search::search` passes it through unchanged to
`index.search(...)`. This is the same "extend the existing call,
don't add a parallel one" principle the design doc Section 3 already
committed to for `Index::search` itself — applied one layer up,
because that is where the real, only production entrypoint actually
lives. No architectural principle changes; this is purely following
the real code where the plan was silent, exactly as instructed.

## 2. `RetrievalMode` — exact type, frozen

```rust
// crates/index/src/search.rs

/// Frozen contract (B4.0, `docs/superpowers/specs/2026-11-15-b4-production-contract-freeze.md`).
/// Selects which retrieval pipeline `Index::search` uses to generate and
/// score candidates. Filtering, privacy/redaction, and the final top-k
/// truncation are IDENTICAL across every variant -- only candidate
/// generation and scoring differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RetrievalMode {
    /// Default. Byte-identical to every `Index::search` call site's
    /// behavior before this enum existed -- B1 trigram+BM25 only. Zero
    /// behavior change for any caller that does not explicitly opt in.
    #[default]
    Lexical,
    /// Opt-in. `v2 MiniLM dense-only`, per
    /// `eval/results/B3-TUNE-v2/frozen-config-v2.json`. Falls back to
    /// `Lexical` behavior, with an explicit logged warning, if the dense
    /// model/index is structurally confirmed unavailable (Section 5
    /// below) -- never a hard `SearchError` for that reason alone.
    DenseV2,
}
```

- **Name:** `RetrievalMode`. **Field name on `Index` / parameter name
  on `search`/`search_profiled`:** `retrieval_mode`.
- **Derives:** `Debug, Clone, Copy, PartialEq, Eq, Default` — mirrors
  the derive style already used by `ProjectScope`/`TimeField` in the
  same file (`search.rs:31-44`), and `Copy` is cheap and correct since
  the enum carries no data.
- **Default:** `RetrievalMode::Lexical`, via `#[default]` (stable Rust
  derive attribute, simpler and less error-prone than a hand-written
  `impl Default` — functionally identical to the design doc's sketch).

## 3. Exact integration points — every real call site named

| Call site | Change | Backward-compat mechanism |
|---|---|---|
| `Index` struct (`search.rs:192-211`) | new private field `retrieval_mode: RetrievalMode` | `new()` sets it to `RetrievalMode::default()` — every existing `Index::new(...)` call compiles and behaves unchanged |
| `Index::new` (`search.rs:231-247`) | no signature change | unchanged — `retrieval_mode` is set via the new builder below, not a constructor parameter, mirroring `identifier_boost`/`status_tiebreak`'s own pattern |
| New: `Index::with_retrieval_mode` | new additive builder, same shape as `with_identifier_boost`/`with_status_tiebreak` | default-off; every existing call site that never calls this builder is unaffected |
| `Index::search` (`search.rs:285-292`) | **no signature change** | reads `self.retrieval_mode` (set at construction via the builder), not a new parameter on `search` itself — this is the simplest resolution consistent with every other B3-TUNE-v2 lever's own precedent (`with_bm25_params`, `with_identifier_boost`, `with_status_tiebreak` are all construction-time builders, never per-call parameters) |
| `Index::search_profiled` (`search.rs:296-305`) | same as above | same |
| `api::search::SearchRequest` (`crates/api/src/search.rs:8-12`) | **no new field** | `api::search::search` does not construct `Index` itself (its caller does, and passes an already-built `&Index<'_>` in) — so `retrieval_mode` is already fully threaded through once `Index` carries it at construction time; no change needed here at all |

**Correction to Section 1's own preliminary analysis, found while
writing this exact table:** on closer reading, `api::search::search`
(`crates/api/src/search.rs:14-19`) takes `index: &Index<'_>` as an
already-constructed value — it does not build an `Index` itself, so
it never needs to know about `retrieval_mode` as a request-level
concept at all. The caller who builds the `Index` (via
`Index::new(...).with_retrieval_mode(...)`) decides the mode once, at
construction time, exactly as `with_bm25_params`/`with_identifier_boost`/
`with_status_tiebreak` already do. **Section 1's analysis above
correctly identified a real risk (the `api` layer could get stuck
unable to reach `DenseV2`) but proposed the wrong fix (a `SearchRequest`
field) before this table forced a precise read of `api::search::search`'s
actual signature.** The real fix is simpler: nothing in `crates/api/src`
needs to change at all, because `RetrievalMode` lives on `Index`
construction, not on the per-call `SearchRequest`. This correction is
left visible here, not silently edited away, per the instruction that
the real code's details prevail and divergences should be documented,
not hidden once found.

**Final, verified integration shape:** exactly one new builder method
on `Index`, zero signature changes to `search`/`search_profiled`, zero
changes anywhere in `crates/api/src/`. This is the minimal change
consistent with the frozen architectural principle and the real,
verified code.

```rust
impl<'a> Index<'a> {
    /// Additive builder (B4.0 frozen contract). Selects `retrieval_mode`
    /// for this `Index` instance. Default (never called): `Lexical`,
    /// identical to every pre-B4 call site.
    pub fn with_retrieval_mode(mut self, mode: RetrievalMode) -> Self {
        self.retrieval_mode = mode;
        self
    }
}
```

## 4. Semantic contract

- **`RetrievalMode::Lexical`:** byte-identical candidate generation and
  scoring to `Index::search`'s behavior before this enum existed. This
  is the regression contract B4.7's "Lexical unchanged" test enforces
  — a diff-free comparison against pre-B4 `search_impl`, not merely "no
  test fails."
- **`RetrievalMode::DenseV2`:** replaces ONLY the candidate-generation-
  and-scoring stage (today: `self.indexer.candidate_ids(query)` +
  `fresh_scores.query_ranked(...)`, `search.rs:371-375` and
  `466-469`) with the frozen v2 dense pipeline. Every other stage of
  `search_impl` — the per-candidate authoritative vault re-read
  (`search.rs:379-464`), structured filters (project/type/status/trust/
  date, `search.rs:428-435`), `Sensitive`-redaction (`search.rs:407,
  419-423`), and final top-k truncation (`search.rs:500`) — runs
  **unchanged**, regardless of `retrieval_mode`. Dense retrieval is a
  new candidate-generation-and-scoring source, never a parallel
  filtering/privacy pipeline (this matches the design doc's own
  Section 3 commitment verbatim).
- **Interaction with `filters`/`k`:** unchanged in shape and meaning
  across both modes — identical to the design doc's own commitment,
  now confirmed against the real `search_impl` code path cited above.
- **Error semantics:** `DenseV2` never returns a `SearchError` merely
  because the dense path is structurally unavailable (not built,
  model-hash-mismatched, runtime init failure) — it falls back to
  `Lexical`'s own existing candidate-generation-and-scoring behavior
  for that request, with an explicit logged warning (mechanism: B4.1's
  decision on whether this project has an existing logging facade;
  absent one, `eprintln!`/a new minimal `log`-crate-style macro is
  B4.1's call, not frozen here). A **transient** failure during the
  availability check itself (e.g. an IO error reading the index file)
  is `SearchError::ReadUnavailable` — the fallback never silently
  swallows a transient error, only a structurally-confirmed-unavailable
  state, per the design doc's own deterministic-fallback-behavior
  commitment.
- **Deterministic ordering:** `DenseV2`'s own candidate ordering is
  cosine-similarity descending, then the SAME note-id lexicographic
  ascending tie-break `search_impl`'s existing sort already uses
  (`search.rs:487-498`) — no new tie-break rule, reusing the existing
  comparator's trailing `.then_with(|| a.note_id.as_str().cmp(...))`
  clause for both modes.
- **Status/trust/privacy filters:** apply identically in both modes,
  because they are evaluated in the per-candidate loop
  (`search.rs:428-435`) AFTER candidate generation, which is the only
  stage `retrieval_mode` changes. This is not a new design decision —
  it falls out mechanically from `retrieval_mode` only touching the
  stage it's scoped to.

## 5. Fallback trigger conditions — frozen, exhaustive

`DenseV2` falls back to `Lexical`'s candidate-generation-and-scoring
behavior (never a hard error) if and only if, at the time of the
search call:

1. The dense vector index has not yet been built for the active
   project (B4.3/B4.4's responsibility to define "built").
2. The dense vector index's stored model-identity hash does not match
   the currently-loaded model's hash (B4.2/B4.3's responsibility).
3. The ONNX Runtime environment or the MiniLM session failed to
   initialize (B4.2's responsibility) — logged loudly, since this
   likely indicates a packaging problem (B4.8), not a transient data
   issue, per the design doc's own distinction.

Any other failure (e.g. a filesystem read error on an index file that
*should* exist and passed its own integrity check) is a `SearchError`,
never a silent fallback — consistent with B1's own existing
fail-closed discipline for storage errors (`search.rs:398`:
`Err(_) => return Err(SearchError::ReadUnavailable)`).

## 6. What remains explicitly open for later B4 tasks (not frozen here)

- The exact vector-index file format, atomic-write mechanism, and
  corruption-detection checksum (B4.3/B4.4).
- The exact embedding-generation trigger (lazy-on-read vs. background
  worker) (B4.3).
- The exact logging mechanism for the fallback warning (B4.1, depends
  on whether a logging facade already exists in this workspace —
  checked at B4.1 time, not assumed here).
- The exact production dependency promotion mechanics (feature flag
  name, vendored-vs-fetched ONNX Runtime) (B4.1, per the design doc's
  own Section 6a, itself deferred to B4.0/B4.1 by the design review).

## 7. STOP — this freeze is complete

B4.1 begins from exactly this contract. No field name, type, default,
or fallback condition above may be changed without stopping and
documenting why, per the user's explicit instruction that frozen
retrieval semantics, the public API contract, and architectural
principles are hard stop conditions, not routine implementation
choices.
