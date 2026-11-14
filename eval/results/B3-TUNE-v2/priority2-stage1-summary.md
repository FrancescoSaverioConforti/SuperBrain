# B3-TUNE-v2 Priority 2 Stage 1 — KEEP/REJECT Summary

**Phase:** B3-TUNE-v2, Priority 2 (lexical tuning), Stage 1 (isolated
ablations).
**Date:** 2026-11-14.
**Scope:** Three independent levers (BM25 `k1`/`b`, punctuation
normalization, deterministic alias table), each measured against the
**official** B1 baseline (`eval/results/B3/frozen-config.json`, commit
`52d42ea`) — never against Priority 1 (closed REJECT,
`docs/research/B3-TUNE-v2-PRIORITY1-CLOSURE.md`) and never against each
other. No Stage 2 (composition) in this document.

All KEEP/REJECT decisions below apply the Frozen decision rules fixed
in the plan (`docs/superpowers/plans/2026-11-14-b3-tune-v2-priority2-lexical-stage1.md`)
**before** any grid or ablation run: Recall@10/nDCG@10 regression ≤0.05
absolute vs. the official B1 baseline, at least one of the two metrics
must improve, and the abstention confusion matrix pattern must stay
unchanged (TP=0, FP=0, matching B1's own constant non-abstaining
behavior) — any lever causing even one `NotPresent` where B1 didn't, or
vice versa, is an automatic REJECT regardless of the other metrics.

---

## Lever A — BM25 `k1`/`b` — **REJECT**

**Config:** `k1=1.8, b=0.75` (selected from a 16-point grid over dev via
the frozen rule: max Recall@10 → tie-break max nDCG@10 → tie-break min
distance to defaults `(1.2, 0.75)`; `(1.8, 0.75)` won the nDCG@10
tie-break uniquely, no further tie-break needed — see
`priority2-lever-a-selection.md`).

**Variants measured:** `A-score` (custom params reach only final
scoring; informational) and `A-full` (custom params reach the entire
real pipeline including the candidate prefilter; this is the decision
variant). At this specific grid point, the two variants agreed exactly
on both splits — no candidate-set drift observed (not evidence drift
never happens at other grid points, just that it didn't happen here).

**Commit:** `efd4ccf` (constructor path), `5d4196a` (grid),
`efd4ccf4` (ablation — commit `efd4ccf`). Corpus manifest/labels
hashes: see individual ablation reports.

| | Recall@10 | Δ vs B1 | nDCG@10 | Δ vs B1 |
|---|---|---|---|---|
| Dev (`A-full`) | 0.8367 | — (diagnostic only) | 0.7170 | — |
| Calibration (`A-full`) | 0.9167 | **+0.0000** | 0.6966 | **−0.0125** |

**Abstention (calibration, `A-full`):** TP=0, TN=24, FP=0, FN=6 —
unchanged from B1's own pattern.

**Latency/resource:** not measured.

**KEEP/REJECT rationale:** Recall@10 regression (0.0000) and nDCG@10
regression (0.0125) both pass the ≤0.05 tolerance. Abstention pattern
unchanged. **But neither metric improves** — Recall@10 is exactly flat,
nDCG@10 regresses (does not improve). Per the frozen rule, "improves
nothing but doesn't regress much either is not a reason to change a
frozen baseline."

**Verdict: REJECT.**

---

## Lever B — Conservative Punctuation Normalization — **KEEP**

**Config:** `strip_trailing_sentence_punctuation` — strips only trailing
`. , ; : ! ? " '` (narrowed from the design doc's original candidate
set, which also included `) ] }`, after Task 4's corpus+dev diagnostic
found 50+ real collisions where those characters closed a function
call, type annotation, or complexity notation rather than a sentence —
see `priority2-lever-b-token-diagnostic.json` and commit `ee5c6a4`).

**Variants measured:** `B-query` (normalization applied to the query
only; informational) and `B-symmetric` (normalization applied to both
indexed text — via a standalone `Bm25Index`/`TrigramIndex` pair that
bypasses `RealIndexer`, since the frozen production tokenizer is never
modified — and the query; this is the decision variant).

**Commit:** `ee5c6a4` (rule + diagnostic), `c191b21` (ablation).

| | Recall@10 | Δ vs B1 | nDCG@10 | Δ vs B1 |
|---|---|---|---|---|
| Dev (`B-query`) | 0.8776 | — (diagnostic only) | 0.7503 | — |
| Dev (`B-symmetric`) | 0.9592 | — (diagnostic only) | 0.7969 | — |
| Calibration (`B-symmetric`) | 0.9167 | **+0.0000** | 0.7126 | **+0.0036** |

**Abstention (calibration, `B-symmetric`):** TP=0, TN=24, FP=0, FN=6 —
unchanged from B1's own pattern.

**Latency/resource:** not measured.

**KEEP/REJECT rationale:** Recall@10 regression (0.0000, a flat tie —
not a regression) passes. nDCG@10 **improves** by +0.0036 — a genuine,
if small, improvement, not a regression. At least one metric improves
(nDCG@10). Abstention pattern unchanged.

**Verdict: KEEP.**

(Note: dev's own `B-symmetric` improvement — 0.9592 vs. B1's 0.8367 —
is much larger than calibration's. Dev is diagnostic only per the
frozen discipline; the calibration number above is what decided this
lever, and it is a smaller but still real, non-negative improvement.)

---

## Lever C — Deterministic Alias Table — **REJECT**

**Config:** 7-entry frozen table (`SEC-001`, `SEC-002`, `SEC-003`, `C1`,
`C5`, `C6`, `Fase-B`), each entry a hand-reviewed expansion phrase read
from its note's real body, built from corpus structure
(`source:` frontmatter `#<IDENTIFIER>` fragments) cross-referenced
against dev's `relevant_note_ids` only — calibration was never read to
identify, choose, or justify any entry (design doc correction #1). Query-time
expansion, whole-token matching only, appends rather than replaces.

**Commit:** `ced0540` (diagnostic), `ec96891` (frozen table),
`676954c` (ablation).

| | Recall@10 | Δ vs B1 | nDCG@10 | Δ vs B1 |
|---|---|---|---|---|
| Dev | 0.8367 | — (diagnostic only) | 0.7139 | — |
| Calibration | 0.9167 | **+0.0000** | 0.7091 | **+0.0000** |

**Abstention (calibration):** TP=0, TN=24, FP=0, FN=6 — unchanged.

**Latency/resource:** not measured.

**KEEP/REJECT rationale:** Both regression tolerances pass trivially
(exact zero delta on both metrics, both splits). Abstention pattern
unchanged. **Neither metric improves** — the table had zero measurable
effect on either split, most likely because neither dev's nor
calibration's query set happens to invoke any of the 7 frozen short
forms by their exact literal identifier often enough to move the
aggregate (dev's own numbers are identical to B1's, which is itself
notable: the 7 queries that originally justified these entries already
retrieved their target notes correctly without the alias expansion,
so the expansion added redundant terms rather than rescuing a miss).

**Verdict: REJECT.**

---

## Secondary diagnostic: Priority 1 signal separation (informational only, non-gating)

Priority 1 (closed REJECT) measured the `ratio` (top1/top2 score)
signal's separation between abstain-expected and answerable queries on
calibration, finding `separation_gap = -0.445` under the plain B1
baseline — heavy overlap, no cutoff of this signal cleared the required
abstention guardrails.

Lever B (KEEP) changes the score distribution itself (both Recall@10
and nDCG@10 shift), which could plausibly sharpen or blur `ratio`'s
separation as a side effect. This was not re-measured in this task
(optional per the plan, not re-run to keep Stage 1's scope to its own
three ablations) — flagging it here as a candidate follow-up measurement
for a future, separately-authorized round, not as a reopening of
Priority 1's closed threshold search. No KEEP/REJECT decision in this
document depends on this diagnostic.

---

## Overall Stage 1 result

| Lever | Decision variant | Verdict |
|---|---|---|
| A (BM25 k1/b) | `A-full` | **REJECT** |
| B (punctuation normalization) | `B-symmetric` | **KEEP** |
| C (alias table) | — | **REJECT** |

**Only Lever B is accepted** from this Stage 1 round.

## Explicit confirmations

- **The new v2 held-out final-test set (`eval/rag/v2-heldout/`) has not
  been read by this session or any prior Priority 2 session.** Its
  content remains completely unread; nothing in this plan opens
  `eval/rag/v2-heldout/labels.jsonl`.
- **No composite configuration (Stage 2) has been built or measured.**
  Composing Lever B's accepted change with anything else requires the
  user's separate, explicit authorization after reviewing this summary.
- **No Priority 3 (reranking) or Priority 4 (hybrid retrieval) work has
  started.**
- No modification to the B3 frozen config, runner, corpus, or labels.
- No MCP, no merge, no tag, no push.
