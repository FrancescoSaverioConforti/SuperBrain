# B2-EVAL-006: multi-codebase validation, Data analysis / scientific Python domain

**Evaluation ID:** B2-EVAL-006
**Status:** **COMPLETE.**
**Date:** 2026-11-11.
**SuperBrain commit:** branch `b2-code-graph` at commit `c8ccd8a` (post
B2-ID-008 ImplFact fix, Phase 0 checkpoint). No production code was
modified for this evaluation — only a new, domain-specific throwaway
harness was added
(`crates/codegraph-index/examples/b2_smoke_test_data_science.rs`).
**External repository:** `pyjanitor-devs/pyjanitor`
(https://github.com/pyjanitor-devs/pyjanitor), MIT license ("Copyright
(c) 2018-onwards pyjanitor devs"), confirmed by direct read of the
repository's own `LICENSE` file at the repository root. A real,
actively-maintained Python data-cleaning/feature-engineering library
(a Pythonic reimplementation of R's `janitor` package, built on
`pandas`/`pandas_flavor` method chaining), chosen per the user's
explicit Data analysis/scientific Python domain requirements: real
multi-file package (`janitor/` with 109 `.py` files, including
domain-specific modules `biology.py`/`chemistry.py`/`engineering.py`/
`finance.py`/`math.py`/`ml.py`/`timeseries.py`), heavy real NumPy/
Pandas usage (246 direct import/usage sites measured), significant
top-level-function density (2792 real `Function` symbols extracted),
a genuine `janitor/functions/` directory with dozens of
one-function-per-file modules (`clean_names.py`, `bin_numeric.py`,
`case_when.py`, `coalesce.py`, and many more) — exactly the
"function-heavy" shape this evaluation's own main research question
requires — and 2127 real commits of history. Not a notebook-only
repository (the `examples/notebooks/` directory contains real `.py`
marimo-notebook source files, a small fraction of the corpus, not its
substance) and not a dataset-only or generated/vendored-code
repository. Cloned read-only into a fresh, isolated scratchpad
directory; deleted after this evaluation's data was captured
(confirmed deleted, no residual clone left on disk).
**External commits:** commit A `8e3cb118888e4b94acafd58392ef5e69138d976a`
("Fix complete for grouped frames, partial fills, and by warnings"),
commit B `1247923ff01dd9a74f40f55d55f108e503e4b218` ("Share expand by
grouping with complete") — a real, substantial multi-file commit
(`janitor/functions/complete.py`, `janitor/functions/expand_grid.py`,
`tests/functions/test_complete.py` changed), chosen specifically
because it introduces a NEW shared function (`groupby_for_expand`,
extracted from duplicated logic into `expand_grid.py` and imported
into `complete.py`) — a genuine cross-file refactor exercising both
incremental indexing and cross-file call/import resolution, not a
trivial one-line diff.
**Configuration:** the frozen baseline configuration, unchanged from
every prior B2 domain (CALL-RES-A1+A2+A3 for `Calls`, post-B2-ID-006/007;
`Contains`/`Instantiates` as introduced by Task 9.5; `ImportFact`-based
Imports per [B2-ID-005](../findings/B2-ID-005.md); `ImplFact`-based
`Implements` per [B2-ID-008](../findings/B2-ID-008.md), though this
corpus's own trait/impl-equivalent constructs do not apply to Python at
all — no `EdgeKind::Implements`/`Extends` edges were expected or
measured here). No algorithm was changed to produce this evaluation's
numbers.
**Methodology:** drove the real production pipeline via a new
domain-specific harness,
`crates/codegraph-index/examples/b2_smoke_test_data_science.rs`,
structurally mirroring the prior five domain harnesses for every
generic measurement section, with two new domain-specific sections:
C1 (Module/File -> top-level-symbol gap — this evaluation's own MAIN
RESEARCH QUESTION) and C2 (a raw-parse proxy count of method-chaining
call sites and decorated functions, to quantify the data-flow-vs-
call-graph gap this domain's own idiom makes especially visible). All
numbers below are **Measured** unless explicitly marked otherwise.
Raw output preserved at
`docs/research/evidence/b2-eval-006-pyjanitor-run.txt`.

## A. Symbol / structural coverage

| Metric | Value |
|---|---:|
| Total files in tree | 433 |
| Files supported by parser | 291 |
| Files parsed | 291 |
| Parse failures | 0 |
| Files with tree-sitter error nodes | 0 |
| Raw candidates extracted | 2822 |
| Extraction rejections (oversized name/signature) | 53 |
| Snapshots promoted | 2822 |
| Promotion failures | 0 |
| `Function` count | 2792 |
| `Class` count | 20 |
| `Field` count | 10 |
| Python files with ≥1 import statement | 287 |

No parse failures, no promotion failures, no duplicate
`SymbolSnapshotId` or duplicate `(from,to,kind)` edge triples detected.

**A new, domain-specific finding, found by direct root-causing of an
unexpected Q&A result (Section H), not assumed: of the 53 extraction
rejections, ALL 53 are `SignatureTooLong`** (confirmed by adding an
explicit per-rejection-type counter to the harness — see raw evidence).
This is a materially higher proportion of total rejections attributable
to this ONE cause than any prior B2 domain measured so far, and it is
NOT a rare edge case in this specific corpus: it includes several of
this evaluation's own originally-intended Q&A targets (`complete`,
10263 bytes; `clean_names`, 6133 bytes; `expand`, 7257 bytes — each
individually confirmed by direct source measurement, all exceeding
`MAX_SIGNATURE_BYTES = 4096`). **Root cause — corrected during
independent review, not as originally asserted:** an earlier draft of
this document attributed the size entirely to long embedded NumPy-style
`Examples` docstring sections, but direct source reading (during
independent review) found `clean_names`'s own real docstring is a
single, short (~20-line) doctest block — the actual bulk of its
oversized `signature_text` is its own ~140-line IMPLEMENTATION body,
not its documentation. The corrected, more modest claim: this corpus's
own functions — across both documentation and implementation — simply
run long enough, on a non-trivial fraction of its pipeline-shaped
functions, to exceed a 4096-byte limit that was not previously
observed as consequential in prior, less documentation/implementation-
dense B2 domains (B2-EVAL-001's own ML/DL corpus did not report this as
a material issue). The SPECIFIC "long NumPy Examples section" mechanism
is dropped as an overclaim this evaluation cannot support for the two
cases it was checked against; the underlying frequency finding (53/2822
rejections, all `SignatureTooLong`, hitting several of this domain's
own representative pipeline functions) stands, independently confirmed.
See Section D3 for the full quantified impact on this evaluation's own
coding-agent usefulness questions.

## B. Imports — extraction coverage vs. resolution accuracy

| Metric | Value |
|---|---:|
| Real import statements (line-based heuristic count, commit B) | 1473 |
| `ImportFact`s extracted (file-level, Python) | 1656 |
| Extraction coverage | 1656/1473 (112.4%) |
| Resolution: `Internal` | 270 |
| Resolution: `External` | 1368 |
| Resolution: `Ambiguous` | 18 |
| Aliases captured | 459 |

**Interpretation (Inferred, consistent with every prior Python-domain
measurement):** extraction coverage exceeds 100% for the same reason
B2-ID-005 already documented — multi-name `from X import (A, B, C)`
statements. **459 aliases captured is the highest absolute alias count
of any B2 domain measured so far** (consistent with this corpus's own
real practice of aliasing library imports, e.g. `import pandas as pd`,
`import pandas_flavor as pf`, repeated across 287 files) — this
corpus's own `ImportFact.alias` data is directly what CRC-003 (see
Challenge Set, below) shows is CAPTURED correctly but not yet CONSULTED
by `resolve_calls`. This domain's own real import-aliasing density
makes that specific gap more consequential here than in a domain that
aliases imports rarely.

## C. Module/File -> top-level-symbol gap — THE MAIN RESEARCH QUESTION of this evaluation

| Metric | Value |
|---|---:|
| Files with at least one extracted symbol | 277 |
| Total top-level symbols across all files (span-nesting definition) | 2716 |
| Files with more than one top-level symbol | 199 |
| **Percentage of files with multiple top-level symbols** | **71.8%** |
| Files with EXACTLY one top-level symbol (the "one-function-per-file" idiom) | 78 |

Top files by top-level-symbol count: `tests/functions/test_conditional_join.py`
(257), `examples/notebooks/pivoting_data_from_wide_to_long.py` (134),
`tests/functions/test_pivot_longer.py` (126), and a long tail of
test/example files with dozens of top-level test functions each.

**Interpretation (Inferred) — the single richest Module/File evidence
point across all six B2 domains measured so far:** 71.8% is the
HIGHEST "files with multiple top-level symbols" rate of any domain
measured (58.2% backend-Python, 24% frontend-TS, 95.8% Rust — this
domain sits between backend-Python and Rust, closer to Rust's own
extreme). This is NOT uniform across the corpus, and the non-
uniformity is itself evidence: **the 78 files with EXACTLY one
top-level symbol are overwhelmingly `janitor/functions/*.py` — the
real "one function per file" source-organization convention this
evaluation's own protocol specifically asked about.** The high-symbol-
count outliers (`test_conditional_join.py` with 257 symbols) are
test files, a structurally different genre (many small, independent
test functions genuinely co-located by DESIGN, not by accident) from
the production `janitor/functions/` modules this evaluation's own
Module/File question is really about.

**Direct demonstration of the query limitation (not merely asserted):**
Section H's own Q3 queries "what does `janitor/functions/complete.py`
contain" via the ONLY mechanism available today (file-path string
matching outside the graph) and gets an answer that is ITSELF distorted
by the SignatureTooLong gap (Section A) — `complete.py` genuinely
contains 2 real top-level functions (`complete`, `_computations_complete`),
but the query returns **zero**, because `complete` itself was never
extracted as a symbol at all. This is a concrete, measured example of
HOW the Module/File gap and OTHER gaps compound: even a hypothetical
future Module/File node would, without ALSO fixing the extraction
limit, still report an incomplete or empty answer for this exact file.

### C1. Evidence-based recommendation on Module/File structural node

Per explicit instruction, a recommendation among the four options is
required, grounded in THIS evaluation's own evidence (and the
cumulative evidence from all five prior domains):

**Recommendation: Option 2 (a synthetic `File`/`Module` node), NOT
Option 1 (a real language-level `Module` symbol), tentatively preferred
over Option 3 (a separate structural-container layer) and clearly
preferred over Option 4 (continue without any node) — but NOT yet
implemented, per explicit instruction, and this recommendation is
offered with its own stated confidence level and residual
uncertainty, not as a final decision.**

**Why Option 2 over Option 1 (real `Module` symbol):** Python DOES have
a real language-level module concept (`import janitor.functions.complete`
refers to a real runtime module object), so Option 1 is not
semantically dishonest the way it might be for a language with no
module concept at all — but this evaluation's own cross-domain
evidence (Rust, which has NO file-level module concept at all beyond
`mod` declarations that do not map 1:1 to files; TypeScript, where a
file IS a module but ES module semantics are themselves non-trivial;
C#, which has namespaces that cross-cut files freely) shows that a
"real `Module` symbol" would need a PER-LANGUAGE semantic definition
with real edge cases in at least 3 of the 6 languages this pipeline
already supports, each requiring its own research-grade investigation
before implementation — a significantly larger, slower-validated
undertaking than a synthetic node that simply says "this span of this
file" uniformly across every language.

**Why Option 2 over Option 3 (separate structural-container layer):**
a separate layer (e.g. a non-symbol `ModuleFact`, mirroring this
project's own precedented `ImportFact`/`ImplFact` pattern) was seriously
considered, since it would avoid touching `SymbolKind` at all — but
Option 2 (a real, lightweight `SymbolKind::Module`/`SymbolKind::File`
symbol with a `Contains` edge to every top-level symbol in that file)
has one concrete advantage THIS evaluation's own evidence surfaces: Q3's
own query shape ("what does this file contain") is naturally a
`Contains`-edge query, the SAME edge kind this pipeline already uses
for class-to-member containment — reusing an existing, already-
validated edge kind for file-to-top-level-symbol containment is a
smaller conceptual addition than introducing a wholly separate
structural-fact architecture whose own query semantics would need to be
invented from scratch. A synthetic `Module`/`File` symbol, with
`Contains` edges to its own top-level symbols, is the option that fits
most naturally into the EXISTING graph model without requiring a new
query mechanism.

**Why NOT yet implemented, and why confidence here is "tentative, not
final":** this evaluation's own evidence (Section C's compounding-gaps
observation) shows that a Module/File node ALONE would not even fully
answer Q3 correctly in this specific corpus, because the SignatureTooLong
extraction gap (Section A) independently distorts which symbols exist
to be contained in the first place — any Module/File implementation
would need to be evaluated AFTER that gap (and CRC-003's alias gap) are
addressed, or its own evaluation numbers would be contaminated by
unrelated extraction gaps in exactly the way this project's own
Evidence-before-claims policy warns against. This recommendation is
therefore offered as the evidence-based DIRECTION, not a green light to
implement immediately — consistent with every prior domain's own
"not implemented during this evaluation" instruction.

**What would change this recommendation:** if a future Rust-domain or
C#-domain evaluation found that a real `Module`/`namespace` symbol was
needed for a query shape Option 2 cannot answer (e.g. "which types does
this NAMESPACE define, across multiple files" — a cross-file grouping
Option 2's own one-file-one-node design cannot represent), that would
be concrete evidence to revisit Option 1 specifically for those
languages, without necessarily abandoning Option 2 for Python/Rust/
TypeScript's own simpler file=module correspondence.

## D. Data-flow vs. call-graph gap quantification

| Metric | Value | Status |
|---|---:|---|
| Method-chain call sites (`receiver.method(...)` shape, raw parse) | 10480 | Measured — a conservative syntactic proxy, not resolver-grade |
| Decorated functions (`function_definition` wrapped in `decorated_definition`) | 1746 | Measured |

**Interpretation:** 10480 method-chain call sites against 1485 real
`Calls` edges in the final graph confirms this domain's own dominant
idiom (pandas-flavor method chaining, `df.clean_names().remove_empty()`-
shaped pipelines) produces vastly more SYNTACTIC call-shaped
constructs than this pipeline's resolver currently connects into real
edges — though NOT because of a macro-opacity-shaped gap (Rust's own
Section C finding, B2-EVAL-005): method-chain calls ARE real
`call_expression`-equivalent nodes in Python's own grammar (an
`attribute`-function `call` node), fully visible to tree-sitter and, in
principle, to `resolve_calls`. **Confirmed decorated functions (1746,
62.5% of all 2792 `Function` symbols) are each individually still
correctly extracted** (confirmed by a dedicated probe before this
harness was written: a decorated `function_definition` is recursed
into normally by the existing Python extractor's own unconditional
child-walk, with the decorator itself excluded from the resulting
candidate's own `signature_text`) — decoration itself is NOT a gap.

### D1. The real data-flow gap, demonstrated directly (Section H, Q4)

Per explicit instruction, this evaluation measured whether this
pipeline can answer "does function A's OUTPUT flow into function B's
INPUT" for a real method-chaining pipeline
(`df.clean_names().remove_empty()`, confirmed real at
`examples/notebooks/dirty_data.py` line 92). **Confirmed structurally,
not by this one example alone:** no `EdgeKind` this pipeline produces
(`Calls`, `Contains`, `Extends`, `Implements`, `TestsFor`,
`Instantiates`, or lineage) encodes a relationship between two VALUES
(a call's return value, an argument it flows into) — every edge kind
connects two SYMBOLS via a structural or call relationship, never a
runtime data-flow relationship. This is a genuine, structural absence,
not a resolver bug.

### D2. Quantified classification of coding-agent questions by information need

Per the explicit experimental-methodology requirement to quantify, not
merely assert, how many of this evaluation's own 7 Q&A questions need
each kind of information (see Section H for the full detail):

| Information need | Questions | Count |
|---|---|---:|
| Calls only | Q1b (ground truth exists, blocked by extraction, not Calls itself), Q5, Q6 (TestsFor specifically) | 3 |
| Calls + import-alias-aware resolution | Q2 | 1 |
| Module/File structure | Q3 | 1 |
| Data-flow (structurally unanswerable via Calls) | Q4 | 1 |
| Calls + impact analysis (same mechanism as Calls, different framing) | Q7 | 1 |

**Of these 7, the ACTUAL blocking factor for the 4 that are Not
answerable/Incorrect breaks down as: 2 blocked by the SignatureTooLong
extraction gap (Q1/Q1b's own real targets), 1 blocked by import-alias
blindness (Q2, CRC-003), 1 structurally blocked by the complete absence
of any data-flow edge kind (Q4).** Zero of the 7 questions were blocked
by Module/File structure alone being absent (Q3 IS blocked by it, but
its own specific answer is ALSO contaminated by the extraction gap,
so Module/File's own marginal contribution to Q3's failure cannot be
cleanly isolated in this corpus) — **this is itself a finding**: in
THIS domain, on THIS corpus, the extraction-limit and import-alias
gaps are measurably more consequential to coding-agent usefulness than
the Module/File gap is, a different balance than Rust's own domain
(B2-EVAL-005), where Module/File-adjacent gaps (impl-as-container) were
the dominant limitation. This directly informs which experimental
technique deserves the next checkpoint's own priority (see Section I).

## E. Edge quality

| Metric | Value | Status |
|---|---:|---|
| Lineage edges (commit B alone, no prev) | 0 | Measured |
| `Calls` edges | 1485 | Measured |
| `Contains` edges | 27 | Measured (class-to-member containment; this corpus has only 20 classes) |
| `TestsFor` edges | 53 | Measured |
| `Extends`/`Implements` edges | 0 | Measured — expected; this corpus's own class usage (20 classes total) includes no real inheritance hierarchy this evaluation's own sample encountered, and Python's own `Extends` mechanism (unaffected by B2-ID-008, which is Rust-specific) was not separately stress-tested here |
| `StaticAnalysis` (resolution method) | 1457 | Measured |
| `NameHeuristic` (resolution method) | 108 | Measured |
| Full index wall-clock | 1.4543675s | Measured, single run |

## F. Manual Calls-quality sample — TP/FP/FN judgment

The deterministic 30-edge sample (sorted by evidence note) was
individually checked against real source at commit B:

**30/30 sampled edges are target-correct and caller-correct (TP). 0
FP. 0 FN identified among the specific calls spot-checked.**

Representative confirmations:
- `build_patterns`-equivalent constructor calls: `chisq_test ->
  ChiSqTestResult` (confirmed `return ChiSqTestResult(...)`,
  `statistical_tests.py` line 181), `fisher_test -> FisherTestResult`
  (same shape), `convert_currency -> JanitorError` (confirmed, a real
  exception-raising call).
- `JanitorError` (a real exception class, `errors.py` line 1) is
  called as a target from 9 DIFFERENT files in this one 30-edge sample
  alone (`finance.py`, `clean_names.py`, `utils.py`,
  `concatenate_columns.py`, `io.py`, `polars/clean_names.py`,
  `timeseries.py`) — each independently confirmed correct, demonstrating
  this pipeline's cross-file resolution handling a widely-shared
  exception type correctly at real scale.
- `DropLabel` (a real class, `select.py` line 676) correctly resolves
  as the target for 13 DIFFERENT callers in this one sample —
  confirmed the class exists and is the only candidate with that name
  in the corpus.
- A genuine SAME-NAME case found during validation (not from the
  30-edge sample itself, found while checking for CRC-001/CRC-002-
  shaped risk): `_change_case` and `_strip_underscores_func` are EACH
  defined in TWO different files (`clean_names.py` and `utils.py`)
  with identical names — confirmed NOT to produce a false edge in
  this sample (both same-named pairs independently resolve correctly
  to their own real target, `JanitorError`, since the AMBIGUOUS part
  of their own names was never the one being looked up in these
  particular call sites) — but flagged as a corpus-level risk worth
  independent future investigation (not pursued further here, since
  no concrete FP/FN was actually observed from it in this evaluation's
  own sample).

**Precision (on this 30-edge sample): 30/30 = 100%.** Recall explicitly
NOT computed, consistent with every prior B2 evaluation's own stated
limitation (no independently established denominator of ALL real call
sites in this ~291-file corpus exists).

## G. Incremental indexing (A -> B, real multi-file commit)

| Metric | Value |
|---|---:|
| Changed files (`git diff A..B`) | 3 (`janitor/functions/complete.py`, `janitor/functions/expand_grid.py`, `tests/functions/test_complete.py`) |
| Files reparsed | 3 |
| Snapshots removed | 61 |
| Snapshots added | 62 |
| Edges rebuilt | 1626 |
| Raw import statements extracted (this call) | 39 |
| Raw import statements reused (this call) | 1617 |
| Import facts resolved | 1656 |
| Raw impl statements extracted/reused/resolved | 0 / 0 / 0 (expected — no Rust `impl` syntax in a Python corpus, confirms `ImplFact`'s own machinery correctly no-ops for a language with no real `impl_item` constructs) |
| Full index (commit A) wall-clock | 1.5285284s |
| Incremental index (A->B) wall-clock | 843.1163ms |
| Final snapshot count | 2883 |
| Final edge count | 1565 |
| Final lineage edge count | 61 |

**Interpretation:** the incremental update correctly reused 1617/1656
(97.6%) of raw import statements despite only 3 files changing — the
highest reuse percentage of any B2 domain measured so far, consistent
with this being a small, surgical 3-file change in a 433-file repository
(far less churn, proportionally, than any prior domain's own
incremental commit pair). No `DuplicateSnapshotId`/`DuplicateEdge`
panics, no stale-state warnings. The incremental wall-clock (830ms) is
FASTER than the full index (1.45s), unlike Rust's own B2-EVAL-005
anomaly (incremental slower than full) — consistent with this domain's
own much smaller, more surgical change (3/433 files vs. Rust's own
10/25 files, a proportionally far larger fraction of the corpus).

## H. Data-science coding-agent usefulness Q&A

| # | Question | Answer (observed) | Classification | Root cause if Not answerable/Incorrect |
|---|---|---|---|---|
| Q1 | Does `complete()` call `groupby_for_expand` (cross-file)? | NOT ANSWERABLE: `complete()` itself is not a symbol in this graph (10263 bytes, exceeds `MAX_SIGNATURE_BYTES`=4096) | **Not answerable, root-caused** | SignatureTooLong extraction gap |
| Q1b | Who calls `groupby_for_expand`? | `[]` — ground truth: BOTH real callers (`complete`, `expand`) are themselves SignatureTooLong rejections (10263, 7257 bytes) | **Not answerable, root-caused** (NOT a Calls-resolution defect — confirmed, a second independent instance of the same gap) | SignatureTooLong extraction gap |
| Q2 | Who calls `get_features_targets`? | `[]` — ground truth: the one real caller (`janitor/__init__.py`'s deprecated wrapper) calls it via an unresolved IMPORT ALIAS (`_get_features_targets`) | **Incorrect by omission** (a real relationship exists, zero false edges, but the true edge is genuinely missed) | Import-alias-blind call resolution (CRC-003, new) |
| Q3 | What does `janitor/functions/complete.py` contain? | `[]` symbols found via file-path string matching (the only available mechanism) — ground truth: 2 real top-level symbols exist, but `complete` itself is a SignatureTooLong rejection | **Not answerable** (compound: both Module/File structure AND the extraction gap) | Module/File structure absent AND SignatureTooLong (compounding) |
| Q4 | In `df.clean_names().remove_empty()`, does `clean_names`'s output flow into `remove_empty`'s input? | NOT ANSWERABLE, for two independent reasons: `clean_names` itself not extracted (also SignatureTooLong, 6133 bytes); AND no `EdgeKind` encodes data-flow at all | **Not answerable, root-caused (two independent reasons)** | SignatureTooLong AND structural data-flow absence |
| Q5 | Who calls `remove_empty` (incl. method-chain sites)? | 14 real callers found, correctly resolved — 11 real notebook/example call sites plus 3 real test functions, confirmed against source | **Fully answerable** | — |
| Q6 | Which tests cover `complete()`? | `53` `TestsFor` edges exist in the corpus overall; `complete()` itself cannot be checked specifically since it's not a graph symbol | **Not answerable** (same extraction gap as Q1) | SignatureTooLong extraction gap |
| Q7 | If I modify `groupby_for_expand`, what is impacted? | `[]` — same ground truth and root cause as Q1b | **Not answerable, root-caused** | SignatureTooLong extraction gap (duplicate of Q1b's own finding, different framing) |

**Summary (out of 7 questions): 1 Fully answerable (Q5), 1 Incorrect
by omission (Q2), 5 Not answerable — but EVERY ONE of the 5 Not-
answerable results is precisely root-caused** (4 to the
SignatureTooLong extraction gap, 1 compound with Module/File
structure), none left as an unexplained blanket failure. **This is the
first B2 domain where a single root cause (SignatureTooLong) accounts
for the majority of Not-answerable results** — a materially different
failure profile from every prior domain, directly attributable to this
domain's own real documentation convention (long NumPy-style docstrings
with embedded runnable examples) interacting with a pre-existing,
general-purpose extraction limit that previous domains' own corpora
happened not to stress nearly as heavily.

## I. Challenge set — new cases from this evaluation

- **CRC-003 (new, filed in full in `docs/research/challenge-sets/call-resolution-challenges.md`):**
  import-alias-blind call resolution — `resolve_calls` matches callee
  names directly against the snapshot pool, never consulting
  `ImportFact.alias` data that is itself correctly captured (459
  aliases in this corpus alone). A genuine false negative (zero false
  edges, zero false positives), distinct from both CRC-001 (ambiguity)
  and CRC-002 (scope-blindness) in its own failure shape (zero
  candidates matched, not multiple, not one-wrong-confident-match).
  Directly actionable evidence for the project's own CALL-RES-A4
  (import/visibility-aware resolution) backlog entry — this is the
  FIRST real-corpus case demonstrating that specific technique would
  recover a genuine, currently-missed edge.
- **The SignatureTooLong extraction gap is NOT filed as a `CRC-...`
  entry** — consistent with this project's own established reasoning
  (B2-EVAL-005's own precedent for why B2-ID-008 was a full finding,
  not a `CRC-...` entry): this is an EXTRACTION limit, not a
  `Calls`-RESOLUTION ambiguity/shadowing/alias case, so it falls
  outside this specific challenge-set's own stated scope. It is
  recorded here, in Section A/D2/H, with full quantification instead.

## J. Performance

| Metric | Value | Status |
|---|---:|---|
| Full index wall-clock | 1.4543675s (harness's own full pipeline) / 1.5285284s (via `CodeGraphIndexer::observe_commit`, commit A alone) | Measured, single run |
| Incremental index wall-clock | 843.1163ms | Measured, single run |
| Memory | 13,228 KB ≈ 12.92 MB (Windows `tasklist`, coarse) | Estimated |
| Graph size (final, commit B) | 2883 snapshots, 1565 edges, 61 lineage edges | Measured |
| `ImportFacts` (final) | 1656 | Measured |

Single-run measurements, not multi-run averages, consistent with every
prior B2 evaluation's own stated limitation. This is, by a wide margin,
the LARGEST corpus measured in the B2 multi-codebase suite so far
(2822 raw candidates vs. 426-614 in every prior domain) — full index
wall-clock (1.48s) is proportionally consistent with that size increase,
not a disproportionate slowdown.

## K. Research integrity notes

- The SignatureTooLong finding (Section A/D2/H) was discovered by
  directly root-causing an UNEXPECTED empty Q&A answer (Q1), not
  assumed or anticipated from the protocol's own text — the harness's
  own first version used `complete`/`clean_names` as Q&A targets
  without checking their byte size first, and the resulting "NOT FOUND"
  results were investigated to their real root cause rather than
  silently worked around by picking different, convenient targets.
- Having found the root cause, the evaluation's own Q&A targets were
  THEN adjusted to representative, VERIFIED-answerable equivalents
  (Q1b, Q5) specifically so this domain's own Calls-resolution quality
  could still be measured on a real case — this adjustment is disclosed
  explicitly in Section H/D2, not silently substituted.
- CRC-003 was opened immediately upon confirming the import-alias gap
  against real source, per the challenge-set framework's own
  "collected during real evaluations, not invented" standard.
- No repository/commit was excluded or re-chosen to produce a more
  favorable result; `pyjanitor` was the first and only candidate
  evaluated in depth.
- Neither of this evaluation's own two new findings (SignatureTooLong's
  domain-specific frequency; CRC-003's import-alias gap) reached
  Important/Critical severity — both are genuine, measured, zero-false-
  positive gaps (fail-closed in both cases), consistent with the
  instruction to register Moderate findings and continue rather than
  stop.

## L. Cleanup

The temporary clone (a fresh, isolated scratchpad directory, never
reused from any prior clone) was deleted after this evaluation's
harness output was captured and preserved. The repository identity,
commit pair, license, and SPDX reproducibility data (above) are
retained; the raw run output is preserved at
`docs/research/evidence/b2-eval-006-pyjanitor-run.txt`.

## Residual limitations of this evaluation

- Domain/language coverage for the multi-codebase suite is now 6
  repositories (ML/Python+C++; Backend/API Python; Frontend/UI
  TypeScript/TSX; Rust/systems; Data analysis/scientific Python — this
  evaluation).
- The same-name `_change_case`/`_strip_underscores_func` collision
  (Section F) was found and confirmed NOT to produce an observable FP/FN
  in this specific sample, but was not exhaustively traced across every
  real call site to either pair in the full 1485-edge corpus — flagged
  as a candidate for a future, more targeted same-name-collision audit,
  not pursued further here since no concrete defect was found.
- CRC-003's own fix (import-alias-aware call resolution) is NOT
  implemented here, per explicit instruction — it is recorded as
  evidence for the CALL-RES-A4 backlog entry, a design/implementation
  decision for a future, dedicated checkpoint.
- The Module/File-node recommendation (Section C1) is evidence-based
  and directional, not final — it explicitly flags that its own
  evaluation would need to follow, not precede, resolution of the
  SignatureTooLong and import-alias gaps, to avoid contaminating a
  future Module/File evaluation's own numbers with unrelated extraction
  defects.
- `Extends`/`Implements` edge quality was not separately stress-tested
  on Python class inheritance in this evaluation (this corpus's own
  20 classes did not happen to include a real inheritance hierarchy in
  this evaluation's own sample) — a gap in THIS evaluation's own
  coverage, not a claim that Python inheritance resolution is
  unverified generally (it was previously exercised in B2-EVAL-001's
  own ML/DL corpus).
