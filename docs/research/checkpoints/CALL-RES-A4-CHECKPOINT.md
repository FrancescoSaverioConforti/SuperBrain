# CALL-RES-A4 Checkpoint — Import/Namespace/Visibility-aware Resolution

**Status:** PARTIAL — the core deterministic import-alias resolution
mechanism (Python) is implemented, tested, and independently reviewed
with zero regressions. Several directive sections (namespace awareness
for C#/C++/TS, an explicit three-way visibility model as a distinct
new type, oracle/ground-truth extension, full multi-corpus evaluation
metrics, `agent_policy.rs`/`conformance.rs` re-checks, mutation
testing, and performance measurement) were **not reached** in this
pass and are documented below as deferred, not silently dropped.

**Authorized by:** the CALL-RES-A4 directive, following the prior
Semantic Resolution & Reliability Checkpoint's completion. Relayed
mid-task by the coordinator with explicit instruction to continue in
order, commit frequently (this being the sixth-plus rate-limit/turn-
limit interruption across this hardening effort), and report honestly
whether everything completes or the pass is cut off again — it was
cut off again, at realistic single-fork scope, before reaching several
sections. This document reports that honestly rather than overstating
completion.

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

## Explicitly deferred (not reached this pass)

Reported honestly per the directive's own instruction, rather than
omitted:

- **Import-visibility candidate filtering beyond alias matching**
  (directive §4) — not implemented. A4 only ever consults the exact
  `alias` field; it does not yet use `imported_symbol`/`module_path`
  to exclude definitively-non-visible candidates in the `>=2` branch.
- **Namespace awareness for C#/C++/TS** (§5) — not implemented. Only
  Python's import/alias extraction was exercised this pass.
- **Explicit three-way visibility model as a distinct new type**
  (§6) — not implemented as a standalone concept. The `ImportResolution`/`ImportAliasOutcome`
  three-way shape (Internal/External/Ambiguous) is structurally
  similar but is not the same thing as a visibility model
  (visible/definitely-not-visible/unknown) applied to the `>=2`
  candidate-filtering case.
- **Bounded candidate-filtering provenance** (§7) — not implemented
  beyond what `Provenance::new` already bounds for the new
  `CallAttempt` constructors.
- **Oracle/ground-truth extension** (§10) — Roslyn/TS-compiler/manual-Python
  oracle work not touched.
- **Full evaluation-metrics pass** (§1, §13) — no baseline freeze or
  before/after metrics run across Polly/golden-corpus/a dedicated
  Python corpus. Candidate-reduction counts, precision/recall/F1, and
  "ambiguities resolved without type information" as a distinct metric
  were not measured.
- **Reliability-policy re-checks** (`agent_policy.rs`, §14) — not run
  against the new code.
- **Full/incremental parity extension** (`conformance.rs`, §15) — the
  multiset gate was not extended to cover import-related evidence or
  the 6 named adversarial cases (import added/removed, alias changed,
  target moved module, local shadow introduced/removed).
- **Mutation testing** (§16) — not run against `resolve_via_import_alias`,
  the new `ImportAliasOutcome` match arms, or the fixed
  `collect_python_import` relative-import logic.
- **Performance measurement** (§17) — not measured.
- **CALL-RES-A5** (§18) — correctly not implemented, per explicit
  directive exclusion.

## Recommendation

**GO** for merging this pass's Python alias-resolution work as a
discrete, net-positive increment (CRC-003 recovered, zero regressions,
one real pre-existing bug fixed as a byproduct) — subject to the
remaining directive sections being picked up in a follow-up pass
before CALL-RES-A4 itself is declared complete.

**NO-GO** on treating CALL-RES-A4 as closed. Namespace awareness,
visibility filtering, oracle extension, full metrics, parity
extension, mutation testing, and performance measurement all remain
open.

**NO-GO** on resuming B2-EVAL-009 until CALL-RES-A4 is either
completed or explicitly descoped by the user/coordinator — consistent
with the standing instruction not to start B2-EVAL-009 as part of this
checkpoint.

**GO/NO-GO on CALL-RES-A5:** not evaluated — A5 was explicitly out of
scope for this pass and no case surfaced during this pass's own work
that specifically required it beyond what CRC-003's real-shape finding
already documents as its own independent ambiguity (not an A5-shaped
gap).
