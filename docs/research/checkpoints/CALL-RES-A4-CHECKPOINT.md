# CALL-RES-A4 Checkpoint — Import/Namespace/Visibility-aware Resolution

**Status:** PARTIAL (continuation pass) — the core deterministic
import-alias resolution mechanism (Python) AND a new C# namespace-
aware candidate-filtering mechanism are both now implemented, tested,
and mutation-closed with zero regressions. An explicit three-way
visibility model as a DISTINCT new type, oracle/ground-truth
extension, full multi-corpus evaluation metrics, `agent_policy.rs`/
`conformance.rs` re-checks, and performance measurement remain **not
reached** and are documented below as deferred, not silently dropped.
C++/TS namespace awareness is documented (not implemented) per the
directive's own instruction to defer honestly rather than fake
support.

**Authorized by:** the CALL-RES-A4 directive, following the prior
Semantic Resolution & Reliability Checkpoint's completion. This
document covers TWO passes: the first pass (Python alias mechanism,
commits `d68cfe9`/`299772b`/`3d12a62`) and this continuation pass (C#
namespace-aware filtering, commits `068a7de`/`f327139`). Relayed
mid-task by the coordinator both times, with explicit instruction to
continue in order, commit frequently (this being the seventh-plus
rate-limit/turn-limit interruption across this hardening effort), and
report honestly whether everything completes. This continuation pass
completed its own assigned sections 1-5 (namespace awareness, the
3-outcome shape already covering the visibility distinction,
candidate-filtering provenance via the existing `Provenance` bounds,
required negative tests, and mutation closure) but did NOT reach
oracle/ground-truth extension, full multi-corpus metrics, policy/
conformance re-checks, or performance measurement — reported honestly
below rather than claimed.

**Branch/worktree:** `b2-code-graph`,
`C:\Users\fsave\Desktop\Python_code\SuperBrain\.worktrees\b2-code-graph`.
No merge, tag, or push performed. B2-EVAL-009 not started. Task 10
untouched throughout. `B2-EVAL-008.md` not touched.

**Commits:**
- `d68cfe9` — feat(calls): CALL-RES-A4 core — import-alias-aware
  zero-candidate resolution. `resolve_calls_with_imports`,
  `ImportAliasOutcome`, `resolve_via_import_alias`, 3 new
  `UnresolvedReason` variants.
- `299772b` — feat(calls,imports): CALL-RES-A4 negative tests + fix
  Python relative-import extraction. The 10 required negative tests,
  CRC-001/002/003 regression checks, and a real pre-existing bug fix
  in `imports.rs`.
- `068a7de` — feat(calls): CALL-RES-A4 — C# namespace-aware candidate
  filtering. `resolve_calls_with_namespace_context`, reusing
  `CSharpTypeDeclarationContext` (B2-ID-010) and bare `using`
  `ImportFact`s to exclude candidates whose namespace is definitely
  not visible, on the `>= 2` ambiguity branch only.
- `f327139` — test(calls): closes 10 genuine-missed mutants the new
  namespace-filtering code introduced.

## What was implemented this continuation pass — C# namespace awareness

`resolve_calls_with_namespace_context(snapshots, signature_too_long_names,
import_facts, csharp_contexts)`: identical to `resolve_calls_with_imports`
in every way, except that when 2+ real candidates remain (after the
existing arity-narrowing step) for a C# call site, it additionally
excludes any candidate whose own containing-type namespace is
DEFINITELY not visible from the caller's position.

**How a candidate's own namespace is determined**: `enclosing_class_of`
(the pre-existing Task-9.5 helper) finds the candidate's structurally
containing `Class`, then a `(file, span)` lookup into
`csharp_contexts` (`CSharpTypeDeclarationContext`, B2-ID-010's own
structural fact) returns that class's own canonicalized namespace
string. If either step fails, the namespace is `None` — **never**
collapsed into "not visible."

**How "visible" is determined**: a candidate's own namespace is
visible if it equals the caller's OWN namespace (also derived via the
same two-step lookup, so a caller needs no `using` directive to see
candidates in its own namespace), OR it appears in
`bare_using_namespaces_in_file` — every `ImportFact` for the caller's
own file whose `alias` is `None` (a BARE `using Namespace;`
directive; an ALIASED `using Foo = X;` is a symbol alias and stays
exclusively in the pre-existing zero-candidate alias path, never
double-counted here).

**Three-way visibility, already satisfied by this shape**: the
directive's own §6 asked for an explicit visible/definitely-not-
visible/unknown distinction. This pass did not introduce a FOURTH,
separately-named type for it — the mechanism above already embodies
exactly that three-way split: a determined-and-matching namespace is
visible (candidate survives filtering); a determined-and-non-matching
namespace is definitely-not-visible (candidate is excluded); an
undetermined namespace (`None`) is unknown (candidate is NEVER
excluded, confirmed by a dedicated test). Introducing a separate
`Visibility` enum on top of this would duplicate, not clarify, an
already-working three-way discrimination — judged out of scope as a
result, consistent with this project's own "don't duplicate an
existing concept" discipline (see `negative_knowledge.rs`'s own doc
comment on the identical judgment call for `UnresolvedReason`).

**Never invents a target**: filtering only ever REMOVES candidates;
if filtering would remove every real candidate (none determined-
visible, all determined-elsewhere), the ORIGINAL unfiltered set is
returned instead of manufacturing a false zero-candidate outcome —
confirmed directly by `using_an_unrelated_third_namespace_never_manufactures_a_false_resolution`.

**Provenance (§7)**: no new provenance mechanism was built beyond
what `Provenance::new`'s own existing bounds (64 candidates, 512-byte
rationale, from Hardening Phase 2) already provide — every `Edge`
this mechanism produces carries a `NameHeuristic` evidence note
naming the exact reason ("resolves to exactly one candidate once
candidates outside the caller's own or any used namespace are
excluded"), and every `CallAttempt` for a still-ambiguous case
preserves the full real candidate-id list via the pre-existing
`CallAttempt::ambiguous` constructor. A dedicated richer "which
candidates were excluded and why" provenance field was judged
unnecessary given the existing bounded-text evidence note already
names the mechanism; not implemented as a distinct enhancement this
pass.

### C++ and TypeScript/JS namespace/module awareness — documented, not implemented

Per the directive's own instruction to document rather than fake
support: this pass did not implement namespace/module-visibility
filtering for C++ or TS/JS. For C++, this is consistent with
B2-ID-011's own independent-review finding that C++ namespace-
qualified calls (`ns::Foo<int>()`) are a pre-existing residual
limitation, unrelated to and not resolved by this pass. For TS/JS,
`ImportFact` extraction already distinguishes named/default/aliased
imports (confirmed in `imports.rs`'s own `collect_js_import`), so the
SAME zero-candidate alias mechanism `resolve_calls_with_imports`
already provides should already recover a TS/JS aliased-import case
identically to Python's — not separately re-tested this pass (no new
mechanism was needed; the existing Python tests already prove the
language-agnostic alias path works for any language whose `ImportFact`
extraction populates `alias`). A genuine module-visibility filter
analogous to this pass's own C# namespace work (narrowing a `>= 2`
ambiguity by which modules are actually imported) was NOT built for
TS/JS — deferred, not implemented.

## Mutation testing (this continuation pass) — CLOSED

`cargo mutants` scoped to the 4 new functions
(`namespace_of_snapshot`, `bare_using_namespaces_in_file`,
`exclude_definitely_not_visible_by_namespace`,
`resolve_calls_with_imports_and_namespaces`) found **35 total
mutants**. Final, clean, from-scratch run (one pass, no interruption):
**25 caught, 7 unviable, 3 genuine-equivalent (documented below, not
missed-and-unaddressed), 0 real gaps.**

Getting to this required two extra rounds of test-fixture debugging
beyond the first two commits (`f327139`, `5bf1a2b`), each one caught
by re-running the mutation sweep clean rather than trusting a test's
own "passing" status:

- **622:67 (`+` → `-`/`*`, absolute call-site position)**: the
  original closing test used two SIBLING C# methods, so
  `call_site_belongs_to_a_more_nested_caller` has no nested-caller
  candidate at all and returns `false` regardless of the position
  arithmetic — unobservable by construction. Fixed by reusing the
  proven B2-ID-006 nested-function shape (Python; this codebase has no
  C# local-function extraction) in commit `347c063`.
- **631:37 (match guard `receiver_field_is_a_known_member` →
  `true`/`false`)**: THREE further fixture attempts each failed for a
  different reason (commit `6ecf855`, then `84f9f97`): (1) every C#
  fixture is structurally incapable of ever killing this mutant —
  `SymbolKind::Field` is produced only from Python's
  `self_attribute_assignment` synthetic kind hint, confirmed directly
  in `kind_mapping.rs` (`map_kind_hint("field_declaration").is_err()`
  for C#'s real field-declaration node) — C# fields are never
  extracted as snapshots at all; (2) a Python fixture with the real
  method on the receiver field's DECLARED TYPE rather than the
  caller's own class exposed that `restrict_to_callers_own_class`
  restricts to the CALLER's own class, not the field's type — the
  fixture's real, unmutated code path was itself broken; (3) even with
  the real method correctly on the caller's own class,
  `narrow_by_scope`'s own class-level check (run unconditionally right
  after, independent of the guard) performs the identical
  `strictly_contains` filter, so the two paths converge on the same
  edge TARGET whenever a real in-class match exists — only their
  EVIDENCE differs (`StaticAnalysis` vs. `NameHeuristic`/`Speculative`),
  closing the `false` direction once asserted on `evidence.method()`.
  The `true` direction needed a wholly separate second scenario
  (`self.other_obj.call()` where `other_obj` is never actually
  assigned as a field) to exercise a case where the real guard
  genuinely evaluates `false`. Both directions, and all three prior
  dead ends, were confirmed by manually applying each mutant to the
  source and re-running the test directly — not by inference.

**672:50 (`&&` → `||`), 694:73 (`<` → `<=`), 697:45 (`&&` → `||`) are
genuine EQUIVALENT mutants, not real test gaps.** By the time control
reaches line 693, `candidates.len() >= 2` is a guaranteed invariant —
both `candidates.len() == 1` escape hatches earlier in the same loop
iteration (line 637, and the generic-arity-narrowed check two lines
above 672) already `continue`d for any smaller count. Working through
each one's own two operands against that invariant:

- **672**: `narrow_candidates_by_call_site_arity` returns its input
  UNCHANGED when `call_site.generic_arity` is `None` (confirmed by
  reading its own early return). So when the left operand
  (`generic_arity.is_some()`) is `false`, `arity_narrowed_candidates`
  is just `candidates` again, whose length is `>= 2` by the invariant
  above — meaning the right operand (`== 1`) is ALSO always `false` in
  that case. `&&` and `||` therefore always agree.
- **694**: `namespace_filtered.len() <= candidates.len()` always holds
  (filtering only shrinks or preserves the set), so mutating `<` to
  `<=` can only ever add the one case `namespace_filtered.len() ==
  candidates.len()` (nothing was excluded) to `namespace_narrowed_this_call`'s
  `true` branch — but per 697's own reasoning below, that branch's
  truth only matters together with `candidates.len() == 1`, which is
  unreachable whenever nothing was filtered out of a `>= 2` starting
  set.
- **697**: given the invariant, `candidates.len() == 1` (the right
  operand) can ONLY become true if `exclude_definitely_not_visible_by_namespace`
  actually removed at least one candidate from a starting set of
  `>= 2` — which is exactly what `namespace_narrowed_this_call` (the
  left operand) already asserts. The right operand being true
  therefore implies the left operand is already true, making `&&` and
  `||` observationally identical.

All three were independently verified two ways before being accepted
as equivalent, not just reasoned about: (1) manually editing the
mutant into the source and re-running the full `call_res_a4_namespace`
module (12/12 still pass) AND the full lib suite (594/594 still pass)
for each of the three, one at a time; (2) the final clean
`cargo mutants` run itself reporting exactly these three and no others
as `MISSED` after the 622/631 fixes landed, with zero timeouts and
zero other surprises. No further tests were written to chase these —
per this checkpoint's own §16 instruction to classify
equivalent/unviable mutants honestly with specific reasoning rather
than writing unwinnable tests against them.

## What was implemented (Python only)

A4's deterministic mechanism: when a call site's callee name matches
**zero** candidates by direct name lookup (the `candidates.is_empty()`
branch only — structurally unreachable from the `>=2` ambiguity branch
CRC-001/CRC-002 occupy), check whether an `ImportFact` from the same
file has an `alias` exactly matching the callee name. Three outcomes,
mirroring `ImportResolution`'s own shape:

- **Internal(target_id)** → re-confirm `target_id` still exists in the
  given snapshot slice, then emit a `NameHeuristic` edge directly to
  it. No re-scanning by name — the alias fact already names the exact
  target.
- **External** → `CallAttempt::unresolved_import_target_external`, a
  more specific, honest claim than plain `Unknown`.
- **Ambiguous** → `CallAttempt::ambiguous_import_target`, re-deriving
  the real candidate-id list by the same lookup convention
  `resolve_raw_import_statements` uses (since `ImportResolution::Ambiguous`
  itself carries no ids).

If no import fact in the caller's file has a matching alias at all,
the fallback is `CallAttempt::unresolved_no_matching_import_alias`
(when `import_facts` was genuinely non-empty for the call) rather than
the old generic `Unknown` (reserved for when A4 was never given any
import data at all) — distinguishing "checked and found nothing" from
"never checked."

`resolve_calls_with_extraction_rejections` is now a thin wrapper over
`resolve_calls_with_imports(..., &[])` — byte-for-byte identical
behavior for every existing call site, confirmed by a dedicated
equivalence test (required negative test case 10).

### Real pre-existing bug found and fixed (byproduct of this work)

`imports.rs`'s `collect_python_import` silently dropped **every**
Python relative import (`from .ml import X`, `from ..pkg import Y`) —
the module-path search only recognized a direct `dotted_name` child of
`import_from_statement`, but a relative import wraps its module path
inside a `relative_import` node instead. This is the exact shape
CRC-003 documents. Before this fix, CRC-003's own recovery was
unreachable because the import statement itself produced zero
`ImportFact`s. Fixed by checking for `relative_import` first,
unconditionally; a bare `from . import X` (no name after the dots)
falls back to the dot prefix itself as `module_path`, never
fabricated. Two iterations were needed — the first attempt had the
check order backwards and was caught by this same commit's own test
suite before being shipped (see `imports.rs`'s own doc comment for the
full account).

## CRC-001/002/003 regression report

- **CRC-003 (import-alias blindness): RECOVERED**, with a genuine new
  finding. `case_1_correct_import_alias_resolves_cleanly_when_unambiguous`
  proves the mechanism recovers a clean alias call when the real
  target name is unambiguous. `crc_003_exact_shape_recovers_only_to_ambiguous_due_to_a_real_name_collision`
  reproduces the *exact* real corpus shape (the wrapper function in
  `janitor/__init__.py` shares its own bare name with the real
  target) and confirms, directly rather than assumed from the CRC-003
  doc's prose, that `resolve_raw_import_statements`'s lack of a
  same-file/self exclusion makes the import fact itself resolve
  `Ambiguous`, not `Internal`. A4 correctly reports this as
  `ImportedSymbolAmbiguous` — the alias mechanism works; the real
  corpus has a second, independent ambiguity A4 does not (and should
  not) paper over.
- **CRC-001 (ambiguous unqualified constructor call): NO REGRESSION.**
  `crc_001_ambiguous_constructor_call_regression_check` reproduces the
  real shape with a real, unrelated import fact present in the same
  file and confirms the call stays honestly `Ambiguous` — A4's
  zero-candidate-only scope never touches the `>=2` branch this
  challenge occupies.
- **CRC-002 (local-binding shadowing): NO REGRESSION**, proven
  byte-for-byte. `crc_002_local_binding_shadowing_regression_check`
  asserts `resolve_calls_with_extraction_rejections` and
  `resolve_calls_with_imports` (with real import facts for the same
  file) produce **identical** edge output for the real CRC-002 shape.
- **NO-GO condition not triggered.** CRC-003 was recovered without any
  new FP on CRC-001 or CRC-002 — this checkpoint is a net improvement,
  not a net-negative tradeoff.

## The 10 required negative tests — all present, all passing

All 10 live in `crates/codegraph-index/src/edges/calls.rs`'s
`call_res_a4_import_alias` test module, using real tree-sitter
extraction and real `resolve_import_facts` output (never hand-built
`ImportFact`s):

1. Correct alias resolves cleanly (split into a clean-mechanism test
   and the faithful CRC-003-shape test — see above)
2. Same name not imported → excluded, resolves by the pre-existing
   bare-name path with no import influence
3. Wildcard import → stays conservatively `Unknown` (see residual
   limitation below)
4. Re-export to a target outside the given pool → never invents a
   target, reports `ImportTargetExternal`
5. Namespace/module-level alias (`import pkg.mod as entry`) resolves
   via the same mechanism
6. Multiple distinct imports in one file, each resolving independently
7. Unresolved external import (`import torch as t`) →
   `ImportTargetExternal`
8. Local symbol shadows an imported symbol of the same name (CRC-002
   territory) — A4 never runs; pre-existing order preserved
9. Nested/local parameter shadowing — confirmed A4 has no awareness of
   it (pre-existing CRC-002 scope, not a new regression)
10. No import information at all → byte-for-byte identical to the
    pre-A4 baseline

## Independent review (performed directly, sequentially, within this fork)

No sub-agent dispatch was available within this forked context, per
the coordinator's own fallback instruction; the review was performed
directly against the full diff between the prior checkpoint commit
and `299772b` (896 lines across `calls.rs`, `imports.rs`,
`negative_knowledge.rs`, and the smoke-test harness).

**Findings:** none Critical or Important. The `imports.rs` fix is
structurally correct (checks `relative_import` first, never
fabricates a module path for a bare `from .`). The `resolve_via_import_alias`
mechanism matches its own doc comments exactly: defensive
re-confirmation of `Internal` targets against the actual snapshot
slice given to *this* call (never trusting a cross-call invariant),
correct re-derivation of the `Ambiguous` candidate set using the same
lookup convention as `resolve_raw_import_statements`, and no path by
which the `>=2`-candidate branch can be reached from the new code. All
10 required negative tests and both CRC regression tests match their
own documented real-world shapes rather than synthetic approximations.
Two residual limitations are already honestly flagged in the test
suite itself (not new findings, just confirmed, not newly discovered
during review):

- **Per-call, not per-file, "was import evidence checked" signal.**
  When `import_facts` for the whole resolution call happens to be
  empty (e.g. a file containing only a wildcard import, which
  extracts zero facts), the fallback reason degrades to the old
  `Unknown` rather than `NoMatchingImportAlias`, because the check is
  `import_facts.is_empty()` globally, not "does any `ImportFact` exist
  for this caller's own file." A per-file-aware signal would recover
  the more specific reason here. Not implemented this pass.
- **No local-binding/parameter-shadowing awareness.** A4 operates
  purely on callee names matched against `ImportFact.alias`; it has no
  visibility into local variable or parameter bindings that might
  shadow an alias. This is pre-existing, CRC-002's own documented
  territory, confirmed unchanged (not worsened) by this pass via
  `case_9`.

**Verdict: APPROVE** the implemented portion for its stated, narrow
scope (Python alias-based zero-candidate resolution). No fix applied
during review — no defect rose to that bar.

## Explicitly deferred (not reached across either pass)

Reported honestly per the directive's own instruction, rather than
omitted. Items resolved by this continuation pass are struck through
in spirit (moved to the sections above) rather than left ambiguous:

- ~~Import-visibility candidate filtering beyond alias matching~~
  (§4) — **done this pass** for C# via namespace evidence (not via
  `imported_symbol`/`module_path` similarity, which the directive
  itself never asked for and this project's own discipline forbids as
  a name-similarity guess). Python's own `>= 2` branch still has no
  equivalent filter — Python's import model has no namespace concept
  analogous to C#'s, so there is no obviously-equivalent mechanism to
  build without inventing one; left open rather than forced.
- ~~Namespace awareness for C#~~ (§5) — **done this pass.** C++
  documented as a pre-existing, unrelated limitation (B2-ID-011).
  TS/JS: the existing alias mechanism already generalizes (not
  separately re-tested); a genuine module-visibility filter analogous
  to the C# one was NOT built — still open.
- ~~Three-way visibility model~~ (§6) — **judged already satisfied**
  by the existing mechanism's own structural shape (visible/
  definitely-not-visible/unknown), not by introducing a duplicate,
  separately-named type. See this pass's own "What was implemented"
  section above for the reasoning.
- ~~Bounded candidate-filtering provenance~~ (§7) — **judged already
  satisfied** by the existing `Provenance::new` bounds plus the
  descriptive `NameHeuristic` evidence text this pass's own edges
  carry. A distinct, richer "which candidates were excluded and why"
  structured field was not built — judged unnecessary, not merely
  skipped; open to revisiting if a future consumer needs it.
- **Oracle/ground-truth extension** (§10) — still not touched. The
  Roslyn differential suite was not extended to the new C# namespace
  cases this pass added.
- **Full evaluation-metrics pass** (§1, §13) — still not done. No
  baseline-freeze re-run against the real Polly corpus with this
  pass's own namespace mechanism wired in; candidate-reduction counts,
  precision/recall/F1, and "ambiguities resolved without type
  information" remain unmeasured at the corpus level (only measured
  directly, per-test, on the synthetic fixtures this pass's own tests
  construct).
- **Reliability-policy re-checks** (`agent_policy.rs`, §14) — still
  not run against either pass's new code.
- **Full/incremental parity extension** (`conformance.rs`, §15) —
  still not done. The multiset gate has not been extended to cover
  import/namespace-related evidence or any of the 6 named adversarial
  cases for EITHER pass's own mechanism.
- ~~Mutation testing completion~~ (§16) — **CLOSED** for this pass's
  own new code (namespace filtering): a final, clean, from-scratch,
  single-pass run confirms 25 caught / 7 unviable / 3 genuine-
  equivalent (documented with specific reasoning) / 0 real gaps across
  all 35 targeted mutants. See "Mutation testing" section above for
  the full account, including two rounds of test-fixture bugs this
  verification itself caught and fixed. The FIRST pass's own Python
  mechanism (`resolve_via_import_alias`, `ImportAliasOutcome`,
  `collect_python_import`'s relative-import fix) has STILL never had
  mutation testing run against it at all — remains open.
- **Performance measurement** (§17) — still not measured for either
  pass's own mechanism.
- **CALL-RES-A5** (§18) — correctly not implemented in either pass,
  per explicit directive exclusion.

## Recommendation

**GO** for merging both passes' work as a discrete, net-positive
increment: Python alias resolution (CRC-003 recovered, zero
regressions, one real pre-existing bug fixed) plus C# namespace-aware
filtering (a genuine new mechanism, 12 tests including a real
cross-namespace ambiguity recovery, a CONFIRMED-CLEAN mutation gate —
25/35 caught, 7/35 unviable, 3/35 genuine-equivalent with documented
reasoning, 0 real gaps — zero regressions on CRC-001/002/003 or the
full 594-test lib suite) — subject to the remaining directive sections below being picked up in a
follow-up pass before CALL-RES-A4 itself is declared fully complete.

**NO-GO** on treating CALL-RES-A4 as closed. Oracle/ground-truth
extension, full corpus-level evaluation metrics, policy/conformance
re-checks, full/incremental parity extension, final mutation-run
confirmation, performance measurement, Python candidate-filtering
beyond alias matching, and TS/JS module-visibility filtering all
remain open.

**NO-GO** on resuming B2-EVAL-009 until CALL-RES-A4 is either
completed or explicitly descoped by the user/coordinator — consistent
with the standing instruction not to start B2-EVAL-009 as part of this
checkpoint.

**GO/NO-GO on CALL-RES-A5:** not evaluated — A5 was explicitly out of
scope for both passes and no case surfaced during either pass's own
work that specifically required it beyond what CRC-003's real-shape
finding already documents as its own independent ambiguity (not an
A5-shaped gap), and beyond the already-separately-tracked generic-
arity-collision gap (B2-ID-010's own closing report) that remains
Moderate backlog, untouched by this checkpoint.
