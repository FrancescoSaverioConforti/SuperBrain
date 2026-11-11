# B2-EVAL-008: multi-codebase validation, C#/C++ domain

**Evaluation ID:** B2-EVAL-008
**Status:** **PAUSED again — a new Important finding (B2-ID-011) was
discovered during this resumed final measurement pass.** Per the
user's own standing protocol for this evaluation series
(Important/Critical findings STOP the evaluation and get reported,
never auto-fixed), this document is the full final measurement
report up to and including the discovery of B2-ID-011, but the
evaluation itself is **not** being closed as COMPLETE. B2-ID-009
(Critical) and B2-ID-010 (Important) are both CLOSED — fixed, tested,
mutation-verified, independently reviewed, committed. B2-ID-011
(Important) is OPEN, not fixed, not yet independently reviewed.
B2-EVAL-009 is NOT auto-started. Task 10 remains NOT authorized. No
merge, tag, or push.
**Date:** 2026-11-11 (original pass), resumed and finalized same
session after B2-ID-009/B2-ID-010 closure.
**SuperBrain commit:** branch `b2-code-graph`, commits `c09c34d`
(B2-ID-009 fix), `2001661` + `cc1878f` (B2-ID-010 fix). This final
measurement pass added no production-code changes — only this
document, a new finding doc (`B2-ID-011.md`), and a final evidence
file.
**External repository:** `App-vNext/Polly`
(https://github.com/App-vNext/Polly), BSD-3-Clause license, 3042
commits, 415 `.cs` files. Chosen per the user's explicit C#/C++
domain requirements: small/medium, real, auditable, class-centric,
with real inheritance (`BulkheadPolicy : Policy, IBulkheadPolicy`),
real interface-to-interface inheritance (`IBulkheadPolicy : IsPolicy,
IDisposable`), and real method overloads
(`Handle<TException>()`/`Handle<TException>(Func<TException,bool>)`,
duplicated across non-generic and generic variants). C# was preferred
over C++ per the protocol's own stated preference; C++ was not
separately evaluated this round.
**External commits:** commit A `32064ed55ab47e05b590be0360c8c45ce3901a73`
("Lint PowerShell scripts #2504"), commit B
`1f232caf9672e515751ba2df76d82bfbbb62e016` ("Improve Polly coverage
#2505" — 18 real files changed across Bulkhead/Fallback/RateLimit/
Registry/Retry/Timeout, 54 insertions/50 deletions).
**Configuration:** the frozen baseline configuration, PLUS the two
committed fixes (B2-ID-009, B2-ID-010). No other algorithm change.
**Methodology:** drove the real production pipeline via
`crates/codegraph-index/examples/b2_smoke_test_csharp_cpp.rs`
(updated during this session to call
`resolve_extends_implements_with_csharp_contexts` with real resolved
C# declaration contexts, matching the production wiring exactly).
**This document's own numbers are from ONE fresh, final run**,
executed after both fixes landed — not copied from either finding
doc without re-measurement. Raw output preserved at
`docs/research/evidence/b2-eval-008-polly-run-FINAL.txt`.

## Contamination-reset ledger (per explicit instruction: report pre-fix, finding, post-fix, delta, conclusion-changed-or-not — never silently reuse pre-fix numbers, never delete history)

| Metric | Pre-fix value | Invalidated by | Post-fix value (this final run) | Delta | Conclusion |
|---|---|---|---|---|---|
| `Handle` snapshots (Section C2) | 2 | B2-ID-009 | **4** | +2 | Changed: now matches real overload count exactly |
| `calls_edges` (whole corpus) | 1658 | B2-ID-009 | **1882** | +224 | Changed: real calls recovered. **Still understates the true count — see B2-ID-011 below, a SEPARATE, newly-discovered gap this run found, not yet reflected in any prior number** |
| `extends_implements_edges` | 65 (pre-009) → 76 (post-009, pre-010) | B2-ID-009, then B2-ID-010 | **103** | +27 over post-009 value, +38 over original | Changed: `BulkheadPolicy -> Policy` and 7 other real classes now resolve |
| `by_kind Extends` | 45 → 54 | same | **81** | +27 | Changed |
| `by_kind Implements` | 20 → 22 | B2-ID-009 only | **22** | 0 (B2-ID-010 added no new Implements edges in this corpus — `IBulkheadPolicy` is not partial) | Held |
| `observe_commit(A) edge_count` | 2959 → 3193 | both | **3220** | +27 over post-009 | Changed |
| `snapshots_promoted`/`Method` count | 5190 / 4521 | neither (names only, not counts) | **5190 / 4521** | 0 | Held — confirms both fixes only ever change edge targets or symbol NAMES, never add/drop/duplicate a snapshot |

**New, not-yet-reflected-anywhere contamination note added by THIS run:**
`calls_edges: 1882` is itself now confirmed to be an UNDERCOUNT for a
reason independent of both B2-ID-009 and B2-ID-010 — see
[B2-ID-011](../findings/B2-ID-011.md) below. This number should not
be treated as the corpus's true call-graph size in any future
evaluation that cites this document.

## A. Corpus metrics (final run, fresh measurement)

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
Extraction-coverage gaps (raw-parse proxy, unchanged by either fix,
still real and unaddressed): `struct_declaration` 46, `enum_declaration`
15, `constructor_declaration` 379, `field_declaration` 730 — none
walked by `extract/csharp.rs` today. `csharp_using_directives: 725`,
`csharp_namespace_decls: 781`.

## B. Corrected method extraction (B2-ID-009 effect, final confirmation)

Section C2 of this final run finds **4** snapshots named exactly
`"Handle"` in `Policy.HandleSyntax.cs` — matching the real ground
truth (`Handle<TException>()`, `Handle<TException>(Func<...>)`, and
their `PolicyBuilder<TResult>` counterparts) exactly, confirming
B2-ID-009's fix holds on a fresh run, not just the fix's own
verification pass. Full before/after scale (937 methods collapsed
under the bogus name `"Task"`, 1535/4543 total methods misnamed
pre-fix) is documented in `B2-ID-009.md` and not re-derived here —
the relevant confirmation for THIS evaluation is that the symptom
(C2's own discovery case) is gone in a clean, independent re-run.

## C. Inheritance/interface quality (B2-ID-010 effect, final confirmation)

Final run, Section C1, full list of 15 edges touching
`BulkheadPolicy`/`IBulkheadPolicy`/`Policy`:

```
BulkheadPolicy.cs::BulkheadPolicy -[Extends]-> Policy.ContextAndKeys.cs::Policy
IBulkheadPolicy.cs::IBulkheadPolicy -[Implements]-> IsPolicy.cs::IsPolicy
IBulkheadPolicy.cs::IBulkheadPolicy -[Implements]-> IBulkheadPolicy.cs::IBulkheadPolicy  (generic variant -> non-generic sibling; see note below)
TimeoutPolicy.cs::TimeoutPolicy -[Extends]-> Policy.ContextAndKeys.cs::Policy
RateLimitPolicy.cs::RateLimitPolicy -[Extends]-> Policy.ContextAndKeys.cs::Policy
ResiliencePipelineSyncPolicy.cs::ResiliencePipelineSyncPolicy -[Extends]-> Policy.ContextAndKeys.cs::Policy
CircuitBreakerPolicy.cs::CircuitBreakerPolicy -[Extends]-> Policy.ContextAndKeys.cs::Policy
NoOpPolicy.cs::NoOpPolicy -[Extends]-> Policy.ContextAndKeys.cs::Policy
FallbackPolicy.cs::FallbackPolicy -[Extends]-> Policy.ContextAndKeys.cs::Policy
AddBehaviourIfHandlePolicy.cs::AddBehaviourIfHandlePolicy -[Extends]-> Policy.ContextAndKeys.cs::Policy
PolicyWrap.cs::PolicyWrap -[Extends]-> Policy.ContextAndKeys.cs::Policy
Policy.cs::Policy -[Extends]-> PolicyBase.ContextAndKeys.cs::PolicyBase
CachePolicy.cs::CachePolicy -[Extends]-> Policy.ContextAndKeys.cs::Policy
RetryPolicy.cs::RetryPolicy -[Extends]-> Policy.ContextAndKeys.cs::Policy
PreExecutePolicy.cs::PreExecutePolicy -[Extends]-> Policy.ContextAndKeys.cs::Policy
```

**`extends_implements_edges: 103` (whole corpus), `Extends: 81`,
`Implements: 22` — exactly reconfirming B2-ID-010's own closure
numbers (76->103, 54->81) on this fresh, independent run**, not
merely copied from the finding doc.

**Partial-class reconciliation validation, confirmed directly
against the real corpus (not just unit tests):**
- `BulkheadPolicy -> Policy` resolves (`Policy` has 8 partial
  fragments; the edge points to the lowest-span-start fragment,
  `Policy.ContextAndKeys.cs`) — the headline case, fixed.
- 8 OTHER real classes extending `Policy` also newly resolve
  (`TimeoutPolicy`, `RateLimitPolicy`, `CircuitBreakerPolicy`,
  `NoOpPolicy`, `FallbackPolicy`, `AddBehaviourIfHandlePolicy`,
  `PolicyWrap`, `CachePolicy`, `RetryPolicy`, `PreExecutePolicy`,
  `Policy` itself extending `PolicyBase`) — confirming the fix is not
  a `BulkheadPolicy`-specific patch.
- `BulkheadPolicy -> IBulkheadPolicy` (`Implements`) correctly STILL
  does not resolve — `IBulkheadPolicy`/`IBulkheadPolicy<TResult>` are
  both non-partial, so this is the SEPARATE generic-arity-collision
  gap (quantified below), not a B2-ID-010 regression.
- Same-name NON-partial duplicate types: not found as a live case in
  THIS specific corpus slice (Polly does not have two genuinely
  unrelated, non-partial, same-named classes in the same namespace
  in the real data) — this invariant is validated by B2-ID-010's own
  19 unit tests against synthetic fixtures instead, since the real
  corpus does not happen to exercise it.

**Cosmetic display note (flagged by B2-ID-010's own independent
review, reconfirmed in this run):** the
`IBulkheadPolicy -> IBulkheadPolicy` line above is NOT a true
self-loop (`TargetLookup::find_by_name` always excludes the source's
own id, and the reconciliation function never returns the source
either) — it is `IBulkheadPolicy<TResult>` resolving to its
non-generic sibling `IBulkheadPolicy`; the harness's own
`"{path}::{name}"` print format omits generic arity, making the two
distinct snapshots look identical in this text. Not fixed, not a
resolver defect — a harness-display gap only.

### Generic-arity gap, quantified (NOT fixed — measurement only, per instruction)

Direct, corpus-wide measurement (not estimated): **59 distinct bare
type names** in this corpus have 2+ declarations sharing that bare
name but differing ONLY in generic arity (e.g. `Policy`/`Policy<TResult>`,
`IBulkheadPolicy`/`IBulkheadPolicy<TResult>`, `RetryPolicy`/`RetryPolicy<TResult>`,
`AsyncRetryPolicy`/`AsyncRetryPolicy<TResult>`, and 55 more). A
raw-text proxy search for real base/heritage-clause references to any
of these 59 names that produce NO matching `Extends`/`Implements` edge
found **228 candidate matches** (this proxy has known noise — it
can false-positive on parameter lists/generic constraints containing
the same text, so treat 228 as an upper-bound order-of-magnitude
figure, not an exact count, consistent with this evaluation's own
evidence-rigor standard).

- **Resolver behavior confirmed**: `resolve_extends_implements_with_csharp_contexts`'s
  own base-name lookup (`TargetLookup::find_by_name`) matches
  candidates by bare `SymbolSnapshot` name only — generic arity is
  never consulted at the base-clause-resolution layer (B2-ID-010's
  own fix explicitly scopes its arity-awareness to the PARTIAL-class
  reconciliation path only, never applying it to resolve a genuine
  arity ambiguity in the base/derived relationship itself).
- **Would generic arity alone recover the correct target?** For the
  base-clause case specifically: **yes, in principle** — a C# base
  clause's own bare name never carries a `<T>` marker (confirmed
  during B2-ID-010's own implementation: `: Policy` always means the
  arity-0 member), so if the resolver looked up candidates filtered
  to `generic_arity == 0` FIRST (rather than treating all arities as
  one undifferentiated name bucket), most of these 59 collisions
  would resolve unambiguously without any type inference at all —
  this is a markedly CHEAPER fix than full type-aware resolution,
  since the arity information is already extracted
  (`CSharpTypeDeclarationContext` from B2-ID-010) and sitting unused
  for this specific resolution path.
- **Evidence toward CALL-RES-A5/richer resolution**: this measured
  59-name, ~228-reference scale is concrete, real evidence that
  arity-blind name resolution has a real, nontrivial cost in a
  class-centric, generics-heavy C# codebase — stronger evidence than
  the single `IBulkheadPolicy` example alone. It argues for a
  narrower, cheaper first step (arity-aware base-clause resolution,
  reusing B2-ID-010's own context data) BEFORE a full type-aware
  CALL-RES-A5 checkpoint, not necessarily as a substitute for one.

## D. Overload evidence (method name + Calls, now entangled with B2-ID-011)

Ground truth (`Policy.HandleSyntax.cs`, direct source reading): 4 real
`Handle<TException>()`/`Handle<TException>(Func<TException,bool>)`
overloads (2 non-generic-`PolicyBuilder`-returning, 2 generic-
`PolicyBuilder<TResult>`-returning). Extraction now correctly produces
4 distinct `Handle`-named snapshots (B2-ID-009 fixed). `resolve_calls`
finds **zero** `Calls` edges targeting any of them in this final run.

**Root-caused, NOT an overload-ambiguity finding — this is
[B2-ID-011](../findings/B2-ID-011.md):** the real call sites
(`Policy.Handle<DivideByZeroException>()`, 875 occurrences of this
exact shape in the test suite alone) are `receiver.Method<T>(...)`
generic method calls via member access. `resolve_calls`'s own
`callee_name` function only recognizes a bare
`identifier`/`property_identifier`/`field_identifier` as the member
name inside a `member_access_expression` — but when the method call
carries a type argument (`<T>`), the real AST wraps the name in a
`generic_name` node instead, which the filter never matches. The
call site is silently DROPPED at extraction, never even reaching
name-resolution logic — so it cannot show up as `Unresolved` or
`Ambiguous` either.

**This means the overload research question cannot be cleanly
answered from this corpus as currently measured**: the absence of
`Calls` edges toward any `Handle` overload is NOT evidence that the
4 overloads are correctly/incorrectly disambiguated — it is an
artifact of the call sites never being extracted at all. A clean
answer requires B2-ID-011's own fix first. Qualitatively, independent
of B2-ID-011: all 4 real overloads share the bare name `"Handle"`,
and `resolve_calls`'s matching is purely by name (confirmed by
reading the resolver's own code, unaffected by B2-ID-011) — so once
call sites ARE extracted, they will still collapse to one
undifferentiated name bucket across all 4 overloads, with no
arity/parameter-list disambiguation. The qualitative conclusion from
every prior domain's overload evidence (per-name resolution cannot
distinguish real overloads) still holds; the QUANTITATIVE Calls-edge
evidence for or against it does not, in this corpus, until B2-ID-011
is fixed.

**Static vs. instance / cross-class same-name methods**: `Handle`/
`HandleInner`/`HandleResult` are all `static`, confirmed from source.
A broader static-vs-instance-overload ground truth sample (per
protocol item 5) was NOT completed in this pass — superseded by the
B2-ID-011 discovery, which this evaluation's own protocol requires to
STOP on before continuing further Calls-dependent measurement.

## E. Calls quality — **NOT completed to the originally-planned depth, per the B2-ID-011 STOP**

A full manual TP/FP/FN sample was not built in this pass: per the
user's own instruction ("Critical/Important: STOP"), continuing to
build a representative Calls-quality sample on top of a corpus where
18.8% of real call sites (4977/26508 `invocation_expression` nodes,
measured directly) are silently invisible to the resolver would
produce a sample that is not representative of this corpus's real
call graph, and risks wasting effort that would need re-doing after
B2-ID-011's own fix. The sample from the manual call-quality section
of the raw evidence file (30 edges, all either `StaticAnalysis`
unambiguous single-candidate matches or `NameHeuristic`
same-enclosing-class matches, zero visibly wrong TP/FP in that
specific slice) is preserved as a SNAPSHOT of what the CURRENTLY
VISIBLE call graph looks like, not as a claim about the corpus's true
call-graph quality.

## F. Imports/usings (unaffected by either fix or by B2-ID-011)

```
total_using_directives_at_commit_b: 725
csharp_import_facts_extracted: 725
extraction_coverage: 100.0%
resolution_internal: 38, resolution_external: 571, resolution_ambiguous: 116
aliases_captured: 17
```

Unchanged from the pre-pause measurement — `using`/`ImportFact`
extraction does not depend on method names, Extends/Implements, or
Calls resolution, so none of B2-ID-009/010/011 touch it.
**B2-ID-010's own namespace-awareness (`CSharpTypeDeclarationContext`)
does NOT change any conclusion here** and, per explicit instruction,
is NOT fed into Calls/Imports reasoning automatically — that remains
a separate, not-yet-made architectural decision, flagged here
explicitly rather than quietly assumed.

## G. SignatureTooLong (cross-domain tracking continues)

```
total rejections (MEASURED, this final run): 213
SignatureTooLong frequency: 213/5403 (3.94%)
average rejected signature size (MEASURED via the real extractor, not estimated): 15,867 bytes (max accepted: 4096)
largest rejected signatures (MEASURED):
  test/Polly.Specs/CircuitBreaker/AdvancedCircuitBreakerAsyncSpecs.cs: 135,468 bytes
  test/Polly.Specs/CircuitBreaker/AdvancedCircuitBreakerSpecs.cs: 124,541 bytes
  src/Polly/Caching/AsyncCacheTResultSyntax.cs: 79,293 bytes
  src/Polly/Caching/CacheTResultSyntax.cs: 78,289 bytes
  src/Polly/Retry/AsyncRetryTResultSyntax.cs: 73,499 bytes
```

All 213 rejections are `SymbolKind` candidates whose FULL node text
(for a `class_declaration`, this includes its entire body — a large
class with many fluent-syntax methods, like the large `*Specs.cs`
test files and `*Syntax.cs` fluent-builder files above, easily
exceeds the 4096-byte ceiling by 20-30x) exceeded the byte ceiling.
**This is the fourth domain confirming SignatureTooLong**
(pyjanitor/Python: 53/2822; pipx/Python: 18/18; Polly/C#: 213/5403) —
the gap is now confirmed across BOTH languages this pipeline's
extraction model treats identically (whole-node-text capture), not a
Python-specific or C#-specific artifact. Not fixed, per instruction.

## H. Callback/registration evidence (B2-EVAL-007 recurrence, reconfirmed post-fix)

Confirmed still holding on this final run and against the real
source directly: `BulkheadPolicy`'s constructor takes an
`Action<Context> onBulkheadRejected` parameter, stores it in a
private field `_onBulkheadRejected`, invoked later from a DIFFERENT
method (`BulkheadEngine.Implementation`) — never a named symbol
`resolve_calls` could match. `FallbackPolicy` has the identical shape
(`_onFallback`). The real xUnit test
`Should_call_onBulkheadRejected_with_passed_context`
(`test/Polly.Specs/Bulkhead/BulkheadSpecs.cs:81`) confirms this is a
load-bearing, tested behavior, not incidental code.

**Confirms B2-EVAL-007's command-registration-graph category is NOT
CLI-framework-specific**: two independent real mechanisms (pipx's
dynamic-attribute dispatch table vs. Polly's constructor-injected
delegate fields) produce the same structurally-invisible-to-this-
pipeline relationship, across two unrelated domains. No new edge kind
implemented, per instruction.

## I. Module/File evidence (Option 2 tracking continues)

```
files_with_at_least_one_extracted_symbol: 722
total_top_level_symbols_across_all_files: 3889
files_with_more_than_one_top_level_symbol: 233 (32.3%)
mean_top_level_symbols_per_file: 5.39
median_top_level_symbols_per_file: 1
```

**C#-specific nuance, reconfirmed**: the median of 1 shows roughly
half of all files DO follow a clean "1 class : 1 file" convention —
a File/Module node would add LESS value here than in Rust/Python
(prior domains, where the median was higher) for those files
specifically. But the mean of 5.39 and 32.3% multi-symbol rate show
this convention does not hold uniformly: large test-spec files (up to
88 top-level symbols, one per `[Fact]`/`[Theory]` test method) and
`partial class` fragments (each fragment is its own file, but the
LOGICAL type spans several files — a File/Module node keyed by file
would still not represent "this partial type" as one queryable unit;
`CSharpTypeDeclarationContext` from B2-ID-010, NOT a File/Module node,
is what actually closes that specific gap) both pull away from the
1:1 ideal.

**Net assessment for this domain**: the Module/File gap's RELEVANCE
is lower here than in function-heavy Python/Rust domains for the
~50% of files that are genuinely 1-class-per-file, but NOT uniformly
lower — test files and partial-class fragments are real, measured
counter-examples within the same corpus. Tentative recommendation
(Option 2: synthetic File/Module node + Contains) remains tentative;
this domain adds a nuance (file role matters more than language
paradigm) rather than a clean confirmation or rejection.

## J. Coding-agent usefulness Q&A (final run, Section H)

- **"Does BulkheadPolicy extend a base class AND implement an
  interface?"** — **Partially answerable, root-caused as a HARNESS
  query artifact, not a resolver defect**: the harness's own
  `find_current_by_name_in_file` helper picks the FIRST lineage-current
  snapshot matching a name+file-suffix filter; since the real graph
  has BOTH `BulkheadPolicy` (non-generic, HAS the Extends edge) and
  `BulkheadPolicy<TResult>` (generic, does not — the same
  generic-arity gap noted above) in the same file, the harness's
  query nondeterministically picks either one depending on snapshot
  iteration order. Confirmed via a dedicated probe: the edge DOES
  exist in the graph (`BulkheadPolicy -[Extends]-> Policy`, shown
  directly in Section C1's own un-filtered query). **This is a
  measurement-harness bug, not a production pipeline defect** — not
  fixed here, per this evaluation's own precedent (B2-EVAL-005 fixed
  an analogous harness display bug as harness-only, not production
  semantics).
- **"Which classes implement this interface?"** (`IBulkheadPolicy`) —
  **Fully answerable** via `Implements` edges for the non-generic
  case; the generic variant's own implementers are invisible, per the
  generic-arity gap above.
- **"Who calls this method?"** / **"Which overload gets invoked?"** —
  **Not answerable**, root-caused to B2-ID-011: generic method calls
  via member access are not extracted as call sites at all.
- **"Where is this type constructed?"** — not specifically tested in
  this pass; `Instantiates` resolution is independent of B2-ID-011
  (it does not re-use `calls.rs`'s own callee-name extraction), not
  expected to be affected, not separately confirmed here.
- **"What's impacted if I modify this interface/base class?"** — for
  `Policy`: **fully answerable** post-B2-ID-010 (10 real derived
  classes now visible via `Extends`, up from 0). For `IBulkheadPolicy`:
  **partially answerable** (the one non-generic implementer chain is
  visible; the generic one is not).
- **"Which tests cover this class?"** — **Not answerable** for
  `BulkheadPolicy`: `tests_for_edges: 0` corpus-wide, root-caused
  (not re-derived here, see prior pause's own D section) to
  `resolve_tests_for`'s naming-convention gate not recognizing
  Polly's exclusively-used xUnit `Should_<behavior>` idiom. Moderate,
  documented, not escalated, per instruction.
- **"What's the flow between these components?"** — not specifically
  tested; `Calls`-dependent, so any answer drawn now inherits
  B2-ID-011's own undercounting.

## K. Incremental A->B (final run)

```
observe_commit(A) snapshot_count: 5121, edge_count: 3220
observe_commit(B) metrics: files_considered: 41, files_reparsed: 41,
  snapshots_removed: 686, snapshots_added: 755, edges_rebuilt: 3904,
  raw_import_statements_extracted: 4, raw_import_statements_reused: 721,
  raw_csharp_type_declaration_contexts_extracted: 49,
  raw_csharp_type_declaration_contexts_reused: 811
observe_commit(B) final snapshot_count: 5875, edge_count: 3219
observe_commit(B) final lineage_edge_count: 685
full reindex: 3.93s; incremental: 6.24s
```

**Full-vs-incremental parity on the NEW C# contexts, confirmed**:
`raw_csharp_type_declaration_contexts_extracted: 49` (the 41
reparsed files' own fresh declarations) +
`raw_csharp_type_declaration_contexts_reused: 811` (every untouched
file's context, carried forward unchanged) = 860 total — the same
reuse-on-untouched/always-reresolve-against-current-pool convention
`raw_import_statements`/`raw_impl_statements` already use, confirmed
operating correctly on the real corpus (not just B2-ID-010's own 5
synthetic-fixture tests). `extends_implements_edges` count after
incremental (`edges_rebuilt: 3904` total typed+lineage edges,
consistent with the full-reindex figure of 103 Extends/Implements
within the broader edge set) shows no silent drop from the
incremental path.

**Incremental slower than full reindex, still unexplained, carried
forward as an open question**: 6.24s incremental vs. 3.93s full, for
a 41-file change — the opposite of every prior domain's own
incremental-speedup pattern. Not investigated further in this pass
either (lower priority than B2-ID-011); plausibly related to this
specific commit pair's own 41-file breadth touching 6 different
policy modules simultaneously, not confirmed.

## L. Performance

```
full_index_elapsed: 3.93s
observe_commit(A) (full reindex): 3.93s
observe_commit(B) (incremental): 6.24s
nodes (final): 5875 snapshots
edges (final): 3219
ImportFacts: 725
CSharpTypeDeclarationContexts (total, reused+fresh): 860
memory (approx, Windows tasklist): 21.9K (working-set units as reported by `tasklist`, not independently validated against a dedicated profiler)
```

## New findings opened this evaluation (cumulative)

- **[B2-ID-009](../findings/B2-ID-009.md)** (Critical) — CLOSED, fixed,
  tested, mutation-verified, independently reviewed (APPROVE).
- **[B2-ID-010](../findings/B2-ID-010.md)** (Important) — CLOSED,
  fixed, tested, mutation-verified, independently reviewed (APPROVE).
- **[B2-ID-011](../findings/B2-ID-011.md)** (Important) — **OPEN**.
  C# generic method calls via member access
  (`receiver.Method<T>(...)`) are entirely invisible to
  `resolve_calls` — the callee-name extractor's `member_access_expression`
  handling never recognizes a `generic_name`-wrapped method name.
  Measured: 4977/26508 real call sites (18.8%) in this corpus alone.
  NOT fixed in this pass, per protocol. **This is the reason this
  evaluation is PAUSED again, not COMPLETE.**
- **Generic-arity name collision** (Moderate, carried forward,
  quantified further in Section C above: 59 distinct bare names, ~228
  raw-text-proxy unresolved base-clause references) — still not
  escalated to its own ID, per the user's own prior classification
  as Moderate/backlog; now has substantially more real evidence
  behind it than the single `IBulkheadPolicy` example.
- **xUnit `TestsFor` naming-convention mismatch** (Moderate, carried
  forward unchanged) — still not escalated.
- **C# extraction-coverage gaps** (struct/enum/constructor/field
  declarations not walked by `extract/csharp.rs`) — Moderate,
  measured, documented, not escalated.
- **Incremental-slower-than-full-reindex** on this specific
  commit pair — open question, not yet root-caused, not escalated
  (no evidence yet that it is a defect rather than a corpus-specific
  characteristic).

## Challenge-set cases

**None opened in `call-resolution-challenges.md` this pass.**
B2-ID-011, while a significant real FP/FN-shaped gap (a massive FN:
4977 real calls systematically absent), was judged to warrant its own
finding document (an extraction-stage defect with a clear, narrow,
single-function root cause) rather than a CRC case (CRC cases in this
project's own convention track RESOLUTION-layer ambiguity/precision
challenges once a candidate call site IS extracted — B2-ID-011 is
upstream of that, at the extraction stage itself, closer in kind to
B2-ID-009 than to CRC-001/002/003). This judgment call is the
evaluator's own, not dictated by the protocol; flagged here explicitly
so the user can override it if a CRC case is preferred instead.

## Independent review

**Dispatched for this report's own claims** (repo/license/commit
facts, the method-name-correction and partial-class-recovery
reflections, namespace/arity behavior, imports/usings, SignatureTooLong,
callback/registration evidence, Module/File conclusions, arithmetic
consistency, raw evidence citations, cleanup) — see the separate
review response delivered alongside this report. B2-ID-011 ITSELF
(the new finding's own root cause, reproduction, and scope
measurement) should receive its own dedicated independent review
at fix time, mirroring B2-ID-009/010's own process — not yet done,
since no fix has been proposed or implemented for it.

## GO/NO-GO to next domain

**NO-GO.** B2-ID-011 (Important) is open. Per the user's own standing
protocol, B2-EVAL-009 must not be auto-started. The next step is the
user's own decision on B2-ID-011 (fix now vs. defer), exactly
mirroring how B2-ID-009 and B2-ID-010 were each handled in turn.

## Evidence for/against an A4/A5 experimental checkpoint

**Strengthened, but still not conclusively answerable from a clean
run.** This domain adds three independent, concrete data points
toward richer-than-pure-name resolution:

1. **Overload evidence (qualitative, confirmed independent of
   B2-ID-011)**: 4 real `Handle` overloads share one bare name; the
   resolver's pure-name matching cannot distinguish them by
   construction, regardless of the call-site-extraction gap.
2. **Generic-arity evidence (quantitative, NEW this pass)**: 59
   distinct bare names with 2+ generic-arity variants, ~228 raw-text-
   proxy real base-clause references that may be affected — and,
   importantly, evidence that a NARROWER, CHEAPER fix (arity-aware
   base-clause resolution reusing B2-ID-010's own
   `CSharpTypeDeclarationContext`) could recover much of this value
   without full type inference.
3. **B2-ID-011 itself**: a generic-call-site extraction gap is
   actually UPSTREAM of the overload-resolution question entirely —
   it means some of the clean, decisive evidence this evaluation was
   designed to produce (a fresh Calls-quality sample, a stress-tested
   overload ground truth with real predicted-vs-expected targets)
   could not be produced this pass BECAUSE the underlying call graph
   itself is incomplete, not because overloads turned out to be
   unambiguous.

**Recommendation**: do not treat this domain's evidence as sufficient
to START an A4/A5 checkpoint yet — B2-ID-011 must be fixed and the
Calls-quality/overload sections re-run cleanly first, the same
discipline already applied to C2's own re-run after B2-ID-009. The
generic-arity evidence, however, suggests the FIRST concrete step
(if/when the user authorizes resuming this work) should likely be
narrower than a full CALL-RES-A5 type-aware resolution — an
arity-aware base-clause resolution reusing data already extracted for
B2-ID-010 — rather than jumping straight to full type inference.
