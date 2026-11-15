# B3-TUNE-v2 Priority 4 Stage 1 Plan — DEV Benchmarking + Promotion-Criteria Proposal ONLY

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Benchmark the 4-candidate embedding model shortlist
(`B3-TUNE-v2-PRIORITY4-MODEL-SHORTLIST-AUDIT.md`) on **DEV only**,
select exactly one model via a pre-frozen deterministic rule, freeze
retrieval/fusion hyperparameters and embedding representation
semantics, and produce a concrete, numeric promotion-criteria proposal
for user approval — per the corrected workflow in
`B3-TUNE-v2-PRIORITY4-HYBRID-DESIGN.md` Sections 4a/3a/7.

**This plan explicitly stops before:** model download execution
without prior authorization of this plan itself, embeddings generation
beyond DEV, any calibration run, any held-out read, implementation of
weighted score fusion (deferred), and any merge/tag/push. Each task
below is scoped to DEV and design artifacts only.

**Architecture:** New, isolated code path alongside the existing `index`
crate — a dense/hybrid prototype must not modify B1's production
`Index::search` path at all (same additive-opt-in discipline as every
Priority 2/3 lever). Embedding generation and vector search run as a
separate, clearly-labeled experimental harness (mirroring the existing
`crates/api/examples/b3_tune_v2_*` ablation-binary pattern), consuming
the same `eval/rag/dev` fixtures Priority 1-3 already use, producing a
report in `eval/results/B3-TUNE-v2/priority4-*` following the existing
`AblationReport`/`MetricBreakdown`/bootstrap-CI conventions.

**Tech Stack:** Rust workspace, plus whichever local embedding-model
runtime binding the implementer selects to run the 4 shortlisted models
locally offline (not pinned in this plan — selecting that binding is
part of Task 1, since it must itself satisfy the offline/local-only
constraint before any model is loaded through it).

**Spec:**
`docs/research/B3-TUNE-v2-PRIORITY4-HYBRID-DESIGN.md` (frozen fusion
strategy, hyperparameter/representation freezing requirements, privacy
parity requirement, cost-benchmarking requirement, promotion-criteria
workflow) and `docs/research/B3-TUNE-v2-PRIORITY4-MODEL-SHORTLIST-AUDIT.md`
(the 4 candidates and their documented specs).

## Global Constraints

- **DEV only.** No task in this plan touches `eval/rag/calibration/`
  or `eval/rag/v2-heldout/` labels for any measurement, diagnosis, or
  selection purpose. Calibration and held-out remain unread through
  every task below.
- **No calibration run of any kind** — not even a "quick check" —
  happens until a separate, explicitly authorized plan picks this work
  back up after Task 6's STOP gate.
- **RRF only.** Weighted score fusion is not implemented in this plan
  (deferred per the design doc, Section 3).
- **One model, selected on DEV, before any further tuning.** Tasks 1-3
  benchmark all 4 shortlisted candidates; Task 4 applies the frozen
  selection rule and locks in exactly one. No task after Task 4 may
  re-open model choice.
- **Embedding representation semantics (design doc Section 4b) must be
  written down per-candidate before Task 2's benchmarking run** — not
  decided ad hoc while coding the harness.
- **Privacy parity (design doc Section 4c) is enforced from the first
  line of embedding-generation code** — the harness must build its
  embedding input from each note's existing `text_searchable`
  (already redaction-aware per B1), never from a raw note-body read
  that bypasses `Sensitive`-note redaction. A `Sensitive`-note test
  (design doc Section 4c) is required before Task 2 is considered
  complete, not deferred to a later cleanup pass.
- **No production `index` crate code changes** — this is a standalone
  experimental harness; B1's `Index::search` and its existing
  Priority 2/3 ablation hooks are untouched by this plan.

## Task 0 — Freeze representation semantics and retrieval/fusion hyperparameters

- [ ] For each of the 4 shortlisted models, write down (per design doc
      Section 4b): exact document text embedded (expected: B1's
      existing `text_searchable`), exact query representation,
      model-specific query/document prefix (required for
      `bge-small-en-v1.5` and `nomic-embed-text-v1.5` per the shortlist
      audit; confirm none needed for the two Sentence-Transformers
      models), max sequence length, truncation policy, chunking policy
      and overlap (if any note/chunk exceeds the model's max sequence
      length — check this against the actual corpus before assuming
      chunking is even needed), pooling, multi-chunk aggregation (if
      applicable), embedding normalization, similarity metric.
- [ ] Freeze retrieval/fusion hyperparameters (design doc Section 3a):
      lexical candidate depth, dense candidate depth, final top-k
      (default: match B1's existing `@10` convention), RRF constant
      `k` (starting value: `60`, the literature default), deduplication
      semantics for a note appearing in both candidate lists, tie-break
      fallback (B1's existing note-id lexicographic order).
- [ ] Freeze the **model-selection tie-break** (distinct from the
      retrieval-result tie-break above — this one decides between
      candidate *models*, not between search results): if the primary
      DEV quality-and-cost selection rule (frozen in this same task,
      below) ends in an exact tie between two or more of the 4
      shortlisted models, resolve it, in order: (1) lower DEV
      warm-query P95 latency; (2) if still tied, lower model-weight
      on-disk footprint; (3) if still tied, the fixed precedence
      `all-MiniLM-L6-v2` > `bge-small-en-v1.5` > `all-mpnet-base-v2` >
      `nomic-embed-text-v1.5` (ascending deployment cost/complexity,
      per `B3-TUNE-v2-PRIORITY4-MODEL-SHORTLIST-AUDIT.md`'s frozen
      tie-break section — lowest-risk/no-custom-code candidates first,
      the `trust_remote_code`-requiring candidate last). **This
      tie-break never uses calibration and is frozen now, before any
      DEV benchmark runs — it must not be reordered after seeing
      results.**
- [ ] Commit this freeze — including the primary selection rule, the
      hyperparameters above, and the model-selection tie-break — as a
      short written spec (new file under `docs/superpowers/specs/`)
      before Task 1 begins. Nothing in this spec may change after Task
      1 benchmarking starts.

## Task 1 — Select and validate an offline-only local embedding runtime

- [ ] Select a Rust-callable (direct binding, FFI, or subprocess to a
      strictly-local process) runtime capable of running all 4
      shortlisted models fully offline, with no runtime network call
      at load or inference time.
- [ ] Verify the selected runtime's own license is compatible with
      this project's Apache-2.0 licensing and local-first distribution
      model — do not assume compatibility from popularity.
- [ ] **Re-confirm the `trust_remote_code`/custom-code status of each
      of the 4 shortlisted models directly against the exact pinned
      revision**, before benchmarking any of them — the shortlist
      audit's entries are this document's expected/intended state, not
      a verified local load, and must be re-checked here. For
      `nomic-embed-text-v1.5` specifically: confirm whether the
      selected runtime requires executing the model's custom remote
      modeling code, and if so, pin and cache that code file's exact
      revision/hash locally (not just the weights) before any offline
      benchmark run — consistent with the project's own
      allowlist/version/hash-pinning principle for its acquisition
      network (`docs/Spec_mvp.md:27`).
- [ ] Document the exact runtime + version chosen and why, alongside
      the Task 0 freeze spec.

## Task 2 — Build the DEV-only embedding + dense-retrieval harness

- [ ] Build a standalone example binary (e.g.
      `crates/api/examples/b3_tune_v2_priority4_dense_dev.rs`) that:
      embeds each DEV-corpus note's `text_searchable` (never raw body —
      Section 4c) under the frozen Task 0 spec, for one model at a
      time; embeds each DEV query under the same frozen
      representation; performs brute-force cosine (or the
      model-appropriate frozen metric) nearest-neighbor search over
      the note embeddings; reports candidate-recall behavior (does the
      correct note appear in the dense top-N at all?) separately from
      any fused ranking metric, per the design doc's Section 6
      pre-condition check.
- [ ] Add the required `Sensitive`-note privacy test (design doc
      Section 4c): construct a `Sensitive` note, assert its embedding
      input equals its redacted `text_searchable` and never its raw
      body, assert a query targeting only its redacted-away content
      does not surface it with materially above-chance similarity.
- [ ] Run this harness once per shortlisted model, on DEV only,
      producing one report per model under
      `eval/results/B3-TUNE-v2/priority4-dev-<model>.json`.

## Task 3 — Measure cost metrics for each model, DEV scale, plus a B1 cost-control run

- [ ] For each of the 4 models: measure startup/model-load time, warm
      per-query embedding inference time, vector-search time, peak
      RAM during indexing and during query, model-weight on-disk size,
      vector-index on-disk size, full-DEV-corpus build/index time.
- [ ] Run a **B1 cost-control measurement** (design doc Section 6a) in
      the same harness/machine/configuration: B1's own
      startup/warm-query-latency/peak-RAM/build-time, measured for the
      first time (B3's existing closure reports contain no cost
      metrics) — this is not a new quality evaluation and must not
      touch Recall@10/nDCG@10/thresholds/verdict.
- [ ] Record all cost numbers in a single comparison table across the
      4 models + the B1 cost-control run.

## Task 4 — Apply the frozen selection rule; lock in one model

- [ ] Using only DEV quality (Task 2) and DEV cost (Task 3) numbers,
      apply the deterministic selection rule frozen in Task 0's spec
      (not invented after seeing these results) to pick exactly one
      model.
- [ ] **If the primary rule produces an exact tie**, apply Task 0's
      frozen model-selection tie-break mechanically, in order: DEV
      warm-query P95 latency, then model-weight footprint, then the
      fixed precedence `all-MiniLM-L6-v2` > `bge-small-en-v1.5` >
      `all-mpnet-base-v2` > `nomic-embed-text-v1.5`. Do not resolve a
      tie by any means other than this frozen sequence, and do not
      consult calibration at any point in this step.
- [ ] Record the selection decision and the rule's (and, if invoked,
      the tie-break's) mechanical application — not a fresh judgment
      call — in a short writeup.
- [ ] From this point forward, no other model may be substituted
      without restarting this plan's Task 0-4 sequence.

## Task 5 — Build the RRF hybrid harness with the single frozen model, on DEV

- [ ] Using the one model locked in by Task 4 and the hyperparameters
      frozen in Task 0, build the lexical+dense RRF-fused
      configuration (design doc Section 3) as a third harness
      configuration, alongside the existing B1 and dense-standalone
      ones.
- [ ] Measure Recall@10, nDCG@10, per-category/per-difficulty
      breakdowns, and the full cost-metric set (Task 3's categories)
      for the hybrid configuration, on DEV.
- [ ] Produce a DEV-only three-way comparison report: B1 vs.
      dense-standalone vs. hybrid, quality + cost, no calibration
      numbers anywhere in this report.

## Task 6 — Propose numeric promotion criteria; STOP for user approval

- [ ] Using Task 5's real DEV numbers (not invented ahead of data,
      per the design doc's corrected Section 7), draft concrete,
      numeric promotion criteria covering: Recall@10 regression
      tolerance, required nDCG@10 improvement (or defined equivalent),
      latency P50/P95 budget, peak RAM budget, model-weight footprint,
      vector-index disk footprint, indexing/build-time budget.
- [ ] Present this proposal to the user. **STOP. Do not proceed past
      this point without explicit user approval of the frozen
      criteria.** No calibration run, no held-out read, and no further
      implementation happens until that approval is given and a
      separate plan is authorized to carry it out.

## What this plan does not cover

- Calibration run with the frozen model/hyperparameters/criteria —
  authorized only after Task 6's approval, as a separate follow-on
  plan.
- Held-out (`eval/rag/v2-heldout/`) read — remains untouched.
- Weighted score fusion implementation — deferred, not part of this
  round.
- Any change to B1's production `index`/`search` code.
- Merge, tag, or push of any kind.
