# B2-EVAL-008: multi-codebase validation, C#/C++ domain

**Evaluation ID:** B2-EVAL-008
**Status:** **COMPLETE.** This is the full, final measurement pass —
the first real stress-test of the Semantic Reliability Model built in
Quality & Semantic Reliability Hardening Phase 1/1.1/2 (all CLOSED,
independently reviewed, committed on this branch before this resumption).
B2-ID-009 (Critical), B2-ID-010 (Important), and B2-ID-011 (Important)
are all CLOSED — fixed, tested, mutation-verified, independently
reviewed, committed. No new Critical/Important production defect was
found during this resumption. B2-EVAL-009 is NOT auto-started. Task 10
remains NOT authorized. No merge, tag, or push.
**Date:** 2026-11-11/12 (spans the B2-ID-009/010/011 fix sequence,
Hardening Phase 1/1.1/2, and this final resumption, all in one
continuous session).
**SuperBrain commit:** branch `b2-code-graph`. Fix commits:
`c09c34d` (B2-ID-009), `2001661`+`cc1878f` (B2-ID-010), `21382ef`+`c037b08`
(B2-ID-011). Hardening commits: `c06b72a`,`7bfcc5b`,`e9d1448`,`031d134`,`9cd9b4f`
(Phase 1), `a19e92f`,`0d631e6`,`87aab4b`,`06c9426`,`f64d653`,`60d75c7`,`94fb27a`
(Phase 1.1+2). This resumption's own commits: `72f2020` (harness wired
to `resolve_calls_with_negative_knowledge`), `694fbf1` (Roslyn
differential suite extended with real Polly files), `9628b61` (30-row
ground-truth sample for Calls).
**External repository:** `App-vNext/Polly`
(https://github.com/App-vNext/Polly), BSD-3-Clause license, 3042
commits, 415 `.cs` files. Chosen per the user's explicit C#/C++
domain requirements: small/medium, real, auditable, class-centric,
with real inheritance (`BulkheadPolicy : Policy, IBulkheadPolicy`),
real interface-to-interface inheritance (`IBulkheadPolicy : IsPolicy,
IDisposable`), and real method overloads
(`Handle<TException>()`/`Handle<TException>(Func<TException,bool>)`,
duplicated across non-generic and generic variants, and — a NEW
finding this resumption surfaced — ALSO duplicated on a second,
genuinely unrelated class). C# was preferred over C++ per the
protocol's own stated preference; C++ was not separately evaluated
this round.
**External commits:** commit A `32064ed55ab47e05b590be0360c8c45ce3901a73`
("Lint PowerShell scripts #2504"), commit B
`1f232caf9672e515751ba2df76d82bfbbb62e016` ("Improve Polly coverage
#2505" — 41 files changed, 886 insertions(+), 57 deletions(-),
confirmed via `git diff --stat` directly against the clone during an
earlier independent review).
**Configuration:** the frozen baseline configuration, PLUS all three
committed fixes (B2-ID-009/010/011) and the Phase 1/1.1/2 reliability
layer. No other algorithm change.
**Methodology:** drove the real production pipeline via
`crates/codegraph-index/examples/b2_smoke_test_csharp_cpp.rs`, EXTENDED
this resumption to call `resolve_calls_with_negative_knowledge` instead
of the plain `resolve_calls` wrapper (byte-for-byte identical `Edge`
output, confirmed by that function's own dedicated equivalence test —
every pre-existing section of this harness that reads `calls_edges` is
therefore unaffected), with new Sections R/R1/R2/R3 measuring
`EvidenceTier`/`Provenance`/`CallAttempt`/agent-policy data the prior
pass could not. **This document's own numbers are from ONE fresh,
final run**, executed after all three fixes and the full reliability
layer landed. Raw output preserved at
`docs/research/evidence/b2-eval-008-polly-run-RELIABILITY-FINAL.txt`.

## The core shift this resumption makes

Every prior pass of this evaluation measured `edge present / edge
absent`. This pass additionally measures, wherever available:
`EvidenceTier`, `Provenance` (resolver, candidate count, rationale,
method limitations), `CallAttempt` status (`Ambiguous`/`Unresolved`),
and whether the executable agent-reliability policy (`agent_policy.rs`)
produces the correct verdict on real data. The two questions this
whole resumption is organized around:

1. **When the graph does NOT find a relationship, can it correctly
   explain why?** — Answer, grounded in real measurement below:
   **partially**. `CallAttempt`'s own `method_limitations` string is
   currently a single CANNED explanation for every `Ambiguous`
   outcome ("name-based matching cannot distinguish overloads by
   parameter list or generic arity"), applied identically whether the
   real cause is a genuine overload family, an unrelated same-named
   method in a different class, or (new this pass) a same-named
   method on a completely unrelated class across module boundaries.
   The TIER is always correct (`Ambiguous` genuinely means 2+ real
   candidates survived); the EXPLANATION is not yet differentiated
   enough to answer "why specifically" beyond "name collision."
   Documented as a real, measured limitation — not fixed here.
2. **When the graph DOES produce a relationship, is the assigned tier
   consistent with the real evidence?** — Answer: **yes, on every
   case checked**. The reliability-calibration sample below (Section
   M) found zero cases where a `Resolved`/`Heuristic` tier's own
   prediction was wrong, and the `Ambiguous`/`Unresolved` cases
   checked were all genuinely ambiguous/genuinely external on direct
   source inspection.

## Contamination-reset ledger (cumulative across all three fixes, re-confirmed on this final fresh run)

| Metric | Pre-fix value | Invalidated by | Final value (this run) | Delta | Conclusion |
|---|---|---|---|---|---|
| `Handle` snapshots (Section C2) | 2 | B2-ID-009 | **4** (named; 6 total pool-wide, see below) | +2 | Changed: extraction now correct |
| `calls_edges` (whole corpus) | 1658 | B2-ID-009, then B2-ID-011 | **1928** | +270 total (+224 from B2-ID-009, +46 from B2-ID-011) | Changed: real calls recovered at TWO separate layers |
| `extends_implements_edges` | 65 | B2-ID-009, then B2-ID-010 | **103** | +38 total | Changed: `BulkheadPolicy -> Policy` and 11 other real classes now resolve |
| `by_kind Extends` | 45 | same | **81** | +36 | Changed |
| `by_kind Implements` | 20 | B2-ID-009 only | **22** | +2 | Held since B2-ID-010 (not partial-fragmented) |
| `Calls` edges targeting any `Handle` snapshot | 0 | B2-ID-009 (naming), then B2-ID-011 (extraction) | **1** | +1 | Changed: the invisibility layer is gone; the overload-ambiguity layer is now the ONLY remaining reason it is not higher |
| `snapshots_promoted`/`Method` count | 5190 / 4521 | none (names/edges only) | **5190 / 4521** | 0 | Held — confirms every fix changes names/edges, never snapshot identity or count |

**Methodological correction this resumption makes to its own prior
pass**: the earlier B2-ID-011 closure measured "6 `Handle`-named
snapshots" without identifying WHERE the 2 non-`Policy` ones came
from. This resumption traced them directly: `src/Polly.Core/PredicateBuilder.TResult.cs`
declares its OWN `PredicateBuilder<TResult>.Handle<TException>()` pair
— a genuinely separate class, unrelated to `Policy`, that happens to
share the bare name. This is NEW, stronger evidence for the overload
question (the ambiguity pool spans module/class boundaries, not just
within-one-class overloads) and is reflected in Section D below.

## A. Corpus metrics (final run, fresh measurement, unchanged from every prior fix)

```
total_files_in_tree: 1015
files_supported_by_parser: 792
files_parsed: 792
parse_failures: 0
files_with_tree_has_error: 4
raw_candidates_extracted: 5190
extraction_rejections (SignatureTooLong): 213
snapshots_promoted: 5190
promotion_failures: 0
```

Symbol coverage: `Class: 634`, `Interface: 35`, `Method: 4521`.
Extraction-coverage gaps (raw-parse proxy, unchanged by any fix, still
real and unaddressed): `struct_declaration` 46, `enum_declaration` 15,
`constructor_declaration` 379, `field_declaration` 730 — none walked
by `extract/csharp.rs` today. `csharp_using_directives: 725`,
`csharp_namespace_decls: 781`.

## B. Corrected method extraction (B2-ID-009 effect, re-confirmed)

4 snapshots named exactly `"Handle"` in `Policy.HandleSyntax.cs` —
matching real ground truth exactly. Full before/after scale (937
methods collapsed under the bogus name `"Task"`, 1535/4543 total
methods misnamed pre-fix) is in `B2-ID-009.md`, not re-derived here.

## C. Inheritance/interface quality (B2-ID-010 effect, re-confirmed, now with Roslyn cross-check)

Final run, 15 edges touching `BulkheadPolicy`/`IBulkheadPolicy`/`Policy`
(full list preserved in the raw evidence file). `extends_implements_edges: 103`,
`Extends: 81`, `Implements: 22` — exactly reconfirming B2-ID-010's own
closure numbers on this fresh run, independent of the two newer fixes.

**Partial-class reconciliation, reconfirmed directly against the real
corpus:**
- `BulkheadPolicy -> Policy` resolves (`Policy` has 8 partial
  fragments; the edge points to the lowest-span-start fragment,
  `Policy.ContextAndKeys.cs`).
- 11 other real classes extending `Policy` also resolve (full list in
  the raw evidence file).
- `BulkheadPolicy -> IBulkheadPolicy` (`Implements`) correctly STILL
  does not resolve — the separate generic-arity-collision gap.

**NEW this resumption — real Roslyn differential confirmation**
(`docs/research/evidence/b2-eval-008-roslyn-differential-output.jsonl`,
produced by running the real Roslyn C# compiler/semantic-model
against the actual, UNMINIMIZED Polly source files for
`Policy`/`IBulkheadPolicy`/`BulkheadPolicy`/`IsPolicy`, compiled
together as one real compilation unit):

```
BulkheadPolicy (arity 0): semantic_base_type="Polly.Policy", semantic_interfaces=["Polly.Bulkhead.IBulkheadPolicy"]
IBulkheadPolicy<TResult> (arity 1): semantic_interfaces=["Polly.Bulkhead.IBulkheadPolicy"]
IBulkheadPolicy (arity 0): semantic_interfaces=["Polly.IsPolicy","IDisposable"]
```

Roslyn's OWN real semantic model — not this project's own resolver —
resolves `BulkheadPolicy`'s `IBulkheadPolicy` reference UNAMBIGUOUSLY
to the arity-0 interface, and resolves `IBulkheadPolicy<TResult>`'s
own base reference equally unambiguously to the SAME arity-0
interface. **This is concrete, cross-oracle evidence that the
generic-arity-collision gap is genuinely recoverable with
arity-aware disambiguation alone — it is not a case requiring full
type inference to resolve**, strengthening (not just asserting) the
recommendation already floated in the prior pass.

### Generic-arity gap, re-confirmed and now cross-oracle-validated

The 59-distinct-bare-name / ~228-raw-text-proxy-reference figures from
the prior pass are unchanged by this resumption (neither B2-ID-011 nor
the reliability layer touches `resolve_extends_implements_with_csharp_contexts`).
What changes: the Roslyn differential above is now DIRECT evidence
(not inference from how the resolver's own code is written) that
arity information alone recovers the `IBulkheadPolicy` case
specifically, and by the same structural argument (a C# base clause's
own bare name never carries `<T>`) very likely most of the other 58
collisions too — though this resumption did not individually Roslyn-
check all 59, which is named as real future work, not claimed as done.

## D. Overload evidence — the generic-call-invisibility layer (B2-ID-011) vs. the overload-ambiguity layer, now CLEANLY separated

Ground truth, re-verified by direct source reading this resumption:
**6 real snapshots named exactly `"Handle"` exist in the whole corpus**
— not 4. `Policy.HandleSyntax.cs` declares the 4 originally identified
(`Handle<TException>()`/`Handle<TException>(Func<...>)` on both
`Policy` and `Policy<TResult>`), and `src/Polly.Core/PredicateBuilder.TResult.cs`
separately declares its OWN `PredicateBuilder<TResult>.Handle<TException>()`
pair — a genuinely unrelated class. `resolve_calls`'s own candidate
pool for the bare name `"Handle"` therefore spans module/namespace
boundaries, not just within-one-class overloads.

**The two layers, measured separately for the first time**:

| Layer | Measurement | Value |
|---|---|---|
| **Syntax visibility** (B2-ID-011's own fix) | Generic method calls now reach candidate status at all | Confirmed: 846 distinct `(caller, "Handle")` `CallAttempt`s produced, where before the fix this count was 0 — every real `Handle<TException>()` call site was silently dropped at extraction |
| **Candidate discovery** | Real candidates found for a `Handle`-named call site | Every one of the 846 attempts found **exactly 6** real candidates — the full pool-wide `Handle` name bucket, confirmed uniformly across the sample (no mixed outcomes) |
| **Overload/generic-arity ambiguity** | Resolution outcome given 6 real candidates | **Ambiguous** for 846 of 847 occurrences; exactly **1** resolves via `NameHeuristic` (scoped to the caller's own file, where only one `Handle`-named candidate happened to survive that specific scope-narrowing step) |
| **Resolved edges** | Real `Calls` edges targeting any `Handle` snapshot | **1**, via `NameHeuristic`, never `StaticAnalysis` |

**Raw-occurrence-count discrepancy, measured and reported honestly,
not silently reconciled**: B2-ID-011's own closing report cited "931
real generic `Handle` call sites... within caller-eligible
signatures" via a dedicated raw-parse proxy; this resumption's own
`CallAttempt` count (deduplicated per `(caller, callee_name)`, per
`resolve_calls`'s own occurrence-vs-relation discipline — B2-ID-002)
is 846 + 1 resolved = 847. A third, even cruder raw-text `grep` count
of literal `.Handle<`/`Handle<` text across the whole corpus (including
the 6 declarations themselves and any comment/string match) gives 981.
These three numbers measure three DIFFERENT things (raw syntactic
text occurrences vs. deduplicated-per-caller resolution attempts vs.
"within caller-eligible signatures," whose exact scoping this
resumption did not re-derive from B2-ID-011's own probe code) — the
847 figure is the one actually produced by the real production
resolver path and is the authoritative one for THIS evaluation's own
claims; the other two are preserved as-measured for transparency, not
silently dropped or forced to reconcile.

**Research question answered**: per-name resolution is confirmed
INSUFFICIENT for this domain, independent of any extraction gap — 6
real, independently-declared overloads sharing one bare name, 0 of
which are distinguishable by `resolve_calls`'s own pure-name matching.
This is now backed by a CLEAN measurement (the extraction layer is no
longer confounding the result, per B2-ID-011's own fix), not the
qualitative-only conclusion the prior pass had to settle for.

## E. Calls quality — full manual sample completed this resumption, with real computed metrics

A 30-row, source-verified, machine-readable ground-truth sample
(`crates/codegraph-index/tests/ground_truth_fixtures/b2-eval-008-calls-manual-sample.jsonl`,
schema per `docs/research/GROUND_TRUTH_SCHEMA.md`, status computed via
the real `ground_truth.rs` module, never hand-assigned) — the FIRST
methodologically valid precision/recall this evaluation series has
ever produced for this domain:

```
true_positives: 21
false_positives: 0
false_negatives: 2
ambiguous_correct: 1
unresolved_trivial: 6
precision: 1.0
recall: 0.913
f1: 0.955
```

**Taxonomy, per the user's own explicit classification scheme, applied
to this real sample**:

- **Correctly resolved** (21 rows): target correct, evidence tier
  consistent — 11 `StaticAnalysis`/`Resolved` (`AcceptOutcome`,
  `ActionCore`, `ActionGenerator`, plus 4 `Extends`/`Implements`
  cases), 7 `NameHeuristic`/`Heuristic` (scope-narrowed to the
  correct enclosing class or file, confirmed by direct source reading
  that a real 2nd same-named candidate existed corpus-wide before
  narrowing — `ActualiseCurrentMetric_NeedsLock` in two unrelated
  classes, `Add` narrowed to file scope), 3 more `Extends` cases from
  the partial-class/generic-arity set.
- **Correctly ambiguous** (1 row): `BulkheadPolicy -> IBulkheadPolicy`
  — 2 syntactically-equally-valid candidates (arity 0 and arity 1),
  the resolver correctly declines, per the schema's own strict
  2-or-more-equally-valid-targets definition of this category.
- **Correctly unresolved** (6 rows): all 6 are genuinely EXTERNAL BCL
  targets, confirmed by direct source reading — `RateLimiter.AcquireAsync`
  (`System.Threading.RateLimiting`), `DateTime.AddDays`,
  `ILoggingBuilder.AddConsole`. None declared anywhere in this corpus;
  zero real candidates is the structurally correct outcome.
- **Incorrectly resolved** (0 rows found in this sample): no FP
  observed.
- **Incorrectly ambiguous** (0 rows found in this sample): every
  `Ambiguous` case checked was genuinely ambiguous (2+ real
  candidates), never a case where exactly one real target existed but
  the resolver failed to distinguish it.
- **Incorrectly unresolved — a real, measured finding**: the 2
  `Handle<TException>` rows score as `FalseNegative` by the ground-
  truth schema's own strict rule, NOT `AmbiguousCorrect` — per the
  schema's own documented table, a row with exactly ONE real intended
  target (which every individual real call site has — a specific
  developer wrote `Policy.Handle<DivideByZeroException>()` meaning
  ONE specific overload) scores as a missed real relationship when the
  resolver's own output is empty, REGARDLESS of whether the empty
  output came from `Unresolved` or `Ambiguous`. **This is an important
  methodological finding in its own right**: the `EvidenceTier::Ambiguous`
  classification is the structurally CORRECT resolver behavior
  (declining to guess among 6 real candidates), but it is still,
  honestly, a missed answer from the ground-truth schema's own
  consumer-facing point of view. The two framings are not in tension —
  they are deliberately answering different questions (resolver-
  behavior-correctness vs. consumer-outcome-completeness) — but
  conflating them would overstate how "healthy" this corpus's own
  Calls quality looks.
- **One extra flagged, not fully root-caused row**: `AddBuilder` on a
  generic `ResiliencePipelineRegistry<T>` scored `Unresolved`, but
  this resumption did NOT confirm whether it is genuinely external or
  a recoverable FN (e.g. a generic-method-declaration coverage gap)
  — flagged honestly as unconfirmed rather than asserted either way,
  per this evaluation's own evidence discipline.

## F. Imports/usings (unaffected by any of the three fixes or the reliability layer)

```
total_using_directives_at_commit_b: 725
csharp_import_facts_extracted: 725
extraction_coverage: 100.0%
resolution_internal: 38, resolution_external: 571, resolution_ambiguous: 116
aliases_captured: 17
```

Unchanged from every prior pass. B2-ID-010's own namespace-awareness
is still NOT fed into Calls/Imports reasoning automatically, per
explicit instruction — that remains a separate, not-yet-made
architectural decision.

## G. SignatureTooLong — cross-domain tracking continues, now with reliability-layer behavior checked

```
total rejections: 213, frequency: 213/5403 (3.94%)
average rejected signature size: 15,867 bytes (max accepted: 4096)
```

Fourth domain confirming this gap (pyjanitor/Python: 53/2822;
pipx/Python: 18/18; Polly/C#: 213/5403).

**NEW this resumption — reliability-layer behavior when a symbol is
lost to `SignatureTooLong`**: a symbol rejected at extraction never
reaches `SymbolSnapshot` at all, so it can never appear as a
`CallAttempt` caller OR as a real candidate for anyone else's call —
it is simply ABSENT from both the positive graph and the negative-
knowledge layer. This means: a caller inside a rejected symbol
produces NO `CallAttempt` (the caller itself does not exist to the
resolver), and a callee that was rejected produces an ordinary
`Unresolved` `CallAttempt` for anyone who calls it — INDISTINGUISHABLE
from a genuinely external/BCL target. **This is a real, confirmed
"absence of evidence looks like evidence of absence" gap in the
CURRENT reliability layer**: `CallAttempt::unresolved`'s own
provenance note says "zero candidates matched this callee name in the
given snapshot pool," which is TRUE but does not distinguish "this
name genuinely does not exist in this codebase" from "this name was
rejected at extraction for being oversized." The reliability layer
does not yet link an extraction rejection to a downstream query
failure — documented here as a real, measured limitation, per this
resumption's own instruction not to redesign the model mid-evaluation.

## H. Callback/registration evidence (B2-EVAL-007 recurrence, reconfirmed with CallAttempt data)

Reconfirmed: `BulkheadPolicy`'s constructor-injected `_onBulkheadRejected`
delegate field, invoked from a different method, is structurally
invisible to `resolve_calls` by name alone. **NEW this resumption**:
checked whether the negative-knowledge layer at least surfaces this as
an informative `Unresolved`/`Ambiguous` `CallAttempt` for the
INVOCATION site inside `BulkheadEngine.Implementation` — it does not,
because `_onBulkheadRejected(...)` is called via a field reference,
not a bare/member-access identifier `call_names_in_tree` recognizes as
a call site AT ALL (the callee syntax itself, `_onBulkheadRejected(context)`,
IS recognized as a plain bare-identifier call — but resolves to
`Unresolved` since no symbol is named `_onBulkheadRejected`, with the
SAME generic "zero candidates matched" provenance note as any other
unresolved call, giving no hint that a constructor-parameter-to-field
registration relationship is the real structural cause). Confirms
B2-EVAL-007's command-registration-graph category is NOT CLI-
framework-specific (two independent real mechanisms — pipx's dynamic-
attribute dispatch table, Polly's constructor-injected delegate
fields — produce the identical structurally-invisible relationship).
No new edge kind implemented, per instruction; the negative-knowledge
layer's own generic provenance note is confirmed to not yet
distinguish this category either, consistent with Section G's own
finding above (the current model's provenance text is not yet
differentiated enough to answer "unresolved, AND HERE IS SPECIFICALLY
WHY" beyond the single tier-level reason).

## I. Module/File evidence (Option 2 tracking continues, unaffected by this resumption's own new work)

```
files_with_at_least_one_extracted_symbol: 722
total_top_level_symbols_across_all_files: 3889
files_with_more_than_one_top_level_symbol: 233 (32.3%)
mean_top_level_symbols_per_file: 5.39
median_top_level_symbols_per_file: 1
```

Unchanged from the prior pass — none of B2-ID-011 or the reliability
layer touch Module/File measurement. **Structural-navigation vs.
semantic-resolution, distinguished explicitly per this resumption's
own instruction**: the Module/File gap (no node represents "this
file's own top-level symbols") is a STRUCTURAL-NAVIGATION problem a
synthetic File/Module node would directly fix. The generic-arity
collision, the overload-ambiguity layer, and B2-ID-011's own
extraction gap are all SEMANTIC-RESOLUTION problems a File/Module node
would NOT fix regardless of implementation — a coding agent asking
"what does this file contain" gets a structural-navigation answer
today only via error-prone file-path string matching (confirmed, see
Section J's own Q4), but asking "which overload does this call
invoke" would remain exactly as unanswered with or without a
File/Module node. Net assessment (unchanged): tentative Option 2
recommendation remains tentative; this domain's own nuance (file role
matters more than language paradigm) holds.

## J. Coding-agent usefulness, now two-dimensional (Answerability + Reliability, per explicit instruction)

| Question | Answerability | Reliability | Notes |
|---|---|---|---|
| "Does BulkheadPolicy extend a base class AND implement an interface?" | **Fully answerable** (Extends) / **Not answerable** (Implements) | Extends: **Safe to state** (`Resolved` tier, `EvidenceTier::Resolved`, agent-policy `evaluate(Resolved, AssertAsFactOrSoleJustification)` = `Permitted`). Implements: **Forbidden to act** (`Ambiguous` tier, policy = `Forbidden` for every intended use) | The `Q1` harness query itself was previously flagged as a nondeterministic-iteration-order artifact in an earlier pass; this resumption's own un-filtered Section C1 query confirms the real edge exists cleanly. |
| "Which classes implement this interface (IBulkheadPolicy)?" | **Fully answerable** for the arity-0 interface via `Implements` edges; the generic variant's own implementers are invisible | **Safe to state** for the resolved half; **Forbidden to act** on any claim about the generic variant without disclosure | Matches the generic-arity gap exactly. |
| "Who calls this method?" / "Which overload gets invoked?" | **Partially answerable** — generic calls ARE now extracted (B2-ID-011), but "which overload" is **Not answerable**: 846/847 real `Handle` call sites resolve to `Ambiguous`, 1 to `Heuristic` | The 1 resolved case: **Safe with caveat** (`Heuristic` tier — policy forbids presenting it as certain fact without disclosure). The 846 `Ambiguous` cases: **Forbidden to act** — the agent-policy's own `Ambiguous` rule is unconditional across every intended use, confirmed directly against this real data in Section K below. | This is the single most concrete real-data demonstration of the Ambiguous policy's own purpose in this whole evaluation series. |
| "Where is this type constructed?" | Not specifically re-tested this pass; `Instantiates` resolution is independent of B2-ID-011/the reliability layer | Not separately assessed | Carried forward unchanged from the prior pass. |
| "What's impacted if I modify this interface/base class?" | For `Policy`: **Fully answerable** (12 real derived classes now visible via `Extends`). For `IBulkheadPolicy`: **Partially answerable** (arity-0 implementer chain visible; arity-1 is not) | For `Policy`: a HIGH-IMPACT-MODIFICATION simulation (Section K) returns `Permitted` only if every one of the 12 edges is independently `Resolved` — confirmed true for this specific case in Section K below. For `IBulkheadPolicy`: the SAME simulation would return `Forbidden` the instant the generic-arity-collision case enters the blast radius. | Directly exercises `evaluate_high_impact_modification` on real data, not a synthetic test. |
| "Which tests cover this class?" | **Not answerable** for `BulkheadPolicy` (`tests_for_edges: 0` corpus-wide — the xUnit `Should_<behavior>` naming-convention gap, Moderate, carried forward unchanged) | **Forbidden to act** by construction (an empty blast radius / zero candidates is never silently `Permitted`, confirmed by `agent_policy.rs`'s own dedicated test for exactly this case) | — |
| "What's the flow between these components?" | `Calls`-dependent, inherits whatever reliability tier each hop in the chain carries | Mixed — would require per-hop evaluation, not attempted as a single aggregate claim in this pass | Flagged as real future work for a genuine multi-hop blast-radius tool, not built here. |

## K. Agent-reliability-policy simulation on real evaluation data (NEW this resumption)

Applying `agent_policy::evaluate`/`evaluate_high_impact_modification`
directly to real `CallAttempt`/edge data from this corpus (not
synthetic test fixtures) — raw output in Section R3 of the evidence
file:

```
callee_name="Action" tier=Ambiguous informational_display=Forbidden high_impact_modification=Forbidden
callee_name="ActionAsync" tier=Ambiguous informational_display=Forbidden high_impact_modification=Forbidden
[...all 10 sampled real Ambiguous CallAttempts: identically Forbidden for both intended uses...]
```

**Confirms, on real data, the exact invariant this policy exists to
protect**: every single one of the 8,465 real `Ambiguous` `CallAttempt`s
in this corpus (the full count, not just the 10-row sample) would be
blocked from being presented as a certain fact OR used as sole
justification for a high-impact modification, with ZERO exceptions —
the policy's own `(EvidenceTier::Ambiguous, _) => Forbidden` match arm
is unconditional across `IntendedUse`, confirmed structurally (not
just by the 10-row sample) since the match has no other reachable
arm for that tier.

**High-impact-modification simulation on the `Policy` blast radius**:
all 12 real `Extends` edges pointing at `Policy` are individually
`Resolved`-tier (confirmed in Section C's own edge list) — so
`evaluate_high_impact_modification` on that exact 12-edge set returns
`Permitted`. Changing even ONE of those 12 relationships to
`Heuristic`/`Ambiguous`/`Unresolved` (e.g. if a 13th caller existed
whose own target was ambiguous) would flip the WHOLE modification to
`Forbidden`, per the policy's own "any non-Resolved tier blocks
everything" rule — confirmed by direct code reading, not re-derived
from scratch via a new test in this pass (the existing
`a_single_heuristic_edge_among_resolved_ones_blocks_the_whole_modification`
test in `agent_policy.rs` already proves this synthetically; this
section's own contribution is confirming the REAL `Policy` blast
radius happens to be the all-`Resolved` case that make this concretely
checkable on real data).

## L. Negative-knowledge quality (NEW this resumption)

Direct inspection of 15 real `Ambiguous` `CallAttempt`s (Section R1 of
the evidence file) and the `Handle` case specifically:

- **Candidate sets are genuinely informative and correct**: every
  sampled attempt's own `candidates` list, cross-checked against real
  source, names the actual real same-named symbols — e.g. the `Action`
  callee's 3 candidates are confirmed (by direct source reading) to be
  2 genuinely distinct `Action` methods in `Fallback.cs` plus 1 in
  `Workloads.cs`, not a duplicate-snapshot artifact.
- **`method_limitations` is a single canned string, not yet
  differentiated by real cause** — confirmed as a real, measured
  limitation (not asserted from reading the code alone): EVERY
  sampled `Ambiguous` attempt, regardless of whether the real cause is
  a genuine overload family (`Handle`), an unrelated same-named method
  in a sibling benchmark file (`Action`/`ActionAsync`), or a
  cross-class/cross-module collision (`Handle` spanning `Policy` and
  `PredicateBuilder<TResult>`), carries the IDENTICAL limitations text:
  `"name-based matching cannot distinguish overloads by parameter list
  or generic arity; 2+ same-named candidates remained even after scope
  narrowing"`. This is TRUE in every case checked, but it does not
  distinguish "this IS a real overload family" from "this is an
  unrelated same-named symbol" — a consumer reading only the
  `method_limitations` string cannot tell these apart, even though the
  `candidates` list itself (which IS differentiated, per the point
  above) would let a more sophisticated consumer infer it by
  inspecting the candidates' own file paths/signatures.
- **`Unresolved` provenance has the identical limitation**, confirmed
  in Section G above: `"zero candidates matched this callee name in
  the given pool"` is equally true for a genuine BCL external call
  and for a `SignatureTooLong`-rejected in-corpus symbol — the
  reliability layer does not yet distinguish these.

## M. Reliability-tier calibration (NEW this resumption)

Direct, source-verified check of a stratified sample across all 4
tiers — the real 30-row ground-truth sample IS this calibration check
(Section E above), summarized here against the tier-level question
specifically:

| Tier | Sample size | Confirmed correct | FP risk observed |
|---|---|---|---|
| `Resolved` | 11 | 11/11 (100%) | None — every `Resolved` prediction matched real source |
| `Heuristic` | 7 | 7/7 (100%) | None in this sample, but see the structural FP-risk note on the `ActionGenerator` row (a coincidental cross-module same-name match that happens to be correct today, flagged as a real risk pattern for a FUTURE corpus change, not a current defect) |
| `Ambiguous` | 1 (ground-truth-row) + 846 (`Handle` population, uniformly checked) | 847/847 genuinely ambiguous (2+ real candidates in every case checked) | None — zero cases found where exactly one real target existed but the tier said `Ambiguous` |
| `Unresolved` | 6 | 6/6 confirmed genuinely external (zero real candidates in-corpus) | None confirmed; 1 additional case (`AddBuilder`) left honestly unconfirmed rather than asserted either way |

**No tier-assignment error found in any case directly checked.** The
honest caveat: this is a 30-row MANUAL sample plus one large
uniformly-consistent population (`Handle`'s own 846 attempts, which
however are NOT independent observations — they are all instances of
the SAME underlying 6-candidate ambiguity, so they constitute strong
evidence for ONE case, not 846 independent data points). A broader,
independently-sampled stratified check across many DIFFERENT ambiguous
names (not just `Handle`/`Action`/`ActionAsync`) is named as real
future work, not claimed as exhaustive here.

## N. Full/incremental reliability parity — a confirmed, honest gap

**Snapshots/edges/structural facts**: full parity reconfirmed on this
run exactly as the prior pass found —
`raw_csharp_type_declaration_contexts_extracted: 49` +
`raw_csharp_type_declaration_contexts_reused: 811` = 860, matching the
reuse-on-untouched/always-reresolve convention `raw_import_statements`
already use.

**`CallAttempt`/`EvidenceTier`/negative-knowledge parity — NOT
measurable in this pass, for a confirmed, structural reason, not an
oversight**: `resolve_calls_with_negative_knowledge` is wired into
THIS HARNESS directly, but is **NOT** wired into the real
`CodeGraphIndexer`/`incremental::pipeline::index_files` production
path at all — `resolve_all_typed_edges` (the function the real
incremental/full-reindex pipeline actually calls) still uses the
plain `resolve_calls`, confirmed by direct code reading
(`crates/codegraph-index/src/edges/mod.rs`). This means **there is
currently no incremental `CallAttempt` output to compare against a
full-reindex `CallAttempt` output AT ALL** — Hardening Phase 2's own
negative-knowledge work, per its own documented scope ("wired into
`resolve_calls` only... `resolve_extends_implements`/`resolve_instantiates`/
`resolve_tests_for` are NOT wired in this pass"), was never extended to
the incremental indexer itself either. This is a REAL, previously-
unstated residual gap this resumption surfaces for the first time —
flagged here explicitly as a finding for a future Hardening Phase, NOT
fixed in this evaluation (which only measures), and NOT escalated to
a new finding ID since it is a documented scope boundary of already-
closed Phase 2 work, not a new defect.

## O. Performance

```
full_index_elapsed: 3.95s
observe_commit(A) (full reindex): 3.89s
observe_commit(B) (incremental): 6.21s
nodes (final): 5875 snapshots
edges (final): 3265
ImportFacts: 725
CSharpTypeDeclarationContexts (total, reused+fresh): 860
total_call_attempts (negative knowledge, this harness's own direct call, NOT part of the production incremental path per Section N above): 16,635 (Ambiguous: 8,465; Unresolved: 8,170)
```

**Incremental-slower-than-full-reindex, now confirmed a THIRD time**
across the pre-B2-ID-011 pass, the post-B2-ID-011 pass, and this
reliability-layer pass (6.2s incremental vs. 3.9s full, consistently,
for the same 41-file commit pair) — strengthening that this is a real,
reproducible characteristic of this specific commit pair (not run-to-
run noise), still not root-caused, still lower priority than the
findings this evaluation closed.

**Reliability-layer overhead**: the `CallAttempt` computation itself
adds negligible measured wall-clock (the harness's own full-index
elapsed, 3.95s, is within normal run-to-run variance of the pre-
reliability-layer figure of 3.93s from the prior pass) — `resolve_calls_with_negative_knowledge`
reuses the identical scope-narrowing/candidate-lookup logic
`resolve_calls` always ran, adding only the bookkeeping to RECORD the
already-computed ambiguous/unresolved outcome rather than discarding
it, confirmed by direct code reading (`calls.rs`'s own module doc
comment states this explicitly and this resumption's own measurement
does not contradict it).

## New findings opened this evaluation (cumulative, all now CLOSED except where noted)

- **[B2-ID-009](../findings/B2-ID-009.md)** (Critical) — CLOSED.
- **[B2-ID-010](../findings/B2-ID-010.md)** (Important) — CLOSED.
- **[B2-ID-011](../findings/B2-ID-011.md)** (Important) — CLOSED.
- **No new Critical/Important finding this resumption.**
- **Generic-arity name collision** (Moderate, carried forward) — now
  cross-oracle-confirmed via real Roslyn semantic-model output (Section
  C above), strengthening the case for it as the first concrete
  candidate for a future narrow fix.
- **Negative-knowledge provenance is not yet differentiated by real
  cause** (Moderate, NEW this resumption, documented in Sections G/H/L
  above) — `CallAttempt`'s own `method_limitations`/provenance text is
  currently one canned string per tier, not yet distinguishing
  "genuine overload family" from "unrelated cross-module collision"
  from "SignatureTooLong-rejected symbol" from "genuine BCL external
  target." Real, measured, not escalated to its own ID per this
  evaluation's own convention of keeping Moderate findings in the
  backlog unless the user elevates them.
- **Negative-knowledge/`CallAttempt` is not wired into the real
  incremental indexer** (Moderate, NEW this resumption, Section N) —
  a confirmed scope boundary of already-closed Phase 2 work, flagged
  as real future work for a future Hardening phase, not a defect in
  anything already shipped.
- **xUnit `TestsFor` naming-convention mismatch** (Moderate, carried
  forward unchanged).
- **C# extraction-coverage gaps** (struct/enum/constructor/field)
  (Moderate, carried forward unchanged).
- **Incremental-slower-than-full-reindex** on this specific commit
  pair (open question, now confirmed reproducible a third time, still
  not root-caused).
- **`AddBuilder` on `ResiliencePipelineRegistry<T>`** (Section E) —
  an honestly-unconfirmed `Unresolved` case; may be a genuine BCL-style
  external target or a recoverable FN from generic-method-declaration
  coverage; flagged for future root-causing, not asserted either way.

## Challenge-set cases

**None opened this resumption.** The two layers this pass cleanly
separated (B2-ID-011's own extraction gap, already its own finding;
the overload-ambiguity layer, already qualitatively known since
B2-EVAL-004) do not constitute a NEW resolution-layer ambiguity pattern
distinct from what CRC-001 (B2-EVAL-004) already names — this
resumption STRENGTHENS the evidence behind that existing category
rather than discovering a new one.

## Independent review

Dispatched for this resumption's own new claims specifically (Calls
ground truth, tier calibration, provenance correctness, negative-
knowledge correctness, the Roslyn differential, overload/generic-arity
evidence, SignatureTooLong reliability-layer behavior, agent-policy
simulation, full/incremental reliability parity, arithmetic/raw-
evidence consistency, cleanup) — see the separate review response
delivered alongside this report for the full verdict and any
corrections applied.

## GO/NO-GO to next domain

**GO**, pending the independent review's own verdict (delivered
separately). All three findings this evaluation opened (B2-ID-009/010/011)
are CLOSED, fixed, tested, mutation-verified, and independently
reviewed. No new Critical/Important defect was found during this
resumption. Per the user's own standing protocol, B2-EVAL-009 is NOT
auto-started regardless of this GO status — the next domain requires
the user's own explicit authorization.

## Evidence for/against an A4/A5 experimental checkpoint, and the arity-aware/A4/A5 recommendation

**Substantially strengthened this resumption, with a clear, evidence-
grounded recommendation on sequencing**:

1. **Overload evidence (now CLEAN, not confounded by extraction gaps)**:
   6 real `Handle` overloads, spanning TWO unrelated classes, share one
   bare name; `resolve_calls`'s pure-name matching resolves exactly 1
   of 847 real call sites (the rest correctly `Ambiguous`). Per-name
   resolution is confirmed insufficient for this domain.
2. **Generic-arity evidence, now CROSS-ORACLE CONFIRMED**: Roslyn's own
   real semantic model resolves the `IBulkheadPolicy` generic-arity
   collision unambiguously using only arity information (no type
   inference needed for this specific relationship) — direct evidence,
   not inference, that a narrow arity-aware fix would recover real
   value cheaply.
3. **The reliability layer itself is now stress-tested on real data**:
   `EvidenceTier`/`Provenance`/`CallAttempt`/agent-policy all behaved
   correctly on every case directly checked (Sections K/L/M) — this
   evaluation found ZERO tier-assignment errors, strengthening
   confidence that any FUTURE richer resolution (arity-aware, or
   eventually A4/A5) can be built ON TOP of this reliability
   infrastructure rather than needing to reinvent it.

**Recommendation, sequenced**:

1. **First**: a narrow, arity-aware base-clause/call-target
   disambiguation step (reusing `CSharpTypeDeclarationContext`'s
   already-extracted `generic_arity` field) — the generic-arity
   collision is now the single most concretely evidenced,
   cheapest-to-fix gap in this whole domain, with direct Roslyn
   confirmation that arity alone suffices for at least one real case.
2. **Then**: re-evaluate whether the overload-ambiguity layer (6
   `Handle` candidates, 0 distinguishable by arity alone since all 6
   share the SAME arity shape within their own non-generic/generic
   pairs) still requires full CALL-RES-A5 type-aware resolution, or
   whether a cheaper heuristic (e.g. receiver-type-aware narrowing,
   since `Policy.Handle<T>()` and `PredicateBuilder<TResult>.Handle<T>()`
   have different, inferable receiver types even without full generic
   instantiation) recovers meaningful additional value first.
3. **A4** (import/visibility-aware resolution) was not directly
   stress-tested this pass — no new evidence for or against it beyond
   what prior domains already established.

Neither A4 nor A5 is recommended for implementation yet; this
resumption's own evidence argues for the narrower arity-aware step
FIRST, as a concrete, well-evidenced, low-risk next increment — not a
decision this evaluation itself is authorized to execute.
