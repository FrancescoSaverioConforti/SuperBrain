---
schema: 1
id: m9rr
type: milestone
title: >-
  B3-TUNE-v2 Priority 2 Stage 1 complete: Lever A REJECT, Lever B KEEP, Lever C
  REJECT
status: superseded
confidence: confirmed
provenance:
  origin: auto
  created: '2026-11-14T17:44:46.896Z'
supersedes: []
superseded_by: m10zy
depends_on: []
tags: []
superseded_reason: >-
  Independent fresh-context review found the summary's characterization of all 3
  verdicts needed correction (though no verdict itself was wrong); fixes applied
  and committed.
---

All 10 tasks (1, 1b, 2-9) of docs/superpowers/plans/2026-11-14-b3-tune-v2-priority2-lexical-stage1.md executed natively with TDD, committed (range 7ea8a9b..2312ac6). Final verdicts per the plan's own Frozen decision rules (applied mechanically, calibration decides): Lever A (BM25 k1=1.8/b=0.75, A-full variant) REJECT -- passes regression tolerances but neither metric improves. Lever B (conservative punctuation normalization, B-symmetric variant) KEEP -- nDCG@10 +0.0036 on calibration, zero Recall@10 cost, abstention pattern unchanged. Lever C (7-entry frozen alias table, corpus+dev-only candidate selection) REJECT -- exact zero delta on both metrics, both splits. Only Lever B accepted. Full report: eval/results/B3-TUNE-v2/priority2-stage1-summary.md. Notable mid-execution correction: Lever B's original candidate punctuation set (including ) ] }) caused 50+ real corpus token collisions (list[str]->list[str, _check_programs()->_check_programs() during the Task 4 diagnostic; narrowed to exclude those 3 chars per the plan's own 'if collides, narrow' rule. Held-out v2 set never read. No Stage 2 composition executed -- requires separate user authorization. Full regression suite green (89+6+33 index tests, 16 api lib tests, all 5 example binaries). Independent whole-branch review dispatched to a fresh opus context before reporting the final result to the user.
