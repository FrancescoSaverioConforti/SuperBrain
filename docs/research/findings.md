# B2 Project Findings Index

See [`POLICY.md`](POLICY.md) for the policy this index implements, and
[`EVALUATION_POLICY.md`](EVALUATION_POLICY.md) for how evaluations
(like [B2-EVAL-001](evaluations/B2-EVAL-001.md), which discovered the
findings below, and [B2-EVAL-002](evaluations/B2-EVAL-002.md), the
Task 9.5 before/after re-evaluation) are recorded. Each row links to
the full finding file under `findings/`.

| ID | Problem | Root cause | Impact | Solution | Validation | References |
|---|---|---|---|---|---|---|
| [B2-ID-001](findings/B2-ID-001.md) | `SymbolSnapshotId` collides for two distinct symbols with identical name/body in one file (`init_weights` in two different classes, `pytorch/examples`) | Identity tuple had no notion of a symbol's position/scope in its file | Critical — crashed the pipeline on the first real repository tried | Added the symbol's own `ByteSpan` to the identity hash, bumped `superbrain-symbol-snapshot-v1` → `v2` | 6 new regression tests + golden vector (0 missed, 6 mutants), full workspace green, clippy/deny clean, real-repo reproduction confirmed fixed, independent review APPROVE | Godfrey & Zou (2005), Falleri et al. (2014, GumTree) |
| [B2-ID-002](findings/B2-ID-002.md) | `resolve_calls`/`resolve_imports`/`resolve_extends_implements` emit one literal `Edge` per syntactic occurrence, so a repeated call/import/base-mention produces duplicate `(from,to,kind)` triples | Syntactic-occurrence loop never collapsed into the semantic one-edge-per-relation model `CodeGraph` enforces | Critical — crashed `observe_commit`/`build_graph` on realistic, common input (same name called more than once) | Per-resolver deduplication by `(from,to[,kind])`, deterministic evidence tie-break in `resolve_calls`; `resolve_tests_for` audited and found not vulnerable | 15 new tests across 3 files (0 missed, 96 mutants after closing 11 initial gaps), full workspace green, clippy/deny clean, real-repo reproduction confirmed fixed (827 duplicate triples → 0), independent review APPROVE | No directly applicable authoritative source identified (standard call-graph-construction practice, no specific citation confirmed) |
| [B2-ID-003](findings/B2-ID-003.md) | A chained-call "false negative" reported in B2-EVAL-001 turned out to be an evaluation-harness subject-selection bug, not a pipeline defect | Harness selected "the current snapshot named X" by local name alone, ambiguous once >1 class in a file shares a member name | Important — affected research validity of a published evaluation claim, not production correctness | Scope-aware harness lookup via `Contains` edges (`find_current_member_of_class`), reusing Task 9.5's own Priority 2 addition | Isolated + full-body + 5× real-corpus reproduction all confirm correct resolution once the harness is fixed; B2-EVAL-001 corrected with an explicit erratum, not a silent edit | No directly applicable authoritative source identified |
| [B2-ID-004](findings/B2-ID-004.md) | `resolve_calls` matched receiver-qualified calls (`self.src_mask.size(0)`) and unqualified common-name calls (`forward`) purely by bare name, corpus-wide, producing confirmed cross-class/cross-language false positives (Q3) and a 19-for-19 false-positive rate on incoming callers (Q6) | No receiver awareness (the receiver segment was extracted then discarded) and no scope awareness (ambiguous candidates were never narrowed before being treated as equally plausible) | Important — the single most commonly-asked coding-agent question ("what does X call"/"who calls X" for a common name) was effectively unanswerable; 97.6% of B2-EVAL-001's original typed edges were `NameHeuristic` fan-out | Receiver-aware filtering for `self`/`this`-qualified calls (restrict to the caller's own class); scope-hierarchy narrowing (same class → same file) for unqualified ambiguous calls, with "no edge" replacing "N Hypothesized edges" once narrowing is exhausted — both recorded as an experimental configuration (CALL-RES-A1+A2+A3) per the new Evidence-before-claims Experimental Policy | 318 tests (up from 308) all passing, full workspace/clippy/deny clean; real-corpus re-run: Q3 false positives 2→0 (5 genuine Field calls found instead), Q6 false positives 19→0 | Standard class-hierarchy-analysis/receiver-type-pruning practice in static call-graph construction; no single paper cited for this project's own specific narrowing rule |
| [B2-ID-005](findings/B2-ID-005.md) | The original symbol-to-symbol `EdgeKind::Imports`/`resolve_imports` captured 0 of 589 real import statements in a real 97-file corpus | Granularity mismatch: an import statement is a file/module-level fact, never honestly attributable to one arbitrarily-chosen symbol in that file, and Task 4's own `signature_text` extraction boundary never includes a file's leading import statements anyway | Important — a core, frequently-needed capability (knowing what a file imports/who imports a given module) was entirely non-functional on real data despite passing its own (artificial-fixture) unit tests | Removed `EdgeKind::Imports`/`resolve_imports` outright; replaced with a separate file-level `ImportFact` model (`codegraph_index::imports`), splitting local syntax extraction (incrementally reused) from global semantic resolution (always recomputed from the full current pool), wired into the real `CodeGraphIndexer` production pipeline with its own atomic-publication guarantee | 353 tests (up from 325) all passing, full workspace/clippy/deny clean; mutation testing closed (319 mutants tested across all Task 9.5 files, 275 caught, 41 unviable, 3 missed — all 3 confirmed and documented as equivalent mutants, 0 timeouts); real-corpus re-run via the real indexer: 589 line-based → 691 real `ImportFact`s extracted (41 internal, 621 external, 29 ambiguous, 155 aliases); incremental reuse confirmed (687/691 raw statements reused on a 1-file-changed update); independent review APPROVE | Standard compiler front-end practice (syntax extraction vs. semantic/symbol resolution as separate phases); no single paper cited for this project's own specific `ImportFact` design |
| [B2-ID-006](findings/B2-ID-006.md) | `resolve_calls` attributes a call site to EVERY enclosing container snapshot whose span happens to contain it (e.g. a class), not only the nearest real executable caller (its method) — confirmed to generalize to nested functions and Rust trait default methods, not Class-specific | Every extractor sets a container's `signature_text` to its FULL body (nested calls included); `resolve_calls` has no "nearest enclosing executable scope" concept and no `SymbolKind` caller-eligibility filter | Important — measured 7.7% (25/326, B2-EVAL-003 backend corpus) to 29.2% (256/878, B2-EVAL-001/002's own `pytorch/examples` corpus) of all `Calls` edges are container-originated, most confirmed duplicates of a real method-level edge to the same target; corrupts aggregate edge/fan-out counts and any caller-kind-unaware "who calls X" query | **CLOSED — independent review APPROVE, no Critical/Important issues, no changes required** — a `SymbolKind::can_originate_calls` caller-eligibility filter plus a nested-scope span boundary in the call-site walk, reusing the same span-nesting fact `resolve_contains` already computes | 13 regression tests (Python/Rust/C#/C++/JS, all passing), 366/366 `codegraph-index` tests green, mutation testing CLOSED (107/114 caught, 6 unviable, 1 pre-existing equivalent mutant, 0 genuine gaps); contamination repair CLOSED on both corpora (`pytorch/examples` 878→535, backend corpus 326→293, zero new edges, Q3/Q4/Q6 unchanged); errata added to B2-EVAL-001/002 | Néron, Tolmach, Visser & Wachsmuth (2015), *A Theory of Name Resolution*, ESOP 2015 — scope graphs support attributing a construct to its nearest enclosing scope via an explicit relation, not raw span containment; cited as general support, not as this project's own algorithm |
| [B2-ID-007](findings/B2-ID-007.md) | The SAME structural pattern found in B2-ID-006 (`resolve_calls`) also affects `resolve_extends_implements` (nested type declaration's base clause attributed to its outer container too) and `resolve_tests_for` (narrower trigger); `resolve_instantiates` and `resolve_contains` are confirmed NOT affected | Shared structural cause (unconditional recursion into nested AST nodes with no declaration boundary), but each relation needs its OWN ownership rule — `Extends`/`Implements` is owned by the specific declaration whose header names the base, not by "nearest executable scope" (B2-ID-006's own rule, meaningless for inheritance) | Important — confirmed on Python and C# (`Extends`), Python (`TestsFor`); `Extends` edges are always asserted as `StaticAnalysis`-certain, making a spurious one a more direct invariant violation than `Calls`'s own heuristic case | **CLOSED — independent review APPROVE, no Critical/Important issues, no changes required** — a parse-tree-local node-kind boundary for `Extends`/`Implements`; a cross-snapshot span check (not reusing `Calls`'s own eligibility filter) for `TestsFor` | 12 new regression tests (10 Extends/Implements across Python/C#/C++/JS/TS/Rust, 2 TestsFor including the exact adversarial collision case), 378/378 `codegraph-index` tests green, 0 regressions; mutation CLOSED separately for each resolver (Extends/Implements: 39 mutants, 0 missed; TestsFor: 20 mutants, 0 missed) | Same Néron et al. (2015) scope-graphs citation as B2-ID-006, extended with Sun et al. (2023), *ACER: An AST-based Call Graph Generator Framework*, arXiv:2308.15669, as context for AST-based relationship extraction generally — neither prescribes this project's own per-relation fix mechanisms |
| [B2-ID-008](findings/B2-ID-008.md) | `resolve_extends_implements` produces ZERO `Extends`/`Implements` edges on any real Rust repository, despite real traits/impls existing and being individually extracted as symbols (`sharkdp/fd`: 2 traits, 41 impls, 0 edges produced) | `extract/rust.rs` never extracts `impl_item` as a `RawSymbolCandidate` at all, so no `SymbolSnapshot`'s own signature span ever contains one, and the resolver's own Rust-specific collector (`collect_rust_impl_trait`, otherwise correct) is structurally unreachable on real Rust source — genuinely dead code on the real-world path, not merely under-tested | Important — complete (100%), silent absence of the entire trait/impl relationship on Rust, independent of corpus; every trait/impl-focused coding-agent question ("which traits does X implement," "which types implement this trait") is Not answerable even though both endpoints exist as real graph symbols | **OPEN — design decision pending, not implemented.** Four alternatives presented (impl-as-its-own-SymbolKind node; impl-as-a-non-symbol ImplFact reusing the ImportFact pattern; file/module-level resolver with no new intermediate type; an evidence-driven hybrid) with explicit pros/cons, corpus-based classification (inherent vs. trait impls, generics, lifetimes, multi-impl-per-type), and a confirmed tree-sitter grammar fact (`impl_item`'s own named `trait`/`type` fields) relevant to whichever is chosen | N/A — no fix implemented yet; [B2-EVAL-005](evaluations/B2-EVAL-005.md) PAUSED pending this finding's resolution | rustc HIR's own `Impl` node (official compiler-internals documentation) models impl/trait/type as three distinct identities, supporting the impl-as-its-own-node alternative; `ferrograph`/`tldr-core` (real open-source Rust code-graph tools) independently confirm both design directions are used in practice; no peer-reviewed paper found specific to this exact design choice |

## Discussion: B2-ID-001

**Problem.** `SymbolSnapshotId`'s `v1` derivation assumed `(repo, commit,
file path, kind, name, signature)` uniquely identifies a declaration
within one file. It does not: two distinct methods can share a name and
a byte-identical body while living in different enclosing classes.

**Evidence.** The real-repository smoke test against `pytorch/examples`
(commit `2095514...`) found exactly this shape —
`RNNModel.init_weights` and `TransformerModel.init_weights` — and the
production pipeline correctly, but fatally, rejected the collision as
`DuplicateSnapshotId`.

**Root Cause.** A data-model deficiency: no positional or scope
disambiguator in the identity tuple.

**Alternatives.** A `QualifiedName` (scope-chain) alternative was
considered and rejected for this fix specifically because Task 4 has no
real scope-chain resolver yet, and the project's own standing
instruction was not to fake one just to patch this bug. A random UUID
was rejected for losing determinism. Relaxing `CodeGraph`'s duplicate-id
invariant was rejected for hiding real collisions instead of preventing
them.

**Solution.** Added the symbol's own `ByteSpan` to the hash; bumped the
domain separator to `v2` since the canonical field set itself changed.

**Experimental Validation.** New unit tests (including a golden vector
and a direct regression reproducing the real collision) pass; the full
workspace test suite, clippy, and `cargo deny check` are all clean; a
re-run of the exact real-repository pipeline no longer fails and a
diagnostic scan over the full real snapshot pool found zero remaining
collisions. An independent review (`agent-protocols:code-reviewer`)
traced the hash encoding by hand, confirmed every real call site of
`SymbolSnapshotId::derive` was updated, and returned APPROVE with no
critical or important findings.

**Literature Support.** Godfrey & Zou (2005) and Falleri et al. (2014)
support the general principle that name-based identity is known to be
insufficient and that structural position is a recognized disambiguating
signal in program analysis — the specific engineering choice (add
byte-span to this hash) is this project's own, not a claim drawn from
either paper.

**Remaining Limitations.** `SymbolSnapshotId` still carries no scope/
class qualification (tracked as a possible future `QualifiedName`
enhancement, not implemented here). A symbol's id now changes whenever
its position in the file shifts — intentional, with continuity across
such shifts remaining `SymbolLineageId`'s job, not this id's.

## Discussion: B2-ID-002

**Problem.** Three of four typed-edge resolvers walked every syntactic
occurrence of a call/import/base-mention and pushed one `Edge` per
occurrence, with no notion that repeated occurrences of the same
relation should collapse to one edge.

**Evidence.** After B2-ID-001's fix, the same real-repository run
failed again with `GraphError::DuplicateEdge`; a diagnostic scan found
827 duplicate `(from, to, kind)` triples, overwhelmingly `Calls` edges
involving common names (`forward`, `train`, `test`) called more than
once by the same function.

**Root Cause.** A semantic/syntactic mismatch: the resolvers operate at
the call-SITE level, while `CodeGraph` represents relations at the
call-GRAPH level (at most one edge per pair).

**Alternatives.** Silent deduplication inside `CodeGraph::insert_edge`
was rejected for hiding genuine future bugs behind the same mechanism
meant to catch them. Preserving full occurrence multiplicity via a new
`CallSite`/`EdgeOccurrence` concept was deferred as out of this fix's
scope (recorded as a residual limitation), per the explicit instruction
not to widen `Edge`'s shape beyond what fixing the bug requires.

**Solution.** Per-resolver deduplication, keyed by `(from, to)` (or
`(from, to, kind)` where relevant), with a deterministic evidence
tie-break where more than one `ResolutionMethod` could apply.
`resolve_tests_for` was audited and confirmed structurally immune — left
unchanged.

**Experimental Validation.** New regression tests across all three
affected resolvers (plus direct white-box unit tests on the
`prefer_edge` tie-break after mutation testing's first pass found 11
missed mutants there), full workspace green, clippy/deny clean, and a
real-repository reproduction showing the duplicate-triple count drop
from 827 to 0 on the same commit. An independent review traced the
dedup logic in all three resolvers by hand, confirmed no false-negative
merges of distinct targets, and returned APPROVE with no critical or
important findings.

**Literature Support.** No directly applicable authoritative source
identified for this specific finding; the call-site-vs-call-graph-edge
distinction is standard practice in static call-graph construction, but
no specific citation was confirmed as genuinely on-point during this
finding's literature pass.

**Remaining Limitations.** Call-site multiplicity (how many times, and
where, a relation was observed) is lost by design — only the relation's
existence is now represented. The evidence tie-break rule is narrow and
specific to today's two `ResolutionMethod`s in play, not a general
confidence-ranking framework.

## Discussion: B2-ID-004

**Problem.** `resolve_calls` matched every call purely by the callee's
bare final-segment name, against the full corpus-wide candidate pool,
regardless of whether the call had a resolvable receiver or whether the
caller had any enclosing scope at all.

**Evidence.** B2-EVAL-001's manual Q3/Q6 analysis against
`pytorch/examples` confirmed, by reading real source, two distinct
false-positive patterns from this one root cause: a receiver-qualified
call (`self.src_mask.size(0)`) matched two unrelated C++ `size()`
methods; and asking "who calls this `forward`" returned 19 edges, every
one confirmed unrelated.

**Root Cause.** No receiver awareness (the receiver was extracted by
`callee_name` only to find the final segment, then discarded) and no
scope awareness (an ambiguous candidate set was never narrowed by the
caller's own class/file before being treated as equally plausible).

**Alternatives.** Full type inference/points-to analysis was considered
and deferred (CALL-RES-A5/A6 in the Experimental Policy's backlog) as
categorically larger than this task's evidence-driven scope.
Unconditional ambiguity suppression without scoping was considered as a
simpler alternative, left open as a future ablation comparison rather
than adopted outright. Heuristics keyed on the specific common names
involved (`size`, `forward`) were explicitly rejected as overfitting to
this one corpus.

**Solution.** Receiver-aware filtering for `self`/`this`-qualified calls
(CALL-RES-A3) restricts the candidate pool to the caller's own
enclosing class when the receiver names a real member of it. Scope-
hierarchy narrowing (CALL-RES-A2) for unqualified ambiguous calls tries
same-class, then same-file; "no edge" (CALL-RES-A1) replaces the
previous unconditional "one Hypothesized edge per ambiguous candidate"
once narrowing is exhausted and ambiguity remains.

**Experimental Validation.** 36 new/updated unit tests, full workspace
test suite (318 tests in `codegraph-index`, up from 308) green,
clippy/deny clean. A real-corpus re-run against the exact
B2-EVAL-001 repository/commit confirmed Q3's 2 false positives dropped
to 0 (5 genuine `Field` calls found instead, matching the real
forward-path composition) and Q6's 19 false positives dropped to 0 (one
structural `Contains` edge remained, not a spurious `Calls` edge).
B2-ID-003's own chained-call regression check remained correct across
this re-run.

**Literature Support.** Receiver/scope-aware pruning of call-graph
candidates is standard practice in static analysis (class-hierarchy
analysis and its refinements); this project's own specific narrowing
rule (span-containment-derived class membership, then file identity) is
its own engineering choice, not a direct implementation of a named
published algorithm — recorded per the Evidence-before-claims
Experimental Policy as a project hypothesis informed by general
literature-supported principles.

**Remaining Limitations.** Only resolves the `self`/`this`-qualified
receiver shape — a plain local/parameter/alias receiver is unaffected
(no type inference exists yet). The scope hierarchy stops at "same
file"; import/visibility-aware narrowing (CALL-RES-A4) is a planned
future level. `resolve_tests_for`/`resolve_imports`/
`resolve_extends_implements` were not re-audited for this same class of
defect as part of this finding. (Note: `resolve_imports` named here was
the design later REMOVED by [B2-ID-005](findings/B2-ID-005.md) — this
text is kept as the accurate historical record of what existed when
this finding was written, not edited retroactively.)

## Discussion: B2-ID-005

**Problem.** The original `EdgeKind::Imports`/`resolve_imports`
(Task 7) modeled an import statement as a relationship between two
specific symbols, the same shape `Calls`/`Extends` use.

**Evidence.** B2-EVAL-001 measured 0 `Imports` edges against 589 real
import statements in a real 97-file corpus — not a near-miss, a
complete non-function on real data, despite the resolver's own unit
tests passing (because those tests artificially inlined the import
text into a fake symbol's `signature_text`, a shape real extracted data
essentially never has).

**Root Cause.** A granularity mismatch: an import statement is a FILE/
MODULE-level fact. Task 4's own extraction boundary (a symbol's
`signature_text` starts at that symbol's own construct) never includes
a file's leading import statements, and even if it did, no single
symbol in that file is the "correct" owner of a file-level fact.

**Alternatives.** Emitting the edge from every symbol in the importing
file was rejected for fabricating a false per-symbol dependency.
Picking one arbitrary "representative" symbol was rejected as the same
class of unmotivated, convenience-driven ambiguity resolution
B2-ID-003 already rejected elsewhere. Introducing a synthetic
`Module`/`File` node now was deferred (not rejected) — Priority 2 of
this same task already decided against inventing one prematurely, and
this finding does not reopen that decision.

**Solution.** Removed the old model outright and replaced it with a
separate, file-keyed `ImportFact` (`codegraph_index::imports`), split
into local syntax extraction (`RawImportStatement`, incrementally
reused across untouched files) and global semantic resolution
(`ImportFact`, always recomputed from the full current snapshot pool —
a deliberate correctness-first choice, since an untouched file's import
can still change its correct resolution when some OTHER file's target
changes). Wired into the real `CodeGraphIndexer` pipeline with the same
atomic-publication/race-safety guarantee the graph itself already had.

**Experimental Validation.** 353 tests (up from 325 at Task 9.5's
start) across the new `imports` module (20 tests), the indexer's own
Priority 6 scenarios (10 tests, matching every scenario the task's
authorization explicitly required: initial extraction, untouched-file
reuse, changed-file recomputation, deleted-file removal, new-file
addition, resolution changing when an untouched file's target is added
elsewhere, failed-update rollback, stale-generation rejection,
determinism, and no legacy `Imports` variant), and the remaining
mutation-closing tests added across the whole task. Full workspace
green, clippy/deny clean, mutation testing closed (319 mutants, 275
caught, 41 unviable, 3 missed — all confirmed equivalent and
documented, 0 timeouts), independent review APPROVE. A real-corpus
re-run via the ACTUAL production pipeline (not a manual resolver call,
per explicit instruction) found 691 real import facts (41
internal, 621 external, 29 ambiguous) where the old design found 0, and
confirmed the incremental extraction/always-recompute-resolution split
behaves exactly as designed (687 of 691 raw statements reused on a
1-file-changed incremental update).

**Literature Support.** Separating syntax extraction from semantic
resolution is standard compiler front-end practice; this project's
specific `ImportFact` shape and correctness-first re-resolution policy
are its own engineering choices informed by that general principle, not
a direct implementation of a named published algorithm.

**Remaining Limitations.** Import resolution is never incrementally
reused, even for an unchanged import statement in an untouched file — a
deliberate, measured, documented cost, not an oversight. `ImportFact`
has no `Module`/`File` structural owner yet (queried directly by
`FileIdentity`). A module PATH itself is not resolved to a real file,
only the imported NAME is resolved against the snapshot pool. Not
exposed through any persistence/API/MCP layer.

## Discussion: B2-ID-006

**Problem.** `resolve_calls` treats every given snapshot as an
independent caller candidate and re-parses its full `signature_text`
for call sites with no scope boundary — so a container snapshot
(`Class`, `Interface`/trait, and in principle `Struct`/`Enum`) whose
own text includes its members' bodies rediscovers every call inside
every one of those members, attributing it a second time to the
container itself.

**Evidence.** B2-EVAL-003 (multi-codebase validation, Backend/API
domain, `nsidnev/fastapi-realworld-example-app`) found a real call
(`Comment(...)` inside `CommentsRepository._get_comment_from_db_record`)
attributed to both the method (correct) and the class (spurious). A
dedicated synthetic audit then confirmed the SAME duplication occurs
for a plain nested Python function (outer/inner) and a Rust trait
default method — proving the root cause is span-nesting generally, not
Class or Python specifically. A full-corpus count found 25/326 (7.7%)
container-originated `Calls` edges in the B2-EVAL-003 corpus and
256/878 (29.2%) in `pytorch/examples` at the exact commit
B2-EVAL-001/002 used — meaning a significant fraction of those two
already-published evaluations' own aggregate `Calls`/`StaticAnalysis`
counts are now known to include this contamination (their specific
published Q&A answers were insulated by unrelated scope-aware lookup
logic and are not themselves falsified; the raw counts are affected).

**Root Cause.** Every extractor (`extract/python.rs`, `cpp.rs`,
`csharp.rs`, `javascript.rs`, `rust.rs`) sets `signature_text` to the
full node-text of a candidate's span, with no distinction between "this
span IS the executable body" and "this span CONTAINS other symbols'
own executable bodies." `resolve_calls` has no "nearest enclosing
executable scope" concept to consult instead.

**Alternatives.** An ad hoc `if kind == Class { skip }` special case
was explicitly rejected — the synthetic audit proves it would leave the
nested-function and Rust-trait cases completely unfixed. Changing what
extraction captures for a container's `signature_text` globally was
considered and left open as a real but larger-blast-radius alternative
to fixing attribution specifically in `resolve_calls`. Leaving it
undocumented as an inherent limitation was rejected given the measured
magnitude (up to 29.2% of all `Calls` edges in an already-published
corpus).

**Solution.** Implemented and approved: a `SymbolKind::can_originate_calls`
caller-eligibility predicate (`Function`/`Method` only; `Field`
confirmed `false` by a concrete nested-scope reproduction; `Const`/
`Macro` stay `false` by conservative default since neither is produced
by any current extractor) combined with a nested-scope span boundary
in the call-site walk, reusing the same span-nesting fact
`resolve_contains` already computes for Class→Method, generalized to
any nested snapshot-to-snapshot relationship rather than invented
twice.

**Experimental Validation.** Root cause confirmed via a minimal,
placeholder-free synthetic reproduction (4 cases: Class/Method,
nested Function, Rust trait default method, sibling methods sharing a
target) plus a full-corpus contamination count on two independently
re-cloned real repositories. The fix itself: 13 regression tests
(Python/Rust/C#/C++/JS, all real-extraction-based, all passing), 366
`codegraph-index` tests green, 0 regressions. Mutation testing CLOSED:
107/114 caught, 6 unviable, 1 pre-existing equivalent mutant
(`narrow_by_scope`, unrelated to this fix), 0 genuine gaps remaining.
Contamination repair CLOSED via full before/after edge-level diffs
(not samples) on both corpora: `pytorch/examples` 878→535 `Calls`
edges, B2-EVAL-003's backend corpus 326→293 — every removed edge
confirmed spurious, zero new edges introduced, Q3/Q4/Q6 answers
unchanged. Raw output for every step preserved under
`docs/research/evidence/b2-id-006-*`.

**Literature Support.** No directly applicable authoritative source
identified for this specific implementation defect. The violated
principle — a call site's caller is determined by lexical scope (the
nearest enclosing executable unit), not by arbitrary AST ancestor
containment — is foundational static-analysis/compiler-construction
material, not tied to one specific citation.

**Remaining Limitations.** Measured directly on Python (real +
synthetic) and Rust (synthetic only); C++/C#/JavaScript/TypeScript are
Inferred-affected by the identical extraction pattern (confirmed by
source reading) but not yet empirically reproduced per-language.
**Update:** the fix design was approved and implemented (11 new
regression tests across Python/Rust/C#/C++/JS, all passing; mutation
testing 106/114 caught). `Field` caller-eligibility is now resolved
(`false`, confirmed by a concrete nested-scope reproduction);
`Const`/`Macro` remain `false` by conservative default since neither
is produced by any current extractor. **This finding is NOT closed**:
while verifying the fix, the same structural pattern was found in two
more resolvers — see [B2-ID-007](findings/B2-ID-007.md) — and final
closure (one missed-mutant boundary test, both corpora's contamination
repair, the B2-EVAL-001/002 erratum) is deferred until B2-ID-007
resolves. B2-EVAL-003 itself remains paused.

## Discussion: B2-ID-007

**Problem.** While verifying B2-ID-006's own fix, the identical
structural pattern (re-parsing a snapshot's full `signature_text`,
recursing unconditionally into every child node with no boundary at a
nested symbol's own declaration) was found in `resolve_extends_implements`
and `resolve_tests_for` too — prompting a systematic audit of every
resolver that re-parses a snapshot's own source text, rather than
assuming B2-ID-006's exact fix generalizes everywhere.

**Evidence.** A real-parse reproduction confirmed a nested Python class's
own base clause (`class Outer: class Inner(Base): ...`) produces BOTH
`Inner -> Base` (correct) and `Outer -> Base` (spurious) in
`resolve_extends_implements`; the same shape was independently
reproduced in C#. `resolve_tests_for` was confirmed affected too, but
with a narrower real-world trigger (an outer container must itself
match the test-naming convention AND something nested inside it must
call the exact implied target name). Two other resolvers,
`resolve_instantiates` and `resolve_contains`, were independently
confirmed NOT affected by direct reproduction, not assumed safe by
analogy — `resolve_instantiates`'s own `SymbolKind::Field`-only filter
structurally excludes any container from ever being a candidate "from"
side; `resolve_contains` already selects the innermost (smallest-span)
candidate by explicit design.

**Root Cause.** A shared STRUCTURAL pattern producing a shared FAILURE
MODE, but each affected relation needs a DIFFERENT ownership rule:
`Calls` is owned by the nearest executable enclosing scope (B2-ID-006's
own rule); `Extends`/`Implements` is owned by the specific type
declaration whose own header syntax names the base (inheritance has no
"executable scope" concept at all, so B2-ID-006's own mechanism cannot
simply be reused verbatim); `TestsFor` is owned by the specific test
function/method whose own body contains the real call, gated by this
resolver's own separate naming-convention check.

**Alternatives.** Assuming B2-ID-006's exact fix mechanism generalizes
to every resolver was explicitly rejected — this audit's own findings
disprove it directly (two resolvers need no fix at all; two more need
a DIFFERENT ownership rule, not the same span-boundary check under a
different name). Folding this into B2-ID-006 itself was rejected per
explicit instruction, since the fix is not yet demonstrated identical.
Fixing every affected resolver immediately, in the same pass as the
audit, was rejected since the ownership rule for `Extends`/`Implements`
and `TestsFor` has not yet been through its own design/approval step.

**Solution.** **Not implemented — audit only, per explicit instruction.**
Recommends separate, relation-specific fixes: a parse-tree-local,
node-kind-based recursion boundary for `resolve_extends_implements`
(stop recursing into a child that is itself a new type-declaration
boundary); a cross-snapshot span check closer to B2-ID-006's own
mechanism for `resolve_tests_for`. No fix chosen or implemented for
either without further approval.

**Experimental Validation.** Every classification in the resolver-
ownership matrix (6 resolvers plus lineage) is backed by a real,
reproducible probe — not reasoned by analogy — with raw output
preserved at `docs/research/evidence/b2-id-007-resolver-ownership-audit.txt`.
The NOT-affected classifications were independently, empirically
checked, specifically because they were the ones most at risk of being
wrongly assumed affected (or safe) by analogy alone.

**Literature Support.** Néron, Tolmach, Visser & Wachsmuth (2015), the
same scope-graphs citation as B2-ID-006, supports the general
principle that ownership must be determined by an explicit lexical/
semantic relation, not raw span containment. Sun et al. (2023), *ACER:
An AST-based Call Graph Generator Framework* (arXiv:2308.15669), is
cited as context confirming AST-based relationship extraction is an
established, legitimate approach in the literature generally — relevant
background for why these resolvers' own AST-re-parse design is
reasonable even though it needs this family of ownership-boundary
fixes. Neither source prescribes this project's own specific per-
relation fix mechanisms.

**Remaining Limitations.** `resolve_extends_implements`'s Rust/C++/JS/TS
collectors are Inferred-affected (identical code pattern) but not yet
empirically reproduced per-language — only Python and C# were actually
run against real nested-declaration source. `resolve_tests_for`'s
affected shape has not yet been assessed for real-world frequency. No
fix is implemented for either affected resolver; B2-ID-006 itself also
remains open pending this audit's resolution.

## Discussion: B2-ID-008

**Problem.** [B2-EVAL-005](evaluations/B2-EVAL-005.md) (Rust/systems
domain, `sharkdp/fd`, MIT/Apache-2.0) measured `extends_implements_edges:
0` against a real corpus containing 2 real traits (`Matcher`, 6
implementors; `SanitizeErr`, 3 implementors) and 41 real `impl` blocks,
all individually confirmed by direct source reading. This is the first
B2 evaluation to exercise `resolve_extends_implements`'s own
Rust-specific collector against genuine Rust `impl` syntax at all.

**Root Cause.** `extract/rust.rs`'s symbol extractor never produces a
`RawSymbolCandidate` for an `impl_item` node — only `struct_item`,
`enum_item`, `trait_item`, `function_item`, `function_signature_item`
are recognized. Since `resolve_extends_implements` only ever re-parses
EXISTING snapshots' own signature text looking for an `impl_item`
inside it, and Rust's grammar never nests an `impl` block inside a
struct/enum/trait/function's own span, the resolver's own Rust
collector (`collect_rust_impl_trait`) is structurally unreachable on
any real Rust source — confirmed genuinely dead code on the real-world
path, not merely thin test coverage. A direct tree-sitter probe
further confirmed a SECOND, currently-latent defect inside that same
dead code: it searches for the trait name via a positional
`type_identifier` child instead of `impl_item`'s own named `trait`
field, which would also silently fail to extract the implementing
TYPE's name correctly for any generic impl (`impl<T> Trait for
Generic<T>`, confirmed by probe to have `type` field kind
`generic_type`, never matched by the existing positional search).

**Impact.** Important — a complete (100%), silent, corpus-independent
absence of the entire trait/impl relationship on Rust; this
evaluation's own explicit priority questions ("which traits does this
type implement," "which types implement this trait") are Not
answerable despite both endpoints existing as real, correctly-extracted
graph symbols. No prior B2 evaluation reported a Rust-specific
`Extends`/`Implements` figure, so no erratum is owed to any prior
publication — this is a newly surfaced gap. Not Critical: no crash, no
corrupted/spurious edge (the resolver fails closed by accident of
unreachability).

**Solution.** **Not implemented — design decision pending, per explicit
instruction.** Four alternatives presented with corpus-grounded
pros/cons: (A) `impl` as its own first-class `SymbolKind` node
(`ImplBlock`, with `Contains`/`Implements` edges to its methods and
trait), aligned with rustc HIR's own `Impl` node model; (B) `impl` as a
non-symbol `ImplFact`, reusing this project's own precedented
`ImportFact` pattern from B2-ID-005; (C) a dedicated file/module-level
resolver pass with no new intermediate symbol or fact type; (D) an
unspecified evidence-driven hybrid, not ruled out. A corpus-based
classification (18 inherent impls, 23 trait impls, 4 generic impls, 2
with lifetime parameters, confirmed multiple impl blocks for the same
type — `DirEntry`'s 6 separate blocks) and a direct grammar probe
(`impl_item`'s own named `trait`/`type` fields) are recorded to ground
whichever alternative is eventually chosen.

**Experimental Validation.** Every classification (impl counts, trait-
impl vs. inherent split, generics, lifetimes, multi-impl-per-type) is
Measured directly against the real `sharkdp/fd` corpus at the exact
commit this finding cites, not estimated. The grammar-field claim
(`impl_item.trait`/`impl_item.type`) was confirmed via a direct,
throwaway tree-sitter probe against this workspace's own
`tree_sitter_rust` version, not assumed from documentation.

**Literature Support.** rustc's own HIR (`rustc_hir::hir::Impl`,
official compiler-internals documentation) models an impl block, its
optional trait, and its implementing type as three distinct
identities — supporting Alternative A as the model closest to the
language's own semantic ground truth, without by itself mandating a
full mirror of rustc's own field set. Two real, independently-published
open-source Rust code-graph tools (`ferrograph`, a two-phase
placeholder-edge approach closer to Alternative B; `tldr-core`, direct
field-based extraction via the same `trait`/`type` fields this
finding's own probe confirmed) show both design directions are used in
practice elsewhere, cited as precedent evidence, not as an
architecture to copy. No peer-reviewed paper specific to this exact
design choice was found; rust-analyzer's own internal semantic model
was sought but not found via search and is flagged honestly as an
unestablished reference, not guessed at.

**Remaining Limitations.** No fix is implemented or decided.
[B2-EVAL-005](evaluations/B2-EVAL-005.md) remains PAUSED, with its
already-collected pre-fix/partial metrics preserved and explicitly
marked, pending a design decision on this finding.
