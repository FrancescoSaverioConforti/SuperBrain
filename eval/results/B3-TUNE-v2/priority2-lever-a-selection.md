# Lever A — Grid-Point Selection (Task 3 Step 1)

Applying the Frozen decision rules' grid-point-selection rule (plan
`docs/superpowers/plans/2026-11-14-b3-tune-v2-priority2-lexical-stage1.md`)
verbatim to `priority2-lever-a-grid.json`'s 16 rows:

1. **Maximize dev Recall@10.** All 16 grid points tie at Recall@10 =
   0.836734693877551 — `k1`/`b` have no effect on which notes rank in
   the top 10 for this dev split (only on their relative nDCG@10
   ordering within that top 10, since all queries' relevant notes are
   already being retrieved regardless of the grid point tested).
2. **Tie-break: maximize dev nDCG@10.** Among the 16 tied-recall points,
   `(k1=1.8, b=0.75)` has the highest nDCG@10 = 0.7169988752566947,
   uniquely (no further tie).
3. **No further tie-break needed** — Step 2 already resolved to one
   point.

**Selected: `k1 = 1.8, b = 0.75`.**

This is NOT the current defaults (`1.2, 0.75`) — reported as a valid
outcome per the plan's own instruction ("if the selected point IS the
current defaults, that is itself a valid, reportable outcome"); here
the dev-selected point differs from the default in `k1` only.
