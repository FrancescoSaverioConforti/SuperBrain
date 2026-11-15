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
- **`trust_remote_code` / custom-code audit:** at this candidate
  revision, loading via the standard Sentence-Transformers/
  `transformers` API does **not** require `trust_remote_code=True`.
  The model's architecture (a standard distilled BERT encoder) is
  natively supported by stock `transformers` modeling classes — no
  model-specific Python modeling code outside the standard library API
  is executed at load or inference time. No additional local-caching
  requirement beyond the ordinary model-weight/tokenizer files. No
  supply-chain exposure beyond the weights/config themselves (no
  third-party code execution surface). No redistribution/bundling
  implication beyond the already-noted Apache-2.0 weights license.
  **This is not a reason to exclude this model.**

**Note on this audit's basis:** this is this document's expected/
intended state for the named revision, based on the model's published
architecture family (standard BERT-class encoder, natively supported
by stock `transformers`) rather than a re-derivation from memory of
how any specific past model release behaved. It has not been verified
against an actual local load, per this audit's no-download
constraint — Task 1 of the Stage 1 plan must re-confirm this directly
against the exact pinned revision before any benchmark run, since a
later revision could in principle change this.

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
- **`trust_remote_code` / custom-code audit:** at this candidate
  revision, same as MiniLM above — standard MPNet architecture,
  natively supported by stock `transformers` modeling classes, no
  `trust_remote_code=True` required, no model-specific remote code
  executed, no additional caching requirement, no supply-chain
  exposure beyond the weights/config themselves, no redistribution
  implication beyond the Apache-2.0 weights license. **Not a reason to
  exclude this model.** Same re-confirmation-at-Task-1 caveat as
  MiniLM applies.

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
- **`trust_remote_code` / custom-code audit:** at this candidate
  revision, the BGE-small architecture is a standard BERT-class
  encoder, natively supported by stock `transformers` modeling
  classes — `trust_remote_code=True` is **not** expected to be
  required to load this specific model, and no model-specific remote
  modeling code is expected to execute at load or inference time.
  Unlike the model's required query-prefix string (an *inference-time
  usage convention*, carried in the model card's documented usage
  example, not executed code), this is a load-time code-trust
  property and the two should not be conflated. No additional
  caching requirement beyond ordinary weight/tokenizer files. No
  supply-chain exposure beyond the weights/config. No redistribution
  implication beyond the already-noted MIT weights license. **Not a
  reason to exclude this model.** Same re-confirmation-at-Task-1
  caveat applies — this audit's basis is the model's published
  architecture family, not a verified local load.

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
- **`trust_remote_code` / custom-code audit — treated explicitly, per
  instruction:** this candidate's published HuggingFace model card, at
  the `nomic-embed-text-v1.5` revision, documents loading via
  `AutoModel.from_pretrained(..., trust_remote_code=True)`. This means:
  - **`trust_remote_code=True` IS expected to be required** to load
    this model through the standard `transformers` API at this
    revision — this is not a generic `transformers`-class load like
    the other three candidates.
  - **Model-specific Python modeling code outside standard
    `transformers`/runtime APIs IS expected to execute** at load
    time — the custom code implements this model's long-context
    (rotary-embedding-class) architecture, which is not a stock
    `transformers` model class.
  - **This custom code must itself be cached/vendored locally for
    fully-offline operation** — it is fetched from the model's
    HuggingFace repository alongside the weights on first load (same
    mechanism as the weights themselves), and a genuinely offline
    deployment must treat this code file as part of the "model
    artifact" to be cached/pinned, not assume only the `.safetensors`
    weights need to be captured for offline use.
  - **Supply-chain implications:** this is a materially different
    trust posture than the other three candidates. Loading this model
    means executing third-party Python code that is not reviewed as
    part of the `transformers` library's own release/review process —
    it ships and is versioned independently, alongside the weights.
    For a local-first/offline tool, this means the custom modeling
    code's exact revision/hash must be pinned and verified (consistent
    with this project's own stated principle of "allowlist, versioni e
    hash bloccati" for the acquisition network, `docs/Spec_mvp.md:27`)
    at the point it is first fetched, not trusted implicitly on every
    load.
  - **Redistribution/bundling implications:** bundling this model for
    offline use means bundling both the weights AND this custom code
    file together, under whatever license covers the code file
    specifically (expected to also be Apache-2.0 per the repository,
    but this is the code's own license grant, distinct from the
    weights' license grant, and should be confirmed specifically
    before bundling, not assumed identical by default).
  - **Does this create an additional reason to exclude the model?**
    **Yes, as a cost/risk factor, not an automatic disqualification.**
    This is a real, additional trust-surface and offline-packaging
    cost that the other three candidates do not carry. It does not by
    itself rule the model out — its long-context advantage (Section
    above) may still be worth this cost — but it must be weighted
    explicitly in model selection, not treated as a wash against the
    other three candidates' simpler, stock-`transformers` loading
    path. This is reflected as its own row in the comparison matrix
    below.
  - **Basis of this entry:** drawn from the model's publicly published
    HuggingFace model card/repository configuration for this revision,
    not from a local load (no download has occurred, per this audit's
    constraint) and not assumed from memory of unrelated models. Task
    1 of the Stage 1 plan must re-confirm this directly against the
    exact pinned revision before any benchmark run, since the
    publisher could in principle change this in a later revision.

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
| `trust_remote_code` required | No | No | No | **Yes** |
| Custom remote modeling code executed | No | No | No | **Yes** (long-context/rotary architecture) |
| Additional offline-packaging cost | None | None | None | **Yes** — custom code file must be cached/pinned alongside weights |

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

`nomic-embed-text-v1.5`'s `trust_remote_code` requirement (above) is
kept as a real, explicitly weighted cost factor in model selection
(see the Stage 1 plan's frozen selection rule and tie-break), not
smoothed over — it remains on the shortlist because its long-context
advantage is a genuine, distinct hypothesis worth testing on DEV, but
it does not get a free pass on this cost just because it is one of
only 4 candidates.

## Frozen model-selection tie-break (deterministic, set before any DEV benchmark)

This tie-break applies only if the Stage 1 plan's primary selection
metric/rule (DEV quality-and-cost rule, frozen in that plan's Task 0)
produces an exact tie between two or more candidates. **Calibration is
never used at any step of this tie-break.** In order:

1. **Lower DEV warm-query P95 latency** wins.
2. **If still tied:** lower model-weight on-disk footprint wins.
3. **If still tied:** the following fixed precedence order, frozen
   now, before any benchmark, decides — earlier in this list wins:
   `all-MiniLM-L6-v2` > `bge-small-en-v1.5` > `all-mpnet-base-v2` >
   `nomic-embed-text-v1.5`.

This final fixed order is deliberately **not** arbitrary: it ranks by
ascending deployment cost/complexity exactly as audited above —
smallest/no-prefix first (MiniLM), then small-with-prefix-but-no-
custom-code (BGE), then larger-with-no-custom-code (mpnet), then the
one candidate carrying the `trust_remote_code` custom-code cost last
(Nomic) — so that if step 1 and step 2 both end in an exact tie, the
simplest, lowest-risk deployment wins by default rather than an
unprincipled coin flip. This order is frozen as of this document and
must not be reordered after seeing any DEV result.
