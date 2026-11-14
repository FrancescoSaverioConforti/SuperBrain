# Evaluation & Paper Evidence Policy

Effective 2026-11-05, for the SuperBrain project generally and B2 (Code
Graph) specifically, and binding for every future session — Claude or
Codex — working on this project. Companion to
[`POLICY.md`](POLICY.md) (the Research-grade Findings Policy); this
document governs how EVALUATIONS (not just findings) are recorded.

## Why this exists

Reporting "tests passed" after a real-repository smoke test discards
exactly the evidence the project's eventual closing report/paper will
need: raw counts, denominators, methodology, and the link between what
was measured and which findings it surfaced. From 2026-11-05 onward,
every significant evaluation is recorded as a reusable artifact, not
just narrated in a chat response.

## What counts as a significant evaluation

Any of: a real-repository validation run, a benchmark, a mutation
testing campaign, a security test, a performance test, a regression
test, a comparative evaluation, or a smoke test.

## Required fields per evaluation

Each evaluation gets an ID (`B2-EVAL-xxx`, zero-padded, sequential,
never reused) and a file at `docs/research/evaluations/<ID>.md`. Record
at minimum:

- evaluation ID, date, SuperBrain commit;
- repository/dataset tested, its exact external commit, and its
  license/SPDX identifier when applicable;
- configuration and methodology;
- raw counts (not just derived percentages);
- derived metrics;
- pass/fail;
- interpretation;
- limitations;
- related Project Findings (see Linking, below);
- relevant scientific/technical references, when genuinely available.

## Measured vs. Manually validated vs. Inferred vs. Estimated

Always tag a claim with exactly one of these four words, and never
blur them:

- **Measured** — a tool/script produced the number directly and
  reproducibly (a test count, a byte count, an elapsed-time reading).
- **Manually validated** — a human (or an agent acting as one, reading
  real source) checked a sample by hand and judged it correct/incorrect.
- **Inferred** — a conclusion drawn from Measured/Manually validated
  data, not itself directly observed (e.g. "this gap generalizes beyond
  this one repository").
- **Estimated** — a rough approximation where no exact measurement was
  taken or is practical (e.g. approximate memory via a coarse OS tool).

Never present an Estimated or Inferred value as if it were Measured.

## Metrics must carry their denominator

Never write `coverage = 80%` alone. Write `80% (80/100 relevant
constructs manually validated)`, and state how the denominator (100
here) was defined and who/what defined it.

When a sufficiently reliable ground truth exists, collect precision,
recall, F1, false positives, false negatives, ambiguous count, and
correctly-unresolved count. Do not compute these formally when no real
ground truth supports them — report raw counts and a qualitative
judgment instead.

## Multi-codebase validation requirement

B2 is not considered validated by one codebase alone. Before B2's
closure, a multi-domain validation suite must run across public,
explicitly and compatibly licensed repositories covering at least: ML/
deep learning, data analysis/scientific, backend/API, frontend/UI,
desktop/UI, Rust/systems, C++/C#, full-stack, and CLI/tooling. One
repository may cover more than one category, but the corpus as a whole
must represent these different architectural styles for real.

## Per-codebase data to collect

Repository, exact commit, license, languages, domain, LOC (if easily
measurable), total files, supported files, parsed files, parse
failures, tree-sitter error rate, raw symbols, promoted snapshots,
promotion failures, graph nodes, graph edges per `EdgeKind`, lineage
edges, unresolved/ambiguous relations, initial indexing time,
incremental update time, files reparsed, snapshots reused, and memory
usage if available.

## Cumulative semantic coverage matrix

Maintained across evaluations as `domain × language × semantic
construct × support × measured coverage × priority`, covering at
least: classes, methods, functions, fields/attributes, properties,
inheritance, interface implementation, calls, imports, instantiations,
containment, type aliases, namespaces/modules, tests, macros,
decorators/annotations. Future model expansions (new `SymbolKind`/
`EdgeKind` variants) must be evidence-driven from this matrix, never
justified only by "the grammar happens to expose this."

## Call-graph / relationship quality

On manually validated samples, record correctly resolved, correctly
heuristic, correctly unresolved, false positives, false negatives, and
ambiguous counts — evaluated separately per `EdgeKind` where
applicable (`Calls`, `Extends`, `Implements`, `Imports`, `TestsFor`, and
future `Contains`/`Instantiates`/`References`).

## Lineage evaluation

On real commit pairs with a reasonable ground truth, record correct
lineage, missed lineage, incorrect lineage, and separately judge
`Confirmed` correctness vs. `Hypothesized` correctness. Compute
precision/recall only when methodologically justified by the available
ground truth.

## Incremental indexing

Record changed files, files reparsed, total files, snapshots added/
removed/reused, edges rebuilt, full-index time, incremental-index time,
and derive reuse ratios such as `files_reparsed / total_files` when
useful.

## Coding-agent usefulness

For each codebase, prepare realistic questions (e.g. "where is this
class used," "who calls this method," "what does this class depend
on," "which methods form this execution path," "if I modify this
symbol what may be impacted," "which tests cover this component,"
"explain the flow from API/UI entry point to core logic," "explain the
model forward/training path"). Classify each answer as Fully
answerable / Partially answerable / Not answerable / Incorrect, and
keep significant examples verbatim in the evaluation file.

## Linking findings and evaluations

It must always be reconstructable: `Evaluation → Finding → Root Cause
→ Fix → Regression Test → Re-evaluation`. Every finding opened under
the companion Findings Policy must name the evaluation ID that
discovered it; every evaluation that discovers a finding must name
that finding's ID in return.

## Research integrity

Record failure cases, false positives, false negatives, unsupported
constructs, negative results, and rejected approaches alongside
favorable results. Never select only favorable cases for the record.

## Storage layout

```
docs/research/evaluations/   — one file per evaluation (Markdown; JSON/CSV alongside for aggregable metrics)
docs/research/findings/      — one file per finding (companion policy)
docs/research/references.bib — shared BibTeX, when useful
```

The end goal is to be able to produce, from these records alone: per-
codebase tables, coverage-by-domain charts, precision/recall by edge
kind, lineage quality, incremental-indexing efficiency, a mutation-
testing summary, a finding-distribution summary, performance
comparisons, and coding-agent-usefulness results — all without
re-deriving anything from scratch.

## Persistence and discoverability

Recorded in Continuity so a future session (Claude or Codex) discovers
this policy even before encountering the first evaluation file, exactly
like the companion Findings Policy.
