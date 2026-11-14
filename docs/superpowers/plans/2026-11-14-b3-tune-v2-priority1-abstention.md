# B3-TUNE-v2 Priority 1 — Abstention Thresholding Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task (Native execution, consistent with B3's own execution — implement in the current session, one independent review of the whole result at the end). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Measure which score-derived signal best discriminates
answerable from abstention-expected queries on the B3 corpus's dev split,
propose and freeze one global (not per-category) threshold from
calibration, build and freeze a brand-new held-out final-test set
**before** that threshold is chosen, and report a KEEP/REJECT ablation
decision for "B1 + abstention threshold" versus the official B1 baseline
— without running the new final-test yet (that is this plan's own final
authorized step, same two-stop discipline B3.4/B3.5 used).

**Architecture:** A new diagnostic Rust binary reads `SearchResult::score`
(already present in `index::search`, no production code change needed)
for the top-1 and top-2 results of every dev-split query, computing the
three candidate signals (top1 score, top1−top2 margin, top1/top2 ratio)
per query, and reports which signal separates answerable from
abstention-expected queries most cleanly on dev. A second small addition
to `crates/api/examples/b3_metric_runner.rs` (or a sibling binary) adds
an optional abstention-threshold parameter: if the winning signal falls
below the threshold, the runner reports the query as `NotPresent`
regardless of what `index::search` actually returned, and recomputes the
abstention confusion matrix and Recall@10/nDCG@10 under that override —
this is purely a reporting-layer decision, `index::search`'s own
production behavior is never changed.

**Tech Stack:** Rust (existing `api`/`index` crates), the existing B3
corpus/dev/calibration label files, a new held-out query/label set built
in this plan's own early tasks.

**Spec:** `docs/superpowers/specs/2026-11-14-b3-tune-v2-design.md`
(Priority 1 section + the 5 corrections; read in full before starting).
Builds on the accepted B1 baseline recorded in
`eval/results/B3/frozen-config.json` (commit `52d42ea`) and
`docs/research/B3-CLOSURE-SUMMARY.md`.

## Global Constraints

- `index::search`/`index::Index`'s production behavior is never changed.
  Abstention thresholding is applied only in the diagnostic/runner
  layer, as a post-hoc override of the reported outcome for measurement
  purposes — this plan does not touch `crates/index` or `crates/storage`.
- No per-category threshold. One single global threshold is proposed and
  (if accepted) frozen. Per-category thresholds are explicitly out of
  scope for this plan (design doc correction #2).
- The new held-out final-test set is built, committed, and hashed
  **before** any threshold is chosen from calibration — not designed or
  adjusted after seeing tuning results. Its content is not read again
  after this plan's build step until this plan's own final authorized
  run (if and when separately authorized — see "What this plan stops
  short of" below).
- B3's existing `dev`/`calibration`/`test` splits and `eval/results/B3/`
  reports are read-only inputs to this plan. Nothing in `eval/rag/` or
  `eval/results/B3/` from the B3 phase is modified.
- Every variant tested is logged per the design doc's experimental
  logging rule: full config, commit, corpus/labels hashes, Recall@10,
  nDCG@10, abstention confusion matrix + derived metrics, latency/
  resource delta if available (or explicitly "not measured"), delta vs
  the official B1 baseline, and an explicit KEEP/REJECT decision.
- No MCP, no merge/tag/push, no Priority 2/3/4 work.

## Review Focus

- A signal comparison that looks at aggregate separation only (e.g. "the
  mean margin for answerable queries is higher") without checking the
  actual distributions overlap — with only ~17 abstention-expected
  queries across dev+calibration, a mean-only comparison could look
  decisive while hiding heavy overlap that would make any single
  threshold perform poorly. Task 2's analysis must report the full
  sorted value lists (or histograms), not just means, for the final
  signal choice to be defensible.
- A threshold chosen by eyeballing dev's own numbers, then verified only
  on the same dev data, producing a threshold that is really fit-to-dev
  rather than independently calibrated — Task 3's proposal must be
  evaluated against calibration, not re-justified against dev after the
  fact.
- The new held-out set's queries accidentally resembling the already-
  consumed B3 test split's queries closely enough that it isn't really
  independent evidence (e.g. reusing the same note as a near-duplicate
  target) — Task 1's construction must check this against B3's own
  corpus the same way B3.1 checked for cross-split duplication.
- The threshold-override logic silently applying to abstain-expected
  AND answerable rows differently than intended, or double-counting a
  query that was already `NotPresent` from `index::search` itself
  (should pass through unchanged, not get re-evaluated against the
  threshold a second time) — Task 4's tests must cover both an
  already-`NotPresent` row and a `Matches`-but-below-threshold row
  explicitly.
- Reporting the threshold's effect on Recall@10/nDCG@10 using only the
  aggregate number, hiding that the degradation might be concentrated in
  one or two categories — Task 5's report must include the by_category/
  by_difficulty breakdown under the threshold override, same as B3's
  own reports did, not just the aggregate delta.

---

## File Structure

- Create: `crates/api/examples/b3_tune_v2_signal_analysis.rs` — reads
  top-1/top-2 scores per dev-split query, reports the three candidate
  signals and their separation between answerable and abstention-
  expected queries. Diagnostic only; not part of the frozen pipeline.
- Create: `eval/rag/v2-heldout/` — the new held-out query/label set's
  home, mirroring `eval/rag/{dev,calibration,test}/`'s existing
  structure and label schema exactly (same `labels.jsonl` format, same
  validation rules).
- Create: `eval/rag/v2-heldout/manifest.json` — records the new held-out
  set's own hash, build date, build commit, query count, and category/
  difficulty distribution (metadata only — never the query/label
  content itself, so this file is safe to read at any time without
  contaminating the held-out set).
- Modify: `crates/api/src/metrics.rs` — add a pure function applying a
  threshold override to an already-computed per-query score, used by
  both the signal analysis binary and (later, if accepted) the runner.
- Modify: `crates/api/examples/b3_metric_runner.rs` — add an optional
  `--abstention-threshold <signal>:<value>` CLI argument that, when
  present, overrides `NotPresent`/`Matches` classification per query
  before scoring, and includes the threshold's own value and signal
  name in the report for traceability.
- Create: `eval/results/B3-TUNE-v2/priority1-signal-analysis.json` — Task
  2's signal comparison output.
- Create: `eval/results/B3-TUNE-v2/priority1-dev-ablation.json`,
  `priority1-calibration-ablation.json` — the threshold-override ablation
  reports, same schema as B3's own reports plus the KEEP/REJECT fields.

---

### Task 1: Build, commit, and hash the new v2 held-out final-test set

**Files:**
- Create: `eval/rag/v2-heldout/labels.jsonl`
- Create: `eval/rag/v2-heldout/manifest.json`

**Interfaces:**
- Consumes: the existing B3 corpus at `eval/rag/corpus/notes/` (53
  notes, unchanged, reused as the document collection — same "one
  frozen corpus, split only at the query/labels level" design B3 itself
  used, per the design doc's correction that this is built before any
  tuning, not newly authored corpus content).
- Produces: `eval/rag/v2-heldout/labels.jsonl` in the exact schema
  `eval/rag/runner.py::load_labels` already validates (category,
  difficulty, abstain_expected, relevant_note_ids, graded_relevance,
  citation_spans — identical rules), and
  `eval/rag/v2-heldout/manifest.json` recording its SHA-256, build date,
  build commit, and query count — consumed by Task 3 (threshold
  proposal, metadata only) and this plan's own final step (content read
  only once, if and when authorized).

- [ ] **Step 1: Author new queries against the existing 53-note corpus**

Write ~20-30 new queries (a smaller held-out set is acceptable here —
this validates one threshold choice, not a full corpus re-benchmark),
stratified across the same 6 categories and 3 difficulties B3 used,
including several abstention-expected queries (proportionally similar
to B3's ~17%). Each query must be independently formulated (never a
substring of its target note, same B3.1 rule), and none may be a
near-duplicate of any question already in `eval/rag/{dev,calibration,test}/labels.jsonl`
(check this explicitly against all three existing files' `question`
fields before finalizing — the Review Focus risk above).

- [ ] **Step 2: Write `eval/rag/v2-heldout/labels.jsonl` in the existing schema**

Full schema per B3.2's extended `load_labels` rules: `id`, `question`,
`answer`, `category`, `difficulty`, `abstain_expected`,
`relevant_note_ids`, `graded_relevance`, `citation_spans`. Validate with
the existing `eval/rag/validate_corpus.py` (it already works against
any `eval_root`/corpus pair — point it at this new directory) before
committing.

Run: `python -c "import sys; sys.path.insert(0, 'eval/rag'); from validate_corpus import validate_corpus; from pathlib import Path; print(validate_corpus(Path('eval/rag/corpus'), Path('eval/rag/v2-heldout').parent / 'v2-heldout'))"` —
adjust the path arguments to point `validate_corpus` at the new
directory as its `eval_root`, confirming `[]` (no problems) before
proceeding. If `validate_corpus` needs a small path-handling adjustment
to accept a directory outside `eval/rag/`'s existing three split names,
make that adjustment now (it currently hardcodes `SPLITS = ("dev",
"calibration", "test")` — this will need generalizing to accept an
arbitrary directory, a small, mechanical change, not a design change).

- [ ] **Step 3: Compute and record the manifest**

Write `eval/rag/v2-heldout/manifest.json` with: `sha256` (of
`labels.jsonl`'s raw bytes), `build_date`, `build_commit` (git rev-parse
HEAD), `query_count`, `category_counts`, `difficulty_counts`,
`abstain_count`. This is metadata only — never copy any `question`,
`answer`, or `relevant_note_ids` content into this file, so it remains
safe to read at any point without touching the held-out content itself.

- [ ] **Step 4: Commit**

```bash
git add eval/rag/v2-heldout/
git commit -m "feat(eval): build and freeze the v2 held-out final-test set, unread"
```

---

### Task 2: Signal analysis — which discriminator separates answerable from abstention best

**Files:**
- Create: `crates/api/examples/b3_tune_v2_signal_analysis.rs`
- Create: `eval/results/B3-TUNE-v2/priority1-signal-analysis.json`

**Interfaces:**
- Consumes: `api::search`, `index::search::{SearchOutcome, SearchResult}`
  (existing, unchanged — reads `SearchResult::score` for `first` and
  `rest.first()`), the dev split's real labels
  (`eval/rag/dev/labels.jsonl`), the same vault/corpus construction
  pattern `crates/api/examples/b3_metric_runner.rs`'s `TestFixture`
  already uses.
- Produces: `eval/results/B3-TUNE-v2/priority1-signal-analysis.json`
  with, per dev-split query: `id`, `abstain_expected`, `top1_score`,
  `top2_score` (nullable if no second result), `margin` (`top1 - top2`,
  nullable), `ratio` (`top1 / top2`, nullable if `top2` is `None` or
  `0.0`); plus a summary block per signal with the full sorted value
  lists for `abstain_expected: true` vs `false` groups (not just means —
  Review Focus risk above) and a simple separation measure (e.g. the gap
  between the highest abstain-expected value and the lowest answerable
  value, or their rank overlap) for each of the three signals. Consumed
  by Task 3 (human/report-driven threshold proposal — this task
  produces evidence, not an automated threshold choice).

- [ ] **Step 1: Write the failing test for score extraction**

```rust
#[test]
fn extracts_top1_and_top2_scores_from_a_real_matches_outcome() {
    let fixture = TestFixture::new(); // reuse b3_metric_runner.rs's pattern
    fixture.commit_corpus();
    let idx = fixture.build_index();
    let scores = top_scores(&idx, "resolve_configured_timezone", 10);
    assert!(scores.top1_score > 0.0);
}
```

(If `TestFixture` and its construction helpers are not already
reusable across the two example binaries, duplicate the minimal
fixture-building code into this new file rather than refactoring
`b3_metric_runner.rs` — this plan does not touch that file's own
frozen-adjacent logic except in Task 4.)

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p api --example b3_tune_v2_signal_analysis`
Expected: FAIL (`top_scores` not yet defined).

- [ ] **Step 3: Implement `top_scores` and the three signal computations**

`struct QueryScores { top1_score: f64, top2_score: Option<f64>, margin: Option<f64>, ratio: Option<f64> }`.
`top_scores(idx: &Index, question: &str, k: usize) -> QueryScores` calls
`api::search`, and on `SearchOutcome::Matches { first, rest }` sets
`top1_score = first.score`, `top2_score = rest.first().map(|r| r.score)`,
`margin = top2_score.map(|t2| first.score - t2)`,
`ratio = top2_score.filter(|t2| *t2 != 0.0).map(|t2| first.score / t2)`.
On `NotPresent`, this function isn't meaningful (no candidate exists);
the caller (Step 5's `main`) skips `NotPresent` rows in the signal
analysis — they're a separate, already-distinguishable case (B1 already
correctly says "no match" there; the interesting case Priority 1 is
analyzing is the "has a low-quality match" case, i.e. `Matches` with a
low score).

- [ ] **Step 4: Run the test to verify it passes**

Run the same command as Step 2. Expected: PASS.

- [ ] **Step 5: Implement `main` to run signal analysis over the full dev split and write the report**

Loads `eval/rag/dev/labels.jsonl` (reusing `b3_metric_runner.rs`'s
`parse_label_rows`/`exclude_canary`/`validate_label_rows` — either by
duplicating the small amount of parsing code, or, if it's already
`pub(crate)` and the two files are both in the same crate's
`examples/`, confirm whether Rust's example-binary visibility allows
sharing code between them without a `src/`-level refactor; if not,
duplicate rather than refactor, to avoid touching the frozen runner's
file more than Task 4 already requires), computes `QueryScores` for
every non-canary row via `run_one_query`-equivalent search calls, splits
by `abstain_expected`, and writes the full sorted value lists and
separation summary described in the Interfaces block to
`eval/results/B3-TUNE-v2/priority1-signal-analysis.json`.

- [ ] **Step 6: Run it against the real dev split**

Run: `cargo run --release -p api --example b3_tune_v2_signal_analysis`
Expected: exits 0, writes the report, prints its path.

- [ ] **Step 7: Read the report and determine which signal separates best**

This is an analysis step, not a coding step: read the written JSON,
compare the three signals' separation (sorted value lists, not just a
single summary number — Review Focus risk above), and record which
signal (top1 score, margin, or ratio) shows the cleanest separation
between answerable and abstention-expected queries on dev. Write this
conclusion as a short paragraph appended to the report file's own
`"analysis_conclusion"` field (re-run Step 6's script with a
`--conclusion "<text>"` flag, or simply hand-edit the committed JSON
file once — either is fine, this is data, not executable logic).

- [ ] **Step 8: Commit**

```bash
git add crates/api/examples/b3_tune_v2_signal_analysis.rs eval/results/B3-TUNE-v2/priority1-signal-analysis.json
git commit -m "feat(eval): B3-TUNE-v2 Priority 1 signal analysis (top1 score, margin, ratio)"
```

---

### Task 3: Propose one global threshold from calibration — STOP for approval

**Files:**
- Create: `eval/results/B3-TUNE-v2/priority1-threshold-proposal.md`

**Interfaces:**
- Consumes: Task 2's signal analysis conclusion (which signal to
  threshold), the calibration split's real labels/search results (same
  `top_scores` function, rerun against `eval/rag/calibration/labels.jsonl`
  instead of dev).
- Produces: a written proposal (not a frozen config — this task does
  not freeze anything) for the user's approval, per the two-stop
  discipline B3.4 used.

- [ ] **Step 1: Rerun the signal analysis binary against calibration**

Add a `--split calibration` argument to
`b3_tune_v2_signal_analysis.rs`'s `main` (defaulting to `dev` to keep
Task 2's existing invocation working unchanged), run it, and write a
second report,
`eval/results/B3-TUNE-v2/priority1-signal-analysis-calibration.json`.

- [ ] **Step 2: Propose one global threshold value for the winning signal**

Using calibration's own distribution (never re-deriving the threshold
from dev a second time — dev already did its diagnostic job in Task 2),
propose a single cutoff value for the signal Task 2 identified, chosen
so it would correctly flag calibration's own abstention-expected queries
as `NotPresent` while flagging as few answerable queries as possible as
false negatives. Write this proposal, with its rationale and the
resulting hypothetical confusion-matrix shift on calibration (computed
by hand from the signal analysis report's own per-query values — no new
code needed for this arithmetic), to
`eval/results/B3-TUNE-v2/priority1-threshold-proposal.md`.

- [ ] **Step 3: Commit the proposal document**

```bash
git add eval/results/B3-TUNE-v2/priority1-signal-analysis-calibration.json eval/results/B3-TUNE-v2/priority1-threshold-proposal.md
git commit -m "docs(eval): B3-TUNE-v2 Priority 1 threshold proposal from calibration"
```

- [ ] **Step 4: STOP and report the proposal to the user**

Per the design doc's two-stop discipline (same as B3.4): report the
proposed signal, threshold value, and rationale; do not implement the
threshold override in the runner (Task 4) until the user approves this
specific proposal. This task's own deliverable is the proposal, not a
working override.

---

### Task 4: Implement the threshold override as a runner option (only after Task 3's approval)

**Files:**
- Modify: `crates/api/src/metrics.rs`
- Modify: `crates/api/examples/b3_metric_runner.rs`

**Interfaces:**
- Consumes: Task 3's approved signal + threshold value.
- Produces: `pub fn apply_abstention_threshold(top1_score: f64, top2_score: Option<f64>, signal: AbstentionSignal, threshold: f64) -> bool`
  (returns `true` if the query should be overridden to `NotPresent`) in
  `metrics.rs`, where `AbstentionSignal` is a small enum
  (`Top1Score`, `Margin`, `Ratio`) matching Task 2's three candidates.
  `b3_metric_runner.rs` gains a `--abstention-threshold <signal>:<value>`
  CLI argument (e.g. `--abstention-threshold margin:0.42`) that, when
  present, applies this function to every `Matches` outcome's top-1/
  top-2 scores before scoring — an already-`NotPresent` row from
  `index::search` passes through unchanged (Review Focus risk above).

- [ ] **Step 1: Write the failing test for the threshold function**

```rust
#[test]
fn apply_abstention_threshold_flags_a_low_margin_query() {
    assert!(apply_abstention_threshold(0.5, Some(0.48), AbstentionSignal::Margin, 0.1));
}

#[test]
fn apply_abstention_threshold_does_not_flag_a_high_margin_query() {
    assert!(!apply_abstention_threshold(2.0, Some(0.1), AbstentionSignal::Margin, 0.1));
}

#[test]
fn apply_abstention_threshold_never_flags_when_top2_is_absent() {
    // No second candidate at all: margin/ratio are undefined, never
    // trigger an override regardless of top1_score or threshold.
    assert!(!apply_abstention_threshold(0.01, None, AbstentionSignal::Margin, 0.5));
    assert!(!apply_abstention_threshold(0.01, None, AbstentionSignal::Ratio, 0.5));
}
```

(Exact test cases depend on which signal Task 3 proposed; adapt the
test names/values to the actually-approved signal, keeping the
three distinct cases: triggers, doesn't trigger, and the no-second-
candidate edge case.)

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p api metrics::apply_abstention_threshold`
Expected: FAIL (not yet implemented).

- [ ] **Step 3: Implement `AbstentionSignal` and `apply_abstention_threshold`**

Straightforward per-signal match: `Top1Score` compares `top1_score`
directly; `Margin`/`Ratio` require `top2_score` to be `Some` (return
`false` — never override — if it's `None`, per the test above); compute
the signal value and compare against `threshold` with the direction
Task 3's proposal specifies (below threshold → abstain).

- [ ] **Step 4: Run the tests to verify they pass**

Run the same command as Step 2. Expected: PASS.

- [ ] **Step 5: Wire the CLI argument into `b3_metric_runner.rs`**

Parse an optional `--abstention-threshold <signal>:<value>` argument in
`main` (alongside the existing split argument and
`FINAL_TEST_AUTHORIZATION_FLAG` handling — this argument is independent
of that one and must work for `dev`/`calibration` without touching the
test-split gate at all). When present, pass it through to `run_split`
(a new optional parameter), which applies
`apply_abstention_threshold` to every `Matches` outcome before scoring,
using the row's actual `top1_score`/`top2_score` (requires `run_one_query`
to also return the scores, not just the note ids — extend `QueryOutcome::Matches`
with the score fields, or return them as a sibling value; keep
`metrics.rs`'s existing `recall_at_10`/`hit_at_10`/`ndcg_at_10` functions
completely unchanged, since they only need note ids, not scores).
Include the threshold's signal name and value in the report's existing
JSON structure (a new top-level field, e.g. `"abstention_threshold":
{"signal": "margin", "value": 0.42}` or `null` when not used).

- [ ] **Step 6: Run the full example test suite to verify no regression**

Run: `cargo test -p api --example b3_metric_runner`
Expected: PASS, same count as before plus any new tests from this task.

- [ ] **Step 7: Commit**

```bash
git add crates/api/src/metrics.rs crates/api/examples/b3_metric_runner.rs
git commit -m "feat(api): abstention threshold override as an optional runner parameter"
```

---

### Task 5: Run the dev and calibration ablation, log per the experimental rule, propose KEEP/REJECT

**Files:**
- Create: `eval/results/B3-TUNE-v2/priority1-dev-ablation.json`
- Create: `eval/results/B3-TUNE-v2/priority1-calibration-ablation.json`
- Create: `eval/results/B3-TUNE-v2/priority1-ablation-summary.md`

**Interfaces:**
- Consumes: Task 4's `--abstention-threshold` flag.
- Produces: the two ablation report files (same schema as B3's own
  `dev-*.json`/`calibration-*.json` reports plus the
  `abstention_threshold` field) and a written summary comparing this
  ablation against the official B1 baseline, with an explicit KEEP/
  REJECT recommendation — reported to the user, not a unilateral
  decision.

- [ ] **Step 1: Run the runner against dev with the approved threshold**

Run: `cargo run --release -p api --example b3_metric_runner -- dev --abstention-threshold <signal>:<value>`
(using Task 3's approved signal/value). Expected: exits 0, writes a
report with `abstention_threshold` populated.

- [ ] **Step 2: Run the runner against calibration with the same threshold**

Run: `cargo run --release -p api --example b3_metric_runner -- calibration --abstention-threshold <signal>:<value>`
Expected: same, calibration-split report.

- [ ] **Step 3: Write the ablation summary per the experimental logging rule**

For each of dev and calibration, record (per the design doc's logging
rule): the full config (B1 + this specific threshold, named explicitly),
commit, corpus/labels hashes (already in each report), Recall@10 (with
CI), nDCG@10 (with CI), the abstention confusion matrix and derived
metrics, latency/resource delta (write "not measured" — this plan does
not add latency instrumentation), and the delta versus the **official**
B1 baseline numbers recorded in `docs/research/B3-CLOSURE-SUMMARY.md`
(not just versus an unthresholded rerun of this same commit). Conclude
with an explicit KEEP/REJECT recommendation for Priority 1's own
ablation, checked against the multi-objective acceptance criterion in
the design doc (abstention recall > 0, abstention precision
sufficiently high, Recall@10/nDCG@10 within tolerance — propose a
tolerance value here if one hasn't already been discussed, consistent
with the design doc's "proposed after dev/calibration measurement"
framing).

- [ ] **Step 4: Commit**

```bash
git add eval/results/B3-TUNE-v2/priority1-dev-ablation.json eval/results/B3-TUNE-v2/priority1-calibration-ablation.json eval/results/B3-TUNE-v2/priority1-ablation-summary.md
git commit -m "docs(eval): B3-TUNE-v2 Priority 1 dev/calibration ablation and KEEP/REJECT proposal"
```

- [ ] **Step 5: STOP and report to the user**

Report the ablation results, the KEEP/REJECT recommendation and its
rationale, and explicitly confirm: the new v2 held-out final-test
(Task 1) has still not been read for its content. Do not run it. Running
it against this ablation (or any accepted/rejected variant) is a
separate, later authorization — same two-stop discipline as B3.5 — and
is NOT part of this plan's own scope.

---

## What this plan stops short of

This plan ends at Task 5's KEEP/REJECT proposal for Priority 1's own
ablation. It does **not** include:
- Running the new v2 held-out final-test (Task 1 only builds and
  freezes it).
- Priority 2 (lexical tuning), Priority 3 (reranking), or Priority 4
  (hybrid retrieval) — each remains design-only until separately
  planned and authorized.
- Declaring any v2 configuration the new official baseline — that
  requires the full multi-priority ablation chain and the new held-out
  final-test run, neither of which this plan performs.

## Self-Review Notes

- **Spec coverage**: the design doc's 5 corrections map directly to
  this plan's tasks — held-out-first ordering (Task 1), multi-signal
  measurement before tuning (Task 2), global-only threshold (Task 3's
  single proposed value, no per-category logic anywhere), the
  multi-objective trade-off (Task 5's KEEP/REJECT criterion), and the
  experimental logging rule (Task 5 Step 3's explicit field list).
  Priority 1's own two-stop points (Task 3 before implementing, Task 5
  before running the new held-out set) mirror B3.4/B3.5's discipline.
- **Step scan**: Task 2 Step 5 and Task 2 Step 7 both flag real
  uncertainty (whether code can be shared between the two example
  binaries without a refactor; that the "which signal wins" conclusion
  is a judgment call, not an automated decision) and give the
  implementer an explicit, safe default (duplicate rather than
  refactor; write the conclusion as data, not logic) rather than
  assuming either answer.
- **Type consistency**: `QueryScores`, `AbstentionSignal`, and
  `apply_abstention_threshold`'s signature are each defined once (Task
  2, Task 4) and referenced consistently afterward; `run_one_query`'s
  extension to return scores (Task 4 Step 5) is the one existing-type
  change this plan makes, and it's scoped to exactly that task.
- **Review Focus**: each of the 5 named risks has an owning
  task/step — full-distribution signal comparison (Task 2 Step 5/7),
  calibration-not-dev threshold verification (Task 3 Step 2's explicit
  "never re-deriving from dev a second time"), held-out/existing-split
  duplication check (Task 1 Step 1), the already-NotPresent passthrough
  and no-second-candidate edge cases (Task 4 Step 1's three test cases),
  and the by_category/by_difficulty breakdown requirement (Task 5 Step
  3, via the reused B3 report schema which already includes these
  breakdowns).
- **Proportion**: 5 tasks, mostly small Rust additions (a diagnostic
  binary, one pure function, one CLI flag) plus two measurement/
  analysis steps that are inherently judgment calls rather than code —
  proportionate to a plan whose main risk is measurement/statistical
  rigor, not code volume.
