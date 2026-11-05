# B2 Real-Repository Smoke Test — Python / Deep Learning

Full structured data backing this report: [`docs/research/evaluations/B2-EVAL-001.md`](../research/evaluations/B2-EVAL-001.md).
Findings this evaluation discovered and their fixes:
[B2-ID-001](../research/findings/B2-ID-001.md),
[B2-ID-002](../research/findings/B2-ID-002.md).

## A. Repository / commit

- **Repository:** [`pytorch/examples`](https://github.com/pytorch/examples)
- **Commit A (before):** `20955149ad6ea97a4f403e5a56c95a39e568ccbe`
- **Commit B (after):** `001d493285083b54cacbc1334cb26338bdff2455`
- **License:** BSD 3-Clause (`LICENSE` at repo root)
- **Why this repository:** small enough to index completely in one
  sitting (214 files, 97 parseable), real PyTorch `nn.Module`
  subclasses with `__init__`/`forward`/submodule assignments, real
  training/inference code, multiple files with real inheritance/
  import/call relationships, deep real commit history (made finding a
  natural, non-cherry-picked A→B pair with a substantive model-code
  change straightforward).

## B. Pipeline results

214 files total, 97 supported/parsed (0 parse failures, 0 files with
tree-sitter error nodes), 567 raw candidates → 567 promoted snapshots
(0 promotion failures; 11 extraction rejections for oversized name/
signature, correctly dropped before promotion). Full index: ~1.03s.

## C. Symbol coverage

98 `Class`, 462 `Function`, 7 `Struct`. No `Method`/other kinds
observed in this corpus (Python's `def` is mapped to `Function`
regardless of whether it is a free function or a class method, in the
current `kind_mapping`).

## D. Edge quality

11,934 `Calls`, 0 `Imports`, 4 `Extends`/`Implements`, 0 `TestsFor`.
286 `StaticAnalysis`, 11,652 `NameHeuristic`. A 20-edge manually
validated sample found **18/20 correctly resolved, 2/20 (8 individual
edges) correctly heuristic, 0 false positives, 0 false negatives** —
full detail and the one noteworthy interaction with the bounded-
signature rejection policy (an oversized `Agent` class silently
dropped, incidentally making an otherwise-ambiguous call look
unambiguous) is in the full evaluation record, section D.

## E. Deep-learning usefulness

Target: `word_language_model/model.py`'s `TransformerModel`. Of the 6
realistic questions asked:

- **Fully/mostly answerable:** Q5 ("which tests are linked") — correctly
  reports none exist, because none do.
- **Partially answerable:** Q1 ("main model class") — found, but
  distinguishing the current vs. a historical snapshot of the same
  name requires non-obvious graph navigation; Q4 ("who calls the
  model") — `RNNModel`'s real caller was found correctly, but
  `TransformerModel`'s was missed.
- **Not answerable:** Q2 ("what does it extend") — the real base class
  is `nn.Transformer`, a PyTorch stdlib class with no snapshot in this
  repository, so no edge can exist for it.
- **Incorrect:** Q3 ("what composes the forward path") and Q6 ("what's
  impacted if I modify forward") — both polluted by a confirmed false
  positive (a tensor `.size()` call heuristically matched against two
  unrelated C++ `size()` methods) and Q3 also misses a confirmed real
  false negative (a chained same-class method call,
  `self._generate_square_subsequent_mask(...).to(device)`, never
  resolved). **Q6 in particular returned 19 "callers," every single one
  a false positive** — asking "who calls this commonly-named method"
  is, today, dominated by noise on a real multi-file corpus.

## F. Incremental A→B

1 file changed (`word_language_model/model.py`) out of 97 parseable
files; exactly 1 file reparsed (~1% of the corpus); 13 snapshots
removed, 13 added; 11,951 edges rebuilt (full-pool recomputation, by
design — see B2-ID-002); 0 stale snapshots or dangling edges across 3
repeated runs. Incremental update took ~0.75s vs. ~1.05s for a full
reindex — not yet proportionally faster than the 1% reparse ratio
would suggest, because edge resolution still runs over the full
current snapshot pool on every update (a correctness-first design
choice from B2-ID-002, not yet performance-tuned).

## G. Performance observations

Full index ~1.03s, incremental update ~0.75s (both MEASURED,
wall-clock, 3 repeated runs). Memory: ESTIMATED only (~14 MB, coarse
OS tool) — not a real profiler reading, not a benchmark.

## H. Missing capabilities

- **`Field`/`Attribute`:** 109 real submodule assignments
  (`self.X = nn.*` or equivalent, independently re-measured — see the
  correction in the full evaluation record) found repo-wide; 9 in the
  one file analyzed in depth across its two model classes. Zero representable
  today.
- **`Imports`:** 589 real import statements counted independently; 0
  `Imports` edges produced. **0% capture confirmed on real data** —
  this is not a suspected gap anymore, it is a measured one.
- **Scope-aware/qualified names:** directly implicated in both
  B2-ID-001 (now fixed at the identity level) and the Q3/Q6 false-
  positive pattern (not fixed — same-named methods/classes across
  unrelated files remain ambiguous or wrongly merged by name alone).
- **`Contains`/`Instantiates`:** every DL-relevant resolved `Calls`
  edge in the manual sample is semantically a constructor
  instantiation, currently represented only as an undistinguished
  `Calls` edge.

## I. Recommended coverage expansion

Ranked by how directly this evaluation's own evidence showed each gap
blocking real understanding: **(1) `Field`/`Attribute` — high
priority**, **(2) `Imports` redesign — high priority, 0% capture now
confirmed**, **(3) scope-aware/qualified names — moderate**,
**(4) `Instantiates`/`Contains` — moderate**, **(5) method-name-
collision-aware call resolution — lower priority, same root cause as
(3)**, **(6) a newly-observed chained-call extraction gap
(`self.foo(...).bar(...)`) — flagged for a dedicated root-cause
investigation, not yet diagnosed to a specific line of code.** None of
these were implemented during this evaluation, per its own scope
boundary.

## J. GO/NO-GO to continue B2

**GO.** Both blocking findings (B2-ID-001, B2-ID-002) are fixed,
reproduced-fixed against this exact repository and commit pair, and
independently reviewed with an APPROVE verdict. The pipeline completes
real indexing, real incremental updates, and real multi-file edge
resolution with no crashes across repeated runs. Six concrete,
evidence-based coverage-expansion candidates are now on record for a
future task's consideration. Task 10 remains **NOT authorized** by this
evaluation — per the project's standing protocol, that decision is the
user's. A multi-domain validation suite (backend/API, frontend/UI,
Rust/systems, C++/C#, full-stack, CLI/tooling) remains required before
B2's closure, per the Evaluation & Paper Evidence Policy's multi-
codebase requirement — this evaluation covers ML/Python (plus some C++)
only.

**Independent review of this evaluation's own conclusions** (a second,
separate review from the code-fix review above — this one re-cloned
the repository fresh and checked the manual claims against real
source, specifically to avoid the pipeline grading its own output):
trustworthy as written, with one number corrected (the repo-wide
submodule-assignment count was revised from an unreproduced 62 to an
independently-confirmed 109 — see the full record for detail). Every
other specific, falsifiable claim checked — the two `Agent`/
`CustomDataset` class collisions, the false positive/negative pair, the
external base-class case, the import-capture architecture, and the
file-level submodule count — reproduced exactly as reported.
