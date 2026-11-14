# B2-EVAL-002: Task 9.5 before/after re-evaluation

**Evaluation ID:** B2-EVAL-002
**Date:** 2026-11-06
**SuperBrain commits compared:**
- **Before:** `9c1d06f` (the exact commit B2-EVAL-001's original run was performed against, i.e. the state immediately preceding Task 9.5)
- **After:** `7c35741` (Task 9.5, approved)
**External repository:** `pytorch/examples` (https://github.com/pytorch/examples, BSD-3-Clause) — same repository as B2-EVAL-001 and B2-ID-003/004/005, freshly re-cloned into a temporary scratchpad location for each of the two harness runs this evaluation required (one clone for the initial before/after pair, a second re-clone when the Q4 harness correction below required a third, separate re-run), with every clone deleted once its own run's output was captured. An independent review of this evaluation's own methodology (see Section I) initially found a leftover, undeleted clone plus unrelated stale debug files from an EARLIER, unrelated investigation in this same session (Priority 5(A)'s chained-call root-cause work, predating Task 9.5's authorization) still present in the scratchpad at review time; that leftover material was confirmed unrelated to this evaluation's own claims, contained no evidence bearing on any number in this report, and has since been deleted. Recorded here transparently rather than silently cleaned up without mention, per the Research-grade Findings Policy's own disclosure expectations.
**External commits:** commit A `20955149ad6ea97a4f403e5a56c95a39e568ccbe`, commit B `001d493285083b54cacbc1334cb26338bdff2455` — identical to every prior evaluation on this corpus. Corpus and commits were NOT changed to produce a more favorable result.
**Methodology:** both the "before" and "after" `codegraph_index` crate were built from the SAME `b2_smoke_test.rs` harness logic (one minor, explicitly-labeled correction applied only to the "before" worktree's own Q4 diagnostic — see "Harness correction" below) and run against the identical corpus/commit pair, in two separate, isolated git worktrees. All numbers below are **Measured** unless explicitly marked otherwise, per the Evaluation & Paper Evidence Policy.

## Harness correction (methodology note, not a production-code change)

While producing this re-evaluation, Q4's original harness code (`find_by_name(model_name).first()`) was found to silently keep only ONE candidate whenever a name is ambiguous — exactly the same class of defect [B2-ID-003](../findings/B2-ID-003.md) already found and fixed elsewhere in this harness. This was NOT present in the previously-fixed `find_current_member_of_class` path (which B2-ID-003 corrected); it was a second, separate instance of the identical ambiguous-`.first()` pattern, in Q4's model-constructor-caller lookup specifically. Corrected ONLY in the temporary "before" worktree used to reproduce this evaluation's baseline (never in the project's committed history, which stays as the accurate historical record of what was run at the time); the correction reports callers for EVERY candidate matching a name, not just the first one the pool happens to return. See "Q4" below for what this reveals.

## A. Symbol / structural coverage

| Metric | Before (9c1d06f) | After (7c35741) |
|---|---:|---:|
| Raw candidates extracted | 567 | 943 |
| Snapshots promoted | 567 | 943 |
| `Class` count | 98 | 98 |
| `Function` count | 462 | 462 |
| `Struct` count | 7 | 7 |
| `Field` count | 0 (kind did not exist) | 376 |
| Field coverage (self.X = <expr> instance attributes captured) | 0/376\* | 376/376 |
| `Class -> Method/Function/Field` Contains edges | 0 (kind did not exist) | 548 |
| `Instantiates` edges | 0 (kind did not exist) | 78 |

\* "0/376" uses the AFTER run's own Field count as the denominator — the BEFORE pipeline had no concept of a Field at all, so there is no independent "true Field count" from the before run itself; 376 is the number this evaluation treats as the best available measured count of real `self.X = <expr>` assignments in this corpus at this commit, produced by the AFTER pipeline. This is an **Inferred** denominator, not independently cross-measured by a BEFORE-side method, and is recorded as such rather than presented as a bare percentage.

## B. Calls — precision/recall/F1 by configuration

Configuration IDs per `docs/research/EXPERIMENTAL_POLICY.md`. The BEFORE pipeline ran **CALL-RES-A0** (name-only, corpus-wide, no scoping, no receiver-awareness) exclusively. The AFTER pipeline runs **CALL-RES-A1+A2+A3** (ambiguity suppression + scope-aware narrowing + receiver-aware filtering) exclusively. Both configurations' results are preserved below — neither overwrites the other.

| Metric | CALL-RES-A0 (Before) | CALL-RES-A1+A2+A3 (After) |
|---|---:|---:|
| Total `Calls` edges | 11,934 | 878 |
| `StaticAnalysis` (resolved) | 286 | 1,168\*\* |
| `NameHeuristic` (ambiguous/scoped-heuristic) | 11,652 | 340 |
| Manual 20-sample precision (Manually validated, same deterministic sort, N=20) | 18/20 correct, 2/20 correctly-flagged-ambiguous (B2-EVAL-001's own figure) | 20/20 correct (all `StaticAnalysis`, each manually re-checked against real source during this evaluation) |
| Q6 false-positive rate on a common name (`forward`, Manually validated) | 19/19 confirmed false positives (B2-EVAL-001's own figure) | 0/1 — the one remaining incoming "edge" to `forward` is a `Contains` edge (structural, not a `Calls` guess), not a false positive |
| Confirmed cross-class/cross-language false positive (`self.src_mask.size(0)` → unrelated C++ `size()`) | Present (2 false `Calls` edges, B2-ID-004) | Absent (0 edges; correctly produces no edge instead) |
| Confirmed true-positive loss from ambiguity suppression (`_run_trainer` → `RNNModel`, this evaluation's own new finding) | Present as a HEURISTIC edge, mixed with a false positive to the WRONG `RNNModel` (see Q4 below) | Absent (0 edges; the true target is lost, not just the false one) |

\*\* `StaticAnalysis` count rose in absolute terms (286 → 1,168) because `Contains`/`Instantiates` (always `StaticAnalysis`-evidenced, per their own design) are included in this corpus-wide `by_resolution_method` tally alongside `Calls`. Counting `Calls`-only `StaticAnalysis` edges specifically was not separately instrumented by the harness in this run; recorded as a measurement granularity limitation of this evaluation, not claimed as a `Calls`-specific number here.

**Precision/recall is not computed as a single global number for either configuration.** Per the Evidence-before-claims Experimental Policy, no full corpus-wide ground truth exists (that would require manually verifying thousands of edges); precision is estimated from the 20-sample (Manually validated, N=20 only) and the two targeted false-positive/true-positive cases found across both evaluations. A global F1 is not computed, since recall's true denominator (every real call relationship in the corpus) is not independently known. This is recorded explicitly rather than presented as a global metric that does not exist.

> ## ERRATUM (2026-11-06, [B2-ID-006](../findings/B2-ID-006.md))
>
> **Previous claim:** the AFTER-configuration (CALL-RES-A1+A2+A3) row
> above reports **878** total `Calls` edges and **1,168**
> `StaticAnalysis`-resolved edges (footnoted as including `Contains`/
> `Instantiates`), measured against this evaluation's own harness run
> at SuperBrain commit `7c35741`. The 20-sample manual precision check
> (`20/20 correct`) validated TARGET correctness only — whether each
> edge pointed at the right symbol — and did not separately check
> CALLER correctness — whether the snapshot recorded as the caller was
> itself a real, executable caller. Neither this evaluation nor
> B2-EVAL-001 before it checked that second axis.
>
> **What B2-ID-006 found:** `resolve_calls` attributed a call site to
> EVERY enclosing container snapshot whose span happened to contain it
> (e.g. a `Class`, or an outer `Function` containing a nested one), not
> only the nearest real executable caller — confirmed, on a full-corpus
> contamination count run on this exact repository/commit AFTER this
> evaluation was published, that **256 of the 878 `Calls` edges (29.2%)**
> originated from a `Class`/`Struct`-kind snapshot, with a further
> number originating from a `Field`/nested-`Function` nesting pattern
> this specific count did not separately isolate until a later, more
> precise re-run (see below).
>
> **Fix and corrected result (Measured, re-run via a freshly re-cloned
> copy of this exact repository at this exact commit, with B2-ID-006's
> fix applied, raw output preserved at
> `docs/research/evidence/b2-id-006-pytorch-after-fix-run.txt`, full
> before/after edge-level diff preserved at
> `docs/research/evidence/b2-id-006-pytorch-before-after-edge-diff.txt`):**
>
> | Metric | This evaluation's original AFTER figure | Corrected (post-B2-ID-006-fix) |
> |---|---:|---:|
> | Total `Calls` edges | 878 | **535** |
> | Container-originated (`Class`/`Struct`/`Interface`/`Enum` as caller) | not separately measured at the time | **0** (was 256, confirmed via the dedicated contamination-count tool) |
> | `Field`-originated (structurally always nested inside its own method, confirmed never a real caller) | not separately measured at the time | **0** (was 81) |
> | Nested-`Function`-in-`Function` spurious duplicates (e.g. `validate`'s outer function rediscovering `run_validate`'s own calls to `accuracy`/`display`/`update`) | not separately measured at the time, not previously known as a distinct case | **0** (was 6, confirmed via a full before/after edge-level diff, not merely a count) |
> | `by_resolution_method StaticAnalysis` (all edge kinds, same corpus-wide tally this evaluation's own footnote describes) | 1,168 | 942 |
> | `by_resolution_method NameHeuristic` | 340 | 223 |
>
> **Every one of the 343 removed edges (878 → 535) was confirmed, by a
> full edge-level diff (not a sample), to be a spurious duplicate of a
> real edge that remains present under its correct, more specific
> caller** — zero new edges were introduced by the fix, and two
> specific cases (`CommentsRepository`-shaped and `validate`/
> `run_validate`-shaped) were independently traced against real source
> to confirm the real, correct edge survived unchanged while only the
> spurious ancestor-attributed duplicate was removed.
>
> **Impact on this evaluation's own Q3/Q4/Q6 (re-run, Measured,
> identical to this evaluation's own original AFTER answers — see
> `docs/research/evidence/b2-id-006-pytorch-after-fix-run.txt`):**
> Q3's `TransformerModel::forward` callees, Q4's empty
> `TransformerModel`/`RNNModel` caller lists, and Q6's single `Contains`
> edge are **unchanged** — none of these three specific answers were
> built from a container-originated edge, so none of them were
> corrupted by this defect, and this fix introduces no change to them.
> **No precision/recall conclusion in this evaluation's Section D is
> invalidated** by this correction, for the same reason.
>
> **What this changes:** the `878`/`1,168`/`340` figures in the table
> above describe a MEASURED state that included a now-confirmed,
> now-fixed source of spurious edges, and must not be read as the
> pipeline's current behavior. **What this does NOT change:** this
> evaluation's own 20-sample manual check and its Q3/Q4/Q6 answers
> remain valid as TARGET-correctness findings — they were never wrong
> about which symbol an edge pointed at, only silent about a second,
> independent axis (caller correctness) this evaluation did not think
> to check at the time. This correction does not retroactively
> invalidate the GO decision in Section J below; if anything, the
> corrected numbers make the AFTER configuration's own precision case
> stronger (fewer, more precisely-attributed edges), not weaker.

## C. Imports — extraction coverage vs. resolution accuracy, kept separate

| Metric | Before (9c1d06f) | After (7c35741) |
|---|---:|---:|
| Real import statements (line-based heuristic count) | 589 | 589 |
| **Raw extraction** — import facts/edges produced at all | 0 | 691 (file-level `ImportFact`s, all languages; 589 is itself a per-line undercount since 52 real lines import more than one comma-separated name each, each now correctly counted as its own fact) |
| Extraction coverage | 0/589 (0%) | 691 facts extracted against a 589-line heuristic denominator — the two numbers measure different things (statements vs. facts) and are not a single ratio; see B2-ID-005 |
| **Semantic resolution** (AFTER only; BEFORE produced no facts to resolve at all) | N/A | `Internal`: 41; `External`: 621; `Ambiguous`: 29 |
| Aliases captured | N/A | 155 |
| False positives on resolution (invented internal targets for an external/unresolved import) | N/A (no resolution existed) | 0 confirmed (by design: `External`/`Ambiguous` never carry a target) |
| False negatives on resolution (manual sample) | N/A | Not separately sampled in this evaluation; see "Residual limitations" |

## D. Coding-agent usefulness — the same 6 questions

| Question | Before (CALL-RES-A0, 9c1d06f) | After (CALL-RES-A1+A2+A3, 7c35741) |
|---|---|---|
| Q1. Main model class? | Partially answerable (name found; disambiguating "current" needs extra graph-navigation logic) | Partially answerable (unchanged — this is a `CodeGraph` current-vs-historical limitation, out of this task's scope) |
| Q2. What does it extend? | Not answerable (`nn.Transformer` is external) | Not answerable (unchanged — correctly produces no edge for an external base class) |
| Q3. What composes the forward path? | **Incorrect** — `["cpp/custom-dataset/custom-dataset.cpp::size", "cpp/transfer-learning/main.h::size"]`, both confirmed false positives | **Fully answerable** — `["encoder", "decoder", "_generate_square_subsequent_mask", "pos_encoder", "input_emb"]`, all 5 genuine, 0 false positives (manually re-confirmed against real source during this evaluation) |
| Q4. Who calls the model? | **Partially answerable, but contaminated** — `RNNModel`'s true caller (`_run_trainer`) WAS present, but the harness's own `.first()` bug masked that the SAME ambiguous call ALSO produced a false-positive heuristic edge to an unrelated `RNNModel` in `word_language_model/model.py` (confirmed via this evaluation's harness correction — see below) | **Not answerable for `RNNModel`** — the true `_run_trainer` → `RNNModel` relationship is now LOST (0 edges), a genuine recall cost of ambiguity suppression, not a bug; `TransformerModel` is unchanged (`[]`, both before and after — never had a resolvable caller in this corpus) |
| Q5. Which tests are linked? | Correctly answerable as "none exist" (unchanged ground truth: no test file) | Correctly answerable as "none exist" (unchanged) |
| Q6. If I modify `forward`, what is impacted? | **Incorrect** — 19/19 confirmed false positives | **Correct** — exactly 1 edge, a `Contains` edge (structural fact, not a `Calls` guess); 0 false-positive `Calls` edges |

**Summary counts** (Fully/Partially/Not-answerable/Incorrect, out of 6):
- **Before:** 0 Fully answerable, 2 Partially answerable (Q1, Q4\*), 2 Not answerable (Q2, Q5), 2 Incorrect (Q3, Q6).
- **After:** 1 Fully answerable (Q3), 1 Partially answerable (Q1), 2 Not answerable (Q2, Q5), 1 Incorrect... **0 Incorrect** (Q6 corrected), **1 new Not-answerable** where there was previously a contaminated Partial (Q4).

\* Q4's BEFORE classification is itself corrected by this evaluation: B2-EVAL-001's original report did not classify Q4 as containing a false positive, because its own harness's `.first()` bug hid it. Under this evaluation's harness correction, Q4's BEFORE answer is more accurately: **Partially answerable, contaminated by an undetected false positive** — not a clean "Partially answerable."

## E. Q4 — the `RNNModel` ambiguity, in full (new finding from this evaluation)

**Observed (Measured — the harness-correction re-run's raw output is preserved at `docs/research/evidence/b2-eval-002-before-corrected-q4.txt`):** two real classes named `RNNModel` exist in this corpus — `distributed/rpc/rnn/rnn.py::RNNModel` (the real target of `distributed/rpc/rnn/main.py::_run_trainer`'s `rnn.RNNModel(...)` constructor call, confirmed by direct source reading) and `word_language_model/model.py::RNNModel` (unrelated).

- **Before (CALL-RES-A0):** the ambiguous call produced TWO heuristic `Calls` edges — one to each `RNNModel`. The preserved corrected-harness output shows exactly this: `callers of RNNModel [distributed/rpc/rnn/rnn.py]: ["distributed/rpc/rnn/main.py::_run_trainer"]` AND `callers of RNNModel [word_language_model/model.py]: ["distributed/rpc/rnn/main.py::_run_trainer"]`, both present in the same run. `_run_trainer` therefore appears as a caller of BOTH the correct target and the unrelated one. The original B2-EVAL-001 report's Q4 answer, and this evaluation's own first (uncorrected) before-run, only showed the correct one, because the harness code kept only the first pool match — an artifact of pool ordering, not evidence the pipeline distinguished the two. (A second, independently-known harness ambiguity — B2-ID-003's unfixed `current_forward` selection, present in this same frozen pre-Task-9.5 commit by construction, since that fix landed in a LATER commit — also caused this particular corrected re-run's Q3 answer to differ in which `forward` candidate it picked compared to this evaluation's first before-run; this is expected, already-documented non-determinism in the FROZEN HISTORICAL harness itself, not a new finding, and does not affect the Q4/`RNNModel` claim this section makes.)
- **After (CALL-RES-A1+A2+A3):** `_run_trainer` is a free function (no enclosing class), so Case A's class-level narrowing does not apply; Case B's same-file narrowing also finds 0 candidates (`_run_trainer`'s own file, `distributed/rpc/rnn/main.py`, contains neither `RNNModel` definition). With no scope signal narrowing the ambiguity to one candidate, `narrow_by_scope` returns `None` — **no edge at all**, for either target.

**Classification:** a genuine recall loss (one true positive removed) traded for removing one false positive, consistent with — and in fact a second, independently-discovered real-world instance of — the exact precision/recall tradeoff `docs/research/EXPERIMENTAL_POLICY.md` already names as needing future measurement via an ablation study. **This is not a new pipeline defect** and is not treated as one; no fix is applied here. This specific case (repository, commit, caller, call site, expected target, both configurations' actual output) is added to a new persistent resolution challenge set (see `docs/research/challenge-sets/call-resolution-challenges.md`) for reuse in future ablation work comparing CALL-RES-A0 through A8.

## F. Incremental indexing

| Metric | Before (9c1d06f) | After (7c35741) |
|---|---:|---:|
| Files reparsed (A→B, 1 real file changed) | 1 | 1 |
| Snapshots removed | 13 | 32 |
| Snapshots added | 13 | 31 |
| Edges rebuilt | 11,951 | 1,539 |
| Raw import facts reused (untouched files) | N/A (no import-fact concept existed) | 687 |
| Raw import facts newly extracted (the 1 changed file) | N/A | 4 |
| Import facts resolved (always full recompute, by design) | N/A | 691 |
| Final snapshot count | 580 | 974 |
| Final edge count | 11,938 | 1,508 |
| Final lineage edge count | 13 | 31 |

## G. Performance

| Metric | Before (9c1d06f) | After (7c35741) |
|---|---:|---:|
| Full index wall-clock (Measured, single run — see B2-EVAL-001's own note that 3-run averages were used there; this checkpoint ran once per side to keep the comparison itself the priority) | ~1.115s (direct) / ~1.094s (via `CodeGraphIndexer`) | ~0.987s (direct) / ~1.046s (via `CodeGraphIndexer`) |
| Incremental index wall-clock | ~0.775s | ~0.671s |
| Memory (Estimated only, Windows `tasklist`, coarse) | 14,832 KB ≈ 14.48 MB | 11,748 KB ≈ 11.47 MB |

**Interpretation:** the AFTER pipeline is not slower despite doing substantially more semantic work (Field/Contains/Instantiates/ImportFact extraction and resolution) — it processes far FEWER total edges (878 Calls vs. 11,934), because the old corpus-wide ambiguous-call fan-out (CALL-RES-A0) was itself the majority of the old edge count's cost. This is a single-run Measured observation, not a benchmark-grade multi-run average; stated as such rather than overclaimed.

## H. Regressions found

One real behavioral change was found and is classified per the Research-grade Findings Policy:

- **Q4 `RNNModel` recall loss** (Section E above) — Important, but NOT a defect: a deliberate, previously-anticipated tradeoff of the CALL-RES-A1+A2+A3 configuration, not an unintended regression. No fix applied in this checkpoint. Added to the persistent challenge set for future ablation comparison.

No Critical or Important **defects** were found. No other discrepancy between the before and after runs was found beyond the ones Task 9.5's own prior findings (B2-ID-003/004/005) already documented and this evaluation's measurements directly confirm.

## I. Independent review

An independent review was performed (separately from Task 9.5's own closing-gate review, which covered the CODE; this review covers THIS EVALUATION's own methodology and numbers). **Verdict on the first draft: REQUEST CHANGES — one Critical finding, two Important findings**, all now addressed in this document:

- **Critical (fixed):** this evaluation's Section E originally labeled its central Q4 claim "Measured, via the harness-correction re-run" without ever having preserved that re-run's actual output to a file — the claim, while independently re-confirmed as true, was not yet backed by durable evidence at the time it was written. Fixed by re-running the corrected-before harness a second time (a fresh clone of the same repository at the same commits, since the first clone had already been deleted per policy) and preserving its raw output at `docs/research/evidence/b2-eval-002-before-corrected-q4.txt`, now cited directly in Section E.
- **Important (fixed):** the reviewer found a leftover, undeleted repository clone plus three unrelated debug-run text files in the shared scratchpad, left over from an EARLIER, unrelated investigation in this same session (Priority 5(A)'s chained-call root-cause work, which predates Task 9.5's own authorization) — contradicting this document's original claim that every clone had been deleted. Confirmed the leftover material contained no evidence bearing on any claim in this report, deleted it, and corrected this document's own wording (see the repository-cleanup note near the top) to describe accurately what was and was not cleaned up, and when.
- **Important (addressed):** the reviewer asked whether the AFTER side's own Q4 diagnostic could hide an equivalent, yet-undiscovered `.first()`-style bug. Checked: the AFTER run's Q4 output is `[]` for both model names — an empty result has nothing left for a truncating bug to hide (there is no "second candidate" that `.first()` could be dropping when the result set is already empty), so this specific bug class cannot produce a false empty-looking answer the way it can produce a false single-answer one. Recorded here rather than left silently unaddressed.
- **Suggestion (checked, confirmed, and fixed):** the reviewer flagged the Section G memory unit conversion as worth double-checking. Verified: Windows `tasklist`'s memory column reports whole KB (`"14.832 K"` means 14,832 KB, using a locale decimal separator where a thousands comma would otherwise appear — not 14.832 KB). The correct conversion divides by 1,024, giving 14.48 MB and 11.47 MB — both figures corrected in the table above (previously mislabeled "~14.8 MB"/"~11.7 MB," which read the raw KB count as if it were already in MB). This same rounding error appears to be inherited from B2-EVAL-001's own prior use of the identical tool/phrasing; not corrected there retroactively by this document (B2-EVAL-001 stays as the historical record), but worth a future, explicit erratum on B2-EVAL-001 itself if that document's own memory figures are ever relied upon for a precise claim.

The reviewer separately confirmed (no changes needed): the external repository/commit pair matches every prior evaluation on this corpus exactly; ~20 spot-checked numbers across Sections A, B, C, F, G trace exactly to the two preserved raw-output files; Section A/C's denominators are explicitly and honestly hedged (not presented as more certain than they are); the Q4 regression (Section E) is given full, unminimized treatment rather than being cherry-picked around; Section B's precision/recall/F1 honesty holds (no global F1 computed, and the `StaticAnalysis`-count footnote is unambiguous); the CRC-001 challenge-set entry is grounded in the same evidence as Section E; and the GO/NO-GO conclusion (Section J) follows from the evidence actually presented, with the residual-limitations section stating plainly what this evaluation does not establish.

## J. GO/NO-GO to multi-codebase validation

**GO.** Task 9.5's changes hold up under re-evaluation against the exact same real corpus used to originally motivate them: Field/Contains/Instantiates now provide real, previously-impossible structural coverage; the two confirmed B2-ID-004 false positives (Q3, Q6) are gone; the Imports redesign moved from 0 real captured facts to 691, with resolution correctly never inventing an internal target. The one genuine tradeoff found (Q4's `RNNModel` recall loss) is exactly the kind of result the Evidence-before-claims Experimental Policy exists to surface honestly, not grounds to block progress — it is now a tracked, reusable challenge-set case for the next resolution-technique ablation round. Task 10 remains **NOT authorized** by this evaluation; that decision is the user's.

## Residual limitations of this evaluation itself

- Single codebase, single domain (ML/Python + some C++), same as B2-EVAL-001 — this evaluation does not by itself satisfy the multi-domain validation requirement; it only confirms Task 9.5's changes did not regress THIS corpus's own previously-measured baseline.
- No global precision/recall/F1 is computed for `Calls` resolution (see Section B) — only sample-based and targeted-case evidence exists.
- Import resolution false negatives were not separately manually sampled in this checkpoint (Section C) — a candidate addition for a future, dedicated Imports-focused evaluation.
- Performance numbers are single-run, not multi-run averages (Section G) — sufficient to confirm no regression, not benchmark-grade.
- The challenge-set mechanism introduced here (Section E) is new; its own long-term maintenance process is not yet formalized beyond "add real cases found during evaluations."
