# B2-EVAL-001: real-repository smoke test — `pytorch/examples`

- **Evaluation ID:** B2-EVAL-001
- **Date:** 2026-11-05
- **SuperBrain commit:** `47abdf3` (the commit immediately after B2-ID-001/
  B2-ID-002's fixes, before this evaluation's own harness-only follow-up
  edits)
- **Related Project Findings:** this evaluation DISCOVERED
  [B2-ID-001](../findings/B2-ID-001.md) and
  [B2-ID-002](../findings/B2-ID-002.md); this record covers the
  re-evaluation performed AFTER both were fixed.

## Repository / dataset

- **Repository:** `pytorch/examples` — https://github.com/pytorch/examples
- **Exact commit (B, "after"):** `001d493285083b54cacbc1334cb26338bdff2455`
  ("Update TransformerModel using nn.Transformer module (#1138)")
- **Exact commit (A, "before"):** `20955149ad6ea97a4f403e5a56c95a39e568ccbe`
  ("Bugfix in vision transformer example - change lr datatype to
  float (#1161)")
- **License:** BSD 3-Clause — `LICENSE` at repository root, "Copyright
  (c) 2017, Pytorch contributors." No formal SPDX tag in the file, but
  the license text is unambiguous BSD-3-Clause.
- **Languages:** Python (majority), C++, CUDA/C++ hybrids under `cpp/`.
- **Domain:** deep learning / ML examples (official PyTorch example
  collection — covers vision, NLP, RL, distributed training, GANs,
  etc., not a single-purpose repo).
- **Scope of this evaluation:** the WHOLE repository tree at the given
  commits (214 files total) was indexed — not a cherry-picked
  subdirectory — with manual deep-dive analysis focused on
  `word_language_model/` (the PyTorch RNN/Transformer language-model
  example) as the deep-learning-relevant case.

## Configuration / methodology

Driven via `crates/codegraph-index/examples/b2_smoke_test.rs`, a
throwaway investigative harness calling exactly the same public API any
other caller of this crate would use: `LocalGitRepositorySource` →
`extract_symbols_reporting_rejections` → `promote` → `resolve_lineage`/
`resolve_calls`/`resolve_imports`/`resolve_extends_implements`/
`resolve_tests_for` → `CodeGraph` → `CodeGraphIndexer::observe_commit`.
No fixture-specific hacks, no hardcoded mapping on this repository's
name, no special-casing. The repository was cloned read-only into the
session scratchpad (never inside the SuperBrain project tree) and left
untouched throughout.

A deterministic, line-based import-statement count (grep-equivalent
over each Python file's text) served as an independent, non-pipeline
baseline for the Imports-gap measurement. Manual source reading (via
`git show <commit>:<path>`) served as the ground truth for the call-
quality sample and the deep-learning Q&A section — no second parser was
built; this is the "simple structural counts + manual sample" baseline
the checkpoint asked for, not a second complete extractor.

## B. Pipeline results (full index at commit B) — MEASURED

| Metric | Value |
|---|---|
| Total files in tree | 214 |
| Files supported by parser | 97 |
| Files parsed | 97 |
| Parse failures | 0 |
| Files with `tree.has_error()` | 0 |
| Raw candidates extracted | 567 |
| Extraction rejections (oversized name/signature) | 11 |
| Snapshots promoted | 567 |
| Promotion failures | 0 |
| Python files with ≥1 import statement (MEASURED, line-based) | 86 |
| Full index elapsed | ~1.03s (MEASURED, 3 repeated runs: 1.0287s / 1.0269s / 1.0341s) |

**Note on "0 promotion failures" vs. "11 extraction rejections":** these
are two different, correctly-distinct outcomes per Task 4/5's own
design — an extraction rejection (oversized name/signature) means the
candidate never became a `RawSymbolCandidate` eligible for promotion at
all; it is silently dropped before promotion is ever attempted, per the
documented "drop-and-continue, not fail-the-file" policy. This
evaluation independently confirmed at least one of these 11 rejections
by hand (see section D's `Agent` case below) — the rejection count is
real and has a real, observable downstream effect on resolution
availability.

## C. Symbol coverage (SymbolKind counts, commit B) — MEASURED

| `SymbolKind` | Count |
|---|---|
| Class | 98 |
| Function | 462 |
| Struct | 7 |

No `Method`, `Interface`, `Enum`, `Module`, `Const`, `Macro`,
`MethodSignature`, HTML/CSS kinds observed — consistent with this
corpus being almost entirely Python (where this pipeline's
`kind_mapping` currently maps every `def` inside or outside a class to
`Function`, never `Method` — see Coverage Expansion section I) plus a
smaller amount of C++ (`Struct` from `struct` declarations).

## D. Edge quality (commit B) — MEASURED + MANUALLY VALIDATED SAMPLE

| Metric | Value |
|---|---|
| Lineage edges (B built alone, no prior commit) | 0 (expected — no `prev_snapshots` supplied for a standalone full index) |
| `Calls` edges | 11,934 |
| `Imports` edges | 0 |
| `Extends`/`Implements` edges | 4 |
| `TestsFor` edges | 0 |
| By resolution method: `StaticAnalysis` | 286 |
| By resolution method: `NameHeuristic` | 11,652 |

**Manually validated sample (20 `Calls` edges, deterministically
selected by sorting on evidence note text, each checked against the
real source via `git show`):**

| # | Edge | Method | Manual verdict |
|---|---|---|---|
| 1 | `reinforce.py::run_worker → rl/main.py::Agent` | StaticAnalysis | Correctly resolved — but see note below |
| 2 | `rl/main.py::run_worker → rl/main.py::Agent` | StaticAnalysis | Correctly resolved |
| 3 | `imagenet/main.py::train → imagenet/main.py::AverageMeter` | StaticAnalysis | Correctly resolved |
| 4 | `imagenet/main.py::validate → imagenet/main.py::AverageMeter` | StaticAnalysis | Correctly resolved |
| 5 | `parameter_server.py::run_ps → ...::BatchUpdateParameterServer` | StaticAnalysis | Correctly resolved |
| 6 | `mingpt/model.py::__init__ → ...::Block` | StaticAnalysis | Correctly resolved (DL-relevant: submodule instantiation) |
| 7 | `mingpt/main.py::get_train_objs → char_dataset.py::CharDataset` | StaticAnalysis | Correctly resolved (cross-file) |
| 8-11 | `transformer_net.py::* → ...::ConvLayer` (×4) | StaticAnalysis | Correctly resolved (DL-relevant) |
| 12-19 | `cpp/*/main.cpp::main`/`custom-dataset.cpp::main → *::CustomDataset` (×8) | NameHeuristic | Correctly heuristic — genuinely two distinct `CustomDataset` classes exist in two unrelated example directories; no type info exists to disambiguate, so flagging both as ambiguous is the honest, correct outcome |
| 20 | `mingpt/main.py::main → char_dataset.py::DataConfig` | StaticAnalysis | Correctly resolved |

**Summary of the 20-sample:** 18/20 correctly resolved (`StaticAnalysis`),
2/20 (really 8/20 counting all `CustomDataset` instances, collapsed to
one row above) correctly heuristic, **0 false positives, 0 false
negatives** in this specific sample.

**Note on row 1 (`Agent`):** manual investigation found something more
interesting than a plain correct resolution. There are TWO classes
named `Agent` in this repository (`distributed/rpc/batch/reinforce.py`
and `distributed/rpc/rl/main.py`), which should make this call
ambiguous. It resolved as `StaticAnalysis` (exactly one candidate)
because `reinforce.py`'s own `Agent` class body is 5,541 bytes —
over `MAX_SIGNATURE_BYTES` (4,096) — and was almost certainly one of
the 11 extraction rejections, silently removed from the candidate pool
before promotion. The resolution is technically correct given the pool
it was handed (only one `Agent` survived to be a candidate), but this
is a real, observable interaction between the bounded-signature policy
and call resolution: an oversized symbol's own name becomes
unavailable as a call target anywhere in the corpus, which can make an
otherwise-ambiguous call look falsely unambiguous. Not a new finding on
its own (the bounded-text policy works exactly as documented — "reject,
don't truncate," applied correctly), but recorded here as a concrete,
Measured interaction worth tracking if MAX_SIGNATURE_BYTES or
real-repository class sizes become relevant to a future task.

## E. Deep-learning usefulness — MANUALLY VALIDATED

Target: `word_language_model/model.py`'s `TransformerModel` class at
commit B (`class TransformerModel(nn.Transformer)`, with
`self.pos_encoder`, `self.input_emb`, `self.encoder` (now `nn.Transformer`'s
own inherited encoder, not a local field), `self.decoder` as submodules,
and a `forward` method).

| Question | Graph answer | Verdict |
|---|---|---|
| Q1. What is the main model class? | `TransformerModel` found (2 snapshots: 1 current, 1 historical from before the A→B edit — `CodeGraph` has no current-vs-historical distinction of its own, so disambiguating required checking `lineage_from(id).is_empty()` manually in this harness) | **Partially answerable** — the NAME is found immediately; distinguishing "the current one" requires extra, non-obvious graph-navigation logic a naive consumer would not know to apply |
| Q2. What does it extend? | 0 `Extends` edges for the current `TransformerModel` | **Not answerable** — `TransformerModel` now extends `nn.Transformer`, a PyTorch stdlib class never present in this repository's own source, so `resolve_extends_implements` correctly finds no candidate and correctly produces no edge ("no edge beats a false edge" working as designed) — but the practical effect is the graph cannot tell a user what the model's real base class is when that base class is external |
| Q3. What composes the forward path? | ORIGINAL (pre-fix): `CURRENT forward calls: ["cpp/custom-dataset/custom-dataset.cpp::size", "cpp/transfer-learning/main.h::size"]` | **ORIGINAL VERDICT, SUPERSEDED — see the [B2-ID-004](../findings/B2-ID-004.md) erratum above.** Original verdict was **Incorrect** (these were false positives; the REAL calls `forward` makes were invisible to this pipeline entirely). Fixed by Task 9.5's Priority 5(A): now correctly returns the 5 genuine `Field` calls (`input_emb`, `decoder`, `_generate_square_subsequent_mask`, `encoder`, `pos_encoder`), 0 false positives. (Separately, [B2-ID-003](../findings/B2-ID-003.md): the originally-reported miss on `self._generate_square_subsequent_mask(...)` was an evaluation-harness bug, not a real omission — see that erratum above.) |
| Q4. Who calls the model? | `callers of TransformerModel: []`; `callers of RNNModel: ["distributed/rpc/rnn/main.py::_run_trainer"]` | **Partially answerable** — `RNNModel`'s one real cross-repo caller was found correctly (manually confirmed: `_run_trainer` does instantiate `RNNModel`); `TransformerModel` itself is only instantiated from `word_language_model/main.py`, which failed to resolve (see false negative below) |
| Q5. Which tests are linked? | `TestsFor` edges in the whole 97-file corpus: 0 | **Correctly answerable as "none exist"** — `word_language_model` genuinely has no test file in this repository, and the corpus-wide 0 count is consistent with that; this is the one question where "the graph found nothing" is the CORRECT answer, not a gap |
| Q6. If I modify `forward`, what is impacted? | ORIGINAL (pre-fix): 19 incoming edges found, ALL to unrelated `forward` methods in other files | **ORIGINAL VERDICT, SUPERSEDED — see the [B2-ID-004](../findings/B2-ID-004.md) erratum above.** Original verdict was **Incorrect** (every one of the 19 was a false positive). Fixed by Task 9.5's Priority 5(A): now returns exactly 1 incoming edge, a `Contains` edge (the class containing its own method), 0 false-positive `Calls` edges. |

**False positive, manually confirmed (Q3/Q6):** `forward`'s real body
calls `self.src_mask.size(0)` (a PyTorch tensor method, not a project
symbol) and `self._generate_square_subsequent_mask(...).to(device)`
(a chained call). `resolve_calls`'s method-call-by-final-segment
heuristic extracted `size` from the tensor-method call and matched it
— ambiguously — against two unrelated C++ `size()` methods in
`cpp/custom-dataset/` and `cpp/transfer-learning/`, which have nothing
to do with this model. This is a genuine false positive produced by
name-based heuristic matching with no type information, not a bug in
the matching LOGIC (the logic does exactly what it is documented to
do) but a real, Measured limitation of what that logic can achieve on
a method name that happens to collide with an unrelated built-in/
stdlib method name elsewhere in a large corpus.

**False negative — ORIGINAL CLAIM, SUPERSEDED (see erratum immediately
below):** `_generate_square_subsequent_mask` genuinely exists as 2
snapshots in `word_language_model/model.py` (one current, one
historical) and IS a real method `forward` calls
(`self._generate_square_subsequent_mask(len(src)).to(device)`), but it
never appears in `forward`'s resolved callees. Root cause (diagnosed,
not yet fixed — explicitly out of this evaluation's scope per the
checkpoint's own instructions): the call is a CHAINED call —
`(self._generate_square_subsequent_mask(...)).to(device)` — whose
OUTERMOST `call_expression` node's callee attribute chain ends in `to`,
not in the inner call's own name. `calls.rs`'s `callee_name` extraction
walks each `call_expression` node independently (via the recursive
`walk_for_calls`), so the INNER call `self._generate_square_subsequent_mask(...)`
should still be visited as its own node — but it evidently was not
picked up, or was picked up and then excluded by `find_by_name`'s
exclude-self-id logic in a way this evaluation did not fully trace to
the exact tree-sitter node shape. Recorded honestly as an UNRESOLVED
root cause for a confirmed real false negative — not patched during
this evaluation per the explicit "do not implement fixes during the
smoke test" instruction. Flagged as a candidate for a dedicated,
focused investigation (see section I).

> ## ERRATUM (Task 9.5, [B2-ID-003](../findings/B2-ID-003.md))
>
> **Previous claim:** the chained call
> `self._generate_square_subsequent_mask(len(src)).to(device)` never
> resolves to a `Calls` edge — a production-pipeline false negative
> with an undiagnosed root cause.
>
> **Corrected claim:** the production pipeline resolves this chained
> call CORRECTLY. The apparent false negative was caused by a bug in
> THIS EVALUATION's own harness (`crates/codegraph-index/examples/b2_smoke_test.rs`),
> which selected "the current `forward` snapshot in
> `word_language_model/model.py`" by local name alone — ambiguous,
> because that file has TWO classes (`RNNModel`, `TransformerModel`)
> each with their own `forward` method, producing three simultaneously
> "current" `forward`-named snapshots across one A→B update. The
> harness sometimes selected `RNNModel.forward` (which genuinely does
> not call `_generate_square_subsequent_mask`) instead of
> `TransformerModel.forward` (which does), and reported the former's
> empty callee list as if it were the latter's.
>
> **Cause of the previous (incorrect) result:** an evaluation-harness
> subject-selection bug, not a product defect. Full root-cause analysis,
> reproduction evidence (an isolated 2-snapshot case, a full-real-body
> case, and 5 repeated real-corpus runs all confirming correct
> resolution once the harness was fixed to disambiguate by the
> symbol's actual `Contains`-parent class rather than by local name
> alone), and the harness fix itself are recorded in
> [B2-ID-003](../findings/B2-ID-003.md).
>
> **What this changes:** the Q3/Q6 table rows above, and the "19-for-19
> false positive rate" discussion below, remain accurate for what they
> actually measured (the common-name false-positive pattern on
> `size()`/`forward` IS real and unaffected by this correction) — but
> the specific claim that `_generate_square_subsequent_mask` was a
> false negative is WITHDRAWN. The chained-call resolution mechanism
> itself needed no fix. This correction does not retroactively validate
> the original "Incorrect" verdict on Q3/Q6 as still being for the
> reason originally stated — Q3/Q6 remain genuinely "Incorrect" rows
> because of the common-name false positives alone, not because of any
> chained-call issue.

**19-for-19 false positive rate on Q6 is the single most important
finding of this evaluation's manual analysis — ORIGINAL CLAIM, NOW
FIXED (see erratum immediately below).** asking "what calls this
specific method" by following `incoming_edges` on a commonly-named
method (`forward`, in a 97-file corpus where it appears dozens of
times) returns overwhelmingly noise under the current heuristic-
matching design, with no signal distinguishing a real caller from an
unrelated same-named method elsewhere. This is Measured (the edges
genuinely exist in the graph) and Manually validated (every one of the
19 was read against real source and confirmed unrelated to this
specific `TransformerModel.forward`).

> ## ERRATUM (Task 9.5, Priority 5(A), [B2-ID-004](../findings/B2-ID-004.md))
>
> **Previous claim:** Q3 (`TransformerModel::forward`'s outgoing calls)
> returned 2 confirmed false positives (two unrelated C++ `size()`
> methods); Q6 (incoming callers of `forward`) returned 19 confirmed
> false positives, described as "the single most important finding of
> this evaluation's manual analysis." Both were attributed to
> `resolve_calls`'s purely name-based matching, with the root cause
> left undiagnosed in this evaluation (per its own "do not implement
> fixes during the smoke test" scope).
>
> **Fix and corrected result:** Task 9.5's Priority 5(A) formally
> root-caused both as one underlying defect (no receiver awareness, no
> scope awareness) with two distinct manifestations, and fixed both —
> see [B2-ID-004](../findings/B2-ID-004.md) for the full investigation,
> solution, and validation. Re-running the exact same pipeline against
> a freshly re-cloned copy of this evaluation's own repository/commit
> (Measured): Q3's outgoing calls from `TransformerModel::forward`
> are now `["word_language_model/model.py::input_emb",
> "word_language_model/model.py::decoder",
> "word_language_model/model.py::_generate_square_subsequent_mask",
> "word_language_model/model.py::encoder",
> "word_language_model/model.py::pos_encoder"]` — 0 false positives, and
> this list now matches the forward-path composition a human reading
> the source would report (the "REAL calls" this evaluation's own Q3
> row named as invisible to the pipeline). Q6's incoming edges to the
> current `forward` are now exactly 1 edge, a `Contains` edge (the
> class containing its own method — a structural fact, not a spurious
> `Calls` guess) — 0 false-positive `Calls` edges, down from 19.
>
> **What this changes:** the Q3/Q6 "Incorrect" verdicts in the table
> below, and this section's "19-for-19" framing, describe the PRE-FIX
> state only and must not be read as still describing the current
> pipeline's behavior. **What this does NOT change:** this evaluation's
> manual methodology (reading real source to confirm each edge) was
> correct and is exactly what surfaced the defect in the first place —
> the false positives were real defects in the pipeline at the time,
> not evaluation-harness artifacts (unlike the separate, differently-
> caused chained-call correction in the erratum above). This correction
> does not retroactively validate the original claim as still current;
> it records that the defect it correctly identified has since been
> fixed, with evidence.

## F. Incremental A→B — MEASURED

| Metric | Value |
|---|---|
| Changed files (via `git diff --name-only A..B`) | `word_language_model/model.py` (1 file) |
| Files considered | 1 |
| Files reparsed | 1 |
| `files_reparsed / total_files_at_B` reuse ratio | 1/97 ≈ 1.03% reparsed, ~98.97% reused |
| Snapshots removed | 13 |
| Snapshots added | 13 |
| Edges rebuilt (lineage + typed, full-pool recomputation per B2-ID-002's documented scope) | 11,951 |
| `observe_commit(A)` (full reindex) elapsed | ~1.05s (3 runs: 1.0446s / 1.0518s / 1.0658s) |
| `observe_commit(B)` (incremental) elapsed | ~0.75s (3 runs: 0.7529s / 0.7475s / 0.7589s) |
| Final snapshot count (current + historical) | 580 |
| Final typed edge count | 11,938 |
| Final lineage edge count | 13 |

**No stale snapshots/edges observed.** Every one of the 567 untouched
snapshots from the other 96 files was confirmed present unchanged in
the final graph (580 − 13 new = 567). No `DuplicateSnapshot`/
`DuplicateEdge`/dangling-edge error occurred across 3 repeated runs.
The 13 lineage edges correspond to the real `model.py` symbols whose
byte spans shifted due to the real code edit (the `nn.Module`→
`nn.Transformer` base-class change, the `self.encoder`→`self.input_emb`
rename, and the resulting shift of every symbol after that point in
the file) — consistent with B2-ID-001's own documented, intentional
span-sensitivity.

**Rename/move note:** this A→B pair is a real code edit (base-class
change + field rename inside one file), not a cross-file move or a pure
rename — the checkpoint's own instructions said not to force cherry-
picking a rename/move commit if it requires artificial manipulation, so
this pair (found naturally in the repository's real history as the
first substantive `model.py` edit after the chosen A) was used as-is.

## G. Performance observations — MEASURED / ESTIMATED

- Full index time: ~1.03s for 97 real files, 567 snapshots, 11,938
  edges — MEASURED, `std::time::Instant` wall-clock, 3 repeated runs
  with <1% variance.
- Incremental update time: ~0.75s for the same corpus with 1 file
  changed — MEASURED, same methodology. Note this is NOT proportionally
  faster than the full reindex despite only reparsing 1/97 files,
  because B2-ID-002's documented design recomputes typed edges over
  the FULL current snapshot pool on every incremental update (a
  deliberate correctness choice, not yet a performance-tuned one) —
  the dominant cost here is edge re-resolution over ~567-580 snapshots,
  not re-parsing.
- Memory: ESTIMATED only, via Windows `tasklist`'s reported working-set
  column (~14.2–14.4 MB for the whole example process, including the
  Rust runtime and this harness's own overhead) — explicitly not a
  reliable or precise measurement, recorded as an order-of-magnitude
  observation only, per the checkpoint's own "not a final benchmark"
  framing.
- No optimization was attempted during this evaluation, per its own
  explicit scope boundary.

## H. Missing capabilities — MEASURED + INFERRED

- **`Field`/`Attribute` (highest-priority gap, MEASURED):** repo-wide
  `self.X = nn.*` submodule assignments were counted via `git grep`.
  **Correction (post independent review):** this evaluation's own
  first pass reported 62, via a command (`git grep -nE ... | wc -l`)
  run against the (since-deleted) temporary clone; the independent
  review below re-measured the same pattern against a FRESH clone of
  the same commit and got **109** matches across 25 files, and could
  not reproduce 62 by any reasonable variant of the pattern. The
  reviewer's number is the one on record going forward — this
  evaluation's original 62 is WITHDRAWN as inaccurate/unreproduced
  rather than defended, since the original clone no longer exists to
  re-check the discrepancy's source, and the policy requires a
  "Measured" figure to actually be reproducible. The qualitative
  conclusion is unaffected and, if anything, strengthened (the gap is
  larger than first reported, not smaller). This also undercounts
  indirect cases (e.g. `self.rnn = getattr(nn, rnn_type)(...)`,
  `self.pos_encoder = PositionalEncoding(...)` — both real submodule
  assignments that reference a user-defined or dynamically-resolved
  constructor, not caught by the `nn.*`-literal grep). In
  `word_language_model/model.py` alone: 17 total `self.X = ...`
  assignments, of which 9 are genuine submodule/layer components
  across the file's two model classes — this file-level count WAS
  independently re-verified exactly by the reviewer and stands
  unchanged. NONE of these produce any symbol in the current pipeline
  — `Field`/`Attribute` genuinely does not exist as a `SymbolKind` yet, so every
  `self.encoder`/`self.pos_encoder`/`self.input_emb`/`self.decoder`-
  style component is invisible to `get_code_context` or any future
  coding agent built on this graph today.
- **`Imports` (MEASURED, confirms the Task 7 finding completely) —
  ORIGINAL CLAIM, SUPERSEDED (see erratum below):** 589 real Python
  import statements counted (independent, line-based, non-pipeline
  count) across the 86 Python files that have at least one; **0
  `Imports` edges produced**. 0% capture rate on real data. Root cause
  already documented in Task 7/`imports.rs`'s own module doc comment
  and reconfirmed empirically here: `signature_text` never includes
  anything outside a symbol's own span, and a file-level `import`
  statement is never inside any symbol's own span.

> ## ERRATUM (Task 9.5, Priority 6, [B2-ID-005](../findings/B2-ID-005.md))
>
> **Previous claim:** 0 of 589 real import statements captured —
> described as a 0% capture-rate GAP in the original symbol-to-symbol
> `EdgeKind::Imports` design, with the fix deferred to a future
> redesign.
>
> **Corrected claim:** the original design's granularity itself was
> wrong (a file-level fact forced into a symbol-to-symbol edge shape),
> not merely under-implemented — see
> [B2-ID-005](../findings/B2-ID-005.md) for the full analysis. It was
> REMOVED, not patched, and replaced with a separate file-level
> `ImportFact` model wired into the real production pipeline
> (`CodeGraphIndexer`). Re-running the exact same pipeline against a
> freshly re-cloned copy of this evaluation's own repository/commit
> (Measured, via the real indexer, not a manual resolver call): 691
> real `ImportFact`s extracted (the 589 figure was itself a crude
> per-LINE heuristic; the correct per-STATEMENT count is higher because
> 52 real lines in this corpus import more than one comma-separated
> name, each counted as its own fact) — 41 resolved `Internal`, 621
> correctly left `External` (no invented target), 29 correctly left
> `Ambiguous`. Incremental behavior confirmed: 687 of 691 raw statements
> reused unchanged across a real 1-file-changed A→B update, with
> resolution always recomputed from the full current pool (a
> deliberate, documented correctness-first choice, not an oversight).
>
> **What this changes:** `EdgeKind::Imports` no longer exists at all —
> any future reader of this evaluation looking for that edge kind in
> the current codebase will not find it; this is intentional, not a
> regression. The "0% capture rate" framing above describes the
> PRE-REDESIGN state only.


- **Scope-aware / qualified names:** confirmed directly relevant to
  B2-ID-001's own root cause — `init_weights` in two different classes
  in the same file is exactly the ambiguity a `QualifiedName` would
  resolve at the SEMANTIC level (today it is resolved only at the
  byte-position level, correctly for identity purposes but not for
  human/agent readability). The Q1/Q3/Q6 false-positive/ambiguity
  findings above (same-named methods across unrelated files/classes)
  are the SAME underlying gap manifesting in call resolution, not
  identity.
- **`Contains`/`Instantiates` (INFERRED, not directly measured by a
  dedicated resolver since neither exists yet):** every "DL-relevant"
  correctly-resolved `Calls` edge in the manual sample (rows 1-11,
  e.g. `Agent(...)`, `AverageMeter(...)`, `ConvLayer(...)`,
  `Block(...)`, `CharDataset(...)`, `DataConfig(...)`) is SEMANTICALLY
  an instantiation (`ClassName(...)` constructor call), not a plain
  function call — but this pipeline represents it as `Calls` because
  Python's constructor-call syntax is textually indistinguishable from
  a function call to the current resolver. This works adequately for
  "is there a relationship" but loses the semantic distinction a
  dedicated `Instantiates` edge kind (or a `Contains` edge for "this
  class's `__init__` instantiates and owns this submodule," closely
  tied to the `Field`/`Attribute` gap above) would add.

## I. Recommended coverage expansion (evidence-based, per this
## evaluation's own data — NOT implemented here)

Ranked by how often this evaluation's own evidence showed the gap
actually blocking real understanding:

1. **`Field`/`Attribute`** — HIGH PRIORITY. 109 real `self.X = nn.*`
   submodule assignments repo-wide (independently re-measured; see the
   correction in section H), 9 in the one file manually analyzed in
   depth, zero of which are representable today. This is the single
   gap most directly blocking "explain the model's architecture" —
   exactly the use case B2 exists to serve for deep-learning code.
2. **RESOLVED by [B2-ID-005](../findings/B2-ID-005.md).** `Imports`
   redesign — originally listed HIGH PRIORITY on a confirmed 0%
   real-data capture rate. Task 9.5's Priority 6 confirmed the real
   defect was the edge's own granularity (symbol-to-symbol, when an
   import is a file-level fact), removed `EdgeKind::Imports` entirely,
   and replaced it with a separate file-level `ImportFact` model wired
   into the real production pipeline — see the erratum above for the
   measured before/after (0/589 line-based → 691 real facts via the
   actual indexer).
3. **Scope-aware/qualified names** — MODERATE PRIORITY. Would resolve
   both B2-ID-001-adjacent identity confusion (addressed at the byte-
   span level already) and the Q1/Q3/Q6 false-positive/ambiguity
   pattern (NOT addressed — same-named methods/classes across
   unrelated files remain a source of heuristic noise). Explicitly not
   a quick fix: Task 4 has no real scope-chain resolver, as already
   documented in B2-ID-001's own rejected-alternative analysis.
4. **`Instantiates`/`Contains`** — MODERATE PRIORITY. Would let a
   future consumer distinguish "A calls B" from "A constructs/owns an
   instance of B," directly useful for the exact DL architecture-
   understanding use case this evaluation targeted, and naturally
   follows once `Field`/`Attribute` exists (an `__init__` assigning
   `self.encoder = TransformerEncoder(...)` would produce both a new
   `Field` snapshot AND an `Instantiates` edge from `__init__` to
   `TransformerEncoder`).
5. **RESOLVED by [B2-ID-004](../findings/B2-ID-004.md).** Method-name-
   collision-aware call resolution — originally listed as LOWER
   PRIORITY, same root cause as #3 (scope-awareness) rather than a
   separate gap. Task 9.5's Priority 5(A) fixed the two confirmed
   manifestations of this gap directly (receiver-aware filtering for
   `self`/`this`-qualified calls, scope-hierarchy narrowing for
   unqualified ambiguous calls) — see the Q3/Q6 erratum above for the
   measured before/after. Note this does NOT fully resolve #3 (full
   qualified-name/scope-chain resolution, Priority 4, remains not yet
   implemented as a product feature) — only the specific call-
   resolution false positives this evaluation measured.
6. **WITHDRAWN** — the originally-listed "chained-call extraction gap"
   is RETRACTED per the erratum above and [B2-ID-003](../findings/B2-ID-003.md).
   Task 9.5's dedicated root-cause investigation confirmed
   `calls.rs`'s chained-call walk is correct; the apparent gap was an
   evaluation-harness subject-selection bug, now fixed. No coverage
   expansion is needed for chained calls.

## Independent review

An independent review (`agent-protocols:code-reviewer`) was performed
specifically to avoid this evaluation grading its own homework: the
reviewer was instructed to RE-CLONE `pytorch/examples` into its own
fresh scratch location (the original clone had already been deleted
per this project's cleanup policy) and independently verify the
specific, falsifiable manual claims in this document against real
source — not just re-run the same pipeline and call that verification.

**Verdict: trustworthy as written, with one correction applied.** The
reviewer independently confirmed, against a fresh clone: the two
`Agent` classes and the 4096-byte `MAX_SIGNATURE_BYTES` rejection
explaining row 1 of the manual sample; the two `CustomDataset` classes;
the tensor-`.size()` false positive and its two unrelated C++ collision
targets; the chained-call false negative and its exact source location;
the `nn.Module`→`nn.Transformer` base-class change and the confirmed
absence of any local `Transformer` class definition; the `Imports`
0%-capture architecture (read directly from `imports.rs`'s own module
doc comment) plus a 5-file spot-check; the exact 9-of-17 submodule
count in `model.py`; the single-file A→B diff scope; and the cross-
reference to B2-ID-001's real `init_weights` collision. The reviewer
also judged the report's research-integrity discipline (Measured/
Manually validated/Inferred/Estimated tagging, honest reporting of
false positives/negatives alongside favorable results) as sound and
not one-sided.

**One discrepancy found and corrected:** the repo-wide
`self.X = nn.*` submodule-assignment count, originally reported as 62
("MEASURED via `git grep`"), could not be reproduced by the reviewer
against a fresh clone of the same commit (`git grep` of the same
pattern returned 109 across 25 files, confirmed by manual inspection to
be genuine `nn.*` constructions, not an over-broad pattern match). The
original clone no longer existed to diagnose exactly where the 62
count went wrong, so rather than defend an unreproduced number, this
document now reports 109 (the independently reproduced figure) and
withdraws 62 — see the correction note in section H. This does not
change this evaluation's qualitative conclusion about the `Field`/
`Attribute` gap; if anything it strengthens it.

## J. GO/NO-GO to continue B2

**GO**, with the explicit evidence-based conclusions above now on
record, and confirmed by independent review (see above) to not be an
artifact of the pipeline evaluating its own output. Task 10 remains NOT
authorized by this evaluation — that decision is the user's, per the
project's own standing protocol.

## Residual limitations of this evaluation itself

- Single codebase, single domain (ML/Python + some C++). Per the new
  Evaluation & Paper Evidence Policy, B2 is not considered validated by
  this evaluation alone — a multi-domain corpus (backend/API, frontend/
  UI, Rust/systems, C++/C#, full-stack, CLI/tooling) remains required
  before B2's closure.
- The manual call-quality sample (20 edges) and the deep-learning Q&A
  section are Manually validated against real source by one reader
  (this session), not independently cross-checked by a second human —
  mitigated by routing this evaluation's conclusions through an
  independent review pass (see below) before being accepted as final.
- Memory measurement is Estimated only (coarse OS tool), not a real
  profiler reading.
- **RESOLVED by [B2-ID-003](../findings/B2-ID-003.md):** the chained-call
  false negative's root cause (listed here, at the time of this
  evaluation's original writing, as an open question) was fully traced
  by Task 9.5's dedicated investigation — it was an evaluation-harness
  subject-selection bug, not a pipeline defect. See the erratum above.
- **RESOLVED by [B2-ID-004](../findings/B2-ID-004.md):** the Q3
  receiver-qualified false positive and the Q6 19-for-19 common-name
  false-positive rate (both listed here, at the time of this
  evaluation's original writing, as confirmed but un-root-caused
  defects) were formally root-caused and fixed by Task 9.5's Priority
  5(A) — see the erratum above for the measured before/after. This fix
  is itself recorded as an experimental configuration
  (`docs/research/EXPERIMENTAL_POLICY.md`, CALL-RES-A1+A2+A3), not a
  proven-final answer — it still awaits comparison against alternative
  configurations once the multi-domain validation suite provides
  sufficient ground truth.
- **RESOLVED by [B2-ID-005](../findings/B2-ID-005.md):** the `Imports`
  0% real-data capture (listed here, at the time of this evaluation's
  original writing, as a confirmed gap awaiting a future redesign) was
  formally root-caused (a granularity mismatch, not a fixable matching-
  logic bug) and fixed by Task 9.5's Priority 6 — the old
  `EdgeKind::Imports` was removed, replaced by a file-level `ImportFact`
  model wired into the real production pipeline. See the erratum above
  for the measured before/after.
