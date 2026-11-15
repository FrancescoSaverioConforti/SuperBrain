# B3-TUNE-v2 Priority 4 Stage 1, Task 6 — Promotion-Criteria Proposal

**Date:** 2026-11-14. **Status:** Task 6 of
`docs/superpowers/plans/2026-11-14-b3-tune-v2-priority4-stage1-dev-only.md`.
**STOP GATE — this document is a proposal awaiting user approval. No
calibration run, no held-out read, and no further implementation
happens past this point until the user explicitly approves (or
revises) these criteria.**

This proposal is grounded entirely in Task 2/3/5's real DEV
measurements for the selected model (`all-MiniLM-L6-v2`, Task 4), not
invented ahead of data. Where cross-model DEV findings are relevant to
judging whether a criterion is reasonable, they are cited explicitly.

---

## 1. The honest headline finding first

On this 53-note DEV corpus, **hybrid-RRF did not beat dense-standalone
retrieval alone, for any of the 4 shortlisted models** (Task 5):

| Model | B1-lexical-only | Dense-standalone | Hybrid-RRF |
|---|---|---|---|
| all-MiniLM-L6-v2 (selected) | R@10=0.8367, nDCG@10=0.7139 | R@10=0.9796, nDCG@10=0.8757 | R@10=0.9388, nDCG@10=0.8113 |
| bge-small-en-v1.5 | 0.8367 / 0.7139 | 1.0000 / 0.8815 | 0.8980 / 0.8030 |
| all-mpnet-base-v2 | 0.8367 / 0.7139 | 0.9796 / 0.8776 | 0.9184 / 0.8239 |
| nomic-embed-text-v1.5 | 0.8367 / 0.7139 | 1.0000 / 0.9089 | 0.9388 / 0.8402 |

Every model's hybrid-RRF number sits **between** B1-lexical-only and
that same model's own dense-standalone number — fusion improves on
lexical-only, but costs quality relative to dense alone, consistently,
on every one of the 4 models. Task 2's `candidate_recall_rate=1.0` for
all 4 models (every answerable DEV query's correct note appears
somewhere in the dense top-50) is the likely mechanism: on this small,
semantically clean corpus, dense retrieval already ranks the correct
note very highly on its own, and RRF's rank-based blending with B1's
comparatively weaker lexical ranking pulls some of those queries' top
results down rather than up.

**This is a DEV-only finding on a 53-note corpus and may not hold at
a larger, noisier corpus scale** — it is reported here as a candid
input to the promotion decision, not as a final verdict on RRF. But it
directly shapes criterion 2 below: a criteria set that only asks "is
hybrid better than B1" would currently pass trivially for all 4 models
on DEV, while obscuring that dense-standalone alone is the actually
stronger configuration on this same data. The proposal below asks the
calibration-stage question honestly: hybrid vs. B1 AND hybrid vs.
dense-standalone, not just the former.

## 2. Proposed numeric promotion criteria

All thresholds below are proposed against **calibration**, once this
proposal is approved — none of these numbers have been checked
against calibration at any point. "Delta" always means calibration
hybrid-RRF minus the comparison point, computed on the SAME
calibration run.

### 2.1 Recall@10 regression tolerance

**Proposed: hybrid-RRF Recall@10 on calibration must not fall more
than 0.05 absolute below official B1's own frozen calibration
Recall@10** (B3's frozen baseline, `eval/results/B3/frozen-config.json`
— not this round's B1-cost-control run, which measured latency/RAM
only and deliberately did not re-measure B1's quality). This mirrors
the exact regression tolerance Priority 2/3 already used throughout
B3-TUNE-v2, kept here for consistency rather than inventing a new
number.

### 2.2 Required nDCG@10 improvement — against BOTH comparison points

**Proposed: hybrid-RRF nDCG@10 on calibration must exceed BOTH (a)
official B1's frozen calibration nDCG@10 by at least 0.03 absolute,
AND (b) that same calibration run's own dense-standalone nDCG@10 by at
least 0.00 absolute (i.e., hybrid must not be strictly worse than
dense-standalone on the same data).**

Condition (b) exists specifically because of Section 1's finding: on
DEV, hybrid is worse than dense-standalone for all 4 models by a
non-trivial margin (e.g. MiniLM: dense nDCG@10=0.8757 vs. hybrid
nDCG@10=0.8113, a 0.064 absolute gap). If calibration reproduces this
pattern, condition (b) fails and hybrid-RRF is not promoted —
**even if condition (a) alone would have passed** — because promoting
a fusion strategy that is measurably worse than simply using dense
retrieval alone would add RRF's complexity (Section 3a of the design
doc: candidate depths, RRF constant, dedup/tie semantics) for no
benefit over the simpler dense-only configuration. This is the
single most important criterion in this proposal, precisely because
Section 1's finding makes it a live risk, not a hypothetical one.

### 2.3 Latency P50/P95 budget

**Proposed: hybrid-RRF warm-query P95 latency on calibration must not
exceed 3x B1's own warm-query P95 latency**, measured in the same
cost-control harness (Task 3's `B1-cost-control` run,
`eval/results/B3-TUNE-v2/priority4-cost-B1-cost-control.json`:
P95=124.538ms → budget ceiling 373.614ms). This reuses the exact gate
already frozen for Task 4's model-selection rule (Section 7 of the
freeze spec), applied here as the calibration-stage latency criterion
too, for consistency rather than picking a new number. DEV evidence
(Task 3): MiniLM's own dense warm-query P95 was 9.277ms — hybrid adds
a fusion step on top of that, measured directly in a calibration run
rather than assumed to stay this low.

### 2.4 Peak RAM budget

**Proposed: peak RSS during calibration querying must not exceed 300MB**
for the selected model (`all-MiniLM-L6-v2`). DEV evidence (Task 3):
MiniLM's peak RSS after querying was 150.4MB (vs. B1's own 35.0MB in
the same harness) — the proposed 300MB ceiling is exactly 2x that
observed DEV figure, giving headroom for calibration's different query
mix and this prototype harness's own overhead, without being an
arbitrarily large number disconnected from what was actually measured.

### 2.5 Model-weight footprint budget

**Proposed: the selected model's on-disk weight file must not exceed
150MB.** DEV evidence (Task 3): MiniLM's actual fetched
`onnx/model.onnx` is 90.4MB — the proposed 150MB ceiling gives
headroom (e.g. for a different ONNX export variant or quantization
level) without being vacuous, and is far below the other 3 candidates'
footprints (BGE 133MB, mpnet 436MB, Nomic 547MB) specifically because
MiniLM is the selected model and this criterion should reflect that
choice, not a generic "any embedding model" allowance.

### 2.6 Vector-index disk footprint budget

**Proposed: the vector index artifact must not exceed 10MB on this
corpus, scaling proportionally if corpus size grows** (concretely: no
more than ~2KB per indexed note, derived from this round's own
measurement: 384 floats × 4 bytes = 1536 bytes/note for MiniLM's raw
vectors, 81,408 bytes total for 53 notes — Task 3's
`vector_index_bytes_on_disk` figure). **Caveat, stated plainly:** this
round's harness has no real persistent on-disk index format at all —
Task 3's figure is a flat-array size estimate, not a measured file on
disk from an actual index implementation (Task 5 doesn't add one
either, consistent with the Stage 1 plan's scope). This criterion
is proposed as a sizing sanity check for the eventual real
implementation, not validated against an actual artifact yet, and
should be re-confirmed once (if) a real index format is built as part
of any future implementation authorization.

### 2.7 Indexing/build-time budget

**Proposed: full corpus embedding/index-build time must not exceed 3x
B1's own cost-control build time, scaling with corpus size** (B1:
145.0ms for 53 notes; MiniLM: 549.6ms for the same 53 notes —
already under the 3x ceiling of 434.9ms... narrowly **not** satisfied
at exactly 3x: 549.6ms is roughly 3.79x B1's 145.0ms).

**Correction, stated transparently:** a strict 3x multiplier, modeled
mechanically after the latency gate, does not actually fit this DEV
measurement — MiniLM's own build time is already at ~3.8x B1's. A 3x
ceiling here would be inconsistent with the model this proposal
itself selects, which is not a defensible criterion. **Revised
proposal: build-time budget is 5x B1's own cost-control build time**,
which both comfortably covers the real measured 3.79x figure and
leaves a visible, named margin rather than a ceiling picked to exactly
pass the one data point in hand. This is flagged explicitly as a
criterion that needed revising once held against real numbers, rather
than silently adjusted.

## 3. Summary table

| Criterion | Proposed threshold | DEV evidence basis |
|---|---|---|
| Recall@10 regression | ≤ 0.05 absolute below official B1 calibration | Consistent with Priority 2/3's existing tolerance |
| nDCG@10 improvement vs. B1 | ≥ +0.03 absolute above official B1 calibration | New criterion, not yet tested against calibration |
| nDCG@10 vs. dense-standalone | ≥ 0.00 absolute (hybrid must not be worse) | Directly motivated by Section 1's DEV finding |
| Warm-query P95 latency | ≤ 3x B1 cost-control P95 (373.6ms) | Reused from Task 4's frozen cost gate |
| Peak RAM | ≤ 300MB | 2x MiniLM's own DEV figure (150.4MB) |
| Model-weight footprint | ≤ 150MB | Headroom above MiniLM's actual 90.4MB |
| Vector-index disk footprint | ≤ ~2KB/note (≈10MB sanity check) | Estimate only, no real index artifact yet |
| Build/index time | ≤ 5x B1 cost-control build time | Revised from an initially inconsistent 3x |

## 4. What this proposal does not do

- **Does not run calibration.** Every threshold above is a proposal to
  be checked against calibration only after approval.
- **Does not read held-out.**
- **Does not implement weighted score fusion** — still deferred, per
  the design doc.
- **Does not change B1's production `index`/`search` code.**
- **Does not merge, tag, or push.**

## 5. STOP

Per the Stage 1 plan's Task 6 requirement, this session stops here.
The next step, if and when authorized, is a calibration run against
these criteria (once approved or revised) — not started.
