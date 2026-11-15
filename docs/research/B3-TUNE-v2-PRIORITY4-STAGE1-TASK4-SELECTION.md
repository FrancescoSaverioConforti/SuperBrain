# B3-TUNE-v2 Priority 4 Stage 1, Task 4 — Model Selection

**Date:** 2026-11-14. **Status:** Task 4 of
`docs/superpowers/plans/2026-11-14-b3-tune-v2-priority4-stage1-dev-only.md`,
complete. DEV only. No calibration consulted at any step below.

## Primary rule (frozen, Section 7 of the freeze spec)

> Select the candidate with the highest DEV Recall@10 on the
> lexical+dense RRF hybrid configuration (Task 5), subject to a hard
> cost gate: the candidate's DEV warm-query P95 latency must not
> exceed 3x B1's own DEV warm-query P95 latency (Task 3).

## Step 1 — cost gate

B1 cost-control warm-query P95 (Task 3): **124.538 ms**. Gate
threshold: **373.614 ms** (3x).

| Model | DEV warm-query P95 (Task 3) | Passes gate? |
|---|---|---|
| all-MiniLM-L6-v2 | 9.277 ms | Yes |
| bge-small-en-v1.5 | 9.460 ms | Yes |
| all-mpnet-base-v2 | 40.601 ms | Yes |
| nomic-embed-text-v1.5 | 21.384 ms | Yes |

**All 4 candidates pass the cost gate** — none is excluded at this
step.

## Step 2 — highest hybrid-RRF Recall@10 (Task 5)

| Model | Hybrid-RRF Recall@10 (DEV) |
|---|---|
| all-MiniLM-L6-v2 | 0.938775510204... |
| bge-small-en-v1.5 | 0.897959183673... |
| all-mpnet-base-v2 | 0.918367346938... |
| **nomic-embed-text-v1.5** | **0.938775510204...** |

**Exact tie between `all-MiniLM-L6-v2` and `nomic-embed-text-v1.5`**,
confirmed bitwise-equal in the underlying floating-point values (both
correctly answer 46/49 of the same answerable DEV queries within
top-10 — a real tie from identical numerator/denominator, not a
display-rounding artifact). BGE and mpnet are both strictly lower and
are excluded from further consideration at this step.

## Step 3 — tie-break (frozen, Section 6 of the freeze spec / shortlist audit)

1. **Lower DEV warm-query P95 latency wins.**
   - `all-MiniLM-L6-v2`: 9.277 ms
   - `nomic-embed-text-v1.5`: 21.384 ms
   - **`all-MiniLM-L6-v2` has the lower P95 latency.**

The tie-break resolves at step 1 — steps 2 (model-weight footprint)
and 3 (fixed precedence order) are not reached and are not needed.

## Selected model: `all-MiniLM-L6-v2`

**Mechanical result, not a fresh judgment call.** The primary rule
alone produced an exact tie between two candidates; the pre-frozen
tie-break (itself frozen before any DEV benchmark ran, per the freeze
spec and the shortlist audit's own tie-break section) resolved it
deterministically in favor of the lower-latency candidate. No
criterion was adjusted, reweighted, or reinterpreted after seeing this
result.

## From this point forward

Per the plan's Task 4 requirement: **no other model may be substituted
without restarting the plan's Task 0-4 sequence.** Task 6's
promotion-criteria proposal is grounded in `all-MiniLM-L6-v2`'s own
numbers specifically, alongside the cross-model comparison data
already gathered (which remains in the record as context, not as an
invitation to re-open model choice).

## What this step does not do

- No calibration run or read.
- No held-out read.
- No re-weighting of the primary rule or the tie-break after seeing
  this result — both were applied exactly as frozen.
