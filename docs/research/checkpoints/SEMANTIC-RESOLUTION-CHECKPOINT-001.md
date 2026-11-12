# Semantic Resolution & Reliability Checkpoint 001

**Status:** PARTIAL — Priorities 1, 2, 3 implemented, tested, and
committed with real production code changes. Priority 5 investigated
with real measurement evidence but not conclusively resolved. **Priorities
4 and 6, the golden corpus additions, the differential-testing
extension, and the independent review were NOT completed** in this
pass — honestly reported as not done, not glossed over. This document
itself was written by the same implementing pass, not a separate
reviewer; no independent review has yet been dispatched for anything
in this checkpoint.

**Authorized by:** the user's own "Semantic Resolution & Reliability
Checkpoint" directive, following B2-EVAL-008's COMPLETE status and the
closure of the full Hardening Phase 1/1.1/2 work (`EvidenceTier`/
`Provenance`, negative-knowledge `CallAttempt`, `agent_policy.rs`,
`ground_truth.rs`, the exact-multiset `conformance.rs` gate, the
opt-in Roslyn differential suite).

**Branch/worktree:** `b2-code-graph`,
`C:\Users\fsave\Desktop\Python_code\SuperBrain\.worktrees\b2-code-graph`.
No merge, tag, or push performed. B2-EVAL-009 not started. Task 10
untouched throughout.

## Priority 1 — Generic-arity-aware C# call resolution: DONE

**Commits:** `dad6616` (implementation + tests), `d365918`
(mutation-driven simplification fix).

Narrows an ambiguous `Calls` candidate set by the call site's own
generic/template type-argument count, confirmed via direct grammar
probe before writing any code (not assumed):

- C# `generic_name` / C++ `template_function`/`template_method`
  wrapper nodes each have a `type_argument_list`/`template_argument_list`
  direct child whose own non-punctuation children are directly
  countable — the call site's own arity.
- A C# `method_declaration`'s own generic arity is read by re-parsing
  the candidate's `signature()` and counting its
  `type_parameter_list` child — confirmed, via direct probe, that a
  standalone re-parse with no enclosing class produces
  `local_function_statement` rather than `method_declaration` (a
  re-parse artifact, not a real semantic difference), so both node
  kinds are searched for.
- **C++ method-side arity is confirmed NOT recoverable today** — a
  genuinely new, separate finding discovered while implementing this
  priority: a C++ template function (`template<typename T> void
  Handle() {}`) is extracted by `extract/cpp.rs` as a bare
  `function_definition` candidate whose own `signature_text`/span is
  ONLY the inner `function_definition` text; the enclosing
  `template_declaration` and its own `template_parameter_list` are
  OUTSIDE that span entirely, confirmed by direct grammar probe (the
  `template_declaration` is the PARENT, not reachable from a
  standalone re-parse of the promoted fragment). Arity-aware
  narrowing is therefore scoped to C# method-side arity only; C++ is
  documented as a residual limitation, not silently applied as if it
  worked.

Arity-narrowed resolution is classified `NameHeuristic`, never
`StaticAnalysis` — arity narrows a candidate set but does not prove
the same level of certainty bare-name uniqueness does (two real
overloads can share arity while differing only in an ordinary
parameter type — confirmed directly against the real
`Policy.HandleSyntax.cs` shape: `Handle<TException>()` and
`Handle<TException>(Func<TException,bool>)` both have arity 1, and a
dedicated regression test confirms this pair correctly STAYS
`Ambiguous` after arity narrowing, never force-resolved).

One pre-existing B2-ID-011 test asserted "arity-aware disambiguation
is out of scope" (true at THAT finding's own time) — updated, not
regressed, to assert the new, now-authorized behavior instead, with
an inline note explaining the supersession.

**Tests:** 8 new (7 in the dedicated `arity_aware_resolution` module,
covering call-site extraction, method-arity reading, successful
narrowing, the central same-arity-overload negative case, cross-
namespace/ordinary-call/C++-no-regression checks, plus the one
updated B2-ID-011 test).

**Mutation:** `cargo mutants` scoped to `calls.rs`: 149 mutants, 1
genuine-missed from this priority's own new code (`replace > with >=`
in the arity-narrowing guard), confirmed equivalent by construction
(the preceding `candidates.len() == 1` branch already guarantees
`candidates.len()` is `0` or `>= 2` at that point, so `> 1` and `>= 1`
behave identically on every reachable input) and fixed by removing
the redundant check rather than adding an artificial test for an
unreachable distinction. The other mutant found (`delete match arm 0
in narrow_by_scope`) is a PRE-EXISTING, already-documented equivalent
mutant from Task 9.5, confirmed unrelated to this checkpoint. **0
genuine-missed correctness-relevant mutants from this priority's own
code**, after the fix.

**Real-corpus remeasurement:** NOT performed in this pass (the full
`b2_smoke_test_csharp_cpp` harness run against Polly was used for
Priority 5's own investigation, but a dedicated before/after
precision/recall/F1 remeasurement specifically for arity-aware
resolution, re-evaluating the `Handle<TException>` ambiguity-vs-
resolution split with the real 6-candidate pool from B2-EVAL-008, was
NOT done here — flagged as outstanding work).

## Priority 2 — Negative-knowledge reason taxonomy: DONE

**Commit:** `0bd0cb5` (bundled with Priority 3, same logical unit —
both touch `negative_knowledge.rs` and the same `calls.rs` call
sites).

Introduces `UnresolvedReason` (`Unknown`, `ExtractionRejected { rejection_kind }`,
`AmbiguousCandidates`) in `negative_knowledge.rs`, attached to every
`CallAttempt`. `Unknown` is the honest default whenever no more
specific cause has been independently verified — the module's own doc
comment states explicitly that fabricating a specific reason the
resolver cannot actually observe would be worse than reporting
`Unknown`. `CallAttempt::unresolved`/`ambiguous`'s existing public
constructors are unchanged in signature; a third constructor
(`unresolved_with_extraction_rejection`) was added for Priority 3's
own linkage.

**Tests:** 4 new, confirming: a plain unresolved attempt gets
`Unknown` (never fabricated), an ambiguous attempt carries
`AmbiguousCandidates`, the extraction-rejected reason is only
constructible via its own dedicated path, and the three reasons'
`description()` text is genuinely distinguishable (not three copies
of the same generic sentence).

## Priority 3 — SignatureTooLong provenance linkage: DONE

**Commit:** `0bd0cb5` (same as Priority 2).

`CandidateRejection::SignatureTooLong` now carries the real
candidate's own `name` field (confirmed, via direct code reading, that
the name was previously IN SCOPE at the point of rejection but
discarded — `RawSymbolCandidate::new`'s own `name: String` parameter
is consumed by `Ok(Self { name, ... })` on success but was never
captured on the `SignatureTooLong` error path). `CandidateRejection::NameTooLong`
deliberately still does NOT carry a name — the oversized name itself
cannot be safely echoed back without defeating the whole point of the
bound, confirmed as a real, distinct, and intentionally NOT-closed
residual limitation (documented inline, not silently left unexplained).

`resolve_calls_with_extraction_rejections` (the original
`resolve_calls_with_negative_knowledge` is now a thin wrapper calling
it with an empty rejection-name list, confirmed by a dedicated
equivalence test) accepts the caller's own independently-collected
`SignatureTooLong` names and, ONLY for the zero-candidate branch, links
a matching callee name to `UnresolvedReason::ExtractionRejected`
instead of the generic `Unknown`. This function performs no rejection
detection of its own — it only consumes what its caller already
verified, preserving the never-fabricate discipline. Confirmed, via a
dedicated test, that a real ambiguity (2+ promoted candidates) is
NEVER reclassified even if the same bare name coincidentally also
appears in the rejection list — the rejection-name check only ever
applies to the genuinely zero-candidate branch.

**Wiring into the real indexer (Priority 3's own "make the cause
observable" goal) is NOT YET DONE** — `resolve_calls_with_extraction_rejections`
exists and is tested in isolation, but `incremental::pipeline::index_files`/
`incremental::indexer.rs` do not yet call it with the real
`SignatureTooLong` names collected during that same extraction pass;
they still call the two-argument `resolve_calls_with_negative_knowledge`
(via `resolve_all_typed_edges_with_csharp_contexts`, itself unchanged
in this checkpoint). This is flagged as explicitly outstanding —
Priority 3's own library-level mechanism is complete and tested, but
its real-pipeline wiring (the part that would make the Polly corpus's
own 213 SignatureTooLong rejections actually show up as
`ExtractionRejected` in a real evaluation run) is not.

**Tests:** 5 new (1 in `candidate.rs` confirming the name is carried
correctly, 4 in `calls.rs`'s new `extraction_rejection_provenance`
module).

## Priority 4 — CallAttempt incremental integration: NOT DONE

Explicitly not attempted in this pass. This is the most
architecturally invasive of the six priorities (wiring a new raw-fact
type into `incremental::pipeline.rs`/`incremental::indexer.rs`'s own
reuse/re-resolve convention, extending the Phase 1.1 exact-multiset
`conformance.rs` gate to cover `CallAttempt`s, and writing adversarial
full/incremental parity tests through the real indexer) and was
judged too large to implement safely and completely within this
pass's remaining time without risking a half-finished, un-reviewed
change to production incremental-indexing code. Left for a dedicated
follow-up pass.

## Priority 5 — Incremental performance investigation: PARTIALLY DONE, real evidence gathered, not conclusive

Real, direct measurement was performed (temporary `Instant`-based
timing instrumentation added to `incremental::indexer.rs`'s own
`incremental_reindex`, run against the real `App-vNext/Polly` commit
pair via the existing `b2_smoke_test_csharp_cpp` harness, then
REVERTED — never committed, per this project's own "ephemeral probe,
delete after confirming" convention):

```
PERF_PROBE index_files(parse+extract, 41 files): 4.80s (run 1), 4.73s (run 2)
PERF_PROBE resolve_csharp_type_declaration_contexts (full pool 5190 snapshots, 860 contexts): ~7ms
PERF_PROBE resolve_all_typed_edges_with_csharp_contexts (full pool 5190 snapshots): 3.84s (run 1)
PERF_PROBE resolve_raw_import_statements (full pool 5875 snapshots, 725 raw statements): ~2ms
```

**Key finding:** the two full-pool resolution stages
(`resolve_csharp_type_declaration_contexts`,
`resolve_raw_import_statements`) are NEGLIGIBLE (single-digit-to-low-
double-digit milliseconds) despite running against the FULL current
snapshot pool (5190-5875 snapshots) on every incremental call, by
design (see `incremental/indexer.rs`'s own doc comments: import/impl/
C#-context resolution is always recomputed from scratch against the
current pool, never reused incrementally, because an untouched
declaration's correct resolution target can still change when some
OTHER file changes). This REFUTES the a priori hypothesis (formed
from reading the code's own doc comments before measuring) that
"global re-resolution of the full pool" is the dominant cost — it
measurably is not, for these two stages.

`resolve_all_typed_edges_with_csharp_contexts` (which internally runs
`resolve_calls`/`resolve_extends_implements_with_csharp_contexts`/
`resolve_tests_for`/`resolve_contains`/`resolve_instantiates` against
the full pool) DOES cost ~3.8s — a real, substantial, full-pool-
scaled cost, consistent with the "global recomputation dominates"
hypothesis for THIS specific stage.

**However, the single largest, most surprising cost is
`index_files`'s own parse+extract step for just 41 CHANGED files:
4.7-4.8 seconds** — comparable to or exceeding the ENTIRE full-
reindex elapsed time measured in prior evaluation passes (~3.95s for
ALL 792 parseable files). This was investigated further:

- File sizes of the 41 changed files were checked directly (git
  `show` + byte count) — none are pathologically large (largest
  checked: ~41KB), ruling out "one giant file" as the explanation.
- `LocalGitRepositorySource::read_file` was read directly: it calls
  `self.open_repository(...)` → `Repository::open(root.as_path())` —
  **re-opening the entire on-disk git repository from scratch on
  EVERY single file read**, confirmed by direct code reading (not
  assumed). This is a genuine, real, pre-existing structural
  inefficiency, independent of this checkpoint's own scope — but it
  cannot by itself explain "41 files slower than 792 files," since
  the full-reindex path pays this exact same per-file
  `Repository::open` cost 792 times (more often, not less).
- A THIRD run of the full harness was performed as a direct
  variance check: this run's own full-reindex time was **5.28s**,
  itself SLOWER than the prior run's own ~3.95s full-reindex and
  close to this same run's own incremental `index_files` time (4.5s)
  — strong direct evidence of substantial run-to-run environmental
  variance on this specific measurement environment (plausibly OS
  disk-cache state, background system load, or antivirus
  file-scanning interference on this Windows machine — not isolated
  further within this pass's own time budget).

**Classification:** primarily **measurement-artifact /
environmental-variance effect**, with a secondary, real, independently-
confirmed structural inefficiency (`Repository::open` called once per
file instead of once per batch) that is NOT yet confirmed to be the
specific cause of the cross-evaluation "incremental slower than full"
pattern, but IS a genuine, real optimization candidate worth fixing on
its own merits regardless of this specific investigation's outcome.
**This is NOT a confident, conclusive classification** — the user's
own instruction was explicit that the result "must be EXPLAINABLE,"
and the honest state here is "partially explained, with real evidence
pointing at both a measurement-variance effect and one real structural
inefficiency, neither fully isolated as THE cause." A dedicated
follow-up pass with proper statistical repetition (many runs, median/
variance reported, ideally on a less noisy measurement environment)
and a fix-then-remeasure test of the `Repository::open` per-call
pattern specifically would be needed to close this out properly.

**No optimization was implemented** — per the user's own explicit
"do NOT blindly optimize" instruction, and because the measurement
itself is not yet conclusive enough to justify a specific fix's own
real cost/risk.

## Priority 5 (continued) — Resumed with proper multi-run methodology: CONCLUSIVE, root-caused (B2-ID-012)

A later resumption of this checkpoint redid this investigation with
the proper repeated-run protocol this section's own prior pass
recommended but did not have time to execute: 10 full repeated runs
(same real Polly commit pair, same machine, temporary `Instant`
probes at four points inside `pipeline::index_files`/
`indexer::incremental_reindex`, never committed, fully reverted
afterward — confirmed via `git diff` showing zero net change).

**Result: the prior pass's "30% variance, inconclusive" framing does
NOT reproduce.** This pass's own 10-run dispersion is tight (~3%
spread on both the dominant cost and the overall total), and the
dominant cost is now precisely isolated: `resolve_lineage`'s own
O(n² × avg-signature-length²) similarity-scoring path (triggered only
when `prev_snapshots` is non-empty, i.e. only on a genuine incremental
step — the full-reindex case starts with an empty `prev` pool and
never reaches this cost at all, which is WHY the full-reindex
`resolve_lineage` call measures ~131µs against the SAME corpus's
larger 5121-snapshot pool while the incremental call measures ~3.88s
against a SMALLER 755-snapshot pool). Written up in full, with root
cause, measurement methodology, raw evidence, and severity reasoning,
as **B2-ID-012** (`docs/research/findings/B2-ID-012.md`, Moderate,
documented not fixed — a pure-performance finding below the
Critical/Important bar this pass's own instruction requires before
fixing rather than documenting).

This supersedes this section's own prior "not yet fully isolated as
THE cause" conclusion: `resolve_lineage` is now confirmed as the
dominant, specific, reproducible cost for this corpus/commit-pair's
own incremental step — roughly 10x the remaining ~800ms non-lineage
cost of the same 41-file incremental path (which is itself where the
PRIOR, separate, still-real `LocalGitRepositorySource::read_file`
per-file git-reopen inefficiency lives — both findings are real and
independently confirmed; B2-ID-012 ranks them for this specific
measurement rather than superseding either). **Still not fixed**, per
the same "do NOT blindly optimize a Moderate-severity performance
finding" instruction — see B2-ID-012's own "why this is NOT fixed"
section for the full reasoning.

## Priority 6 — Agent reliability re-check: DONE

Resumed and completed in a later pass: `agent_policy.rs`'s own
abstract tier-by-tier rules were re-checked against the REAL cases
this checkpoint's arity-narrowing resolution path actually produces
(`golden_corpus.rs`'s own
`arity_resolved_edge_is_heuristic_tier_and_the_policy_permits_it_only_with_disclosed_uncertainty`
and
`same_arity_ambiguity_produces_a_real_callattempt_the_policy_forbids_for_every_use`
tests, driven through the real production pipeline, not synthetic
`EvidenceTier` values alone). Confirmed: an arity-narrowed
single-survivor edge classifies as `Heuristic` (never `Resolved`) and
the policy correctly permits it for informational display ONLY with
disclosed uncertainty while still forbidding it alone as sole
justification for a high-impact modification; the required negative
case (same generic arity, differing ordinary parameter type) produces
a real `Ambiguous` `CallAttempt` that the policy correctly forbids for
every intended use, including mere display. Cross-checked against the
real Polly corpus run itself (Section R3 of
`b2_smoke_test_csharp_cpp.rs`): all 846 real Handle-named Ambiguous
`CallAttempt`s correctly simulate as Forbidden for both informational
display and high-impact modification.

## Golden corpus additions: NOT DONE

No new permanent fixtures were added for generic-arity disambiguation,
surviving-arity overload ambiguity, SignatureTooLong-to-negative-
provenance linkage, a genuine external-unresolved-target case, or
incremental `CallAttempt` parity. The regression tests added for
Priorities 1-3 (above) exist only in `calls.rs`'s/`negative_knowledge.rs`'s
own inline `#[cfg(test)]` modules, NOT in the permanent `tests/corpus/`
golden-corpus structure Hardening Phase 1 established. This is a real
gap against the checkpoint's own stated deliverables, honestly
reported.

## Differential testing extension: NOT DONE

The existing opt-in Roslyn differential suite (`tools/roslyn-oracle/`
+ its `examples/` harness) was not extended to cover generic arity,
the post-arity-filtering overload candidate set, or expected targets
for the new arity-aware resolution cases this checkpoint added.

## Independent review: NOT PERFORMED

No independent review (fresh-context) has been dispatched for ANY
part of this checkpoint. This document itself was written by the same
pass that implemented Priorities 1-3 and investigated Priority 5 — it
is a self-report, not a reviewed deliverable. This is itself the most
important outstanding item before this checkpoint can be considered
genuinely closed, consistent with every PRIOR finding/evaluation in
this series (B2-ID-009 through 011, B2-EVAL-008, the Hardening phases)
having required and received a real independent review before
closure.

## New findings surfaced during this pass

None rise to Critical or Important (no STOP was triggered). Two real,
Moderate-or-lower findings were surfaced as a direct byproduct of
implementing Priority 1, not separately escalated to their own
B2-ID-NNN docs in this pass (left for the user's own triage):

1. **C++ method-side generic arity is structurally unobservable from
   the current snapshot model** — `extract/cpp.rs`'s own
   `function_definition` span excludes the enclosing
   `template_declaration`/`template_parameter_list` entirely,
   confirmed via direct grammar probe. Documented inline in Priority
   1's own code comments; not a new extraction bug (nothing is
   mis-extracted), but a real, confirmed scope boundary on what
   arity-aware resolution can ever cover for C++.
2. **`LocalGitRepositorySource::read_file` re-opens the entire git
   repository on every single call** (`Repository::open` inside the
   per-file `read_file` path, confirmed via direct code reading) —
   a real, pre-existing structural inefficiency, independent of this
   checkpoint's own scope, surfaced during the Priority 5 performance
   investigation. Not fixed here (would require batching repository-
   handle reuse across a whole `index_files` call, a change to
   `local_git_source.rs` this pass judged out of its own narrow
   scope).

## Test/build health

`cargo test -p codegraph-index --lib`: 550/550 passing after every
commit in this checkpoint. `cargo build --workspace`: clean after
every commit. No regressions introduced anywhere in the existing
suite.

## Commits in this checkpoint

- `dad6616` — Priority 1 implementation + tests.
- `0bd0cb5` — Priority 2 + 3 implementation + tests.
- `d365918` — mutation-driven simplification fix for Priority 1.

No merge, tag, or push. No changes to `B2-EVAL-008.md` (already
COMPLETE, untouched). No new finding doc opened (nothing rose to
Critical/Important). `docs/research/challenge-sets/call-resolution-challenges.md`
and `docs/research/findings/B2-ID-009.md` show as locally modified in
`git status` but are PRE-EXISTING, untouched-by-this-pass modifications
from before this checkpoint began — confirmed via `git diff` showing
no changes attributable to this pass's own work.

## GO/NO-GO recommendation

**NO-GO to resuming B2-EVAL-009** — unchanged from before this
checkpoint; this was never in scope for this pass to authorize or
start, and nothing here changes that recommendation either way.

**This checkpoint itself should NOT be considered closed.** Priorities
4 and 6, the golden corpus additions, the differential-testing
extension, and — most importantly — the independent review are all
real, acknowledged, outstanding work, not silently dropped. The
user's own next decision is whether to: (a) authorize a follow-up
pass to complete the remaining priorities/deliverables before any
independent review, (b) authorize an independent review of ONLY what
was actually completed here (Priorities 1-3 plus the Priority 5
investigation) as a partial-scope review, or (c) reprioritize further
based on what this partial pass already found (particularly the two
new Moderate findings above, and Priority 5's own inconclusive-but-
suggestive performance evidence).
