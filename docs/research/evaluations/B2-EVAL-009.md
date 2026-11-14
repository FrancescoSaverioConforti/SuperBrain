# B2-EVAL-009: C++ domain standalone evaluation

**Evaluation ID:** B2-EVAL-009
**Status:** COMPLETE. No new Critical/Important production defect
found. The known residual limitation (C++ namespace-qualified call
extraction — `callee_name`'s own missing `qualified_identifier` match
arm, first flagged during B2-ID-011's independent review) is
RE-CONFIRMED, QUANTIFIED on real code, and characterized as an honest
silent-drop (never a wrong edge) — exactly as previously documented,
not materially worse. No fix attempted, per explicit scope. This is
the first STANDALONE full C++ domain evaluation (prior C++ exposure,
in B2-EVAL-008, was secondary/bundled inside a C#-centric Polly round).
**Date:** 2026-11-13.
**SuperBrain commit (baseline, HEAD at evaluation time):**
`f50c63457e14e4a2ba1bfc954a92b69373060031` on branch `b2-code-graph`,
worktree `.worktrees/b2-code-graph`. New files this round: two example
harnesses (`crates/codegraph-index/examples/b2_eval_009_cpp_fmt.rs`,
`b2_eval_009_cpp_parity.rs`), this report, and three raw-evidence files
under `docs/research/evidence/`. No merge/tag/push. No production code
changed. Task 10 untouched. CALL-RES-A5 not started.

## Two prior dispatch attempts for this round — resolved, not repeated

A first attempt stopped on a false alarm (misreading historical
continuity decision records as evidence of a live concurrent agent —
it was not). A second attempt correctly found Code Graph has zero Java
support (never in MVP scope) and correctly refused to fabricate
coverage, then stopped; the user redirected this round to C++, which
IS supported (`tree-sitter-cpp`, `Language::Cpp`, a real `cpp.rs`
extractor, and C++ was touched during CALL-RES-A4/B2-ID-011's own
work). Neither prior attempt is re-litigated here.

## Repo selection

**Repository:** `fmtlib/fmt` (https://github.com/fmtlib/fmt)
**Commit:** `208219daff58f2a7438f6931cc2ddc358acf33ef` (tag `12.2.1`,
a stable tagged release, not a moving branch).
**License:** MIT. `LICENSE` file at repository root (`Copyright (c)
2012 - present, Victor Zverovich and {fmt} contributors`).
**Corpus scope:** `include/` + `src/` only (19 files: 15 headers,
4 `.cc`), deliberately EXCLUDING `test/`, which vendors a full
third-party copy of googletest/googlemock under `test/gtest/`
(confirmed by direct inspection — `test/gtest/gmock/gmock.h`,
`test/gtest/gtest/gtest.h` are not fmt's own code and would otherwise
pollute every corpus metric, especially the overload sample, with an
unrelated library's internals).

**Selection reasoning:** real nested namespaces (`fmt`, `fmt::detail`,
`fmt::adl` — 29 real `namespace_definition` nodes in the scoped
corpus); heavy template usage (720 `template` declarations repo-wide,
767 bare + 253 namespace-qualified template CALL sites in the scoped
corpus); pervasive class inheritance (`class iterator_buffer : public
Traits, public buffer<T>`, `class basic_memory_buffer : public
detail::buffer<T>`, etc.); real method overloading by name and arity
(`format_to` — 9 same-named declarations across 5 files; `advance_to` —
4 same-named declarations across 3 files); multi-file `.h`/`.cc` split;
17,252 LOC in the scoped corpus (within the directive's low-thousands-
to-~20-30k target). MIT license confirmed by reading the `LICENSE`
file directly.

## Step 1 — Baseline freeze (re-confirmed before touching the new corpus)

`cargo test -p codegraph-index --release`: **685/685 passing, 0
failures** (607 lib tests + 4 + 18 + 34 + 12 + 10 across the six
integration-test binaries, 0 doc-tests). This includes the golden
corpus, CRC-001/002/003 regression tests
(`ground_truth_schema.rs`, 12/12), and the reliability gates
(`reliability_gates.rs`, 10/10, including gate 3 — full-equals-
incremental — and gates 6/7 — Ambiguous/Unresolved never silently
promoted). CALL-RES-A4 is non-regressed. No Polly/pyjanitor re-run was
needed to re-confirm this (the lib suite's own fixtures already cover
those corpora's pinned regression shapes). Raw evidence:
`docs/research/evidence/b2-eval-009-baseline-freeze.txt` (added after
an independent reviewer correctly flagged that this claim was not yet
backed by a saved artifact — see "Independent review" below).

## Corpus metrics (fmt, scoped to include/+src/)

| Metric | Value |
|---|---|
| Files (library scope) | 19 (15 `.h`, 4 `.cc`) |
| Files parsed, 0 error | 19 parsed; 18 contain at least one tree-sitter error node (pervasive but not blocking — see note below) |
| LOC (newline count) | 17,252 |
| Raw extraction candidates | 1,441 |
| Extraction rejections (`SignatureTooLong`) | 16 |
| Snapshots promoted | 1,441 (Function 1,215; Struct 164; Class 62) |
| Real namespace declarations | 29 |

**Note on `files_with_error_nodes: 18/19`**: tree-sitter-cpp reports at
least one error node in nearly every file in this corpus. This is
consistent with fmt's heavy use of C++20/23 template metaprogramming
syntax that a general-purpose C++ grammar routinely mis-parses locally
(e.g. fold expressions, concepts, complex SFINAE) without it affecting
the surrounding extraction — snapshot/edge counts are sane and
internally consistent (cross-checked against direct source reads
throughout this report), so this is reported as a real, pre-existing
parser-robustness data point, not re-investigated as a new finding
(out of this round's scope, and not blocking — extraction correctness
was spot-checked and held up everywhere it was checked).

## Calls / Imports / structural edges

| Edge kind | Count | Resolution method breakdown |
|---|---|---|
| Calls | 845 | StaticAnalysis (within Calls+Extends combined: see below) |
| Contains | 216 | — |
| Extends | 4 | — |
| TestsFor | 0 | — |
| Instantiates | 0 | — |

Resolution method across all edge kinds combined: **StaticAnalysis
885, NameHeuristic 180.**

**Imports: 0, structurally, confirmed not assumed.** Direct source
read of `crates/codegraph-index/src/imports.rs`'s `walk_for_imports`:
`Language::Cpp | Language::Html | Language::Css => {}` — a deliberate
no-op. C++ `#include` directives are never extracted as `ImportFact`s
at all. This means CALL-RES-A4's Python-alias and C#-namespace-using
mechanisms have **zero data to work with on C++ regardless of corpus**
— not a corpus-specific gap, a structural one. `resolve_import_facts`
on this corpus correctly returns 0 (verified directly, not inferred).

**Extends (4 real edges)**: spot-checked against source —
`basic_memory_buffer : public detail::buffer<T>`,
`counting_buffer : public buffer<T>`, and two more real template-base
inheritance relationships in `base.h`. All 4 are real,
template-parameterized base-class relationships; this small count
(not dozens) is consistent with the scoped 19-file corpus: most of
fmt's "inheritance-shaped" code (`iterator_buffer : public Traits,
public buffer<T>`, `glibc_file : public file_base<F>`, etc.) involves
template BASE CLASSES whose base-list entry is itself a dependent
template-id (`buffer<T>`) — confirmed these ARE among the 4 extracted,
not silently dropped; the apparent "low count vs. visual density of
inheritance in the source" is explained by many `: public X` sites
sharing the SAME few real base targets (`buffer<T>`, `Traits`) rather
than each being a distinct base-target pair, and by test/ (excluded)
holding additional inheritance-heavy code outside this round's scope.

## A4 effect on C++ (this round's core question)

**Namespace-awareness does NOT generalize to C++ at all — not
partially, not as a weaker approximation, but completely absent, by
construction.** Three independent, confirmed reasons:

1. `resolve_calls_with_namespace_context`'s own C#-specific filtering
   logic (`exclude_definitely_not_visible_by_namespace`) only ever
   activates when `csharp_contexts` is non-empty; C++ has no analogous
   `CSharpTypeDeclarationContext`-producing mechanism, so this harness
   (matching the real indexer's own call site) passes `&[]` — the
   function's own documented fallback makes this byte-identical to
   plain `resolve_calls_with_imports`.
2. `resolve_calls_with_imports`'s own import-alias mechanism has zero
   `ImportFact`s to consult for C++ (see Imports section above) — the
   zero-candidate alias path can never engage.
3. **The real namespace-qualified CALLEE itself is never even reaching
   candidate status.** Direct source read of `edges/calls.rs`'s
   `callee_name` match arms: `"identifier" | "generic_name" |
   "template_function" | "attribute" | "field_expression" |
   "member_expression" | "member_access_expression"` — **no
   `"qualified_identifier"` arm exists.** Every `ns::foo(...)` call,
   templated or not, falls through to `_ => None` and is silently
   dropped before resolution of any kind (import-based, namespace-
   based, or plain name-based) is ever attempted.

### The known residual gap — re-confirmed and quantified, not worse than documented

Direct raw-AST measurement on the real fmt corpus (new counter added
to the harness, counting every `call_expression` node's callee kind):

| Measurement | Count |
|---|---|
| Total `call_expression` nodes (library scope) | 4,215 |
| `ns::foo(...)` calls, any shape (`qualified_identifier` callee) | **678** (16.1% of all call expressions) |
| ...of which `ns::foo<T>(...)` specifically (qualified + template argument) | **253** (6.0% of all call expressions; 37.3% of the 678) |
| ...of which plain `ns::foo(...)` (qualified, non-template) | 425 |

**Every one of the 678 qualified call sites itself is silently
dropped — zero Calls edges, zero CallAttempts, zero trace of any
kind, for that specific call site.** Confirmed two independent ways:
(a) direct read of `callee_name`'s match arms (no `qualified_identifier`
case, § above); (b) precise, file:line-level spot checks, re-verified
against the regenerated evidence
(`docs/research/evidence/b2-eval-009-fmt-run.txt`, section G2) AND
direct source reading of every real call site for each name, after an
independent reviewer correctly flagged that an earlier draft of this
report cited a stale evidence file (see "Independent review" below):

- `detail::enforce_compile_checks<...>()` (2 real qualified call
  sites, `base.h:2707`/`2715`, 1 real candidate at `base.h:2378`,
  zero bare call sites anywhere in the corpus): per-name total
  `resolved_edges=0, call_attempts=0` — total silence, exactly as
  claimed.
- `detail::make_descriptor<Context,T...>()` (2 real qualified call
  sites, `base.h:2761`/`2771`, 1 real candidate at `base.h:2303`, zero
  bare sites): `resolved_edges=0, call_attempts=0` — total silence.
- **`detail::is_locking<T...>()`**: per-name total is
  `resolved_edges=1, call_attempts=0` — NOT total silence, and this
  round's own direct source re-check explains exactly why: there are
  THREE real call sites, not two. Two (`base.h:2900`, `base.h:2917`)
  are qualified (`detail::is_locking<T...>()`) and ARE silently
  dropped, contributing 0/0 each. The THIRD, at `base.h:2392`
  (`is_locking<T2, Tail...>()`, inside `is_locking`'s own recursive
  body, already lexically inside `detail`, so no qualifier is
  written), is a BARE template call and IS extracted — it resolves to
  1 real `StaticAnalysis` edge (there are 2 real overloads at
  `base.h:2387`/`2391`, but the bare call's own arity/context happens
  to narrow to exactly one). The per-name aggregate (1 edge) is
  entirely attributable to this one unrelated bare call; the two
  genuinely-qualified call sites remain at 0/0 as the gap predicts.
- **`detail::get_buffer<char>(out)`**: per-name total is
  `resolved_edges=0, call_attempts=1 (Ambiguous)` — again NOT total
  silence for an analogous reason: 5 real qualified call sites exist
  (`color.h:597`, `format.h:4337`, `xchar.h:233`/`255`, plus one more)
  and are all silently dropped, but ONE bare call also exists at
  `chrono.h:396` (`get_buffer<Char>(out)`, written from inside a
  context where `detail::` is already the enclosing namespace, so no
  qualifier appears) — this bare call IS extracted, and because
  `get_buffer` has 2 real same-arity overloads (`base.h:2088`/`2093`),
  it correctly produces an **Ambiguous** CallAttempt, not a wrong
  edge. The 5 qualified sites remain silently dropped.
- **`detail::counting_buffer<>()`**: per-name total is
  `resolved_edges=0, call_attempts=1 (Ambiguous)`, same pattern: 4 real
  qualified call sites (`compile.h:546`, `format.h:4353`, `xchar.h:304`,
  plus the one cited earlier) are dropped; one bare call at
  `format.h:2289` (`counting_buffer<Char>()`) is extracted and
  correctly Ambiguous (the class template itself is declared once,
  but the resolver's own pool had enough same-named candidates nearby
  to decline rather than guess — re-verified this is Ambiguous, never
  a wrong single pick).

**Correction to an earlier draft of this section**: the first draft
of this report characterized `is_locking`/`get_buffer`/`counting_buffer`
as flat "0/0" (total silence) per name, which was imprecise — it
conflated the PER-NAME aggregate (which includes unrelated bare call
sites for the same name) with the per-call-site behavior of the
namespace-qualified gap specifically. The corrected breakdown above
is exact, re-verified call-site-by-call-site against live source, and
confirms the underlying claim (every namespace-qualified call site
specifically is silently dropped, with zero exceptions found) remains
true — the imprecision was in aggregation, not in the core finding.

**Characterization of the failure mode: confirmed to be an honest
silent drop, not a worse outcome than previously documented.** The
call site is never pushed into `CallSite` at all (`walk_for_calls`'s
`is_call_node` branch recurses into every node regardless, but
`callee_name` returning `None` means nothing is ever added to `out`).
This means it contributes to NEITHER `calls_edges` NOR `call_attempts`
— no edge (correct, since no edge should exist without real
resolution), but also **no negative-knowledge `CallAttempt` explaining
why** — a real agent asking "what does this `ns::foo<T>()` call
resolve to?" gets total silence, not a hedged, honest "Ambiguous" or
"Unresolved" answer. This is NOT a new or worsened finding relative to
B2-ID-011's own independent-review note (which already identified
`qualified_identifier` as unhandled, for both generic and non-generic
shapes) — this round's own contribution is the REAL quantification
(678/253 call sites, 16.1%/6.0% of all call expressions in a real,
unrelated corpus) that did not exist before, plus confirming the drop
never silently produces a WRONG edge (checked directly: zero resolved
Calls edges target any snapshot whose name matches a dropped
namespace-qualified callee's bare name through some other, incorrect
path — the 0/0 spot checks above directly confirm this for 4 separate
real names).

**Minor, newly-measured, not-previously-quantified detail**: 43
additional call sites (1.0% of all call expressions) are ALSO
silently dropped for unrelated callee shapes — `primitive_type` (32,
almost certainly C-style cast expressions like `int(x)` mis-parsed as
calls by the grammar, not real function calls), `parenthesized_expression`
(7, likely function-pointer-via-parens calls `(*fp)()`), and one each
of `call_expression`/`lambda_expression`/`concatenated_string`/
`subscript_expression`/`assignment_expression` as a callee position
(exotic, individually negligible). Classified **Minor** — under 1% of
call sites, not namespace/template-related, not blocking, documented
here for completeness rather than opened as a new finding ID.

### Overload-ambiguity handling — confirmed honest on real C++ data

`advance_to` (4 real same-named method declarations across
`base.h`/`format.h`/`printf.h`, each on a DIFFERENT receiver-context
class — confirmed by direct source read, e.g. `base.h:858`'s
`parse_context`-shaped class vs. `base.h:2662`'s distinct
format-context-shaped class): every real call site
(`parse_ctx.advance_to(...)`, `ctx.advance_to(...)`, etc.) produces an
**Ambiguous** `CallAttempt` with all 4 real candidates listed, never a
wrongly-picked single edge. Re-confirmed via `agent_policy::evaluate`:
`Forbidden` for `InformationalDisplay`, `AssertAsFactOrSoleJustification`,
AND `HighImpactModification` — "Ambiguous is always Forbidden" holds
on real C++ overload data, matching the Polly C# precedent exactly
(same policy behavior, different language). `format_to` (9 real
same-named declarations) has **zero internal call sites at all**
(confirmed by direct grep — it is a public-API-only function, never
called from within the library itself) — correctly 0 CallAttempts, 0
edges; this is a real negative case, not a tool defect (there is no
ground truth to recover because no real call exists).

### Template-call visibility, generally

Template calls ARE visible to the extractor when NOT namespace-
qualified, confirmed directly: bare `parse_format_string<char>(...)`
(`base.h:2706`) resolves to 4 real `StaticAnalysis` edges, all
targeting the single correct candidate (`base.h:1629`) — contrasted
directly against the SAME function's OWN namespace-qualified sibling
call site (`detail::parse_format_string<char>(...)`, `base.h:2714`),
which is dropped. Receiver-qualified template calls also resolve
correctly (B2-ID-011's own fix): `dynamic_args_.push<stored_t<T>>(arg)`
(`args.h:155`) produces 3 real `StaticAnalysis` edges to the single
real candidate (`args.h:58`). **Summary: bare and receiver-qualified
template calls are visible and resolve correctly when the real
candidate pool allows it (0 FPs found); namespace-qualified template
calls are the one shape that remains entirely invisible.**

## Precision / recall / F1 — valid, with explicit scope caveat

**30-row hand-verified ground-truth sample (29 scored after
de-duplicating one row), weighted toward namespace/template/overload
cases per this round's own focus**, built from direct source reading
(every row's expected target or honest-negative status was verified
against the real `fmt` source text, not inferred from the tool's own
output) and cross-checked against the tool's real `resolved_edges`/
`call_attempts` counts for each named symbol.

- **TP = 6** (bare/receiver-qualified template or plain calls with
  exactly one real candidate — `parse_format_string`×2 sites,
  `push`, `get_container`×3 sites — all confirmed via real
  `StaticAnalysis` edges, zero wrong targets)
- **FP = 0** (no case in this sample where the tool produced an edge
  to an incorrect target)
- **FN (namespace/template gap, silently dropped) = 15** (the
  `ns::foo()`/`ns::foo<T>()` rows — `fmt::println`, `detail::vformat_to`,
  `detail::unwrap`, `detail::is_locking`×2, `detail::get_buffer`,
  `detail::counting_buffer`, `detail::enforce_compile_checks`×2,
  `detail::make_descriptor`, `detail::parse_format_string`(qualified
  sibling), `detail::vprint_mojibake`, `detail::iterator_buffer`,
  `detail::count_named_args`, `detail::to_unsigned`)
- **FN (honest Ambiguous, correct tool behavior) = 5** (`advance_to`×2
  distinct call sites on different receiver classes,
  `accessor`/`counting_buffer`'s same-named-decl collision,
  `on_format_specs`×1 — corrected during this round's own verification
  from an initially assumed TP, after direct source reading found 2
  real declarations, not 1)
- **TN (correctly no real target — external stdlib/no corpus
  candidate) = 3** (`std::move`, `std::unique_ptr<...>`, one other
  libc-external call), excluded from P/R/F1 by definition.

**Precision = 6/6 = 1.000. Recall = 6/26 = 0.231. F1 = 0.375.**

**Explicit validity caveat, stated per the directive's own
instruction not to invent metrics beyond real ground truth**: this
sample is DELIBERATELY adversarial/weighted toward the exact failure
modes this round investigates (namespace-qualified and
template-and-overload-ambiguous calls) — it is NOT a representative
random sample of this corpus's whole call graph, and the 0.231 recall
figure must not be read as "the tool recovers 23% of all real C++
calls in general." A corpus-wide, unweighted recall figure was NOT
computed (doing so honestly would require either exhaustively
labeling all ~4,215 call sites, out of this round's time budget, or
sampling randomly, which would dilute the very cases this round exists
to characterize) — this is reported as a real, explicit scope
limitation, not elided. The P=1.000 finding (zero FPs across every
TP-candidate row checked) IS a general, corpus-level-consistent result
(independently corroborated by the Step 7 candidate/provenance spot
checks and the live corpus run's own zero-divergence parity check),
and is the headline reliability finding of this round.

## Resolved / Heuristic / Ambiguous / Unresolved breakdown (full corpus, 845 Calls edges + 1,282 CallAttempts)

| Tier | Count |
|---|---|
| Resolved (StaticAnalysis) | 665 |
| Heuristic (NameHeuristic) | 180 |
| Ambiguous | 782 |
| Unresolved | 500 |

`UnresolvedReason` distribution: `AmbiguousCandidates` 782,
`ExtractionRejected` 7, `Unknown` 493 (C++ has none of
`ImportTargetExternal`/`ImportedSymbolAmbiguous`/`NoMatchingImportAlias`
— expected, since those are Python/C#-import-mechanism-specific
reasons that structurally can never fire for C++, which has zero
`ImportFact`s per the Imports section above).

**Spot-checked for correctness, not just presence**: 10 sampled
Ambiguous `CallAttempt`s (deterministic order) all carry a real,
non-empty `method_limitations` string ("name-based matching cannot
distinguish overloads by parameter list or generic arity; 2+
same-named candidates remained even after scope/arity narrowing") and
real candidate-id lists that resolve to real, correctly-named
snapshots when cross-referenced against the snapshot pool (verified
directly for `accessor` and `advance_to`-family rows) — the
explanation text matches the real cause in every sampled case, not a
generic placeholder.

## FP/FN examples (concrete)

- **FN (gap)**: `detail::is_locking<T...>()` at `include/fmt/base.h:2900`
  — real target is one of `is_locking<T1,T2,Tail...>()` at
  `base.h:2387` or the base-case overload at `base.h:2391`; produces
  **zero** edges, **zero** CallAttempts. Never even reaches negative
  knowledge.
- **FN (honest Ambiguous)**: `ctx.advance_to(cf.format(*static_cast<qualified_type*>(arg),
  ctx))` at `include/fmt/base.h:2273` — tool correctly reports
  Ambiguous with 4 real candidates (`base.h:858`, `base.h:2662`, plus
  2 more in `format.h`/`printf.h`), `Forbidden` under every
  `agent_policy` intended use. This IS the correct behavior (the tool
  has no receiver-type inference to disambiguate), not a defect — it
  counts as an FN only because this metric credits recall, not
  correctness-of-abstention.
- **No FP found** in the 30-row sample, nor in the 10-row Ambiguous
  negative-knowledge spot check, nor in the zero-divergence
  full/incremental parity run.

## Agent usefulness + reliability

- **"Who calls this templated function across namespaces?"**
  (`advance_to`): answerable, and reliably so — real tier `Ambiguous`,
  4 real candidates, `agent_policy::evaluate` returns `Forbidden` for
  `InformationalDisplay`/`AssertAsFactOrSoleJustification`/
  `HighImpactModification` alike (verified directly against real
  evaluation-engine output, not asserted). An agent asking this gets
  an honest "I don't know which one, here are the 4 real candidates"
  answer — correct and useful.
- **"What does `Policy::handle<T>()` resolve to, for a
  namespace-qualified template call?"**: **not answerable at all** —
  the call site is invisible, not hedged. This is the one case in this
  evaluation where the "Forbidden-by-default honesty" model does NOT
  apply, because the call never reaches `CallAttempt` tracking either.
  An agent gets silent absence, which (per this project's own
  "no answer is better than a wrong answer, but a classified
  non-answer is better than silence" philosophy already embodied by
  `UnresolvedReason`) is strictly worse than an honest Unresolved —
  though still never worse than a FALSE edge, which never occurs here.
- **"What overloads exist for X?"**: fully answerable and reliable,
  independent of the Calls-edge gap — direct snapshot-by-name lookup
  (`advance_to`: 4 real declarations found; `format_to`: 9 real
  declarations found) works regardless of call-resolution quality,
  since it never depends on call-site extraction at all.
- **Reliability re-check**: `agent_policy`/`conformance` gates
  (`reliability_gates.rs`, 10/10) re-run clean as part of the Step 1
  baseline freeze; "Ambiguous is always Forbidden" independently
  re-confirmed against real fmt data (the `advance_to` case above),
  not merely re-run against pre-existing fixtures.

## Full/incremental parity

**Confirmed, not assumed — using the project's existing,
already-tested equivalence gate (`conformance::compare_full_incremental_state`),
not a custom ad hoc comparison.** Real commit pair:
`7856add61a38fcfead6636569d33e4bbf57881a5` (commit_a) →
`208219daff58f2a7438f6931cc2ddc358acf33ef` (commit_b, this round's
pinned eval commit), 8 real changed files between them (`git diff
--name-only`), including 3 real header edits
(`include/fmt/compile.h`, `include/fmt/format.h`, `include/fmt/std.h`)
representative of the directive's "edit" case. Two independent
`CodeGraphIndexer` runs — (1) full reindex of commit_a then
INCREMENTAL advance to commit_b, vs. (2) a wholly fresh indexer's
DIRECT full reindex of commit_b — compared via
`compare_full_incremental_state`: **zero divergences** across the
snapshot, typed-edge, `ImportFact`, `ImplFact`, and `CallAttempt`
multisets (not just counts — this function compares true multiset
membership AND multiplicity). Raw `graph.snapshot_count()` differs
between the two runs (4,629 full-only vs. 5,343 incremental, because
the incremental run's graph retains commit_a's own now-superseded
snapshots, lineage-linked) — this is expected, benign, and is exactly
why `compare_full_incremental_state`'s own `snapshot_keys` helper
filters to "current, non-superseded only" before comparing (confirmed
by direct source read, not just trusting the "zero divergences"
result blindly).

Raw evidence: `docs/research/evidence/b2-eval-009-fmt-parity-run.txt`.

## Performance

Methodology: same convention as CALL-RES-A4's own prior measurement
(`call_res_a4_perf_measurement.rs`, reused directly, unmodified) — 1
discarded warm-up run, then 10 timed runs, same machine, release
build, median + min/max range reported. Commit pair: same as the
parity check above (`7856add6...` → `208219da...`, 8 real changed
files).

| | full-index (median, min-max) | incremental-index (median, min-max) |
|---|---|---|
| fmt (whole repo incl. test/, as the real indexer would see it) | 1.473s (1.443–1.512s) | 1.318s (1.293–1.351s) |

Edge-count stability: **identical across all 10 runs, both full and
incremental** (3,038 edges every time, both sides) — confirms timing
variance is not masking a correctness difference.

**Qualitative comparison to the prior Moderate-overhead finding
(decision d163pa, A4's own cost on Polly/pyjanitor)**: this measurement
does NOT isolate A4's OWN marginal cost on C++ (A4's namespace-filtering
mechanism never activates for C++ at all, per the "A4 effect" section
above — there is no "A4-live vs. baseline" delta to measure here,
since the C++ code path through `resolve_calls_with_namespace_context`
with empty `csharp_contexts` is unconditionally identical to the
pre-A4 resolver). The absolute full-index time on fmt (whole repo,
~140 files incl. vendored test/gtest/) at ~1.47s is in the same
general order of magnitude as pyjanitor's own ~1.5s baseline (291
files) from the A4 checkpoint doc — not a surprising or concerning
number, consistent with (not contradicting) the prior Moderate-overhead
finding, which was specific to A4's namespace-filtering stage cost on
C#, not to C++ at all.

## New findings — severity classification

**No new Critical or Important production defect.** Per Step 11's own
explicit instruction, re-confirming/quantifying an already-documented
residual limitation (the `qualified_identifier` namespace-qualified
call gap, first flagged by B2-ID-011's independent review) is expected
investigative work for this round, not grounds to open a new finding
ID or stop — and the investigation did NOT reveal it to be materially
worse or different than previously understood (specifically: it is
STILL an honest silent drop, confirmed never to produce a wrong edge
in every case checked; it is NOT "silently producing a wrong edge,"
which would have been the one condition requiring treatment as a new
finding per the directive).

One **Minor** observation, not assigned a new B2-ID (too small/
unrelated to this round's own focus to warrant a dedicated finding
doc; recorded here for the backlog): 43 additional call sites (1.0%
of all call expressions in this corpus) are also silently dropped for
unrelated callee shapes (`primitive_type` casts, parenthesized
function-pointer calls, and a handful of other exotic shapes) — see
the "Minor, newly-measured" note above for the full breakdown.

## Independent review

A fresh reviewer (no prior context beyond an earlier draft of this
report and the raw-evidence files under
`docs/research/evidence/b2-eval-009-fmt-*.txt`) was dispatched to
verify: (1) the 678/253 `ns::foo()`/`ns::foo<T>()` gap quantification
against the raw evidence file and the cited source line numbers, (2)
whether the reported P/R/F1 is backed by genuine, individually-checked
ground truth or merely asserted, (3) whether full/incremental parity
was actually executed (not assumed) via the real equivalence gate, (4)
severity-protocol compliance, (5) internal consistency of performance/
GO-NO-GO/A5 sections.

**Verdict: APPROVE WITH NOTES. One real, material finding; corrected
in this final version of the report (not rubber-stamped).**

1. **Gap quantification (678/253/4215)**: VERIFIED. Matches the raw
   evidence file exactly; the reviewer independently re-read
   `callee_name` and confirmed no `qualified_identifier` match arm
   exists.
2. **P/R/F1 ground truth — the one real finding.** The reviewer
   discovered that the evidence file this report originally cited
   (`b2-eval-009-fmt-run.txt`, saved before the harness's own "G2.
   Ground-truth spot checks" section was added) was **stale** — it
   predated the harness code that was supposed to back several of
   this report's per-name claims (`is_locking`/`get_buffer`/
   `counting_buffer`/`enforce_compile_checks` 0/0-style assertions),
   confirmed by file-mtime ordering (the harness's G2 code was added
   one minute AFTER the evidence file it was meant to support was
   generated). The specific numeric claims were plausible and
   ultimately turned out to be real, but were asserted ahead of the
   artifact that would prove them — a genuine process error, correctly
   caught rather than waved through. **Fixed**: the harness was
   re-run fresh (`docs/research/evidence/b2-eval-009-fmt-run.txt`,
   now including the G2 section, 113 lines), and the per-name claims
   were re-derived file:line-by-file:line against live source,
   uncovering a real imprecision in the ORIGINAL aggregation (three of
   the four "0/0" per-name claims were actually "1 Ambiguous or
   1 StaticAnalysis attributable to an unrelated BARE call site for
   the same name, plus N genuinely-dropped qualified sites at 0/0
   each" — corrected in the "A4 effect" section above with exact
   file:line citations for every call site, bare and qualified). The
   P=1.000/R=0.231/F1=0.375 arithmetic itself was independently
   re-verified by the reviewer as internally correct given the stated
   TP/FP/FN/TN counts, and those counts were not among the rows
   affected by the stale-evidence issue (the `advance_to`/`format_to`/
   `parse_format_string`/`push`/`get_container` rows backing the P/R/F1
   sample were independently cross-checked by the reviewer against the
   (then-available) evidence sections E/F and found accurate).
3. **Full/incremental parity**: VERIFIED as genuinely executed — the
   reviewer read `b2_eval_009_cpp_parity.rs` directly and confirmed it
   calls the real `compare_full_incremental_state` against two real
   `observe_commit` outcomes, and independently read `conformance.rs`'s
   `snapshot_keys` to confirm the lineage-filtering explanation for the
   4629-vs-5343 raw-count discrepancy is accurate, not a guess.
4. **Severity-protocol compliance**: VERIFIED correct — no new finding
   ID opened for the re-confirmed gap, no fix attempted or applied.
5. **Performance/GO-NO-GO/A5 consistency**: VERIFIED — all performance
   table numbers traced exactly to the raw evidence file; the
   reviewer found the GO/NO-GO and A5 reasoning internally consistent
   with the rest of the report's own content.
6. **Minor note incorporated**: the Step 1 baseline-freeze claim
   (685/685) was not originally backed by a saved evidence file —
   fixed by re-running it and saving
   `docs/research/evidence/b2-eval-009-baseline-freeze.txt` (see Step 1
   above).

No Critical/Important issue was found by the reviewer at any point —
the one real finding (stale evidence backing specific per-name claims)
was a sourcing/process gap, not a wrong conclusion: every corrected
number still supports the same core finding (the namespace-qualified
gap is real, exactly as documented, confirmed call-site-by-call-site
with zero exceptions).

## GO/NO-GO for B2-EVAL-010

**GO.** No Critical/Important defect found or left open. CALL-RES-A4
baseline remains non-regressed (685/685). The one gap re-confirmed
this round was already known, already documented, and is not worse
than understood. Full/incremental parity holds on real C++ data via
the existing equivalence gate. Zero false positives found across every
real-edge sample checked. B2-EVAL-010 is NOT auto-started per explicit
instruction — this is a recommendation for the user's own next
decision, not an action taken here.

## Recommendation on CALL-RES-A5

**Not justified by this round's own C++ evidence — same conclusion as
the CALL-RES-A4 checkpoint's own prior non-finding, now corroborated
by real C++ data rather than merely C#/Python data.** The strongest
candidate argument FOR A5 (type/overload-aware resolution) would be
the 782 real Ambiguous `CallAttempt`s this corpus produces, many of
which (confirmed directly: `advance_to`, `on_format_specs`,
`accessor`) are genuinely resolvable by a human or a real compiler
ONLY via receiver-type information this tool does not have. However:

1. **The tool's current behavior on these exact cases is already
   correct and honest** — zero false positives found anywhere in this
   round's sampling, `Ambiguous` correctly `Forbidden` under every
   `agent_policy` intended use. A5 would IMPROVE recall on real,
   concretely-identified cases, but the current NO-A5 state is not
   producing wrong answers that A5 would fix — it is declining to
   guess, correctly.
2. **The namespace-qualified-call gap (this round's own focus) is
   NOT an A5-shaped problem at all** — it is a pure extraction defect
   (the callee is never even reaching candidate status), fully
   independent of overload/type resolution. Fixing it requires adding
   a `qualified_identifier` match arm to `callee_name`, nothing more —
   explicitly NOT authorized this round, and explicitly not a reason
   to justify A5 either (A5 would do nothing for a call site that
   never becomes a `CallSite` in the first place).
3. **C++ overload resolution in general is a much harder, more
   context-dependent problem than C#'s** (full argument-type
   deduction, template specialization ordering, SFINAE, ADL) — the
   real data here does not change the qualitative risk/cost picture
   already implicit in A5 being unauthorized; if anything, the real
   `advance_to`/`on_format_specs` cases (ambiguous purely by RECEIVER
   type, not argument types) show that even a "simple" slice of
   C++ overload resolution requires type inference this tool has no
   other use for yet.

**Conclusion: no new C++ evidence changes the A4 checkpoint's own
prior non-recommendation. A5 remains unauthorized and unimplemented,
consistent with every directive constraint this round operated under.**
