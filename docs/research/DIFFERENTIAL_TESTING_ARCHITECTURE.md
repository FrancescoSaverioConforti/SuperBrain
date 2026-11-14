# Differential-testing architecture

Quality & Semantic Reliability Hardening, Phase 1, deliverable 10.
**Status: architecture designed; the C# (Roslyn) PoC is now a small
AUTOMATED differential suite (Phase 2, deliverable 8) — see results
below. Rust/TypeScript/C++/Python remain design-only in this pass.**

### Phase 2 update: from PoC to automated suite

`crates/codegraph-index/examples/roslyn_differential_suite.rs`
automates what was previously a manual `dotnet run` + eyeball
comparison: it invokes the oracle, runs this pipeline's own real
extraction (`extract_symbols_reporting_rejections` +
`extract_csharp_type_declaration_contexts`) against the identical
golden-corpus fixtures, and produces a structured, machine-readable
`Divergence` list via a dependency-free JSON reader (consistent with
`ground_truth.rs`'s own no-serde discipline). A real run on the shared
corpus slice: 10 type declarations + 6 method declarations on each
side, **zero divergences** — an independent, external-oracle
confirmation that extraction agrees with Roslyn's real semantic model
here. 4 self-tests (including an adversarial case that proves
`compare()` genuinely detects an injected divergence, not vacuously
always-equivalent) back the comparison logic itself. It is deliberately
an `example`, never a `#[test]`: `cargo test` must never require a
working `dotnet` SDK, keeping the oracle a pure opt-in validation tool,
never a runtime or test-suite dependency, per the standing instruction
from Phase 1 authorization.

## What "differential testing" means here

This pipeline's own correctness claims (B2-ID-009/010/011's own
finding docs, the golden corpus, the grammar-conformance matrix) are
all self-referential in one specific sense: they confirm this
pipeline's OWN extractor/resolver produces the output ITS OWN authors
believe is correct, verified by direct source reading and grammar
probes. None of them check this pipeline's conclusions against an
INDEPENDENT authority that has its own, separately-implemented
understanding of the language's real semantics. Differential testing
closes that gap: run the SAME source through a trusted external oracle
(a real compiler, type-checker, or language service) and diff its
conclusions against this pipeline's own.

## Design (applies to every language)

1. **Shared input**: reuse the SAME golden-corpus fixtures
   (`tests/corpus/<language>/`) as both this pipeline's own
   `golden_corpus.rs` test suite and the oracle — never a separate,
   oracle-specific fixture set, so a divergence is a genuine
   disagreement about the SAME input, not an artifact of two
   different inputs that merely look similar.
2. **Oracle invocation**: a small, standalone tool under `tools/`
   (NOT part of any Cargo workspace member, never a build or runtime
   dependency of `codegraph-index` or any other crate) that parses the
   fixtures with the real external toolchain and emits normalized
   output.
3. **Output normalization**: each oracle's own output uses that
   ecosystem's native vocabulary (Roslyn's `ITypeSymbol`/`IMethodSymbol`,
   rustc's own diagnostic/HIR concepts, etc.) — this design does NOT
   propose forcing every oracle into one identical schema, since doing
   so would lose real information specific to the oracle; instead,
   each oracle's output is diffable BY A HUMAN (or a future per-oracle
   comparison script) against this pipeline's own
   `SymbolSnapshot`/`Edge` output for the same fixture, field by
   field, with an explicit mapping documented per language (see the
   C# PoC's own field-by-field mapping below as the template every
   other language's future oracle should follow).
4. **Where it plugs into CI/the test workflow**: NOT as a `cargo test`
   target (the oracle tools have no Rust toolchain dependency at all,
   deliberately — see "never a runtime dependency" below) — as a
   SEPARATE, manually-invoked validation step for now, documented here
   as a candidate for a future CI job once a real per-language
   comparison script exists (not built in this Phase 1 pass). Running
   it is currently: `cd tools/<oracle>-oracle && dotnet run` (or the
   equivalent for a future language), then diff its output against
   the corresponding `cargo test --test golden_corpus -- --nocapture`
   run for the same fixture by hand.

## Never a runtime dependency

Every oracle tool under `tools/` is isolated from the Rust workspace:
no `Cargo.toml` member references it, no `build.rs` invokes it, and
`codegraph-index`'s own production code has zero awareness any of
these tools exist. They are test/validation infrastructure ONLY, per
the user's own explicit instruction. The C# oracle specifically adds a
new, SuperBrain-external dependency (the .NET SDK) that this
repository's own CI/build environment is NOT assumed to have — running
it is an opt-in validation step a contributor performs locally when
touching C# extraction/resolution code, not a build requirement.

## C# oracle: Roslyn — REAL PoC built and run

**Location**: `tools/roslyn-oracle/` (a minimal `dotnet new console`
project, `Microsoft.CodeAnalysis.CSharp` 4.11.0 as its only package
reference).

**What it does**: parses the SAME 6 golden-corpus C# fixture files
already in `tests/corpus/csharp/` (the B2-ID-009, B2-ID-010, generic-
arity-collision, and B2-ID-011 fixtures) using Roslyn's real
`CSharpSyntaxTree`/`CSharpCompilation`/semantic model, and emits one
JSON object per type declaration, method declaration, and call site —
preserved in full at
`docs/research/evidence/roslyn-oracle-poc-output.jsonl`.

### Field-by-field mapping (this pipeline's own output vs. Roslyn's)

| This pipeline's own concept | Roslyn's own concept (as emitted) |
|---|---|
| `SymbolSnapshot.name()` (a type) | `TypeDeclarationSyntax.Identifier.Text` |
| `SymbolSnapshot.kind()` (Class/Interface) | `TypeDeclarationSyntax.Keyword.Text` ("class"/"interface") |
| `csharp_declarations::CSharpTypeDeclarationContext.is_partial` | `TypeDeclarationSyntax.Modifiers` containing "partial" |
| `csharp_declarations::CSharpTypeDeclarationContext.namespace` (hand-canonicalized across 3 syntactic forms) | `ISymbol.ContainingNamespace.ToDisplayString()` — Roslyn's OWN real semantic-model namespace resolution, not a hand-rolled canonicalization |
| `csharp_declarations::CSharpTypeDeclarationContext.generic_arity` | `TypeDeclarationSyntax.TypeParameterList.Parameters.Count` |
| base-list names collected by `collect_csharp_base_list` (syntax-level, pre-disambiguation) | `TypeDeclarationSyntax.BaseList.Types` (syntax-level, same granularity) |
| the final, DISAMBIGUATED `Extends`/`Implements` edge `resolve_extends_implements_with_csharp_contexts` actually produces | `ITypeSymbol.BaseType`/`ITypeSymbol.Interfaces` — Roslyn's OWN real, TYPE-CHECKED semantic resolution, the genuinely independent check |
| `SymbolSnapshot.name()` (a method) | `MethodDeclarationSyntax.Identifier.Text` |
| the exact B2-ID-009 shape (method name vs. return type) | `MethodDeclarationSyntax.Identifier.Text` vs. `.ReturnType.ToString()`, kept as separate fields from the start — Roslyn's own grammar never has this ambiguity's own collision shape at the syntax-tree level the way tree-sitter's positional scan did |
| a `Calls` edge's `to` (this pipeline's own pure-name-match resolution) | `SemanticModel.GetSymbolInfo(invocation).Symbol` — Roslyn's OWN real, TYPE-CHECKED call resolution, including `CandidateSymbols`/`CandidateReason` for genuinely ambiguous/unresolved cases (Roslyn's own equivalent of this project's proposed `Ambiguous`/`Unresolved` tiers) |

### Real results from this PoC run (not hypothetical — see the preserved evidence file)

1. **B2-ID-010 fixture, independently confirmed**: Roslyn's own
   semantic model resolves `BulkheadPolicy`'s `semantic_base_type` to
   `"Polly.Policy"` — exactly matching this pipeline's own post-fix
   reconciled `Extends` edge target. This is genuine, external
   confirmation that B2-ID-010's fix lands on the semantically correct
   answer, not merely an answer this pipeline's own tests are
   internally consistent about.

2. **Generic-arity-collision fixture, concrete oracle-backed evidence
   for the gap inventory's own entry #5**: Roslyn's real type-checked
   resolution of `BulkheadPolicy : IBulkheadPolicy` correctly returns
   `semantic_interfaces = ["Polly.Bulkhead.IBulkheadPolicy"]` — the
   ARITY-0 interface specifically, correctly disambiguated from its
   own arity-1 sibling `IBulkheadPolicy<TResult>` declared in the same
   file. This is now an ORACLE-CONFIRMED existence proof that the
   correct answer IS recoverable from the available syntax (Roslyn
   does it without any additional information this pipeline lacks
   access to in principle) — strengthening the gap inventory's own
   "generic-arity-aware disambiguation... cheaper than full type
   inference" recommendation from a plausibility argument into a
   demonstrated-reachable one.

3. **B2-ID-011 fixture, confirmed consistent**: Roslyn resolves both
   the bare call `Handle<System.Exception>()` to `Caller.Handle` and
   the qualified call `policy.Handle<System.Exception>()` to
   `Policy.Handle` — both calls bind successfully with zero ambiguity
   candidates (`roslyn_candidate_count: 0`, meaning Roslyn found
   exactly one unambiguous target each time, consistent with this
   pipeline's own fix extracting and resolving both call shapes
   post-B2-ID-011).

4. **Semantic Resolution & Reliability Checkpoint, Priority 4 —
   arity-aware resolution and SignatureTooLong, both oracle-confirmed**:
   two new fixture groups (`arity_aware_resolution`,
   `signature_too_long_linkage`) extend this PoC and the automated
   suite (`roslyn_differential_suite.rs`) to this checkpoint's own new
   resolution path.
   - `generic_arity_narrowing_call_resolution.cs`'s
     `picker.Choose<System.Exception>(1)` call: Roslyn resolves this
     to a single, unambiguous `AmbiguousPicker.Choose` target
     (`roslyn_candidate_count: 0`) by using the literal argument's own
     TYPE (`int`) to disambiguate `Choose<T>(int)` from
     `Choose<T>(string)` — full semantic overload resolution SuperBrain
     deliberately does not attempt. SuperBrain's own resolver correctly
     stays `Ambiguous` for this exact call (confirmed by
     `same_generic_arity_with_differing_ordinary_parameter_type_stays_ambiguous`
     in `golden_corpus.rs`). This is NOT a SuperBrain defect: it is the
     expected, honestly-scoped gap between arity-only narrowing and
     full type-aware overload resolution (A5's own future scope),
     oracle-confirmed on a real divergence rather than merely asserted.
   - `signature_too_long_extraction_rejection_linkage.cs`'s
     `Target.VeryLongSignatureMethod(...)` (450 real `int` parameters,
     call site argument count matching): the automated suite's own
     machine-readable diff reports this method as
     `MethodObservationOnlyIn { side: "roslyn", ... }` — Roslyn resolves
     it cleanly (`roslyn_resolved_target: "VeryLongSignatureMethod"`,
     containing type `Target`) because Roslyn has no signature-length
     limit, while SuperBrain's own extraction deliberately drops this
     candidate at `MAX_SIGNATURE_BYTES` (4096 bytes) before it ever
     reaches promotion. This is SuperBrain's own documented, deliberate
     bounded-extraction policy working as designed, oracle-confirmed as
     the EXPECTED divergence shape (never a parser bug silently
     dropping a symbol it should have kept) — exactly the real-corpus
     shape this checkpoint measured 213 instances of against
     App-vNext/Polly. See
     `docs/research/evidence/b2-eval-006-priority4-roslyn-differential-arity-and-signature-too-long.txt`
     for the preserved raw run.

### Residual limitations of this PoC

- The Roslyn oracle itself (`tools/roslyn-oracle/Program.cs`) runs all
  8 of its own defined `fixtureGroups` cleanly. The standalone CRC/
  non-C#-specific fixtures (TypeScript CRC-002, Rust, C++) have NO C#
  oracle equivalent, by definition, and are out of this oracle's own
  scope.
- **Known scope gap, found during the Semantic Resolution &
  Reliability Checkpoint's own Priority 4 work (not a new defect, a
  pre-existing limitation of this comparison harness itself)**: the
  AUTOMATED suite's (`roslyn_differential_suite.rs`)
  `extract_superbrain_observations` walks `tests/corpus/csharp/` with
  a single, non-recursive `read_dir` — it never descends into the
  `b2_eval_008_real/` subdirectory, even though
  `tools/roslyn-oracle/Program.cs`'s own `b2_eval_008_real_polly_files`
  fixture group DOES include those files. This is why every
  `b2_eval_008_real/*.cs` type/method ends up reported as
  `TypeObservationOnlyIn`/`MethodObservationOnlyIn { side: "roslyn" }`
  on every run — a harness-side comparison-scope artifact, confirmed
  by direct code reading, not a genuine semantic divergence about
  SuperBrain's own extraction (SuperBrain's own golden-corpus tests
  against those exact files pass independently). Fixing this
  (recursing into subdirectories) is real, scoped, low-risk future
  work, intentionally not done as part of this checkpoint (out of this
  pass's own authorized scope — this document only records the
  finding honestly so a future pass does not re-discover it from
  scratch).
- This PoC does not yet cover `TestsFor`, `Imports`/`using` resolution,
  or the callback/registration-graph category — scoped deliberately to
  the findings/gaps this project's own evaluation series has worked on
  to date, not a claim of full coverage.
- The suite (`cargo run --example roslyn_differential_suite`) IS now
  automated (Phase 2's own deliverable 8) and extended with two more
  fixture groups by the Semantic Resolution & Reliability Checkpoint's
  own Priority 4 (see result 4 above) — it is still run manually/
  on-demand, not wired into any repeatable CI job, since doing so
  would require a `dotnet` SDK on every CI runner this project uses,
  a cost/dependency decision out of this document's own scope to make
  unilaterally.

## Rust oracle: rustc / rust-analyzer — design only

- **What to compare**: struct/enum/trait declaration names (rustc's
  own HIR, accessible via `rustc_driver`'s own callback API, or more
  practically `rust-analyzer`'s own `ra_ap_*` crates, which expose a
  more stable, documented API surface than `rustc_driver` itself);
  `impl Trait for Type` resolution (directly comparable to this
  project's own `ImplFact`); call-site resolution for the macro-
  opacity gap specifically (does `rust-analyzer`'s own macro expansion
  make `assert_eq!(add(1,2), 3)`'s inner call visible, where
  tree-sitter's own non-expanding parse cannot? — this would be a
  concrete, measurable argument for or against building the
  macro-aware heuristic named in the gap inventory).
- **Why design-only in this pass**: `rust-analyzer`'s own `ra_ap_*`
  crates are real and usable, but integrating them (even as an
  isolated `tools/` binary) is a meaningfully larger lift than the C#
  PoC — `rust-analyzer`'s own crate API is less stable/documented for
  external consumption than Roslyn's, and a careful PoC would need
  real investigation time this Phase 1 pass's own 5-core-deliverable
  priority did not allocate for a second full PoC.

## TypeScript oracle: the TypeScript compiler / language service — design only

- **What to compare**: `tsc`'s own type-checker API
  (`ts.createProgram`/`program.getTypeChecker()`) can resolve
  `class_heritage`'s own `extends`/`implements` clauses with full type
  information, and critically, could directly validate CRC-002's own
  shape: does `tsc`'s own checker correctly recognize
  `handleSubmit` as a local, destructured-parameter binding (which it
  certainly does, since this is ordinary JS/TS scoping any real
  compiler must get right) — making `tsc` a strong, readily-available
  oracle specifically for confirming CRC-002's root cause
  independently.
- **Why design-only**: Node.js/the TypeScript compiler toolchain
  availability was not confirmed in this environment during this pass
  (unlike .NET, which was confirmed present and used); building this
  PoC would require first confirming that dependency is available,
  which this pass did not prioritize checking given the 5-core-
  deliverable scope ruling.

## C++ oracle: Clang tooling / compiler AST — design only

- **What to compare**: `clang -Xclang -ast-dump` or `libclang`'s own
  Python/C bindings could directly validate the two confirmed C++
  gaps (`qualified_identifier` namespace-qualified calls,
  `qualified_identifier` out-of-class method definitions) by showing
  what Clang's own AST looks like for the SAME golden-corpus fixtures,
  and whether Clang's own semantic analysis resolves `ns::foo()`
  trivially (it certainly does — this would be another existence-proof
  argument, like the Roslyn generic-arity result above, that a fix is
  reachable without novel research, just engineering effort).
- **Why design-only**: `libclang`'s own Python bindings or a `tools/`
  C++ program driving `clang -ast-dump` are both plausible shapes;
  neither was attempted in this pass given the scope ruling.

## Python oracle: `ast` module / a type checker — design only

- **What to compare**: Python's own stdlib `ast` module can directly
  confirm this pipeline's own `attribute`/`call`/`class_definition`
  shapes (useful as a lightweight syntax-level cross-check); a real
  type checker (`mypy`/`pyright`) would be needed for anything
  requiring actual name resolution (e.g. independently confirming
  CRC-003's own alias-resolution target, or B2-ID-001's own
  same-name-different-file non-collision).
- **Why design-only**: Python's own `ast` module is trivially
  available (it ships with Python itself) and would be the cheapest
  possible second PoC to build — explicitly named here as the
  recommended NEXT oracle to build in a future hardening pass, given
  it requires zero new external tooling/environment dependencies
  (unlike the Rust/TypeScript/C++ oracles above, all of which need a
  toolchain this pass did not confirm or set up).
