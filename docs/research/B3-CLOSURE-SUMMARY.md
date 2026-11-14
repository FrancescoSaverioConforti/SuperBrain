# B3 Closure Summary

**Phase:** B3 (Retrieval & Evaluation — text-only, per B3.0 scope decision).
**Date:** 2026-11-14.
**Status of this document:** consolidation only. No new code, no new
measurement, no new corpus run. Built entirely from evidence already on
record (committed reports, frozen config, continuity decisions). B3-TUNE-v2
is not started by this document and is not authorized by it beyond the
design draft at the end.

**Verdict: B1 deterministic trigram+BM25 baseline PASSES both frozen
thresholds. B3 is CLOSED. Baseline ACCEPTED.**

---

## 1. Scope recap

B3 validates the MVP's note-based retrieval (per `docs/Spec_mvp.md` Fase B's
acceptance criteria) against a frozen, source-faithful corpus and query
set. Explicitly out of scope throughout B3 (B3.0 decision, continuity
`d192dq`): raw-repository retrieval, graph+text retrieval,
`graph_query`/`get_code_context`, MCP, Task 10, A5.

## 2. Corpus and query set (B3.1/B3.2)

- 53 source-faithful notes, frozen at `eval/rag/corpus/notes/`, two strata:
  SuperBrain project history (30 notes) and real code
  (`controllore@ec274f4680c19f5b335a01a7a563c8f64546e7e2` + SuperBrain
  crates, 23 notes).
- 120 real queries (100 answerable + 20 abstention), independently
  paraphrased, split 60/30/30 (dev/calibration/test) + 1 mechanical
  isolation canary row in test.
- Explicit provenance (`eval/rag/corpus-provenance.json`) and manifest
  (`eval/rag/corpus-manifest.json`), both git-committed.
- Reviewed independently (final B3.1/B3.2 review): 2 Critical findings
  (per-query answer/citation mismatches, 3 mistargeted relevance labels)
  found and fixed before this phase closed.

## 3. Metric runner (B3.3)

Native Rust (`crates/api/src/metrics.rs` + `crates/api/examples/b3_metric_runner.rs`):
true Recall@10 (`|relevant ∩ top10| / |relevant|`), frozen-formula nDCG@10
(`gain(rel) = 2^rel - 1`, `DCG@10 = Σ gain(rel_i)/log2(i+1)`), deterministic
seeded bootstrap CI (SplitMix64, seed 0, 2000 resamples), abstention
confusion matrix with `null`-on-undefined precision/recall/F1 semantics.
Reviewed independently (final B3.3/B3.4 review): 1 latent bias found and
fixed (an answerable query returning `NotPresent` was dropped from
aggregation instead of scored 0.0 — no effect on the reports below, since
observed FP was 0 throughout, but the formula was wrong regardless of
today's data) and one vacuous test strengthened.

## 4. Dev results (diagnostic only, continuity `m5mu`)

- n=60 queries (49 answerable). Recall@10 = 0.8367, CI [0.7347, 0.9388].
  nDCG@10 = 0.7139, CI [0.6059, 0.8149].
- Abstention: TP=0, TN=49, FP=0, FN=11. Precision/F1 `null` (undefined),
  Recall=0.0 CI [0,0], Accuracy=0.8167 CI [0.7167, 0.9].
- Report: `eval/results/B3/dev-20261114T123322Z-e613edd.json` (commit
  `c7ad08e`).

## 5. Calibration results (threshold source, continuity `m5mu`/`m6bf`)

- n=30 queries (24 answerable). Recall@10 = 0.9167, CI [0.7917, 1.0].
  nDCG@10 = 0.7091, CI [0.5658, 0.8353].
- Abstention: TP=0, TN=24, FP=0, FN=6. Precision/F1 `null`, Recall=0.0 CI
  [0,0], Accuracy=0.8 CI [0.6667, 0.9333].
- Report: `eval/results/B3/calibration-20261114T123333Z-e613edd.json`
  (commit `c7ad08e`).

## 6. Frozen configuration (B3.4, continuity `m6bf`)

`eval/results/B3/frozen-config.json` (commit `52d42ea`), approved by the
user after the dev/calibration proposal:

| Field | Value |
|---|---|
| `retrieval_config` | B1 deterministic trigram+BM25, no embeddings |
| Recall@10 threshold | **≥ 0.79** (calibration's 95% CI lower bound) |
| nDCG@10 threshold | **≥ 0.57** (calibration's 95% CI lower bound) |
| Abstention threshold | **none** — explicit finding instead: *"baseline abstention recall = 0; constant non-abstaining behavior on dev+calibration"* |
| Bootstrap seed | `0` |
| Bootstrap resamples | `2000` |
| Runner commit | `e613edd07508985d157b32d81cc5b9704dd7a72f` |
| Freeze commit | `52d42eac27373342b80578e5f0376e70306bdf04` |

### Hashes (independently reverified against disk before freeze)

| Artifact | SHA-256 |
|---|---|
| `eval/rag/corpus-manifest.json` | `5fcc32eab09b9ea738b2cee89a68e051f03dae1a39d83e67b32f07383fcd2f51` |
| `eval/rag/calibration/labels.jsonl` | `7ff4303a12003dbfb1e00b3875f9c1395f8df27d286952c2139562dd2151b049` |
| `eval/rag/dev/labels.jsonl` | `60639a840cff1909344b01a4df49baa54e73428f297d77b1120c432a9e4347b6` |
| `eval/rag/test/labels.jsonl` (final-test input) | `17c80a96c7852ef70484c9feb8f6276a82a2a1edb6ea9e1366e4c7291a2d5191` |

### Hard rule (verbatim from `frozen-config.json`)

> Any change to `retrieval_config`, thresholds, bootstrap seed/resamples,
> or metric formulas made AFTER reading any final-test (B3.5) result
> invalidates that final-test run as independent evidence for this
> configuration.

## 7. Final-test results (B3.5, continuity `m7z0`) — CONSUMED

Executed **exactly once**, with explicit user authorization
(`--authorize-final-test-b3.5`, a minimal authorization-only gate added
to the runner in commit `4a5f7a2`, changing no scoring/retrieval/
bootstrap/threshold logic — verified by 3 new tests plus a full
regression pass).

- n=30 queries (27 answerable). **Recall@10 = 0.9259, CI [0.8148, 1.0] —
  threshold ≥ 0.79 → PASS.**
- **nDCG@10 = 0.8061, CI [0.6885, 0.9060] — threshold ≥ 0.57 → PASS.**
- Abstention: TP=0, TN=27, FP=0, FN=3. Precision/F1 `null`, Recall=0.0 CI
  [0,0], Accuracy=0.9 CI [0.7667, 1.0]. **Same constant non-abstaining
  pattern as dev/calibration — no deviation.**
- Report: `eval/results/B3/test-20261114T132816Z-4a5f7a2.json` (commit
  `d94f083`).

### Category breakdown (final-test)

| Category | Recall@10 | nDCG@10 | n | low_confidence |
|---|---|---|---|---|
| bugs-fixes | 0.80 | 0.689 | 5 | no |
| cross-file-concept | 1.0 | 0.767 | 4 | **yes** |
| identifier-code | 1.0 | 0.877 | 6 | no |
| notes-decisions | 1.0 | 0.831 | 6 | no |
| patterns-knowledge | 1.0 | 1.0 | 4 | **yes** |
| temporal | 0.50 | 0.50 | 2 | **yes** |

### Difficulty breakdown (final-test)

| Difficulty | Recall@10 | nDCG@10 | n |
|---|---|---|---|
| easy | 0.90 | 0.826 | 10 |
| medium | 0.90 | 0.720 | 10 |
| hard | 1.0 | 0.900 | 7 |

### ⚠️ This final-test run is CONSUMED

**The B3.5 final-test result above has been read and reported. Per the
frozen config's own hard rule and explicit user instruction, it CANNOT be
used as held-out evidence for B3-TUNE-v2 or any other tuning cycle.**
Any v2 retrieval configuration requires a **new**, never-before-read
held-out final-test before it can be claimed better than this baseline.

## 8. Low-confidence findings (observational, not gates)

Three category breakdowns were flagged `low_confidence: true` in the
final-test report because their answerable-query count is below the
`LOW_CONFIDENCE_FLOOR` (5): `cross-file-concept` (n=4), `patterns-knowledge`
(n=4), `temporal` (n=2). These numbers are reported, not hidden, per the
frozen protocol, but should not be read as a reliable per-category signal
— `temporal`'s 0.50 Recall@10 on 2 queries in particular is not
statistically meaningful on its own.

## 9. Baseline verdict

**PASS.** The B1 deterministic trigram+BM25 baseline passes both frozen
thresholds on held-out data, with Recall@10 and nDCG@10 slightly higher on
test than on calibration (no overfitting signal — the opposite direction
would have been the concerning one). Abstention is a confirmed, consistent
structural limitation across all three splits (dev, calibration, test):
the baseline never returns `NotPresent`, so it has no working "I don't
know" signal. This is accepted as a known baseline property, not a defect
to fix within B3's own scope.

## 10. What is NOT part of this closure

- No MCP server exists or was built.
- No merge, tag, or push has occurred — all B3 work lives on local branch
  `b3-retrieval-eval`.
- `graph_query`/`get_code_context`/Task 10/A5: not started, not touched.
- No threshold, config, or protocol change occurred after this final-test
  result was read.
