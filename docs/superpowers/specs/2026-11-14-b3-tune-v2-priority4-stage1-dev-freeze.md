# B3-TUNE-v2 Priority 4 Stage 1 — Frozen DEV-Benchmarking Spec

**Date:** 2026-11-14. **Status: FROZEN before any benchmark runs.**
Nothing in this document may change after Task 1 benchmarking starts,
per the Stage 1 plan's Task 0 requirement
(`docs/superpowers/plans/2026-11-14-b3-tune-v2-priority4-stage1-dev-only.md`).

All model-specific facts below were verified against each model's live
HuggingFace repository metadata (`config.json`, `sentence_bert_config.json`,
`1_Pooling/config.json`, `README.md`) at the pinned revisions in
Section 1, not assumed from the shortlist audit's documentation-only
estimates. Two corrections against that earlier audit are called out
explicitly in Section 4.

## 1. Pinned model revisions

| Model | HF repo | Pinned `sha` (full commit) |
|---|---|---|
| MiniLM | `sentence-transformers/all-MiniLM-L6-v2` | `1110a243fdf4706b3f48f1d95db1a4f5529b4d41` |
| mpnet | `sentence-transformers/all-mpnet-base-v2` | `e8c3b32edf5434bc2275fc9bab85f82640a19130` |
| BGE | `BAAI/bge-small-en-v1.5` | `5c38ec7c405ec4b44b94cc5a9bb96e735b38267a` |
| Nomic | `nomic-ai/nomic-embed-text-v1.5` | `e9b6763023c676ca8431644204f50c2b100d9aab` |

Each benchmark run must record which pinned `sha` it actually fetched
against, and fail loudly (not silently proceed) if a re-fetch resolves
to a different revision than pinned here.

## 2. Embedding representation semantics (frozen per model)

### MiniLM (`all-MiniLM-L6-v2`)

- **Document text embedded:** B1's `Document::text_searchable` (via
  `index::document::Document::from_record_bytes`), unmodified.
- **Query representation:** raw query string, no prefix.
- **Query/document prefix:** none required.
- **Max sequence length:** 256 tokens (`sentence_bert_config.json`).
- **Truncation policy:** truncate to 256 tokens (tokenizer truncation,
  tail discarded) — see Section 3 for the chunking-necessity check
  against the actual DEV corpus.
- **Pooling:** mean pooling over all token embeddings
  (`1_Pooling/config.json`: `pooling_mode_mean_tokens: true`), using
  the model's own attention mask to exclude padding tokens from the
  mean.
- **Embedding normalization:** L2-normalize the pooled embedding.
- **Similarity metric:** cosine similarity.

### mpnet (`all-mpnet-base-v2`)

- **Document text embedded:** same as MiniLM.
- **Query representation:** raw query string, no prefix.
- **Query/document prefix:** none required.
- **Max sequence length:** 384 tokens.
- **Truncation policy:** truncate to 384 tokens, tail discarded.
- **Pooling:** mean pooling (`pooling_mode_mean_tokens: true`), same
  rule as MiniLM.
- **Embedding normalization:** L2-normalize.
- **Similarity metric:** cosine similarity.

### BGE (`bge-small-en-v1.5`)

- **Document text embedded:** same as MiniLM — no document-side
  prefix (confirmed from the model's own README usage example, which
  applies the instruction string only to queries).
- **Query representation:** the fixed instruction string
  `"Represent this sentence for searching relevant passages: "`
  prepended verbatim to the raw query string (confirmed exact string
  from the model's published README table).
- **Max sequence length:** 512 tokens.
- **Truncation policy:** truncate to 512 tokens, tail discarded (the
  query-prefix's own tokens count against this limit).
- **Pooling:** **`[CLS]`-token only** — `1_Pooling/config.json`
  reports `pooling_mode_cls_token: true`, `pooling_mode_mean_tokens:
  false`. **Correction against the shortlist audit:** that document
  did not specify BGE's pooling explicitly; it is CLS-token pooling,
  not mean pooling like MiniLM/mpnet/Nomic. Using mean pooling for
  this model would not reproduce its trained behavior.
- **Embedding normalization:** L2-normalize.
- **Similarity metric:** cosine similarity.

### Nomic (`nomic-embed-text-v1.5`)

- **Document text embedded:** B1's `text_searchable`, with the fixed
  prefix `"search_document: "` prepended verbatim (confirmed exact
  string from the model's published README usage examples).
- **Query representation:** raw query string with the fixed prefix
  `"search_query: "` prepended verbatim (confirmed exact string, same
  source).
- **Max sequence length:** 8192 tokens (`sentence_bert_config.json`).
- **Truncation policy:** truncate to 8192 tokens — in practice,
  per Section 3's corpus check, no DEV note or query is expected to
  approach this limit, so truncation is not expected to ever trigger
  for this model on this corpus.
- **Output dimensionality:** this benchmark uses the model's **default
  768-dimension output**, not a truncated/Matryoshka-reduced
  dimension. No dimensionality reduction is applied in Stage 1.
- **Pooling:** mean pooling (`1_Pooling/config.json`:
  `pooling_mode_mean_tokens: true`), same rule as MiniLM/mpnet.
- **Embedding normalization:** L2-normalize.
- **Similarity metric:** cosine similarity.

### Common rule across all 4 models

- **Chunking policy:** no chunking in Stage 1 (see Section 3 for why).
  If truncation actually triggers for any DEV note under a given
  model's max sequence length, that note's embedding is computed from
  the truncated text and this is recorded per-note in that model's
  report — not silently absorbed.
- **Multi-chunk aggregation:** not applicable — no chunking is used.

## 3. Chunking-necessity check (performed before benchmarking, per plan Task 0)

Checked token-length distribution of `eval/rag/corpus/notes/*.md`'s
`text_searchable` (53 notes total) against each model's tokenizer
before committing to "no chunking": MiniLM's 256-token limit is the
tightest constraint of the four. If this check (performed in Task 1/2
using each model's actual tokenizer) finds any DEV-corpus note whose
`text_searchable` exceeds a given model's max sequence length, that is
recorded as a truncation event in that model's report (per Section 2's
truncation-policy rule), not treated as a chunking requirement — this
round deliberately keeps the representation simple (truncate, don't
chunk) and reports truncation frequency as a transparency measure
rather than engineering a chunking policy for Stage 1.

## 4. Corrections against the Stage 1 model-shortlist audit

Two facts turned out to need correction once checked directly against
each model's live repository metadata, rather than relying on the
shortlist audit's documentation-level description:

1. **BGE pooling is `[CLS]`-token, not mean pooling** — the shortlist
   audit's BGE entry did not specify a pooling mechanism explicitly;
   verified here directly from `1_Pooling/config.json`. Section 2
   above is the authoritative, frozen statement.
2. **`trust_remote_code` is a loading-path property, not a model-weight
   property, and does not apply to the ONNX export used in this
   benchmark.** The shortlist audit correctly identified that Nomic's
   PyTorch/`transformers` loading path requires `trust_remote_code=True`
   (confirmed directly: Nomic's `config.json` has `auto_map` entries
   pointing to `nomic-ai/nomic-bert-2048`'s custom
   `modeling_hf_nomic_bert.py`, which is exactly what triggers that
   flag in `transformers`). **This Stage 1 benchmark does not use that
   loading path at all** — Task 1 selects ONNX Runtime (via the Rust
   `ort` crate) as the embedding runtime, consuming each model's
   already-published static ONNX graph (`onnx/model.onnx`, confirmed
   present in all 4 repositories' file listings). An ONNX graph is a
   fixed computational graph exported once by the model's publisher;
   loading and running it through `ort` executes no Python modeling
   code and does not invoke `trust_remote_code` in any form — the
   concept does not apply to this loading path. **This means Nomic's
   custom-code supply-chain cost identified in the shortlist audit is
   resolved as a side effect of the ONNX runtime choice**, not merely
   mitigated or worth accepting — there is no remote code execution to
   audit or pin for any of the 4 models under this runtime. This is a
   narrower, more favorable finding than the shortlist audit's
   documentation-based prediction, and is recorded here as the
   authoritative, re-verified state per Task 1's own requirement to
   re-confirm this directly rather than carry the audit's estimate
   forward unchecked. The model-weight supply chain (pinning the ONNX
   file's own hash once fetched) still applies identically to all 4
   models, Nomic included — this correction is about code execution,
   not about skipping hash verification.

Both corrections are carried into Task 4's selection-rule application:
the model-selection tie-break's fixed precedence order (frozen in
`B3-TUNE-v2-PRIORITY4-MODEL-SHORTLIST-AUDIT.md`, ascending deployment
cost/complexity, Nomic last) is **left unchanged** — the order was
frozen before this correction was discovered and this spec does not
reopen it after the fact (consistent with the plan's own
non-reorderable tie-break requirement). The correction changes *why*
Nomic's position in that list might have been assigned, not the
order itself, since reopening it now, after re-checking a fact (even a
favorable one), would be exactly the kind of post-hoc adjustment the
frozen-before-benchmarking discipline exists to prevent.

## 5. Retrieval/fusion hyperparameters (frozen)

- **Lexical candidate depth:** top 50 B1 lexical candidates (B1's
  `search_impl` candidate generation, unmodified, truncated to 50 by
  score before entering fusion).
- **Dense candidate depth:** top 50 nearest-neighbor dense candidates
  by cosine similarity.
- **Final top-k:** 10 (matches B1's existing `@10` metric convention).
- **RRF constant `k`:** 60 (literature default, per the design doc).
- **RRF formula:** `score(d) = Σ_i 1/(k + rank_i(d))` over each list
  `i` (lexical, dense) in which document `d` appears; a document
  appearing in only one list is scored using only that list's term
  (the other list's term is simply absent from the sum, not treated as
  a zero-rank contribution).
- **Deduplication semantics:** a note appearing in both the lexical
  and dense candidate lists contributes exactly one fused RRF score
  (the sum above, computed once per distinct note id) — never counted
  twice or appended twice to the final result list.
- **Tie-break fallback:** if two notes' fused RRF scores are exactly
  equal, fall back to B1's existing note-id lexicographic ascending
  order (same convention as `search_impl`'s own tie-break).

## 6. Model-selection tie-break (restated from the shortlist audit, unchanged)

If the Task 4 primary selection rule (DEV quality-and-cost,
mechanically applied, not invented after seeing results) produces an
exact tie among two or more of the 4 models:

1. Lower DEV warm-query P95 latency wins.
2. If still tied: lower model-weight on-disk footprint wins.
3. If still tied: fixed precedence `all-MiniLM-L6-v2` >
   `bge-small-en-v1.5` > `all-mpnet-base-v2` > `nomic-embed-text-v1.5`.

This tie-break never consults calibration and is not reordered after
seeing DEV results, per Section 4's note above.

## 7. Primary model-selection rule (frozen)

**Rule:** among the 4 candidates, select the one with the highest DEV
Recall@10 on the lexical+dense RRF hybrid configuration (Task 5),
**subject to** a hard cost gate: the candidate's DEV warm-query P95
latency must not exceed **3x** B1's own DEV warm-query P95 latency
(measured in the same Task 3 cost-control run). A candidate failing
this cost gate is excluded from the primary-rule comparison entirely,
regardless of its quality numbers; if this leaves exactly one
candidate, it is selected without needing to compare Recall@10 at all.
If it leaves zero candidates, this is reported as a Stage 1 finding
(no candidate both passes the cost gate and was benchmarked) rather
than forcing a selection — Task 4 must not silently relax this gate
after seeing results. This rule, including the `3x` constant, is fixed
now, before Task 1 benchmarking begins.
