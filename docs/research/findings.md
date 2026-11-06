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
