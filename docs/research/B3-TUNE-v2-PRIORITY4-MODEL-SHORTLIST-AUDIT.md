# B3-TUNE-v2 Priority 4 — Embedding Model Shortlist Audit

**Phase:** B3-TUNE-v2, Priority 4 (hybrid lexical + dense retrieval).
**Date:** 2026-11-14.
**Status: AUDIT ONLY. No model has been downloaded. No benchmark has
run. This document selects a shortlist for a future, separately
authorized DEV benchmarking step (per
`B3-TUNE-v2-PRIORITY4-HYBRID-DESIGN.md` Section 4a) — it does not run
that step.**

This document is written against each model's publicly documented
specifications (model card / paper / repository metadata), not against
a local installation — no candidate has been fetched. Where a figure
is approximate or depends on a specific variant, that is stated
explicitly rather than presented as exact.

---

## Shortlist (4 candidates)

### 1. `sentence-transformers/all-MiniLM-L6-v2`

- **Exact model/version:** `all-MiniLM-L6-v2`, Sentence-Transformers
  library, originally distilled from Microsoft's MiniLM.
- **Architecture/family:** 6-layer distilled BERT-style transformer
  encoder, mean-pooled sentence embedding.
- **Dimensions:** 384.
- **Max sequence length:** 256 tokens (trained/truncated at this
  length; this is the practical limit for this model regardless of
  the underlying tokenizer's own maximum).
- **Model size:** ~90 MB (fp32 weights), roughly 22M parameters — the
  smallest candidate on this shortlist by a wide margin.
- **Expected CPU suitability:** very strong. This is widely cited as
  one of the fastest sentence-embedding models available specifically
  because of its small size; was designed as a "good enough quality,
  very fast" default. The most CPU-friendly candidate here, directly
  relevant to a local-first/offline tool with no guaranteed GPU.
- **License:** Apache-2.0 (Sentence-Transformers library and this
  specific checkpoint).
- **Offline distribution implications:** Apache-2.0 is compatible with
  bundling/redistribution alongside this project's own Apache-2.0
  license (`crates/index/Cargo.toml` confirms the project itself is
  Apache-2.0) — no field-of-use or attribution-beyond-license-notice
  restriction.
- **Model-specific encoding requirements:** mean pooling over token
  embeddings (not `[CLS]`-only); no special query/passage prefix
  required — this model was trained symmetrically for
  sentence-to-sentence similarity, not asymmetric query/document
  retrieval, which is itself a quality caveat (see rationale below).
- **Rationale for inclusion:** the standard "fast, small, good-enough"
  baseline in the sentence-embedding space; its very low resource cost
  makes it a useful lower bound on the cost side of the Section 6a/7
  trade-off — if this model cannot show enough quality gain, a larger
  model's extra cost will need to be justified by a correspondingly
  larger quality gain, not just "it's a better model in general."

### 2. `sentence-transformers/all-mpnet-base-v2`

- **Exact model/version:** `all-mpnet-base-v2`, Sentence-Transformers
  library, based on Microsoft's MPNet.
- **Architecture/family:** 12-layer MPNet transformer encoder,
  mean-pooled sentence embedding.
- **Dimensions:** 768.
- **Max sequence length:** 384 tokens.
- **Model size:** ~420 MB (fp32), roughly 110M parameters.
- **Expected CPU suitability:** moderate. Noticeably slower than
  MiniLM per-inference (roughly 4-5x the parameter count), but still
  within the range widely reported as usable for CPU-only inference at
  small-to-moderate corpus sizes (this project's 53-note corpus is
  firmly in that range); would need to be measured directly at actual
  query latency, not assumed from parameter count alone.
- **License:** Apache-2.0.
- **Offline distribution implications:** same as MiniLM above —
  Apache-2.0, compatible with this project's own licensing, no
  additional restriction.
- **Model-specific encoding requirements:** mean pooling; no special
  query/passage prefix required — also trained symmetrically, same
  caveat as MiniLM.
- **Rationale for inclusion:** widely reported as one of the
  highest-quality general-purpose sentence-embedding models in the
  Sentence-Transformers family at a size that still plausibly fits a
  local-first/offline tool's CPU/RAM budget. Serves as the "spend more
  to get more quality" candidate against MiniLM's "spend least"
  candidate — the DEV benchmark should show whether the quality gain
  (if any, on this specific 53-note corpus and its query set) is large
  enough to justify roughly 4-5x the inference cost.

### 3. `BAAI/bge-small-en-v1.5`

- **Exact model/version:** `bge-small-en-v1.5`, BAAI General
  Embedding family.
- **Architecture/family:** BERT-style transformer encoder, part of a
  family explicitly designed and trained for **asymmetric**
  retrieval (distinct query vs. passage encoding behavior via
  prefixes), not just general sentence similarity.
- **Dimensions:** 384.
- **Max sequence length:** 512 tokens — notably longer than either
  Sentence-Transformers candidate above, relevant if notes or chunks
  commonly exceed ~256-384 tokens.
- **Model size:** ~130 MB (fp32), roughly 33M parameters — close to
  MiniLM's resource cost, not mpnet's.
- **Expected CPU suitability:** strong — comparable to MiniLM given
  similar parameter count, though not as extensively benchmarked for
  raw CPU throughput in public comparisons as the Sentence-Transformers
  models above; would need direct measurement.
- **License:** MIT.
- **Offline distribution implications:** MIT is permissive and
  compatible with bundling alongside this project's Apache-2.0
  license; no field-of-use restriction identified.
- **Model-specific encoding requirements:** **requires a query-side
  instruction prefix** for retrieval use (the model card specifies a
  fixed instruction string to prepend to queries, not to documents) —
  this is exactly the "model-specific query/document prefix" case
  flagged as a required frozen decision in
  `B3-TUNE-v2-PRIORITY4-HYBRID-DESIGN.md` Section 4b. Omitting this
  prefix during benchmarking would silently degrade this model's
  measured quality relative to its intended use, which is a
  significant risk specifically called out in that section.
- **Rationale for inclusion:** this family is widely used specifically
  for retrieval (as opposed to general sentence similarity), which is
  closer to this project's actual use case (query → matching note) than
  the symmetric training objective of the Sentence-Transformers
  candidates above. Including it tests whether a retrieval-specific
  training objective matters more than raw model size on this
  corpus — at a resource cost close to MiniLM's, not mpnet's.

### 4. `nomic-ai/nomic-embed-text-v1.5`

- **Exact model/version:** `nomic-embed-text-v1.5`.
- **Architecture/family:** BERT-style transformer encoder (long-context
  variant), also trained for asymmetric retrieval with explicit
  task-prefix conventions (e.g. distinct `search_query:` /
  `search_document:` prefixes per the model card).
- **Dimensions:** 768 by default, but explicitly supports **variable
  / truncatable output dimensionality** (Matryoshka-style training)
  down to smaller sizes if a lower-dimension, lower-storage variant is
  wanted without re-training — a distinguishing feature relative to
  the other three fixed-dimension candidates.
- **Max sequence length:** 8192 tokens — far longer than any other
  candidate here, relevant if the project's notes (which include real
  source-code excerpts per the corpus provenance,
  `eval/rag/corpus-provenance.json`) run long enough that the
  256-512-token limits above would force chunking (Section 4b) while
  this model would not.
- **Model size:** ~550 MB (fp32), roughly 137M parameters — the
  largest candidate on this shortlist.
- **Expected CPU suitability:** weakest of the four on raw size, and
  its long-context architecture (commonly implemented with
  rotary-position-embedding-style long-context mechanisms) can carry
  additional per-token compute cost beyond what parameter count alone
  suggests — needs direct CPU latency measurement at the sequence
  lengths this project's actual notes/chunks would use, not an
  assumption from short-sequence benchmarks alone.
- **License:** Apache-2.0.
- **Offline distribution implications:** Apache-2.0, compatible with
  this project's own licensing, no additional restriction. Note:
  some Nomic-published models/tooling are associated with a hosted
  Atlas platform for visualization/analytics — that platform is
  unrelated to and not required for local, offline embedding
  inference with the model weights themselves; must be confirmed not
  to be an implicit dependency before adoption, but is not expected to
  be one for the embedding model itself.
- **Model-specific encoding requirements:** requires task-specific
  prefixes for query vs. document encoding (model-card-documented),
  same category of requirement as BGE above, and also requires an
  explicit choice of output dimensionality if the truncatable-embedding
  feature is used (another frozen-before-benchmarking decision per
  Section 4b).
- **Rationale for inclusion:** the only candidate whose context length
  comfortably covers this project's actual note content (source-code
  excerpts) without chunking, and the only one offering a
  dimensionality/storage trade-off without switching models entirely.
  Included specifically to test whether avoiding the chunking-policy
  complexity (Section 4b) for this corpus's longer notes is worth its
  larger resource cost — a question none of the other three candidates
  can answer, since all three would need a chunking policy decided and
  applied consistently regardless of which of them is chosen.

---

## Summary comparison

| | MiniLM-L6-v2 | mpnet-base-v2 | bge-small-en-v1.5 | nomic-embed-text-v1.5 |
|---|---|---|---|---|
| Dim | 384 | 768 | 384 | 768 (truncatable) |
| Max seq len | 256 | 384 | 512 | 8192 |
| Size (fp32) | ~90 MB | ~420 MB | ~130 MB | ~550 MB |
| License | Apache-2.0 | Apache-2.0 | MIT | Apache-2.0 |
| Query prefix required | No | No | Yes | Yes |
| Training objective | Symmetric similarity | Symmetric similarity | Asymmetric retrieval | Asymmetric retrieval |
| CPU cost (relative) | Lowest | Moderate-high | Low-moderate | Highest |

## What this audit deliberately does not do

- **No model has been downloaded.**
- **No benchmark has run, on DEV or any other split.**
- **No model is selected.** This document narrows the universe of
  options to 4 realistic, well-documented candidates; final selection
  happens only via the frozen DEV-benchmarking workflow in
  `B3-TUNE-v2-PRIORITY4-HYBRID-DESIGN.md` Section 4a, which requires
  an explicitly frozen selection rule before benchmarking and is itself
  not yet started.
- **No numeric figure above should be treated as independently
  re-verified against the actual package/weights** — these are drawn
  from each model's publicly documented card/metadata, not from a
  local install, consistent with this audit's no-download constraint.

## Proposed final shortlist for DEV benchmarking

All 4 candidates above are proposed, unchanged, as the shortlist to
benchmark on DEV once that step is separately authorized:
`all-MiniLM-L6-v2`, `all-mpnet-base-v2`, `bge-small-en-v1.5`,
`nomic-embed-text-v1.5`. They deliberately span: smallest/fastest vs.
largest/most-capable, symmetric vs. asymmetric training objective, and
short vs. long context support — covering the axes most likely to
matter for this project's specific corpus (source-code-heavy notes,
53 documents today, strict offline/local-first requirement) without
expanding the shortlist size, per the 2-4-candidate instruction.
