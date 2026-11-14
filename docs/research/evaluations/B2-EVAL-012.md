# B2-EVAL-012: TypeScript backend/Node.js domain standalone evaluation (resumed)

**Evaluation ID:** B2-EVAL-012
**Status:** COMPLETE. No new Critical/Important production defect found
on resumption. Zero new findings of any severity on the resumed run.
**Date:** original run 2026-11-13 (paused on finding); resumed and
completed 2026-11-14.
**SuperBrain commit (HEAD at resumption):** branch `b2-code-graph`,
worktree `.worktrees/b2-code-graph`, commit `d60d0c2` (tip at resume
time: B2-ID-020 close). This report and its evidence files are the
only new artifacts from the resumed run. No merge/tag/push. No
production code changed during this resumption. A5 not authorized.
Task 10 not authorized. B2-EVAL-013 not auto-started.

## 0. Resumption disclosure

This evaluation was originally dispatched and run on 2026-11-13
(decision `d182qu`), and paused mid-run (decision `d183dc`) on a new
Important finding: TypeScript's `interface_declaration` was never
extracted at all (0/18 interfaces modeled, 0 `Implements` edges to any
local interface), filed as B2-ID-018 (tracked in Continuity, not a
standalone findings doc) and fixed (commit `01ddb37`). That fix's own
remeasurement surfaced two further, narrower residual gaps in the same
family — a generic interface name in a heritage clause parsing as
`generic_type` rather than a bare `type_identifier`, and `abstract
class X implements Y` parsing as a distinct `abstract_class_declaration`
node the extractor's walk never matched — filed as B2-ID-019 and
B2-ID-020 (likewise Continuity-tracked) and fixed (commits `6944094`,
`d60d0c2`). All three findings are CLOSED with independent review
APPROVE and 0 genuine missed mutants (decisions `d185lc`, `d187ew`).
Per the resume directive, none of that diagnosis is redone here — this
report picks up exactly where the pause left off and completes the
remaining measurement scope against the now-fixed working tree.

## Repo selection

**Repository:** `Sairyss/domain-driven-hexagon`
(https://github.com/Sairyss/domain-driven-hexagon), unchanged from the
original dispatch.
**Commit:** `5c2d15a7e2d69e83dfddf28468ee9f30e02c30de` ("docs: fixed
typo") — the tip of `master` at clone time, confirmed by direct
`git log` on a fresh clone; unchanged from the original pause.
**Corpus scope:** full repository as indexed by
`CodeGraphIndexer::observe_commit`, a NestJS-style DDD backend —
decorators, async/await, constructor-injected DI, generics, no
React/JSX. Re-cloned read-only into a fresh, isolated scratchpad
directory for this resumed run; deleted immediately after this
evaluation's data was captured.
**Parity pair:** commit A `c954323539eee13814509687fd76a6e7981ea334`
("fix: deep copy of entity props"), commit B
`6fa27463b50f19d9b2050735f98c453933911871` ("fix: get entity props") —
a real 6-file TypeScript change (`package.json` plus
`src/libs/ddd/entity.base.ts`, `src/modules/user/database/user.repository.ts`,
`src/modules/user/domain/user.entity.ts`, `src/modules/user/user.mapper.ts`,
`src/modules/wallet/wallet.mapper.ts`), both real ancestors of the
frozen commit `5c2d15a` (`5c2d15a` itself is the tip of `master`, so no
commit downstream of it exists to pair with; this nearby ancestor pair
is the real multi-file TS diff closest to the frozen commit).

**Methodology:** drove the real production pipeline
(`RepositorySource` → parse → extract → promote → lineage → edges →
`CodeGraph`, including import-aware `Calls` resolution and negative-
knowledge `CallAttempt` tracking) via the existing domain-specific
harnesses committed at the original pause,
`crates/codegraph-index/examples/b2_eval_012_ts_backend.rs` and
`b2_eval_012_ts_backend_parity.rs` (compiled unmodified against the
current, fixed working tree — no change was needed to either harness,
only to the production extractor they call through B2-ID-018/019/020).
All numbers below are **Measured**. Raw output preserved at
`docs/research/evidence/b2-eval-012-corpus-metrics-run.txt` and
`docs/research/evidence/b2-eval-012-parity-run.txt`.

## 1. Corpus / symbol metrics

| Metric | Value |
|---|---|
| Snapshots (total) | 236 |
| Edges (total) | 229 |
| Import facts | 441 |
| Impl facts | 0 |
| Call attempts (negative knowledge) | 245 |

**Snapshot kind breakdown:**

| Kind | Count |
|---|---|
| Class | 68 |
| Function | 18 |
| Interface | 18 |
| Method | 118 |
| MethodSignature | 14 |

Interface recovery confirmed: 18/18 (full recovery from the 0/18 at the
original pause). `impl_facts: 0` is unrelated to the TS interface fix —
this corpus has no `ImplTargetResolution`-eligible facts to begin with;
`Implements` edges below are the TS-specific route.

## 2. Edge kind breakdown

| Kind | Count |
|---|---|
| Calls | 90 |
| Contains | 107 |
| Extends | 27 |
| Implements | 5 |

`Implements` recovered from 0 at the original pause to 1 immediately
after the B2-ID-018 fix alone (per that fix's own commit message), to 5
now — confirming B2-ID-019 (generic heritage names) and B2-ID-020
(`abstract_class_declaration`) together closed the full residual gap on
this real corpus, not just in the synthetic unit fixtures used to
verify each fix in isolation.

## 3. Resolved / Heuristic / Ambiguous / Unresolved (negative knowledge)

| Tier | Count | Reason breakdown |
|---|---|---|
| Resolved | 0 | — |
| Heuristic | 0 | — |
| Ambiguous | 68 | AmbiguousCandidates: 68 |
| Unresolved | 177 | NoMatchingImportAlias: 176; ImportTargetExternal: 1 |

**Note on the zero Resolved/Heuristic counts — not a gap:** confirmed
by reading `codegraph-index/src/negative_knowledge.rs`: `CallAttempt`
has no `resolved`/`heuristic` constructor at all, only `unresolved*`
and `ambiguous*` variants. A `CallAttempt` is, by construction, only
ever recorded when a call site's resolution did *not* cleanly succeed;
every successfully resolved call becomes a `Calls` edge directly (90 of
them, section 2) and never produces a `CallAttempt` record. The four-
variant `EvidenceTier` enum (`crates/codegraph-domain/src/evidence.rs`)
is shared across edges, `ImportResolution`, `ImplTargetResolution` and
`CallAttempt`, but `CallAttempt` specifically only ever populates its
`Ambiguous`/`Unresolved` half. 68 + 177 = 245, matching total
`call_attempts` exactly — no missing bucket.

`SignatureTooLong` (the pre-existing extraction-rejection limitation,
not a new finding per the resume directive) does **not** appear
anywhere in this corpus's reason breakdown — it simply did not fire on
this corpus. Nothing to report either way.

## 4. A5 (type inference) evidence

68/245 `CallAttempt`s carry tier `Ambiguous` — two or more real
same-name candidates, undecidable by name-matching alone, the only
category full type inference (A5) would actually help resolve. The
remaining 177 are zero-candidate cases (176 no matching import alias, 1
external import target) that A5/type inference does not address at
all. Consistent with every prior domain's A5 evidence shape: ambiguity
is a real, nonzero, minority slice of total unresolved calls, not the
dominant one — still not sufficient justification for A5 on its own.

## 5. Full/incremental parity

Full and incremental indexing produced **zero divergences**
(`compare_full_incremental_state`), confirmed by rerunning the real
two-commit harness. Both paths agree exactly on edges (229),
import_facts (441), impl_facts (0), and call_attempts (245).

Snapshot counts differ between the two paths by design, not defect:
`full_outcome` on commit B alone reports 236 snapshots (B in isolation);
`incremental_outcome` reports 270, because the incremental path's
`observe_commit` call for commit B carries forward commit A's own
already-indexed snapshot state rather than rebuilding from nothing —
`compare_full_incremental_state` does not flag this because its own
divergence check compares *edges and facts*, not raw snapshot totals
between a from-scratch single-commit index and a carried-forward
incremental one. This matches the designed behavior of every prior
B2-EVAL parity harness in this project (e.g. B2-EVAL-010, B2-EVAL-011)
and is not itself new.

## 6. Performance

Full index of the complete corpus (236 snapshots, 229 edges): **245.78
ms** (`full_index_elapsed`). In line with every other small/medium
corpus measured across B2-EVAL-003/009/010/011 (all sub-second); no
regression signal. No separate dedicated perf-run harness exists for
B2-EVAL-012 specifically (none was built before the original pause),
but full-index timing is captured directly by the existing corpus
harness and is sufficient at this corpus size.

## 7. New findings

**None.** Zero new findings of any severity surfaced during this
resumed completion run. The only findings tied to B2-EVAL-012
(B2-ID-018/019/020) were already discovered, fixed, reviewed and
CLOSED before this resumption began.

## 8. Independent review (this resumed run)

Reviewed the three closing commits (`01ddb37`, `6944094`, `d60d0c2`)
directly against this run's real-corpus output, rather than re-trusting
the prior background-fork review alone:

- B2-ID-018's diff adds a single, narrowly-scoped
  `interface_declaration` match arm in `extract/javascript.rs`, gated
  on a node kind that provably does not exist in plain JavaScript's
  grammar (confirmed unreachable from the JS-only walk path) — no
  blast radius onto non-TS extraction.
- The commit's own stated residual scope (generic heritage names,
  `abstract_class_declaration`) exactly matches what became
  B2-ID-019/020, and this run's `Implements` count (5, up from the
  commit's own self-reported interim 1) is the real-corpus confirmation
  that both follow-on fixes closed that residual gap for real, not just
  against synthetic fixtures.
- Negative-knowledge tier/reason totals sum exactly to `call_attempts`
  (68 + 177 = 245) with no unaccounted bucket, and `Implements` (5) is
  consistent with `Interface` (18) + `Class` (68) snapshot counts (not
  every interface is implemented by a local class in this corpus — DDD
  repository-port-style interfaces frequently exist for DI/mocking
  without every consumer being in-corpus, which is expected, not a
  defect).
- No evidence of any new regression: `Calls` (90), `Contains` (107),
  `Extends` (27) are all within the expected range for a corpus this
  size and shape, and full/incremental parity holds exactly (section 5).

No disagreement with the prior fork-session reviews' APPROVE verdicts.
No new Moderate/Minor findings opened as a result of this independent
pass.

## GO/NO-GO

**GO.** B2-EVAL-012 is complete. No new Critical/Important finding.
Zero new findings of any severity. All three findings this domain
surfaced (B2-ID-018/019/020) are CLOSED with independent review
APPROVE and 0 genuine missed mutants, and this resumed run's real-
corpus remeasurement confirms their fixes hold together as a set
(not just individually) against the actual frozen corpus.

## A5 recommendation

**Still not justified**, consistent with every prior domain. This
domain's own A5 evidence (section 4): 68/245 (27.8%) of negative-
knowledge call attempts are genuine same-name-candidate ambiguity —
nonzero and the highest fraction seen in a single domain so far, but
the majority (177/245, 72.2%) remain zero-candidate cases A5 cannot
touch. Six domains evaluated (B2-EVAL-003/006/007/008/009/010/011/012)
have now consistently shown ambiguity as a real but minority slice of
total unresolved calls. A5 (type inference) should be scoped as a
deliberate experimental ablation checkpoint per decision `d102zp`,
not folded into this linear per-domain sweep.

## Proposed B2 exit condition

All eight planned broad-domain evaluations are now complete
(B2-EVAL-003 Backend/API Python, B2-EVAL-006 Python data/CLI,
B2-EVAL-007 Python CLI tooling, B2-EVAL-008 C#, B2-EVAL-009 C++,
B2-EVAL-010 JS/CommonJS, B2-EVAL-011 HTML/CSS, B2-EVAL-012 TS
backend/Node.js) with zero open Critical/Important findings remaining
anywhere in the set. Proposed concrete exit condition for closing B2 as
a phase:

1. **No open Critical/Important finding** across any B2-EVAL round —
   currently true.
2. **Every opened finding (any severity) has a recorded disposition**
   — CLOSED-with-fix, or explicitly accepted as a documented limitation
   (e.g. `SignatureTooLong`) — currently true for all findings through
   B2-ID-020.
3. **A written B2 closure summary** (one short document) listing: every
   domain evaluated, every finding opened and its final disposition,
   and the final aggregate corpus-metrics table across all eight
   domains — not yet written; proposed as the single remaining B2 task
   before declaring the phase closed.
4. **An explicit user GO on each of the three still-parked questions**
   this evaluation sweep was gating: (a) whether to run the A5 ablation
   checkpoint (`d102zp`) as its own dedicated phase, (b) whether Task
   10 is now authorized, (c) whether any further domain beyond these
   eight is still wanted before B2 is considered closed. None of these
   three are decided by this report — each needs your explicit call,
   not an inferred default.

No merge/tag/push performed. A5 not started. Task 10 not started.
B2-EVAL-013 not started.
