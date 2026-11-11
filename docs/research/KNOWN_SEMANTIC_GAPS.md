# Known semantic-gap inventory

Quality & Semantic Reliability Hardening, Phase 1, deliverable 8.
Every entry cites the real finding/evaluation document where it was
first measured — this inventory does not re-derive or re-estimate any
number already published elsewhere; it indexes and classifies what
already exists.

## How to read this inventory

For each gap: **observed evidence** (where it was measured, with the
real citation), **affected languages/domains**, **severity/current
impact**, **FP / FN / missing-capability** (which failure shape it
produces), **candidate analysis technique** (what kind of future work
would address it — naming only, never committing to build it),
**blocks agent trust?** (per the reliability contract above — does
this gap make a `Resolved`/`Heuristic` edge potentially misleading, or
does it only affect `Unresolved` cases, which the contract already
handles safely by construction).

---

## 1. SignatureTooLong (extraction-stage silent drop)

- **Observed evidence**: first measured B2-EVAL-006 (pyjanitor,
  53/2822 rejections), reconfirmed B2-EVAL-007 (pipx, 18/18 — every
  rejection in that corpus), reconfirmed cross-domain a third time
  B2-EVAL-008 (Polly, 213/5403, 3.94%, real extractor-measured byte
  sizes avg 15,867 bytes vs. the 4096-byte limit, max 135,468).
- **Affected languages/domains**: confirmed in Python and C# so far;
  the limit (`MAX_SIGNATURE_BYTES = 4096`,
  `extract/candidate.rs`) is language-agnostic, so any language with
  long method bodies/docstrings captured as `signature_text` is at
  risk.
- **Severity/current impact**: Moderate, cross-domain confirmed (not
  fixed, by explicit instruction across every evaluation it recurred
  in).
  **False negative** — the symbol is entirely absent from the graph
  (no snapshot, no edges to/from it at all), not merely misrepresented.
- **Candidate analysis technique**: raise or make configurable the
  byte limit; or decouple `signature_text`'s own storage from the
  byte-length check that currently governs rejection (e.g. store a
  bounded SUMMARY for display while keeping the full text elsewhere)
  — not committed to, per this inventory's own scope.
- **Blocks agent trust?** Yes, for `Unresolved` cases specifically — an
  agent asking "who calls X" where X was SignatureTooLong-rejected
  will see zero callers and, absent this inventory, could mistake
  that for "X is unused" rather than "X was never indexed at all."

## 2. Module/File structural-container gap

- **Observed evidence**: first measured B2-EVAL-006 (pyjanitor,
  71.8% of files have multiple top-level symbols — highest measured at
  the time), B2-EVAL-007 (pipx, continued), B2-EVAL-008 (Polly, 32.3%
  of files have multiple top-level symbols, median 1/file — LOWER
  need than the function-heavy domains, but not uniformly absent, per
  that evaluation's own partial-class/test-spec-file counterexamples).
- **Affected languages/domains**: Python and Rust (function-heavy,
  higher measured need); C# (class-centric, measured LOWER but
  nonzero need, per B2-EVAL-008).
- **Severity/current impact**: Moderate, tentative "Option 2:
  synthetic File/Module node + Contains" recommendation still not
  implemented (explicitly deferred across every evaluation).
  **Missing capability** — no FP/FN per se; the graph simply has no
  node answering "what does this file/module contain" as a first-class
  queryable unit, only derivable indirectly via `SymbolSnapshot.file()`.
- **Candidate analysis technique**: a synthetic `Module`/`File` node
  type with `Contains` edges to every top-level symbol in that file —
  the tentative Option 2, not yet implemented.
- **Blocks agent trust?** No direct false-positive risk — an agent can
  already derive "what's in this file" by filtering snapshots by file
  path; this gap is about query ERGONOMICS/efficiency, not
  correctness.

## 3. Import/visibility-aware call resolution (CALL-RES-A4)

- **Observed evidence**: named as a backlog item since the original
  Task 9.5 experimental-configuration design; CRC-001
  (`call-resolution-challenges.md`) explicitly notes it has "not yet
  been evaluated" against that case.
- **Affected languages/domains**: every language with a real import/
  module system (Python confirmed via CRC-001/003; C#'s `using` system
  not yet evaluated against this specific technique).
- **Severity/current impact**: not yet built, per explicit instruction
  across every evaluation ("non implementare ancora... A4").
  **False negative/false positive**, depending on direction — A4
  could recover CRC-001's own lost true positive, but must not
  reintroduce CRC-001's own false positive in the process (see that
  case's own explicit "what a future technique must achieve" section).
- **Candidate analysis technique**: named explicitly — import/
  visibility-aware resolution, consulting `ImportFact`'s own already-
  captured alias/resolution data (currently architecturally separate
  from `resolve_calls`, per CRC-003's own root cause).
- **Blocks agent trust?** Yes — CRC-001/003 are both real cases where
  today's absence of an edge could be mistaken for "no relationship,"
  when the real cause is this specific, named, not-yet-built
  technique.

## 4. Overload / type-aware resolution (CALL-RES-A5)

- **Observed evidence**: B2-EVAL-008's own C2 section (Polly's real
  `Handle<TException>()`/`Handle<TException>(Func<...>)` overloads),
  quantified further once B2-ID-011 separated generic-call
  invisibility from overload ambiguity — the `Handle` case specifically
  went from 0 resolved edges (generic calls invisible) to 1 resolved
  edge post-B2-ID-011, with the remaining ambiguity among same-named
  overloads still open and unmeasured in full depth (B2-EVAL-008
  itself remains PAUSED on this point).
- **Affected languages/domains**: C# (measured); any language with
  real parameter-list/return-type overloading is structurally at risk
  (not yet measured for C++, which also supports overloading).
- **Severity/current impact**: not yet built, per explicit instruction.
  **Ambiguous** (correctly declined, not a false positive) OR
  **false negative** depending on whether the resolver's own scope-
  narrowing happens to land on exactly one candidate by name alone
  (a "silent single-candidate luck" scenario this evaluation series
  has explicitly flagged as a risk, not a correctly-disambiguated
  result).
- **Candidate analysis technique**: named explicitly — type-aware or
  arity-aware target disambiguation. B2-ID-010's own
  `generic_arity_of` function is named in B2-EVAL-008's own closing
  report as a plausible, CHEAP precursor (reusing existing context
  data) to a full CALL-RES-A5 checkpoint, cheaper than full type
  inference — a recommendation, not a decision.
- **Blocks agent trust?** Yes, directly — this is exactly the shape of
  case the Ambiguous-tier policy in `AGENT_RELIABILITY_CONTRACT.md`
  exists for: an agent must never pick one overload target
  arbitrarily.

## 5. Generic-arity collision (base-name resolution)

- **Observed evidence**: found during B2-ID-010's own fix (Polly's
  real `IBulkheadPolicy`/`IBulkheadPolicy<TResult>`, both non-partial,
  sharing one bare name), explicitly separated out as a DIFFERENT gap
  from B2-ID-010's own partial-class-reconciliation fix, confirmed
  still present in B2-EVAL-008's own post-fix remeasurement (quantified:
  59 distinct bare type names with 2+ generic-arity variants, ~228
  raw-text-proxy references affected — explicitly flagged as an
  approximate upper bound, not exact).
- **Affected languages/domains**: C# (measured); any language with
  generic/template types sharing a bare name across arities is
  structurally at risk (C++ templates not yet measured for this
  specific collision shape).
- **Severity/current impact**: Moderate, confirmed, not fixed (out of
  scope for B2-ID-010 and B2-ID-011 both, by explicit instruction each
  time).
  **Ambiguous** (correctly declined — confirmed by the golden-corpus
  fixture `generic_arity_collision_gap.cs`, which pins exactly this:
  `BulkheadPolicy : IBulkheadPolicy` currently produces NO edge).
- **Candidate analysis technique**: reuse `CSharpTypeDeclarationContext`'s
  own already-extracted `generic_arity` field to disambiguate a
  base-name lookup by arity BEFORE falling back to "ambiguous" — named
  in B2-EVAL-008's own closing report as the cheap-first-move
  candidate for this and gap #4 above.
- **Blocks agent trust?** Yes, same reasoning as #4.

## 6. Command-registration / callback graph

- **Observed evidence**: first named B2-EVAL-007 (pipx,
  `parser.set_defaults(func=_cmd_install)` / `args.func(args, ctx)` —
  `_cmd_install` confirmed to have ZERO callers in the real `Calls`
  edge set despite being the real dispatch target), reconfirmed via a
  STRUCTURALLY DIFFERENT mechanism B2-EVAL-008 (Polly's
  `BulkheadPolicy`/`FallbackPolicy` constructor-injected delegate
  fields, `_onBulkheadRejected`/`_onFallback`, stored then invoked from
  a DIFFERENT method than the one that received them) — confirming
  this is a real, recurring, cross-domain RELATIONSHIP CATEGORY, not a
  CLI-framework-specific quirk.
- **Affected languages/domains**: Python (pipx's dynamic dispatch
  table), C# (Polly's constructor-injected delegate fields) — two
  independently-confirmed, mechanically different instances of the
  same underlying category.
- **Severity/current impact**: named as its own third relationship
  category (distinct from call-graph and data-flow), explicitly NOT
  implemented as a new edge kind, per instruction every time it
  recurred.
  **Missing capability** — no edge kind in this pipeline represents
  "this value is registered/stored now, invoked dynamically later."
- **Candidate analysis technique**: a new, named edge kind
  (e.g. `Registers`/`InvokesRegistered`) — explicitly not committed to.
- **Blocks agent trust?** Yes, significantly — an agent asking "who
  calls `_cmd_install`" gets zero real callers even though the real
  dispatch relationship exists and matters for impact analysis.

## 7. Data flow (general)

- **Observed evidence**: named explicitly as a distinct, absent
  category B2-EVAL-006 (pyjanitor) — "a file-level import is routinely
  unused by many of that file's own symbols" was the kind of claim
  `ImportFact`'s own module doc explicitly declines to make, since it
  would require real data-flow analysis this pipeline does not do.
- **Affected languages/domains**: all.
- **Severity/current impact**: not built, no plan to build in this
  Phase 1 pass or named as an immediate next step.
  **Missing capability.**
- **Candidate analysis technique**: general data-flow analysis —
  explicitly listed in the user's own "do NOT implement yet" list.
- **Blocks agent trust?** Not directly — no current edge claims to
  represent data flow, so there is no misleading `Resolved`/`Heuristic`
  edge at risk here; this is pure missing capability.

## 8. Control flow / CFG

- **Observed evidence**: named only as a category this pipeline does
  not build, no dedicated measurement exists yet (no evaluation has
  attempted to quantify its absence's impact the way data-flow's
  absence was characterized in #7).
- **Affected languages/domains**: all.
- **Severity/current impact**: not built. **Missing capability.**
- **Candidate analysis technique**: CFG construction — explicitly
  listed in the "do NOT implement yet" list.
- **Blocks agent trust?** Not directly, same reasoning as #7.

## 9. Points-to analysis

- **Observed evidence**: named in the original Task 9.5 experimental-
  configuration backlog (CALL-RES-A6, "points-to-assisted") — not yet
  evaluated against any real case.
- **Affected languages/domains**: all, most relevant to languages with
  heavy aliasing/reference semantics.
- **Severity/current impact**: not built. **Missing capability**
  (though it would also directly address CRC-001-shaped ambiguity
  cases if built).
- **Candidate analysis technique**: points-to analysis — explicitly
  listed in the "do NOT implement yet" list.
- **Blocks agent trust?** Indirectly — its absence is part of why
  CRC-001-shaped ambiguity cases remain unresolved rather than
  disambiguated.

## 10. Flow-sensitive / context-sensitive / interprocedural analysis

- **Observed evidence**: named in the original Task 9.5 backlog
  (CALL-RES-A7 context-sensitive, CALL-RES-A8 hybrid) — not yet
  evaluated against any real case.
- **Affected languages/domains**: all.
- **Severity/current impact**: not built. **Missing capability.**
- **Candidate analysis technique**: named explicitly in the backlog;
  explicitly listed in the "do NOT implement yet" list for this phase.
- **Blocks agent trust?** Not directly measured yet.

## 11. JSX composition

- **Observed evidence**: B2-EVAL-004 (frontend/UI TypeScript domain)
  first noted this gap's own Section D; CRC-002's own ground-truth
  entry explicitly notes "a prop-flow/JSX-composition question this
  pipeline does not currently represent as a typed edge at all."
  Confirmed at the GRAMMAR level in this Phase 1 pass
  (`grammar_conformance.rs::javascript_typescript::tsx_parses_jsx_as_a_distinct_element_node_kind`)
  — the grammar fully supports JSX as real, distinguishable node
  kinds; this is a missing-capability gap, not a parsing limitation.
- **Affected languages/domains**: TypeScript/TSX (React-style
  component composition).
- **Severity/current impact**: not built, explicitly in the "do NOT
  implement yet" list (`Renders` edges).
  **Missing capability** — and a direct contributing cause of CRC-002's
  own false positive (see #16 below): without JSX-composition
  modeling, "which component renders/supplies a prop to this one" has
  no real representation, so a same-named unrelated function wins by
  elimination.
- **Candidate analysis technique**: a `Renders`/prop-flow edge kind —
  explicitly not committed to.
- **Blocks agent trust?** Yes, via its contribution to CRC-002.

## 12. Rust macro call opacity

- **Observed evidence**: documented in `edges/calls.rs`'s own module
  doc comment since early in this project's history; pinned in this
  Phase 1 pass's own golden corpus
  (`tests/corpus/rust/macro_call_opacity_gap.rs`,
  `grammar_conformance.rs::rust::macro_invocation_argument_is_an_opaque_token_tree`).
- **Affected languages/domains**: Rust only (a tree-sitter-specific
  limitation — macros are not expanded before parsing).
- **Severity/current impact**: a real, named, long-standing limitation,
  not fixed. **False negative** — a call wrapped inside a macro's own
  argument list (e.g. `assert_eq!(add(1,2), 3)`) is invisible, which
  the project's own prior finding docs note is PARTICULARLY costly
  because test code is disproportionately macro-heavy
  (`assert_eq!`/`assert!` being exactly where a test's real assertions
  live).
- **Candidate analysis technique**: a macro-aware token-tree heuristic
  — named as a candidate for real-repository-validation-driven future
  work, not committed to.
- **Blocks agent trust?** Yes, specifically for an agent reasoning
  about TEST coverage/call graphs in Rust code, where this gap is
  disproportionately likely to hide real relationships.

## 13. `TestsFor` naming-convention/framework limitation

- **Observed evidence**: `edges/tests_for.rs`'s own module doc comment
  states the naming-convention gate (`test_<name>`/`Test<Name>`) is
  "deliberately conservative" by DESIGN; B2-EVAL-008 (Polly) measured
  this gate producing **ZERO** `TestsFor` edges across the entire
  corpus, root-caused directly: Polly's real test suite exclusively
  uses the xUnit `Should_<behavior>` idiom (confirmed via direct grep,
  zero `Test<Name>`/`test_<name>` occurrences anywhere in the corpus).
- **Affected languages/domains**: C# confirmed (xUnit's own
  `Should_...` convention); any language/framework whose own idiomatic
  test-naming convention does not match `test_`/`Test` is at risk
  (pytest's own `test_*` convention happens to match; many BDD-style
  frameworks across languages would not).
- **Severity/current impact**: Moderate, confirmed cross-domain for
  the FIRST time in this evaluation series (no prior domain's test
  suite exercised this specific mismatch), not fixed — explicitly, per
  the user's own instruction, "Non aggiungere pattern `Should_...`
  specifici di framework senza un design evidence-driven."
  **Missing capability** for this specific naming idiom — not a false
  positive, since the gate is CONSERVATIVE by design (it requires an
  actual call AND the naming match, never just the naming match
  alone).
- **Candidate analysis technique**: NOT yet a committed direction —
  the user's own instruction above explicitly withholds judgment on
  WHICH framework conventions to add until a broader, evidence-driven
  design exists (adding `Should_...` alone would be C#/xUnit-specific
  point-fixing, not a generalizable naming-convention model).
- **Blocks agent trust?** Yes — an agent asking "which tests cover
  class X" in a Polly-like corpus gets a false "zero tests" answer
  that could be mistaken for "untested," when the real answer is "the
  gate's own naming heuristic does not recognize this framework's
  idiom."

## 14. C++ namespace-qualified calls

- **Observed evidence**: NEW, found during B2-ID-011's own independent
  review (not previously known), confirmed via direct grammar probe
  both then and in this Phase 1 pass's own
  `grammar_conformance.rs::cpp::namespace_qualified_call_callee_is_a_qualified_identifier_not_covered_by_the_callee_name_match_arms`
  — `ns::foo()`'s own callee is a `qualified_identifier` node, a kind
  NONE of `callee_name`'s own match arms recognize (not `identifier`,
  `generic_name`, `template_function`, `attribute`, `field_expression`,
  `member_expression`, or `member_access_expression`).
- **Affected languages/domains**: C++ only, confirmed orthogonal to
  B2-ID-011's own generic/template-call fix (a namespace-qualified
  call is invisible regardless of whether the callee is templated).
- **Severity/current impact**: a new residual limitation, explicitly
  NOT fixed as part of B2-ID-011 (out of that fix's own scope), pinned
  in this Phase 1 pass's own golden corpus
  (`tests/corpus/cpp/namespace_qualified_call_gap.cpp`).
  **False negative.**
- **Candidate analysis technique**: extend `callee_name`'s own match
  arms to recognize `qualified_identifier` — not committed to in this
  pass.
- **Blocks agent trust?** Yes, for C++ codebases with real
  namespace-qualified call conventions (common in any
  non-trivially-namespaced C++ project).

## 15. C++ out-of-class qualified method definitions

- **Observed evidence**: `extract/cpp.rs`'s own test suite
  (`an_out_of_class_qualified_method_definition_is_omitted_not_misnamed`)
  confirms `S::method(...)`'s own name-position child is
  `qualified_identifier`, a DIFFERENT kind than the
  `identifier`/`field_identifier` the current extractor recognizes —
  so the definition is correctly OMITTED, never misnamed. Re-confirmed
  on the real Polly corpus B2-EVAL-008 (0 occurrences measured in that
  specific corpus, which uses header-only/inline style throughout —
  the gap's own real-world frequency was NOT measured against a
  header/.cpp-split-heavy corpus in this evaluation series).
- **Affected languages/domains**: C++ only — the real header/.cpp
  split convention this language's own ecosystem commonly uses.
- **Severity/current impact**: a known, documented, by-design omission
  (never a misnaming) — explicitly NOT fixed, and the user's own
  B2-EVAL-008 protocol explicitly forbade inventing declaration/
  definition merge semantics if the model does not support them.
  **Missing capability** (the method is simply absent from the
  graph).
- **Candidate analysis technique**: declaration/definition merging —
  explicitly, repeatedly deferred by the user's own instruction across
  this evaluation series, pending a real design decision (not a quick
  extraction fix).
- **Blocks agent trust?** Yes, potentially significantly, for any
  real-world C++ codebase using the header/.cpp split convention —
  NOT yet measured at scale against such a corpus; flagged as a
  residual risk for a future evaluation to quantify properly.

## 16. CRC-002: local-binding/shadowing scope-blindness

- **Observed evidence**: B2-EVAL-004 (conduit-react),
  `call-resolution-challenges.md`'s own CRC-002 entry — a destructured
  component PROP (`handleSubmit`) is confidently (StaticAnalysis-
  labeled) resolved to an UNRELATED, same-named top-level function,
  because this pipeline has no local-binding/shadowing check at all.
  Confirmed at the grammar level in this Phase 1 pass
  (`grammar_conformance.rs::javascript_typescript::destructured_parameter_is_an_object_pattern_not_a_plain_identifier_parameter`)
  — the grammar fully distinguishes a destructured parameter as a real
  `object_pattern` node; this is a RESOLVER gap, not a parser
  limitation.
- **Affected languages/domains**: JavaScript/TypeScript/TSX (confirmed);
  any language with parameter-shadowing semantics the resolver does
  not check is structurally at risk (not yet measured for Python/
  Rust/C#/C++'s own parameter-shadowing shapes).
- **Severity/current impact**: a CONFIRMED, measured, NOT fixed false
  positive — distinct in kind from CRC-001 (ambiguity) and CRC-003
  (alias-blindness): this is a CONFIDENT, WRONG, `StaticAnalysis`-
  labeled edge, the single most dangerous shape for agent trust of any
  gap in this inventory, since it looks exactly like a `Resolved` edge
  with none of the hedging a `Heuristic`/`Ambiguous` tag would carry.
- **Candidate analysis technique**: a local-binding/shadowing check
  BEFORE any cross-file candidate search begins — named explicitly in
  CRC-002's own "what a future technique must achieve" section.
- **Blocks agent trust?** **Yes, most severely of any gap in this
  inventory** — this is a `Resolved`-tier edge that is factually wrong,
  which the reliability contract's own `Resolved` tier explicitly
  treats as safe to use without further hedging. This is the clearest
  argument in the whole inventory for treating `Resolved` as "this
  pipeline found no ambiguity," never as "this is definitely correct."

## 17. CRC-001: ambiguous unqualified reference, no scope signal

- **Observed evidence**: B2-EVAL-002 (pytorch/examples),
  `call-resolution-challenges.md`'s own CRC-001 entry — two unrelated
  classes both named `RNNModel`; the pre-A1/A2/A3 configuration
  produced BOTH as separate heuristic edges (inseparable true+false
  positive mix); the current (A1+A2+A3) configuration produces NEITHER
  (a clean false negative, zero false positives).
- **Affected languages/domains**: Python (measured); any language
  where two genuinely unrelated symbols share a bare name across files
  is structurally at risk (effectively all of them).
- **Severity/current impact**: a confirmed, measured, NOT fixed
  ambiguity case — explicitly the calibration case for CALL-RES-A4 (see
  #3 above).
  **False negative** in the current configuration (a real true
  positive is lost, but — critically — with zero false positives,
  which this evaluation series treats as the correct trade-off
  direction per its own "no edge beats a false edge" discipline).
- **Candidate analysis technique**: import/visibility-aware resolution
  (A4), per CRC-001's own explicit two-part pass condition (recover
  the true edge WITHOUT reintroducing the false positive).
- **Blocks agent trust?** This is exactly the `Ambiguous`-tier case the
  reliability contract's own policy is written for — handled safely
  BY the current "no edge" behavior, which is why this gap's own
  severity, unlike CRC-002's, is about missing capability (an
  agent correctly sees nothing, rather than a wrong confident answer),
  not misleading evidence.

## 18. CRC-003: import-alias blindness

- **Observed evidence**: B2-EVAL-006 (pyjanitor),
  `call-resolution-challenges.md`'s own CRC-003 entry — a call through
  a LOCAL IMPORT ALIAS (`from .ml import get_features_targets as
  _get_features_targets`) produces zero candidates by direct name
  match, even though `ImportFact.alias` already correctly captures the
  real mapping — the two pieces of information exist independently in
  this pipeline and are never joined.
- **Affected languages/domains**: Python (measured, with a `459`-alias-
  captured real corpus confirming the alias DATA itself is correct);
  any language with import-aliasing syntax is structurally at risk
  (C#'s own `using X = Y;` alias syntax not yet measured against this
  specific gap).
- **Severity/current impact**: a confirmed, measured, NOT fixed false
  negative — the real function and the real call both exist; the
  relationship between them is the ONLY thing missing, and only
  because of an architectural separation (alias data vs. call-name
  matching), not because either piece of information is itself wrong.
- **Candidate analysis technique**: named explicitly in CRC-003's own
  "what a future technique must achieve" section — when a callee name
  matches no snapshot directly, check the calling file's own
  `ImportFact.alias` before giving up.
- **Blocks agent trust?** Same reasoning as CRC-001 — currently handled
  safely (silent absence, not a wrong confident claim), but IS a real,
  recoverable relationship an agent should be told might be missing
  due to this specific, named, understood gap.

## 19. Remaining documented extraction-coverage gaps (grouped)

Smaller, already-documented gaps not warranting their own full entry,
grouped here for completeness:

- **C# `struct_declaration`/`enum_declaration`/`field_declaration`/
  `constructor_declaration` are not currently walked by
  `extract/csharp.rs` at all** (B2-EVAL-008's own C-prelim section,
  real counts measured on Polly: 46 structs, 15 enums, 730 fields, 379
  constructors — none extracted as symbols today). Missing capability,
  confirmed real and nonzero at scale, not fixed.
- **C#'s own explicit interface implementation naming**
  (`PolicyBuilder IFoo.Foo()`) and **operator/conversion-operator
  declarations, record declarations, and local functions** are
  confirmed (by B2-ID-009's own independent review, via direct
  adversarial probing) to be simply NOT WALKED by `extract/csharp.rs`'s
  own `walk()` at all — a pre-existing, unrelated-to-B2-ID-009 coverage
  gap, not re-measured for real-world frequency in this evaluation
  series.
- **The Rust extractor's own macro-opacity gap** (see #12) is the only
  Rust-specific gap with a dedicated full entry; no other Rust
  extraction gaps have been measured in this series beyond that one.

## Residual limitation of this inventory itself

This inventory is a SNAPSHOT as of this Phase 1 hardening pass. It
indexes every gap explicitly named across B2-EVAL-001 through 008 and
B2-ID-001 through 011 as of this document's own writing — it does not
claim to be exhaustive over every possible gap this pipeline might
have, only over every gap this evaluation series has ALREADY measured
or named. New gaps discovered in future evaluations should be added
here, not left to live only in their own individual finding/evaluation
document.
