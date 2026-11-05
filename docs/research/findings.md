# B2 Project Findings Index

See [`POLICY.md`](POLICY.md) for the policy this index implements, and
[`EVALUATION_POLICY.md`](EVALUATION_POLICY.md) for how evaluations
(like [B2-EVAL-001](evaluations/B2-EVAL-001.md), which discovered both
findings below) are recorded. Each row links to the full finding file
under `findings/`.

| ID | Problem | Root cause | Impact | Solution | Validation | References |
|---|---|---|---|---|---|---|
| [B2-ID-001](findings/B2-ID-001.md) | `SymbolSnapshotId` collides for two distinct symbols with identical name/body in one file (`init_weights` in two different classes, `pytorch/examples`) | Identity tuple had no notion of a symbol's position/scope in its file | Critical — crashed the pipeline on the first real repository tried | Added the symbol's own `ByteSpan` to the identity hash, bumped `superbrain-symbol-snapshot-v1` → `v2` | 6 new regression tests + golden vector (0 missed, 6 mutants), full workspace green, clippy/deny clean, real-repo reproduction confirmed fixed, independent review APPROVE | Godfrey & Zou (2005), Falleri et al. (2014, GumTree) |
| [B2-ID-002](findings/B2-ID-002.md) | `resolve_calls`/`resolve_imports`/`resolve_extends_implements` emit one literal `Edge` per syntactic occurrence, so a repeated call/import/base-mention produces duplicate `(from,to,kind)` triples | Syntactic-occurrence loop never collapsed into the semantic one-edge-per-relation model `CodeGraph` enforces | Critical — crashed `observe_commit`/`build_graph` on realistic, common input (same name called more than once) | Per-resolver deduplication by `(from,to[,kind])`, deterministic evidence tie-break in `resolve_calls`; `resolve_tests_for` audited and found not vulnerable | 15 new tests across 3 files (0 missed, 96 mutants after closing 11 initial gaps), full workspace green, clippy/deny clean, real-repo reproduction confirmed fixed (827 duplicate triples → 0), independent review APPROVE | No directly applicable authoritative source identified (standard call-graph-construction practice, no specific citation confirmed) |

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
