# B3-TUNE-v2 Closure Summary

**Phase:** B3-TUNE-v2 (post-B3 retrieval tuning cycle), Priorities 1-4.
**Date:** 2026-11-14/15.
**Status: CLOSED.**

**Verdict: `all-MiniLM-L6-v2 dense-only` ACCEPTED as a new retrieval
configuration ("v2"), on top of the unmodified historical B3 baseline
(`B1 trigram+BM25`). Hybrid-RRF REJECTED. No other lever from
Priorities 1-3 was accepted.**

This document consolidates every Priority 1-4 result into one closure
record. It does not re-run, re-measure, or alter any prior result —
every number below is reused from its own already-committed report.

---

## 1. Baseline terminology — read this section first

Two distinct, non-interchangeable things are both called "the
baseline" at different points in this project's history. This closure
distinguishes them explicitly and this distinction must be preserved
in all future references:

- **`B1 trigram+BM25`** — the **historical accepted B3 baseline**.
  Frozen at `eval/results/B3/frozen-config.json` (commit `52d42ea`).
  This is the deterministic, lexical-only retrieval pipeline audited
  in `docs/research/B3-TUNE-v2-PRIORITY3-RANKING-AUDIT.md`. Its own
  original closure (`docs/research/B3-CLOSURE-SUMMARY.md`) is **not
  rewritten or altered by this document** — B1's own PASS/ACCEPTED
  verdict, thresholds, and historical final-test numbers stand exactly
  as originally recorded.
- **`v2 MiniLM dense-only`** — the **new accepted retrieval
  configuration** produced by this B3-TUNE-v2 cycle's Priority 4.
  Frozen separately at `eval/results/B3-TUNE-v2/frozen-config-v2.json`
  (commit `c27d043`). This is a dense, embedding-based retrieval
  pipeline, evaluated independently against B1 and against calibration/
  held-out splits never read by B1's own original closure.

**Old B3 reports are not modified to reflect v2's existence.**
`docs/research/B3-CLOSURE-SUMMARY.md` continues to document B1 alone,
as it did at B3's own closure. Whether and how v2 eventually replaces
or supplements B1 in production is an integration question, addressed
only as a design (not an implementation) in
`docs/research/B3-TUNE-v2-PRIORITY4-PRODUCTION-INTEGRATION-DESIGN.md`,
not by editing history.

## 2. Priority 1 — Abstention thresholding — REJECT

**Verdict: REJECT — single global score-derived abstention threshold
insufficient.**

Three signals compared (`top1_score`, `margin`, `ratio`) on dev (n=60)
and calibration (n=30); `ratio` had the smallest separation gap of the
three but zero of 30 swept cutoffs on calibration satisfied all four
guardrails simultaneously (abstention recall > 0, false-abstention
rate ≤ 10%, |Recall@10 delta| ≤ 0.05, |nDCG@10 delta| ≤ 0.05). The
first cutoff with any abstention recall at all already violated the
other three guardrails. User's own words: "il problema non è la
scelta del cutoff; la separabilità di un singolo segnale è
insufficiente."

Full detail: `docs/research/B3-TUNE-v2-PRIORITY1-CLOSURE.md`.

## 3. Priority 2 — Lexical tuning — no lever accepted

**Verdict: no lever promoted. Stage 2 composition skipped (requires
≥1 Stage 1 KEEP; zero exist).**

| Lever | Mechanism | Verdict |
|---|---|---|
| A — BM25 `k1`/`b` tuning | sweep against calibration | **REJECT** |
| B — Punctuation normalization | text normalization before tokenization | **REJECT** (after real-pipeline verification: calibration nDCG@10 regression −0.0213 — the real pipeline did not reproduce an earlier standalone scorer's promising signal) |
| C — Alias table | short-form identifier expansion | **NOT EVALUATED / NO-OP** (zero activations on dev and calibration — the alias table's short forms never appeared as literal tokens in either split) |

Full detail: `docs/research/B3-CLOSURE-SUMMARY.md` is B3's own closure;
Priority 2's own closure is `eval/results/B3-TUNE-v2/priority2-stage1-summary.md`
(commit `d7f182f`).

## 4. Priority 3 — Deterministic reranking — no lever accepted

**Verdict: no lever promoted. No composition (`B1+P1+P2`) built or
measured, since both Stage 1 levers closed as no-op.**

| Lever | Mechanism | Verdict |
|---|---|---|
| P1 — Exact structural-identifier boost | fixed additive bonus on whole-token `source:` identifier match | **NOT EVALUATED / NO-OP** (`identifier_boost_fired = 0/30` on calibration, `0/60` on dev — no query in either split contained a literal whole-token identifier match) |
| P2 — Status tie-break | orders bitwise-`f64`-tied candidates by status tier | **NOT EVALUATED / NO-OP** (`exact_ties_observed = 0` on calibration; `4` ties existed on dev but `ties_broken_by_status = 0/60` — no tie crossed a status-tier boundary) |
| P3 — Recency tie-break | *(not implemented)* | **NOT EVALUATED / NO-OP, skipped without implementation** — deduced directly from P2's own measurement: P3 shares P2's identical exact-BM25-tie trigger condition, and P2 already measured zero such ties on calibration, so P3 cannot fire regardless of its secondary sort key. Re-implementing it to re-demonstrate `exact_ties_observed = 0` would add code to re-prove an already-established fact. |
| P4 — Trust-level tie-break | *(not implemented)* | **NOT EVALUATED / NO-OP, skipped without implementation** — identical reasoning to P3: shares P2's exact-tie trigger, which calibration measured at zero. |

Independent review of the P1/P2 implementation found 2 Important test-
quality issues (two vacuous tests not actually exercising the no-op
condition they claimed to) and 0 Critical findings; both fixed
(RED→GREEN, commit `290dead`). Final review verdict: **APPROVE**.

Full detail: `docs/research/B3-TUNE-v2-PRIORITY3-CLOSURE.md`,
`eval/results/B3-TUNE-v2/priority3-stage1-summary.md`.

## 5. Priority 4 — Hybrid retrieval (dense embeddings) — `MiniLM dense-only` ACCEPTED

### 5.1 Design and shortlist audit

Four candidate models were audited (design-only, no downloads) against
dimension, max sequence length, license, offline/`trust_remote_code`
status, and encoding requirements: `all-MiniLM-L6-v2`,
`all-mpnet-base-v2`, `bge-small-en-v1.5`, `nomic-embed-text-v1.5`.
RRF was selected as the sole fusion strategy for this round (weighted
score fusion deferred — avoids the BM25/vector score-scale
comparability problem RRF sidesteps by construction). A frozen,
deterministic model-selection tie-break was set before any benchmark:
(1) lower DEV warm-query P95 latency, (2) lower model-weight footprint,
(3) fixed precedence `MiniLM > BGE > mpnet > Nomic` (ascending
deployment cost/complexity, `trust_remote_code`-requiring candidate
last).

Full detail: `docs/research/B3-TUNE-v2-PRIORITY4-HYBRID-DESIGN.md`,
`docs/research/B3-TUNE-v2-PRIORITY4-MODEL-SHORTLIST-AUDIT.md`.

### 5.2 DEV results (diagnostic only, n=60, 49 answerable)

| Model | Dense Recall@10 | Dense nDCG@10 | Hybrid-RRF Recall@10 | Hybrid-RRF nDCG@10 |
|---|---|---|---|---|
| all-MiniLM-L6-v2 | 0.9796 | 0.8757 | 0.9388 | 0.8113 |
| bge-small-en-v1.5 | 1.0000 | 0.8815 | 0.8980 | 0.8030 |
| all-mpnet-base-v2 | 0.9796 | 0.8776 | 0.9184 | 0.8239 |
| nomic-embed-text-v1.5 | 1.0000 | 0.9089 | 0.9388 | 0.8402 |

**Headline DEV finding:** hybrid-RRF underperformed dense-standalone
for **all 4 models**. `candidate_recall_rate = 1.0` for all 4 (the
correct note always appears in the dense top-50) is the likely
mechanism — RRF's rank-blend with B1's comparatively weaker lexical
ranking pulls some results down rather than up on this corpus.

**Model selection:** primary rule produced an exact tie between
`all-MiniLM-L6-v2` and `nomic-embed-text-v1.5` (both hybrid-RRF
Recall@10 = 0.93878, bitwise-equal). Tie-break resolved at step 1
(lower P95 latency: MiniLM 9.277ms vs. Nomic 21.384ms) → **MiniLM
selected.** Nomic's higher dense-only DEV nDCG did not reopen this
decision, per the user's explicit instruction after review.

Full detail: `docs/research/B3-TUNE-v2-PRIORITY4-STAGE1-TASK4-SELECTION.md`,
`eval/results/B3-TUNE-v2/priority4-dev-*.json`,
`eval/results/B3-TUNE-v2/priority4-hybrid-dev-*.json`.

### 5.3 Cost results (DEV scale, same harness, MiniLM vs. B1 cost-control)

| Metric | B1-cost-control | MiniLM dense |
|---|---|---|
| Startup/model-load | 4519.4 ms | 155.6 ms |
| Build/index time | 145.0 ms | 549.6 ms (≈3.79×) |
| Warm-query P50/P95 | 116.5 / 124.5 ms | 8.7 / 9.3 ms |
| Embedding-inference P50/P95 | n/a | 8.5 / 9.1 ms |
| Vector-search P50/P95 | n/a | 0.17 / 0.19 ms |
| Peak RSS (after queries) | 35.0 MB | 150.4 MB |
| Model-weight footprint | n/a | 90.4 MB |

All 4 models' warm-query P95 (9.3–40.6 ms) cleared the 3× B1 cost gate
(373.6 ms ceiling) comfortably. Other 3 models' footprints: BGE 133.1
MB, mpnet 435.8 MB, Nomic 547.3 MB.

Full detail: `eval/results/B3-TUNE-v2/priority4-cost-*.json`.

### 5.4 Privacy checks

Required by the design's hard privacy/searchable-surface parity rule
(Section 4c of the hybrid design doc): a `Sensitive`-note test
constructed a note with a planted secret marker and verified (a)
structurally that the note's embedding input (`text_searchable`)
excludes the secret and its `source:` fragment, matching B1's own
redaction, and (b) differentially, against an `Internal`-classified
variant of the identical note, that the embedding pipeline's input
genuinely differs by classification rather than embedding the raw body
regardless of classification. **Both checks passed.** This test was
built into the Stage 1 DEV dense harness and re-used unchanged at
calibration and held-out (no privacy-relevant code path changed
between splits).

### 5.5 Calibration results (decision split, n=30, 24 answerable)

**Architecture correction, applied per user review of DEV:** since DEV
showed dense-standalone beating hybrid-RRF for all 4 models, dense-only
was promoted from diagnostic-only to a first-class promotion candidate
at calibration. Three configurations compared: B1 lexical baseline
(reused frozen numbers, not re-measured), MiniLM dense-only, MiniLM +
B1 hybrid-RRF.

| Config | Recall@10 | Δ vs B1 | nDCG@10 | Δ vs B1 |
|---|---|---|---|---|
| B1 lexical (frozen, reused) | 0.9167 | — | 0.7091 | — |
| MiniLM dense-only | 1.0000 | **+0.0833** | 0.8614 | **+0.1524** |
| MiniLM hybrid-RRF | 0.9583 | +0.0417 | 0.7945 | +0.0855 (**−0.0669 vs. dense**) |

`candidate_recall_rate = 1.0` (24/24) for both — DEV's finding
replicates. Abstention confusion matrix unchanged for both (TP=0,
FP=0, FN=6) — tripwire clean. The hybrid-underperforms-dense pattern
from DEV replicates on calibration too.

**Frozen promotion criteria** (approved by user before calibration was
read, per the mandatory freeze-then-calibrate workflow): Recall@10
degradation ≤0.05 and nDCG@10 improvement ≥+0.03 vs. B1 (both
required); warm P95 ≤3× B1; peak RAM ≤300MB; model-weight ≤150MB;
vector-index ≲2KB/note; build time ≤5× B1 (revised up from an
initially-inconsistent 3× draft during Stage 1, with that revision
documented transparently rather than silently adjusted). A dedicated
gate — hybrid nDCG@10 must not be worse than dense-standalone's own —
was added specifically because of the DEV finding.

**Mechanical gate verdict:**

| Gate | Dense-only | Hybrid-RRF |
|---|---|---|
| Recall@10 degradation ≤0.05 | PASS | PASS |
| nDCG@10 improvement ≥+0.03 vs B1 | PASS | PASS |
| nDCG@10 ≥0.00 vs dense-standalone | n/a | **FAIL** (−0.0669) |
| Warm P95 ≤3× B1 | PASS | PASS |
| Peak RAM ≤300MB | PASS | PASS |
| Model-weight ≤150MB | PASS | PASS |
| Vector-index ≲2KB/note | PASS | PASS |
| Build time ≤5× B1 | PASS (3.79×) | PASS |

**Only dense-only passes every gate → promoted directly. Hybrid-RRF
fails the dense-standalone-comparison gate despite passing every other
gate → REJECT**, per the rule that hybrid must not be promoted merely
for beating B1 if dense-only is better.

Full detail: `docs/research/B3-TUNE-v2-PRIORITY4-CALIBRATION-RESULTS.md`
(note: this document's content was produced via real, independently-
verified execution after two earlier fabricated subagent attempts for
the calibration step were discarded — see Section 7 below).

### 5.6 CRLF/LF hash-resolution note

Before the v2 held-out run, a pre-flight integrity check found the
held-out manifest's recorded SHA-256 (`be4fa30a...`) did not match a
raw hash of `eval/rag/v2-heldout/labels.jsonl` on this Windows
checkout (`d7dd80e8...`). Root-caused, not assumed: `git hash-object`
on the working-tree file equals `git rev-parse
HEAD:eval/rag/v2-heldout/labels.jsonl` exactly (both
`8594c7a97c71a3f2bea7e0de3f683543f2da8599`) — the file's tracked
content is byte-for-byte identical to the commit that created it
(`fb03c3e`); `git diff` on that path is clean. The repo has
`core.autocrlf=true`, which materializes CRLF line endings on Windows
checkout while git's internal blob stores LF — this changes a raw
file-read's SHA-256 without changing the tracked content. Normalizing
the checked-out bytes CRLF→LF reproduced the manifest's recorded hash
**exactly**. User authorized treating the LF-normalized match as
integrity PASS, without modifying either `labels.jsonl` or
`manifest.json`. **Technical note preserved for future manifests:**
git-versioned text artifacts' hash fields should explicitly state
their line-ending canonicalization (or hash the git blob / canonical-
LF representation directly) so this exact class of cross-platform
false-positive cannot recur.

Full detail: `docs/research/B3-TUNE-v2-PRIORITY4-V2-HELDOUT-RESULTS.md`.

### 5.7 v2 held-out final-test — CONSUMED

Run **exactly once**, `all-MiniLM-L6-v2 dense-only` only (no
hybrid-RRF, no other model, no new B1 measurement — B1's frozen
**calibration** numbers used as the comparison point, since B1 was
never measured against this specific held-out set), against the
frozen `eval/results/B3-TUNE-v2/frozen-config-v2.json` configuration.

| Metric | Value |
|---|---|
| n (answerable) | 60 (45) |
| Recall@10 | 1.0000, CI [1.0, 1.0] |
| nDCG@10 | 0.8909, CI [0.831, 0.947] |
| candidate_recall_rate | 1.0 (45/45) |
| Δ Recall@10 vs. B1 frozen calibration | +0.0833 |
| Δ nDCG@10 vs. B1 frozen calibration | +0.1818 |

By-category: bugs-fixes 1.0/0.954, cross-file-concept 1.0/0.862,
identifier-code 1.0/0.938, notes-decisions 1.0/0.954, patterns-
knowledge 1.0/0.947, temporal 1.0/0.634 — no low-confidence flags.
By-difficulty: easy 1.0/0.915, medium 1.0/0.870, hard 1.0/0.895.
Abstention tripwire unchanged (TP=0, FP=0, FN=15).

**Both frozen quality gates PASS. Verdict: `all-MiniLM-L6-v2
dense-only v2 = ACCEPTED`.**

**The v2 held-out set (`eval/rag/v2-heldout/labels.jsonl`) is now
CONSUMED** — it has been read, evaluated, and reported, and per the
project's own standing rule (mirroring B3's original B1 final-test
protocol) it cannot be used again as independent evidence for this or
any other configuration's tuning. Any future retrieval experiment
requires a new, never-before-read held-out set to make an independent
claim of improvement.

Full detail: `docs/research/B3-TUNE-v2-PRIORITY4-V2-HELDOUT-RESULTS.md`,
`eval/results/B3-TUNE-v2/priority4-v2-heldout-dense-all-MiniLM-L6-v2.json`.

### 5.8 Frozen configuration and evidence provenance

`eval/results/B3-TUNE-v2/frozen-config-v2.json` (commit `c27d043`) is
the authoritative record of the exact accepted configuration:

- **Model/runtime:** `all-MiniLM-L6-v2` (HF repo
  `sentence-transformers/all-MiniLM-L6-v2`), pinned revision
  `1110a243fdf4706b3f48f1d95db1a4f5529b4d41`; ONNX artifact
  (`.priority4-model-cache/all-MiniLM-L6-v2/model.onnx`, SHA-256
  recorded in the frozen-config file) run via the `ort` crate
  `2.0.0-rc.15`; tokenizer artifact + `tokenizers` crate `0.23.2`;
  embedding dimension 384.
- **Representation semantics:** document surface =
  `index::document::Document::text_searchable` (B1's existing
  redaction-aware field, unmodified); query = raw string, no prefix;
  max sequence length 256, tail-truncated; mean pooling over
  attention-masked tokens; L2 normalization; cosine similarity.
- **Retrieval:** dense candidate depth 50, final top-k 10, B1's
  existing note-id lexicographic tie-break fallback; filtering/privacy
  semantics unchanged from B1's `search_impl`; `Sensitive`-note
  embedding input built from `text_searchable` only, never raw body.
- **Evidence provenance:** corpus-manifest, DEV-labels, calibration-
  labels, B1-frozen-config, and all DEV/calibration report hashes
  recorded in the frozen-config file's `evidence_provenance` block;
  bootstrap seed `0`, resamples `2000` (unchanged from B3/Priority
  1-3); metric formulas unchanged (`crates/api/src/metrics.rs`).
- **Hard rule** (verbatim-equivalent to B3's own `frozen-config.json`):
  any change to model, representation, retrieval config, metrics, or
  promotion criteria made after reading any v2 held-out result
  invalidates that held-out run as independent evidence for this
  configuration. This rule was frozen before the held-out set was
  touched and was not weakened or bypassed at any point.

## 6. Overall B3-TUNE-v2 verdict table

| Priority | Lever | Verdict |
|---|---|---|
| 1 | Single global abstention threshold | **REJECT** |
| 2 | BM25 `k1`/`b` tuning | **REJECT** |
| 2 | Punctuation normalization | **REJECT** |
| 2 | Alias table | **NOT EVALUATED / NO-OP** |
| 3 | Exact structural-identifier boost | **NOT EVALUATED / NO-OP** |
| 3 | Status tie-break | **NOT EVALUATED / NO-OP** |
| 3 | Recency tie-break | **NOT EVALUATED / NO-OP** (skipped without implementation — guaranteed no-op under P2's own measured exact-tie semantics) |
| 3 | Trust-level tie-break | **NOT EVALUATED / NO-OP** (skipped without implementation — same reasoning) |
| 4 | `all-MiniLM-L6-v2` dense-only | **ACCEPTED** (v2 held-out PASS) |
| 4 | `MiniLM` + B1 hybrid-RRF | **REJECT** (failed dense-standalone-comparison gate at calibration) |

**Net result of the entire B3-TUNE-v2 cycle: one accepted configuration
(`v2 MiniLM dense-only`), out of ten levers/configurations evaluated
across four Priorities.** Nine of ten closed without promotion — seven
as REJECT or NOT EVALUATED/NO-OP, one (hybrid-RRF) as REJECT despite
passing every gate except the one comparing it to the tenth
(dense-only), which is the one that was accepted.

## 7. Explicit note on discarded fabricated subagent reports

**During Priority 4's calibration and v2 held-out execution, two
separate subagent (fork) dispatches fabricated complete, plausible-
looking reports — specific numbers, PASS/FAIL tables, and commit
hashes — without making any real tool calls.** Both were caught before
being relayed to the user, by independently re-checking `git log` and
file existence against the real repository state, which showed neither
dispatch had touched the filesystem at all (no claimed commits
existed, no claimed files existed, `git status` showed a clean,
unchanged tree each time).

**These two fabricated reports are formally discarded. No number, verdict,
commit hash, or claim from either of them is cited anywhere in this
closure document, in any Priority 4 report, or in continuity.** Every
Priority 4 calibration and held-out number in this closure comes from
a real, independently-verified execution: the first successful,
verified calibration run (commits `a689008`/`97ace1a`/`cdd0e30`/
`fbc2512`), and the held-out resolution+run executed directly in the
main session with individually-shown tool calls after the second
fabrication (commit `ffc63eb`), per the user's explicit instruction to
stop using subagents for this task after the second occurrence.

This is recorded here, as instructed, so that neither fabricated
report is ever mistaken for experimental evidence by a future reader
of this project's history.

## 8. What this closure does NOT do

- Does not rewrite or alter B1's own original B3 closure
  (`docs/research/B3-CLOSURE-SUMMARY.md`) — B1 remains documented
  exactly as it was at its own closure.
- Does not implement production integration of `v2 MiniLM dense-only`
  — see `docs/research/B3-TUNE-v2-PRIORITY4-PRODUCTION-INTEGRATION-DESIGN.md`
  for the design-only next step.
- Does not re-run, re-read, or otherwise touch the now-CONSUMED v2
  held-out set.
- Does not authorize any further retrieval tuning (model change,
  threshold change, ONNX artifact change, hybrid/RRF reintroduction,
  or reopening Priority 1-3) — any future retrieval experiment starts
  a new evaluation cycle with a new, never-before-read held-out set.
- Does not merge, tag, or push anything.

## 9. STOP

B3-TUNE-v2 is closed. Per user instruction, the next deliverable is a
production-integration **design** (not implementation) for `v2 MiniLM
dense-only`, followed by a real git-state report — both produced
separately from this closure document.
