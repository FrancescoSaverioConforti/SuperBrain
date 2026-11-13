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
- ~~Oracle/ground-truth extension~~ (§10) — **done in a later pass**:
  the Roslyn differential suite was extended to 5 real C# cases
  (namespace alias, qualified name, real ambiguity, plus the existing
  same-name/imported-namespace coverage), 3/3 new call-site comparisons
  agree with Roslyn, 0 divergences. See `call-res-a4-namespace-roslyn-differential.jsonl`
  and `roslyn_differential_suite.rs`.
- ~~Full evaluation-metrics pass~~ (§1, §13) — **done in this pass**.
  See "Final multi-corpus metrics" section below for the full real
  before/after numbers on Polly (C# namespace mechanism) and pyjanitor
  (Python import-alias mechanism), plus the CRC-001/002/003 numbers
  already retrofitted into `call-res-a4-crc-regression.jsonl` by a
  prior pass (`92b7bf8`).
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

## Final multi-corpus metrics (CALL-RES-A4, final item)

**CAVEAT, added by the wiring pass below: every number in this section
is a resolver-level measurement, not live-indexer evidence.** Both
harnesses call A4's resolvers (`resolve_calls_with_namespace_context`/
`resolve_calls_with_imports`) directly against real parsed snapshots —
they do NOT go through `CodeGraphIndexer::observe_commit`. At the time
these numbers were measured, A4 was not yet wired into the real
indexer at all (see the "Reliability-policy / full-incremental-parity
re-check" section's own MAJOR FINDING below) — these Polly/pyjanitor
deltas describe what A4's mechanism does in isolation, not what any
real indexing run actually produced. The wiring gap is now closed (see
"Wiring into the real indexer" section, added after this one) but
these specific numbers were never re-measured through the wired
indexer, and re-measuring them was explicitly out of scope for the
wiring task.

Real before/after runs, two new standalone example harnesses
(`call_res_a4_metrics_csharp.rs`, `call_res_a4_metrics_python.rs`),
reusing the SAME parse/extract/promote loop every prior domain smoke
test already uses, plus `resolve_import_facts` on the SAME per-file
tree each harness already parses (no new parsing infrastructure). Each
harness runs the shipped pre-A4 entry point
(`resolve_calls_with_extraction_rejections`) and A4's own entry point
(`resolve_calls_with_namespace_context` / `resolve_calls_with_imports`)
against the SAME real snapshots, and reports the real delta.

### Polly (C#, `App-vNext/Polly`, commit `1f232caf9672e515751ba2df76d82bfbbb62e016` — same commit prior A4/B2-EVAL-008/Roslyn-oracle work used)

- files_parsed: 792; snapshots_promoted: 5190; real_import_facts_extracted: 725; csharp_type_declaration_contexts_resolved: 860
- **Before** (`resolve_calls_with_extraction_rejections`): edges 2078 (StaticAnalysis 1498, NameHeuristic 580); call_attempts 16515 (Ambiguous 8345, Unresolved 8170)
- **After** (`resolve_calls_with_namespace_context`): edges 2415 (StaticAnalysis 1498, NameHeuristic 917); call_attempts 16178 (Ambiguous 8008, Unresolved 8170)
- **Delta**: edges +337, call_attempts -337
- Of the 8345 real pre-A4 ambiguous (>=2 candidate) call sites: **366 narrowed to exactly one candidate** (now a real edge — "ambiguities resolved without type information", since namespace visibility is a using-directive/declaration-site fact, never a type-inference result), 3985 narrowed but still 2+ candidates, 3994 unchanged. Mean candidate count 15.99 → 13.80.
- Invariant re-check (every genuinely new after-edge's target was a real pre-A4 candidate-pool member, never manufactured): **0 violations**.
- Precision/recall/F1/TP/FP/FN: **N/A** — no ground-truth label set exists for Polly's full ~2000-edge call graph (unlike CRC-001/002/003, which each have one hand-confirmed real target). Building a >=30-row labeled sample for Polly was judged a new ground-truth-labeling effort, not a before/after measurement, and out of this pass's own scope; an unlabeled proxy (e.g. "every produced edge is correct") would hide the exact FP risk this metric exists to catch. Real FP/FN risk is instead covered by the invariant re-check above (0 violations) and by CRC-001/002/003's own ground-truth rows.
- Full raw output: `docs/research/evidence/call-res-a4-metrics-csharp-polly-run.txt`.

### pyjanitor (Python, `pyjanitor-devs/pyjanitor`, commit `1247923ff01dd9a74f40f55d55f108e503e4b218` — same commit CRC-003/B2-EVAL-006 already used)

- files_parsed: 291; snapshots_promoted: 2822; real_import_facts_extracted: 1752
- **Before**: edges 1485 (StaticAnalysis 1430, NameHeuristic 55); call_attempts 8341 (Ambiguous 158, Unresolved 8183)
- **After**: edges 1485 (unchanged); call_attempts 8340 (Ambiguous 159, Unresolved 8181)
- **Delta**: edges +0, call_attempts -1
- Of the 8183 real pre-A4 zero-candidate call sites: **0 recovered to a real edge** on this corpus at this commit; 1 `ImportTargetExternal`; 1 `ImportedSymbolAmbiguous` (the real CRC-003 second-collision shape, recurring here); 7224 `NoMatchingImportAlias` (no candidate, no alias either); 956 `ExtractionRejected` (the callee name is itself a real `SignatureTooLong` rejection — the resolver's own branch order checks this BEFORE the alias path per `calls.rs`, so A4 never gets a chance to act on these; real, not a defect, confirmed directly against real SignatureTooLong names like `conditional_join`/`get_join_indices`); 1 residual unaccounted.
- This is consistent with, not contradicted by, CRC-003's own documented recovery: CRC-003's one specific call site resolves to `ImportedSymbolAmbiguous` (the mechanism found the real alias and the real 2-candidate set, correctly declined to guess), not `Internal` — so a 0-recovery corpus-level count here is the EXPECTED real-world shape, not a sign the mechanism is broken. The mechanism's own clean-recovery path is separately proven by the synthetic `case_1_correct_import_alias_resolves_cleanly_when_unambiguous` unit test (no second real-corpus collision in that fixture).
- Invariant re-check: **0 violations**.
- Precision/recall/F1/TP/FP/FN: **N/A**, same reasoning as Polly — no corpus-level ground-truth label set beyond CRC-003's own single row.
- Full raw output: `docs/research/evidence/call-res-a4-metrics-python-pyjanitor-run.txt`.

### TS/TSX

**N/A, explicitly not measured.** The existing frontend smoke test (`b2_smoke_test_frontend_ts.rs`) calls plain `resolve_calls`, not even the pre-A4 `resolve_calls_with_extraction_rejections` — wiring either A4 entry point into a real TS/TSX corpus would require new harness setup this task's own scope excludes ("TS/TSX only if already available without new setup"). The checkpoint's own prior claim that the alias mechanism "should already generalize" to TS/JS (via `ImportFact` extraction's existing named/default/aliased distinction) remains a structural argument, not a corpus-measured one.

### CRC-001/002/003 (already retrofitted into the machine-readable ground-truth schema by a prior pass, `92b7bf8`)

- **CRC-001** (`pytorch/examples`, ambiguous unqualified constructor call): computes as `FalseNegative` — A4's own zero-candidate-only scope never touches this `>=2` ambiguity; never a `FalsePositive`. No regression.
- **CRC-002** (`yoonge/conduit-react`, local-binding shadowing): `resolve_calls_with_extraction_rejections` and `resolve_calls_with_imports` produce byte-for-byte identical output; confirmed never `FalsePositive`. No regression, pre-existing scope-blindness limitation unchanged.
- **CRC-003** (`pyjanitor-devs/pyjanitor`, import-alias blindness): two-row story — the real corpus's own name-collision call site computes `FalseNegative` with `resolution_method: Ambiguous` (mechanism found the real 2-candidate set, correctly declined to guess); the clean-mechanism row (no real second collision) computes `TruePositive`. Net improvement, not merely non-regression.
- NO-GO condition (`A4 recovers CRC-003 but introduces a new FP on CRC-002` → NO-GO) — **confirmed NOT triggered**, directly re-verified by `no_go_condition_is_confirmed_not_triggered_across_all_four_crc_rows` against the real computed `Status` of every row.
- See `ground_truth_schema.rs` tests and `call-res-a4-crc-regression.jsonl` for the machine-checked source of these numbers.

## Live-indexer re-measurement (CALL-RES-A4, post-wiring)

**The resolver-level numbers in "Final multi-corpus metrics" above are
NOT deleted or overwritten — they remain exactly as recorded, now
explicitly labeled experimental/resolver-level (see that section's own
caveat, added by the wiring pass).** This section adds a SEPARATE,
clearly distinct set of numbers: the SAME Polly/pyjanitor corpora, at
the SAME pinned commits, measured by actually running the real,
now-wired `CodeGraphIndexer::observe_commit` end to end (two new
standalone harnesses, `call_res_a4_live_indexer_csharp.rs` /
`call_res_a4_live_indexer_python.rs`) — not by calling either resolver
function directly. "Before" in this section is reproduced by calling
`resolve_calls_with_extraction_rejections` on the exact same
snapshot/signature-too-long-names pool each harness itself extracts
the same way the real pipeline does; this is mechanically equivalent
to what the real indexer did before commit `ffd0295`, since that
commit's only change at the real call site was swapping which resolver
function is invoked on an otherwise-identical pool (confirmed directly
by reading the pre/post-`ffd0295` diff of
`resolve_all_typed_edges_with_csharp_contexts_and_negative_knowledge`)
— no separate pre-`ffd0295` checkout was needed or performed to
reproduce this baseline.

### Polly (C#), live-indexer

- **AFTER (live, real `observe_commit`)**: calls_edges 2415
  (StaticAnalysis 1498, NameHeuristic 917); call_attempts 16178
  (Ambiguous 8008, Unresolved 8170). total_snapshots 5190;
  real_import_facts_published 725.
- **BEFORE (real-indexer-equivalent baseline, same pool)**: calls_edges
  2078 (StaticAnalysis 1498, NameHeuristic 580); call_attempts 16515
  (Ambiguous 8345, Unresolved 8170).
- **Snapshot-count parity**: baseline (5190) and live indexer (5190)
  agree exactly — the before/after comparison is apples-to-apples.
- Of the 8345 real pre-A4 ambiguous call sites: **366 narrowed to
  exactly one candidate through the LIVE wired path** (a real `Edge`
  via `observe_commit`), 3985 narrowed but still 2+ candidates, 3994
  unchanged.
- **Comparison to the `ab0591d` resolver-level figure: EXACT MATCH.**
  366 (resolver-level) == 366 (live-indexer). Every other number above
  (edges, tier breakdown, call_attempts) also matches `ab0591d`'s own
  "after" figures exactly. The resolver-level measurement and the real
  live indexing run produce byte-identical results for this corpus —
  confirms the wiring in `ffd0295` did not introduce any real-world
  divergence (e.g. from incremental assembly or file-ordering effects)
  for this case.
- Invariant re-check on this run's own live data (every genuinely new
  live edge's key was a real pre-A4 candidate-pool member, never
  manufactured): **0 violations**.
- Precision/recall/F1/TP/FP/FN: **N/A**, same reasoning as the
  resolver-level section — no corpus-level ground-truth label set
  exists for Polly beyond CRC-001/002/003's own rows.
- Full raw output: `docs/research/evidence/polly-live-run.txt` (scratchpad-run; also reproducible via `cargo run --release --example call_res_a4_live_indexer_csharp -- <polly_repo_path> 1f232caf9672e515751ba2df76d82bfbbb62e016`).

### pyjanitor (Python), live-indexer

- **AFTER (live, real `observe_commit`)**: calls_edges 1485
  (StaticAnalysis 1430, NameHeuristic 55); call_attempts 8340
  (Ambiguous 159, Unresolved 8181). total_snapshots 2822;
  real_import_facts_published 1752.
- **BEFORE (real-indexer-equivalent baseline, same pool)**: calls_edges
  1485 (unchanged); call_attempts 8341 (Ambiguous 158, Unresolved
  8183).
- **Snapshot-count parity**: baseline (2822) and live indexer (2822)
  agree exactly.
- Of the 8183 real pre-A4 zero-candidate call sites, through the LIVE
  wired path: **0 recovered to a real edge**; 1 `ImportTargetExternal`;
  1 `ImportedSymbolAmbiguous` (the real CRC-003 second-collision shape);
  7224 `NoMatchingImportAlias`; 956 `ExtractionRejected` (pre-existing
  `SignatureTooLong` names, never reaching the alias path by the
  resolver's own branch order); 1 residual unaccounted — byte-for-byte
  identical breakdown to the resolver-level run.
- **Comparison to the `ab0591d` resolver-level figure: EXACT MATCH.**
  0 (resolver-level) == 0 (live-indexer). Every other number above also
  matches `ab0591d`'s own figures exactly (including the Ambiguous
  158→159 / Unresolved 8183→8181 shift, which is the same genuine
  `ImportedSymbolAmbiguous` conversion already documented in the
  resolver-level section, not a new effect of wiring).
- Invariant re-check: **0 violations**.
- Precision/recall/F1/TP/FP/FN: **N/A**, same reasoning as the
  resolver-level section.
- Full raw output: `docs/research/evidence/pyjanitor-live-run.txt` (scratchpad-run; also reproducible via `cargo run --release --example call_res_a4_live_indexer_python -- <pyjanitor_repo_path> 1247923ff01dd9a74f40f55d55f108e503e4b218`).

### CRC-001/002/003, live-indexer

Re-confirmed through the real wired path directly (not just the
resolver-level/ground-truth-schema tests already covering these):
`golden_corpus.rs`'s own `crc_003_import_alias_blindness_is_fixed_by_call_res_a4`
(Python, real `index()` → real `observe_commit`, asserts the clean
alias call now resolves to `real_target`) and
`crc_002_prop_shadowing_false_positive_is_the_current_documented_gap`
(TSX, same real path, asserts the pre-existing shadowing gap is
unchanged — both already added and passing as of `ffd0295`, re-run
clean here). CRC-001 has no dedicated golden-corpus test (A4
structurally never touches its `>=2`-ambiguity branch for unrelated
reasons), and its non-regression is already covered by
`ground_truth_schema.rs`'s `crc_001_computes_as_a_false_negative_never_a_false_positive`
plus this section's own Polly Ambiguous-tier numbers behaving as
expected — no additional live test was judged necessary. All CRC tests
pass: 8/8 in `ground_truth_schema.rs`+`golden_corpus.rs` combined CRC
coverage.

### Findings (live-indexer re-measurement)

No divergence found between resolver-level and live-indexer numbers on
either corpus — the wiring in `ffd0295` reproduces `ab0591d`'s own
numbers exactly, including the headline 366-case Polly figure. This is
itself a confirmed, not assumed, result (both harnesses independently
re-derive the baseline from a fresh real-corpus parse rather than
reusing any cached resolver-level output).

## Reliability-policy / full-incremental-parity re-check (agent_policy.rs / conformance.rs)

**Status: done, with one major finding.** Prior to this pass, a grep
across this crate's own `src/` confirmed `agent_policy::evaluate`/
`evaluate_high_impact_modification` were called ONLY from their own
module's hand-built-`EvidenceTier` unit tests — never from any real A4
resolver path. This pass adds `agent_policy::tests::call_res_a4_policy_recheck`
(6 new tests, real extraction, real A4 resolver calls, real policy
evaluation — never hand-asserted) and one new test in
`conformance.rs`, and confirms the following against REAL data:

1. **Deterministic narrowing increases reliability only as far as
   justified**: a real namespace-narrowed-to-one edge carries
   `ResolutionMethod::NameHeuristic`, which `EvidenceTier::from_resolution_method`
   maps to `Heuristic`, never `Resolved` — confirmed against a real
   resolved edge, then routed through `agent_policy::evaluate`:
   `PermittedWithDisclosedUncertainty` for display, `Forbidden` for
   assert-as-fact and high-impact modification. Policy-correct.
2. **`unknown visibility` never becomes `not visible`**: re-confirmed
   at the real resolver-entry-point level (not just the
   `exclude_definitely_not_visible_by_namespace` unit `edges::calls`
   already covers) — an undetermined-namespace candidate survives
   filtering, the real ambiguity stays `Ambiguous`, and the policy
   layer correctly still forbids it for every intended use.
3. **`Ambiguous` stays `Forbidden`**: confirmed against a real A4
   cross-namespace ambiguity `CallAttempt` (not a hand-built one) —
   forbidden for every intended use including mere display.
4. **Unresolved-external vs. local-resolution-failure — FINDING, not a
   clean "yes"**: `ImportTargetExternal` and plain `Unknown` are
   genuinely distinct `UnresolvedReason` values (different prose,
   different provenance), but both carry `EvidenceTier::Unresolved`,
   and `agent_policy::evaluate` dispatches purely on `tier` — it is
   `UnresolvedReason`-blind. The two therefore produce an IDENTICAL
   `Forbidden` verdict with identical reason text. The distinction A4
   adds is real at the reason/provenance layer but does NOT currently
   change what an agent is permitted to do with either outcome. Safe
   (both correctly stay Forbidden), but not actually "routed to
   different policy outcomes" — reported honestly rather than forced.
5. **CRC-002 does not regress under policy re-evaluation**: re-run
   through the real policy layer this time (not just the resolver
   level `edges::calls` already proved byte-identical) — the real
   tier sequence and every real policy verdict for CRC-002's call
   sites are identical before/after A4.
6. **The 366 real Polly recoveries are policy-coherent**: every
   `NameHeuristic`-tier edge A4 produces (the exact mechanism behind
   commit `ab0591d`'s 366-case Polly recovery count) maps to
   `Heuristic`, confirmed structurally rather than by inspecting all
   366 individually — `EvidenceTier::from_resolution_method` has no
   path from `NameHeuristic` to anything but `Heuristic`, and the
   dedicated real-fixture test above exercises that exact mapping
   end-to-end. `ImportedSymbolAmbiguous` (A4's own new ambiguity
   reason) is separately confirmed exactly as `Forbidden` as a plain
   name-based `Ambiguous` — A4's new code paths do not bypass the
   "Ambiguous is always Forbidden" rule.
7. **Still-ambiguous cases remain non-actionable**: confirmed directly
   — every real `Ambiguous` `CallAttempt` this pass produced or
   inspected is `Forbidden` for every `IntendedUse`, including
   display.

**MAJOR FINDING — A4 is not wired into the real indexer at all.**
Reading `crate::edges::resolve_all_typed_edges_with_csharp_contexts_and_negative_knowledge`
(the one function `incremental/indexer.rs:687` calls for ITS
call-resolution pass, identically for both the full-reindex and the
incremental path) shows it hard-codes a call to
`resolve_calls_with_extraction_rejections` — the PRE-A4 entry point —
never `resolve_calls_with_imports`/`resolve_calls_with_namespace_context`,
even though the real `csharp_contexts` (and, elsewhere in the same
pipeline, real `import_facts`) are already computed and available at
that exact call site. Confirmed with a new, passing, real-data test
(`conformance::tests::call_res_a4_not_wired_into_the_real_indexer::the_real_indexer_still_reports_the_pre_a4_ambiguous_outcome_for_a_case_a4_itself_resolves`):
the real `CodeGraphIndexer::observe_commit`, given the EXACT fixture
`edges::calls`'s own unit test proves A4 resolves to one candidate,
still reports `Ambiguous(2)` — the pre-A4 outcome. A4's own mechanisms
are real, tested, mutation-closed, and separately measured on real
corpora (commit `ab0591d`) only via standalone example binaries and
unit tests that call the A4 functions directly — they contribute
NOTHING to what `CodeGraphIndexer` actually produces on any real repo
today. Consequently, **"full/incremental parity for A4" is vacuously
true against the real indexer** (both paths call the same, non-A4,
function) — not because A4's own mechanism has been parity-checked,
but because it never runs there. This is the first new finding this
checkpoint doc records that is NOT merely a residual limitation of a
built mechanism, but a wiring gap preventing every already-built,
already-tested A4 mechanism from having any real effect. Not fixed in
this pass (wiring A4 into the real pipeline was not this pass's own
scope — a policy/conformance RE-CHECK, not an implementation change);
reported as a finding for the user's own next decision.

**Tests added this pass**: 6 in `agent_policy.rs`
(`call_res_a4_policy_recheck` module), 1 in `conformance.rs`
(originally `call_res_a4_not_wired_into_the_real_indexer`, now flipped
to confirm the fix -- see below). Full lib suite: 602/602 passing (up
from 595). Full workspace `codegraph-index` test run (lib + 6
integration binaries): 680/680 passing, 0 failures.

## Wiring into the real indexer — CLOSED

**The MAJOR finding above is fixed.** `import_facts` is now resolved
BEFORE typed-edge resolution (not after, as it was) in both
`incremental::pipeline::index_files` (full-reindex path — resolved
locally against that call's own complete snapshot pool, correct
because a full reindex's one `index_files` call already covers every
file) and `incremental::indexer::incremental_reindex` (the incremental
path — moved up from its own prior position after typed-edge
resolution to before it, resolved against `current_snapshots`, the
same pool `typed_edges` itself resolves against). Both now pass
`import_facts` into
`resolve_all_typed_edges_with_csharp_contexts_and_negative_knowledge`,
which gained a 4th parameter and now calls A4's own
`resolve_calls_with_namespace_context` instead of the pre-A4
`resolve_calls_with_extraction_rejections`. No redesign: one resolver
call swapped for another at the SAME call site, with the SAME
fallback behavior A4's own resolvers already provide (empty
`csharp_contexts`/`import_facts` reproduces the pre-A4 behavior
byte-for-byte, confirmed by the pre-existing equivalence tests, now
re-confirmed with a non-empty default `&[]` call shape still compiling
and passing).

**Required test 1 (live case, Ambiguous(2) -> 1 candidate):**
`conformance::tests::call_res_a4_wired_into_the_real_indexer::the_real_indexer_now_resolves_a_case_a4_itself_resolves`
— the exact fixture the prior fork's gap-pinning test used, now
flipped: the real `CodeGraphIndexer::observe_commit` produces a real
`Calls` edge to the `using`-visible `NsA::Foo::Target`, no `CallAttempt`
left behind. PASS.

**Required test 2 (same case via `observe_commit` incrementally):**
`the_same_resolution_reproduces_through_observe_commit_incrementally`
— the identical two-namespace-candidate shape, first commit establishes
the ambiguity, second commit (`NormalAdvance`, changed_files =
`["src/Caller.cs"]`) adds the caller. Identical resolution via the
incremental path. PASS.

**Required test 3 (full/incremental parity — edge + CallAttempt + tier
+ provenance):**
`full_and_incremental_agree_on_edge_call_attempt_tier_and_provenance_for_the_newly_wired_case`
— reuses the pre-existing `assert_full_and_incremental_agree` helper
(now `pub(super)`) for its own `compare_full_incremental_state` check
(edges/CallAttempts/import facts/impl facts/snapshots all agree as a
multiset, which already folds in tier/reason/candidate_count via
`CallAttemptKey`), PLUS a direct comparison of the real, raw `Edge`'s
`Evidence` for the A4-resolved call site — `ResolutionMethod` and the
evidence note text are asserted byte-identical between the full and
incremental outcome, and the method is asserted to be exactly
`NameHeuristic` (never `StaticAnalysis`). PASS.

**Required test 4 (CRC-001/002/003 through the real wired indexer, not
the isolated resolver):** three new tests, each reusing the EXACT real
corpus fixture content the isolated-resolver regression tests in
`edges::calls` already used, now run through
`CodeGraphIndexer::observe_commit` for the first time:
- `crc_001_through_the_real_wired_indexer_stays_ambiguous` — stays
  `Ambiguous`/2-candidates, no new edge. PASS, no regression.
- `crc_002_through_the_real_wired_indexer_does_not_regress` — this one
  surfaced a real, honest subtlety worth recording: CRC-002's own
  PRE-EXISTING, documented limitation (A4 has no local-parameter-
  shadowing awareness) means the pre-A4 baseline ALREADY wires a
  `Calls` edge from `render` to the unrelated module-level `Button` —
  the isolated-resolver test asserts this stays BYTE-IDENTICAL
  before/after A4, not that it is absent. The real-indexer test
  confirms the SAME pre-existing shape reproduces identically through
  the full pipeline — neither fixed nor worsened by wiring. PASS
  (as non-regression, not as a fix).
- `crc_003_through_the_real_wired_indexer_reports_the_same_honest_ambiguity`
  — the real corpus's own name-collision shape reports
  `ImportedSymbolAmbiguous` through the real indexer too, same as the
  isolated resolver. PASS.

Separately, the golden-corpus integration test
`crc_003_import_alias_blindness_is_the_current_documented_gap` (which
pinned the OLD gap — "no edge" — on a clean, collision-free alias
fixture) now correctly FAILS once wiring lands, exactly as its own
comment anticipated ("if this assertion ever fails, CRC-003 has been
fixed and this test... must be updated"). Flipped to
`crc_003_import_alias_blindness_is_fixed_by_call_res_a4`, asserting the
real recovered edge to `helper.py`'s `real_target`. The fixture file's
own header comment is updated to match.

**Required test 5 (mutation testing on the new wiring, 0 genuine
missed):** `cargo mutants --in-diff` scoped strictly to this pass's
own changed lines in `edges/mod.rs`, `incremental/pipeline.rs`, and
`incremental/indexer.rs` (not the whole files — a whole-file scope
pulled in hundreds of unrelated whole-function mutants on pre-existing
code this pass never touched). 200 mutants found; 4 caught
(`resolve_all_typed_edges_with_csharp_contexts`'s and
`..._and_negative_knowledge`'s own bodies, `incremental_reindex`'s
single viable whole-function return-tuple mutant, and a real `!`
negation on the moved-up `raw_import_statements` filter), 196
unviable. **0 missed, 0 timeout.** The 196 unviable figure is
dominated (191 of 196) by a single mutation SITE —
`incremental_reindex`'s own 9-tuple return type — where `cargo mutants`
enumerates every combinatorial `Default::default()`/`vec![]`
permutation as a separate candidate mutant; only one combination
type-checks (and that one WAS caught), the rest fail to compile by
construction. Confirmed by inspecting `mutants.out/unviable.txt`
directly, not assumed.

**Full suite after wiring:** 607/607 lib tests, 0 failures across
every `codegraph-index` integration binary (18/18, 34/34, 12/12, 10/10,
0/0 across the six binaries), full workspace build clean.

**Not done, by this task's own explicit scope:** the full multi-corpus
Polly/pyjanitor/CRC metrics were NOT re-measured through the newly
wired real indexer — the section above now carries the caveat that
those numbers are resolver-level, not live-indexer evidence; no new
corpus run was performed here. Performance measurement and a final
closing independent review of the whole A4 effort remain separately
open, untouched by this pass.

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
extension (DONE), full corpus-level evaluation metrics (DONE), and
policy/conformance re-checks (DONE, see above — with the wiring-gap
finding) are now closed. Still open: performance measurement, Python
candidate-filtering beyond alias matching, TS/JS module-visibility
filtering, and — newly surfaced by this pass, more significant than
any of those — **A4 is not wired into the real `CodeGraphIndexer`
pipeline at all**, so none of its tested, mutation-closed mechanisms
have any effect on a real indexing run today. Wiring it in was not
this pass's own scope (a re-check, not an implementation change) and
is not done here.

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

## Performance measurement — A4's specific cost on the real indexer

**Status: done, measurement only, nothing optimized.** Measures A4's
own real wall-clock cost through the real, now-wired
`CodeGraphIndexer::observe_commit` (not a resolver function called in
isolation), comparing the pre-wiring commit `8584cb1` (BASELINE: the
indexer still calls the pre-A4 `resolve_calls_with_extraction_rejections`)
against current HEAD `468f770` (A4-LIVE: the indexer calls A4's own
`resolve_calls_with_namespace_context`, per the `ffd0295` wiring).

**Scope boundary, explicit:** this is NOT a re-investigation of
[B2-ID-012](../findings/B2-ID-012.md) (`resolve_lineage`'s O(n²)
similarity-scoring cost) or the separate, already-tracked
`LocalGitRepositorySource::read_file` per-file git-reopen finding.
Both stay exactly as previously documented, unfixed, unchanged by this
measurement. Nothing was optimized in this pass, in A4's own code or
anywhere else.

### Methodology

New example binary `call_res_a4_perf_measurement.rs`: calls the real,
public `CodeGraphIndexer::observe_commit` only (never a resolver
function directly) for a full-reindex of `commit_a` followed by an
incremental `observe_commit` of `commit_b` against `commit_a`'s own
state, using the real `git diff --name-only` changed-file list between
them — the exact same entry point and commit-pair convention
`b2_smoke_test_csharp_cpp.rs`'s own Section F and B2-ID-012's own
measurement already use. The SAME binary source is built once at the
baseline commit and once at current HEAD (via a separate, disposable
git worktree for the baseline build — no branch history rewritten,
main worktree's pre-existing uncommitted files untouched throughout),
so any timing difference is attributable only to what changed inside
the real indexer between those two commits.

Per-corpus: **1 discarded warm-up run, then 10 timed runs**, same
machine, same release build profile, same two git worktrees used for
both the baseline and A4-live binary across both corpora. **Median**
reported as the headline number (not mean, per this project's own
B2-ID-012 precedent); **dispersion reported as min/max range**
(simpler than stddev/IQR for a 10-sample set, and B2-ID-012's own
report uses the same convention: "range 107-183µs" style). Edge-count
output was also captured on every run and confirmed byte-identical
across all 10 runs on both sides, before vs. after — confirming
timing variance is not masking a hidden correctness difference.

### Corpora

- **Polly** (C#, `App-vNext/Polly`): same commit pair B2-ID-012 itself
  used, `32064ed5` (commit_a, full-index target) →
  `1f232caf9672e515751ba2df76d82bfbbb62e016` (commit_b, incremental
  target against commit_a's state, 41 real changed files).
- **pyjanitor** (Python, `pyjanitor-devs/pyjanitor`): already available
  without new setup (same repo/pinned-commit prior A4 passes used,
  `1247923ff01dd9a74f40f55d55f108e503e4b218`, as commit_b); commit_a
  is that same repo's own immediately-prior real commit
  (`8e3cb118888e4b94acafd58392ef5e69138d976a`, 3 real changed files) —
  B2-ID-012 itself never established a pyjanitor pair, so this is a
  new but equally real commit pair from the same already-cloned repo,
  not a new corpus.

### Results: Polly

| | full-index (median, min-max) | incremental-index (median, min-max) |
|---|---|---|
| BASELINE (pre-A4, `8584cb1`) | 5.510s (5.454-5.561s) | 8.386s (8.295-8.492s) |
| A4-LIVE (`468f770`) | 6.528s (6.503-6.675s) | 9.438s (9.399-9.636s) |
| **overhead** | **+1.018s (+18.47%)** | **+1.052s (+12.55%)** |

Edge counts stable across all 10 runs on both sides (baseline:
3415/3415 full/incremental; A4-live: 3746/3752 full/incremental — the
A4-live counts are higher because A4 genuinely recovers real edges,
consistent with commits `ab0591d`/`468f770`'s own resolver-level and
live-indexer correctness numbers; this measurement did not re-verify
those counts beyond confirming their own run-to-run stability, which
is this task's own job, not re-litigating correctness already closed
elsewhere).

### Results: pyjanitor

| | full-index (median, min-max) | incremental-index (median, min-max) |
|---|---|---|
| BASELINE (pre-A4, `8584cb1`) | 1.490s (1.462-1.516s) | 0.811s (0.798-0.833s) |
| A4-LIVE (`468f770`) | 1.548s (1.500-1.576s) | 0.856s (0.845-0.889s) |
| **overhead** | **+0.058s (+3.91%)** | **+0.045s (+5.55%)** |

Edge counts stable across all 10 runs on both sides (1563/1565
full/incremental, identical before vs. after — consistent with
`ab0591d`/`468f770`'s own finding that A4 produces 0 corpus-level
alias recoveries on pyjanitor, so no edge-count difference is expected
here, unlike Polly).

### Stage-level attribution (Polly, 3-run confirmation sample)

A temporary, ephemeral `Instant`-based probe (env-var-gated via
`A4_PERF_PROBE`, never committed, fully reverted after this
measurement — confirmed via `git diff` showing zero net change to
`incremental/pipeline.rs`, same "ephemeral probe, delete after
confirming" convention B2-ID-012 itself documents) was added around
two points in `incremental::pipeline::index_files`'s own full-reindex
path: `resolve_raw_import_statements` (import-facts assembly) and
`resolve_all_typed_edges_with_csharp_contexts_and_negative_knowledge`
(the function containing the actual call-resolution stage, both
before and after wiring).

```
import_facts_resolution (A4-live only; did not exist pre-wiring):      ~1.7-1.8ms   (negligible)
resolve_all_typed_edges stage, BASELINE (resolve_calls_with_extraction_rejections): median ~3.507s (3.493-3.522s)
resolve_all_typed_edges stage, A4-LIVE  (resolve_calls_with_namespace_context):      median ~4.631s (4.607-4.713s)
stage-level overhead: +1.124s (+32.05%% of the stage's OWN cost)
```

**The overhead is concentrated almost entirely inside the
call-resolution stage itself** (the `resolve_calls_with_namespace_context`
mechanism A4 wires in) — the stage-level delta (+1.124s) closely
matches the full-index total delta (+1.018s) measured independently
above (same order of magnitude, same corpus, consistent given this is
a 3-run confirmation sample vs. the 10-run headline figure). Import-facts
assembly itself is negligible (low-single-digit milliseconds) — it is
NOT where the cost lives, despite being the part of the wiring change
that moved earlier in the pipeline. This is consistent with, and does
NOT overlap with, the already-tracked B2-ID-012 (`resolve_lineage`,
measured separately, a different function entirely) or the
`LocalGitRepositorySource::read_file` finding (file I/O, not
call-resolution) — neither is implicated by this measurement.

### Classification

**Moderate overhead, real but not negligible, worth tracking but not
blocking.** On Polly (the corpus where A4's namespace-aware filtering
actually does real narrowing work — 366 real cases), A4 adds roughly
**12-18% to real-indexer wall-clock time** (full and incremental
respectively), concentrated in the call-resolution stage's own
namespace-visibility-filtering work across Polly's real ~8,345
ambiguous call sites. On pyjanitor (where A4's Python alias mechanism
finds 0 corpus-level recoveries, per `ab0591d`/`468f770`), the
overhead is much smaller (4-6%) — consistent with there being
structurally less filtering work to do when the zero-candidate alias
path rarely engages. The overhead scales with how much real
ambiguity-narrowing work A4 actually performs on a given corpus, not
with corpus size alone (pyjanitor's own baseline full-index, at 1.49s,
is far faster than Polly's 5.51s, yet pyjanitor's own A4 overhead
percentage is smaller, not larger — consistent with the mechanism's
own cost being proportional to real ambiguous-candidate-set size and
candidate-filtering operations performed, not file/snapshot count).
**Not** classified Negligible (it is a reproducible double-digit
percentage on the corpus where the mechanism does real work) and
**not** classified Significant (it does not double indexing time,
remains well under half the baseline cost on both corpora, and
dispersion on every measured number is tight — under 5% spread on
every median reported above — so this is a real, confirmed cost, not
noise).

**Commit:** `1ff6425` on `b2-code-graph` (new example binary
`call_res_a4_perf_measurement.rs` + this section). No merge/tag/push.
Pre-existing uncommitted files in the worktree untouched throughout.
No performance optimization performed, in A4's own code or elsewhere,
per this task's own explicit scope.
