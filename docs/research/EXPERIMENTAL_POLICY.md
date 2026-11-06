# Evidence-before-claims Experimental Policy

Companion to [`POLICY.md`](POLICY.md) (the Research-grade Findings
Policy) and [`EVALUATION_POLICY.md`](EVALUATION_POLICY.md) (the
Evaluation & Paper Evidence Policy). Binding from the moment it is
recorded, for every future session working on B2 — explicitly including
Codex, not only Claude.

## Principle

> Claims in the final paper must be supported by reproducible evidence
> whenever they are empirically testable.

Every significant architectural choice in the Code Graph / static-
analysis pipeline is an **experimental hypothesis**, not a decision that
is correct merely because it is theoretically plausible or because a
paper recommends it. "We prefer precision" is not, by itself, a result —
it is a choice that must be measured.

Always distinguish, explicitly, between:

- a **literature-supported principle** (a paper or textbook result);
- a **project hypothesis** (an untested expectation specific to this
  codebase);
- an **experimentally measured result** (this project ran it and has
  numbers);
- an **inference** (reasoned from measured results, but not itself
  directly measured);
- an **unverified future hypothesis** (plausible, not yet attempted).

If the literature suggests a technique but this project cannot verify
its effect in this context, it must NOT be presented in the paper as
this project's own result — cite it as literature support, not as a
finding.

## Required metrics for any call/reference-resolution policy change

Every change to how the pipeline resolves a relationship (calls,
imports, extends/implements, instantiates, etc.) must be evaluated
against at least:

- precision
- recall
- F1 (when methodologically valid — see "Ground truth" below)
- false positives
- false negatives
- unresolved rate (split further — see "Unresolved vs. ambiguous")
- ambiguity rate
- graph fan-out (edges per node, edges per common name)
- runtime
- memory
- graph size (nodes, edges)
- coding-agent usefulness (the Q&A-style evaluation this project already
  uses in B2-EVAL-001 and successors)

## Unresolved vs. ambiguous (recorded separately, never merged)

"No edge was produced" has at least two distinct causes, and they must
never be collapsed into one bucket:

- **unresolved because no candidate exists** — the callee name matches
  nothing in the given pool (e.g. an external library call);
- **unresolved because ambiguity remains** — one or more candidates
  exist, but more than one remains indistinguishable even after every
  scoping/evidence signal available has been applied.

Conflating these hides a real signal: a high "ambiguous" rate on a
common name is actionable (it motivates scope/receiver/import-aware
resolution); a high "no candidate" rate usually just reflects how much
of the dependency graph is outside the indexed corpus.

## Progressive call-resolution configurations

A stable history of named, ID-tracked configurations is maintained so
that no evaluation result is ever compared against an untracked or
ambiguous baseline. IDs are permanent once assigned (names may be
adjusted for clarity; IDs are not reused and are not renumbered):

| ID | Name | Description |
|---|---|---|
| CALL-RES-A0 | name-only, corpus-wide | Baseline: any same-named snapshot in the given pool is a candidate, no scope/receiver/import awareness. One Hypothesized edge per ambiguous candidate (original Task 7 design). |
| CALL-RES-A1 | ambiguity suppression | `>1` indistinguishable candidates at the point of resolution → no edge, instead of N Hypothesized edges. |
| CALL-RES-A2 | scope-aware | Progressive narrowing: same enclosing class/scope → same file/module → repo-wide, before applying A1's ambiguity rule at whichever level is being evaluated. |
| CALL-RES-A3 | receiver-aware | `self`/`this`-qualified receiver-chain calls are restricted to the receiver's own known class membership (Task 9.5 Priority 5(A), Case A) before name matching. |
| CALL-RES-A4 | import/visibility-aware | Candidate pool additionally restricted/expanded using resolved import facts (Task 9.5 Priority 6 and beyond) — not yet implemented. |
| CALL-RES-A5 | type-aware | Uses static or inferred type information. Not yet implemented. |
| CALL-RES-A6 | points-to-assisted | Points-to analysis as a disambiguation signal. Backlog only. |
| CALL-RES-A7 | context-sensitive | Call-context/calling-context sensitivity. Backlog only. |
| CALL-RES-A8 | hybrid static/dynamic | Combines static resolution with observed runtime traces. Backlog only. |

**Task 9.5, Priority 5(A)'s own change** introduces **CALL-RES-A3**
(receiver-aware, Case A) **and CALL-RES-A1+A2 combined** (Case B: scope-
aware narrowing with ambiguity suppression at whichever level remains
ambiguous, rather than A0's unconditional fan-out). The pre-existing
Task 7 behavior (A0) must remain documented, not deleted, so later
evaluations can still reference what changed.

For every configuration that is actually evaluated (not merely
implemented), record and retain:

- exact SuperBrain commit;
- enabled components;
- parameters/thresholds (and explicitly: NO arbitrary magic thresholds —
  every parameter must be traceable to a structural/semantic property,
  never a bare tuned number without justification);
- assumptions;
- evaluation corpus (repository + commit);
- ground truth and how it was constructed (see below);
- precision / recall / F1 / unresolved / ambiguity / FP / FN;
- runtime / memory / graph size;
- coding-agent usefulness.

Earlier configurations' results are never deleted when a newer
configuration is evaluated — superseding is recorded the same way
B2-ID-003 corrected B2-EVAL-001: with an explicit erratum/comparison,
never a silent overwrite.

## Current ambiguity experiment (opened by Task 9.5, Priority 5(A))

The policy adopted for THIS task's implementation —

> multiple indistinguishable candidates at the narrowest applicable
> scope → no edge (not N Hypothesized edges)

— is itself an experimental configuration (CALL-RES-A1+A2+A3), not a
proven-optimal end state. It must eventually be compared against at
least:

- **Configuration A — permissive (≈ CALL-RES-A0):** emit every plausible
  Hypothesized edge, unscoped. Measure precision, recall, F1, fan-out,
  FP/FN, impact-analysis correctness, agent-answer correctness.
- **Configuration B — precision-first (≈ CALL-RES-A1, no scoping):** no
  edge whenever the target remains ambiguous, without first narrowing by
  scope. Measure the same.
- **Configuration C — scoped (≈ CALL-RES-A2/A3, this task's actual
  choice):** apply same-class/scope → same-file/module → imports/
  visibility → receiver evidence, THEN decide. Measure the same.

No configuration is assumed better in advance. The final choice for B2's
closure must be justified by measured data, not by which one "sounds
more correct."

## Decision criterion

The final configuration is never chosen by maximizing precision or
recall alone. Also evaluate the downstream effect on: documentation,
navigation, debugging, impact analysis, architectural understanding,
change planning, test discovery, and coding-agent answer correctness. A
strategy can raise recall while making impact analysis far less useful
(too much noise to act on); another can have very high precision but
recall too low to explain real execution paths. The final choice must
be motivated by data from more than one of these dimensions, not a
single metric in isolation.

## Ground truth

Precision/recall require a defined ground truth. Every evaluation must
document how its ground truth was built — any of:

- manual source inspection;
- compiler/type checker output;
- a trusted IDE/language server's own resolution;
- runtime traces;
- repository tests;
- a documented combination of the above.

If ground truth is only a manual sample, state this explicitly in the
metric itself, e.g. `precision/recall on manually validated sample,
N=20` — never present a sample-derived number as a global metric.

## Static-analysis approaches backlog

Maintained persistently (not implemented merely because it exists on
this list — see "Experimental feature introduction" below):

1. **Name-based resolution** — baseline (CALL-RES-A0).
2. **Lexical/scope-aware resolution** — class, function, file/module,
   qualified names (CALL-RES-A2; Priority 4's qualified-name design is
   a deeper version of this).
3. **Receiver-aware resolution** — for `receiver.method()`, uses
   information about the receiver (CALL-RES-A3; this task's Case A).
4. **Import/visibility-aware resolution** — imports, aliases, module and
   symbol visibility (CALL-RES-A4; Priority 6 and beyond).
5. **Type-aware resolution** — static or inferred type information
   (CALL-RES-A5).
6. **Points-to analysis** — kept explicitly in the backlog (CALL-RES-A6).
7. **Context-sensitive analysis** — call-context sensitivity
   (CALL-RES-A7).
8. **Flow-sensitive analysis** — data-flow and assignment ordering.
9. **Interprocedural analysis** — propagates information across
   function/method boundaries.
10. **Dynamic analysis** — runtime traces as observed evidence. Always
    remember: `observed runtime target != all possible runtime targets`.
11. **Hybrid static + dynamic** — a possible future combination
    (CALL-RES-A8).
12. **IDE/LSP semantic services** — kept as an explicit comparison point,
    not a technique this project itself implements.

For every future evaluation of one of these, record: expected benefit,
language applicability, computational cost, expected effect on
precision/recall, literature support, and experimental result. None of
these is implemented automatically just because the literature
considers it useful — see "Experimental feature introduction."

## SuperBrain vs. IDE/LSP

The paper must keep this distinction clear. An IDE/LSP typically uses a
compiler/type checker, project configuration, a language-specific
semantic model, imports, a symbol table, type inference, and
definitions/references — oriented toward go-to-definition, references,
completion, diagnostics, and navigation.

SuperBrain targets a **persistent, cross-language, queryable Code Graph
+ engineering knowledge system**, for documentation, architectural
reasoning, debugging, impact analysis, cross-repository knowledge,
historical lineage, retrieval, long-term engineering memory, and coding
agents.

Never claim SuperBrain is "better than" an IDE. Compare capabilities and
experimental results instead. When possible, build a shared evaluation
set: `question -> IDE/LSP evidence -> SuperBrain evidence -> ground
truth`.

## Experimental feature introduction

For every new technique, follow this order, and do not skip steps:

1. literature review;
2. hypothesis;
3. measurable expected benefit;
4. baseline;
5. prototype/minimal implementation;
6. evaluation;
7. precision/recall/performance measurement;
8. comparison against the previous configuration;
9. decision.

Never: "the paper says X works → implement X → assume improvement."

## Ablation studies

Maintain the ability to compare: name-only, +scope, +receiver, +imports,
+type, +points-to, +context-sensitivity — so the paper can answer "which
component actually produced the improvement," not only "old system vs.
final system."

## Multi-codebase validation

Every significant configuration is eventually evaluated on the
multi-domain validation suite B2 already requires before closure (ML/DL,
data-analysis, backend/API, frontend/UI, desktop/UI, Rust/systems,
C++/C#, full-stack, CLI/tooling). Results are reported per repository,
per language, and per domain; aggregation across those is only done when
methodologically sound, and the method must be stated.

## Negative results

Negative results are retained, not discarded once a later strategy
performs better. Example: "receiver-aware analysis increased precision
but reduced recall by X" is a valid, keepable result in its own right.

## Raw experimental evidence retention

Aggregated metrics alone are not sufficient. Where practical, retain raw
per-observation data (per call-site/symbol/relationship), not only
rolled-up numbers, so metrics are always recomputable rather than
asserted. For call resolution, retain at least:

- evaluation ID;
- configuration ID;
- repository;
- commit;
- language;
- caller snapshot;
- call-site/source span;
- expected target(s);
- predicted target(s);
- resolution method;
- confidence;
- resolved / heuristic / unresolved / ambiguous;
- TP / FP / FN, when ground truth is available for that observation.

When practical, retain CSV/JSON alongside the Markdown report, to
support reproducibility, metric recomputation, auditing, confusion
matrices, per-language/per-domain analysis, ablation comparison,
detection of harness bugs (as in B2-ID-003), and future statistical
analysis.

Every evaluated configuration retains: configuration ID, exact code
commit, parameters, enabled components, corpus version, ground-truth
version.

## Paper evidence rule

For every important empirical claim in the paper, there must exist a
traceable chain:

```
Claim -> Evaluation ID -> Configuration -> Raw observations -> Aggregation -> Result -> Commit
```

If this chain does not exist for a claim, the claim must be qualified as
a hypothesis/discussion point, or not presented as an experimental
result at all.

## Application to Task 9.5, Priority 5(A)

This task's Case A (receiver-aware filtering) and Case B (scope-aware
ambiguity narrowing) together open **CALL-RES-A1 + A2 + A3** as the
pipeline's current configuration, replacing the Task 7 baseline
(CALL-RES-A0) as what the production pipeline actually does — but NOT
as a proven-final answer. At minimum, the following configurations must
remain comparable once sufficient ground truth exists (future
evaluation work, not this task):

- previous ambiguous fan-out (CALL-RES-A0);
- precision-first ambiguity suppression without scoping (CALL-RES-A1
  alone);
- scope-aware filtering (CALL-RES-A2);
- receiver-aware filtering (CALL-RES-A3).

During the corrected B2-EVAL-001 re-evaluation and the future multi-
codebase validation, measure: precision, recall, F1 (when valid), false
positives, false negatives, unresolved, ambiguity, edge count/fan-out,
execution time, memory, coding-agent usefulness. A smaller edge count
after this change is NOT, by itself, evidence of improvement — it must
be demonstrated against the metrics above, not asserted.
