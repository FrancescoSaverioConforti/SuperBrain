# B3-TUNE-v2 Priority 1 — Abstention Thresholding Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task (Native execution, consistent with B3's own execution — implement in the current session, one independent review of the whole result at the end). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Measure which score-derived signal best discriminates
answerable from abstention-expected queries on the B3 corpus's dev split,
propose and freeze one global (not per-category) threshold from
calibration, and report a KEEP/REJECT ablation decision for "B1 +
abstention threshold" versus the official B1 baseline. A brand-new
held-out final-test set is built and frozen **before** that threshold
is chosen, by a separate, dedicated subagent whose context never
carries into this plan's own tuning work (Task 0) — this plan's own
session never reads that set's content, only its hash and non-semantic
metadata, and does not run it (that is a separate, later authorization,
same two-stop discipline B3.4/B3.5 used, and only after the P1→P2→P3→
(P4?) stop/proceed decision has already been made from dev/calibration/
ablation/resource evidence alone).

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
  adjusted after seeing tuning results.
- **The held-out set is authored by a separate, dedicated subagent
  (Task 0), not by whichever session/agent executes the rest of this
  plan.** That subagent's context — including the query text, answers,
  and `relevant_note_ids` it writes — never carries into this plan's own
  tuning tasks (Tasks 1-5). This plan's own execution only ever receives
  the held-out set's SHA-256, query count, category distribution, and
  difficulty distribution from `eval/rag/v2-heldout/manifest.json` — it
  must not read `eval/rag/v2-heldout/labels.jsonl`'s content at any
  point, and no task in this plan does.
- The held-out set is not run by this plan at all (not just "not yet
  chosen when to run" — literally out of this plan's scope). Running it
  is a separate, later authorization, after a final v2 configuration is
  chosen from dev/calibration/ablation evidence alone.
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
  threshold perform poorly. Task 1's analysis must report the full
  sorted value lists (or histograms), not just means, for the final
  signal choice to be defensible.
- A threshold chosen by eyeballing dev's own numbers, then verified only
  on the same dev data, producing a threshold that is really fit-to-dev
  rather than independently calibrated — Task 2's proposal must be
  evaluated against calibration, not re-justified against dev after the
  fact.
- The new held-out set's queries accidentally resembling existing
  dev/calibration queries closely enough that it isn't really
  independent evidence (e.g. reusing the same note as a near-duplicate
  target) — Task 0's dispatched subagent must check this against B3's
  dev and calibration splits the same way B3.1 checked for cross-split
  duplication. The consumed B3 test split (`eval/rag/test/labels.jsonl`)
  is explicitly NOT part of this check (user correction #1): it stays
  unread by v2 entirely, so no direct semantic de-duplication against it
  exists. This is an accepted gap, not an oversight — the stronger
  constraint (never reopen the consumed final-test) wins.
- The threshold-override logic silently applying to abstain-expected
  AND answerable rows differently than intended, or double-counting a
  query that was already `NotPresent` from `index::search` itself
  (should pass through unchanged, not get re-evaluated against the
  threshold a second time) — Task 3's tests must cover both an
  already-`NotPresent` row and a `Matches`-but-below-threshold row
  explicitly.
- Reporting the threshold's effect on Recall@10/nDCG@10 using only the
  aggregate number, hiding that the degradation might be concentrated in
  one or two categories — Task 4's report must include the by_category/
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
  1's signal comparison output.
- Create: `eval/results/B3-TUNE-v2/priority1-signal-analysis-calibration.json`,
  `priority1-threshold-sweep.json` — Task 2's calibration rerun and
  deterministic cutoff-by-cutoff sensitivity/Pareto table.
- Create: `eval/results/B3-TUNE-v2/priority1-dev-ablation.json`,
  `priority1-calibration-ablation.json` — the threshold-override ablation
  reports, same schema as B3's own reports plus the KEEP/REJECT fields.

---

### Task 0: Dispatch a separate subagent to build, commit, and hash the new v2 held-out final-test set

**This task is executed by dispatching a fresh subagent — never by the
session/agent that will execute Tasks 1-5.** This is the mechanism that
makes the held-out set genuinely blind during tuning (design doc
correction #2): the authoring agent's context, including the query
text, answers, and `relevant_note_ids` it writes, must not carry into
this plan's own tuning work. Whoever executes Task 0 does so via a
fresh dispatch (a new agent invocation with no memory of this plan's
later tasks) and does not bring that dispatch's full output back into
this plan's own conversation — only the four non-semantic facts listed
in "Produces" below are retained.

**Files (written by the dispatched subagent, not read back in full by this plan):**
- Create: `eval/rag/v2-heldout/labels.jsonl`
- Create: `eval/rag/v2-heldout/manifest.json`

**Interfaces:**
- Consumes: the existing B3 corpus at `eval/rag/corpus/notes/` (53
  notes, unchanged, reused as the document collection — same "one
  frozen corpus, split only at the query/labels level" design B3 itself
  used, per the design doc's correction that this is built before any
  tuning, not newly authored corpus content); the existing
  `eval/rag/{dev,calibration,test}/labels.jsonl` question text, read
  only to check for near-duplicates (the dispatched subagent needs this
  to do its job; this plan's own later tasks do not need it again).
- Produces, for this plan's own tuning tasks to receive back: **only**
  `eval/rag/v2-heldout/manifest.json`'s four non-semantic fields —
  `sha256`, `query_count`, `category_counts`, `difficulty_counts` (plus
  `build_date`/`build_commit` for traceability). Tasks 1-5 read this
  manifest file directly from disk when they need these facts; they
  never receive the dispatch's own conversation output, and never open
  `eval/rag/v2-heldout/labels.jsonl`.

- [ ] **Step 1: Dispatch the subagent with a self-contained brief**

Dispatch a fresh general-purpose agent (not a fork — a fork would
inherit this session's context, defeating the whole point) with a brief
that includes: the existing corpus's note list and content (same access
B3.1's authoring work had), the exact label schema
(`eval/rag/runner.py::load_labels`'s rules), the instruction to write
**approximately 60 new queries** stratified across the same 6
categories/3 difficulties B3 used, with **at least 15 abstention-
expected queries** (not ~17% of 60, which would be ~10 — the user's
correction #2 sets an explicit floor of 15 because Priority 1 is
specifically about abstention and needs enough abstention-expected
queries to validate a threshold meaningfully), all 6 categories and all
3 difficulties represented among both the answerable and abstention-
expected queries, and enough answerable queries remaining (~45) to keep
measuring Recall@10/nDCG@10 meaningfully. Each query independently
formulated (never a substring of its target note), and explicitly
checked for near-duplication against the existing
`eval/rag/{dev,calibration}/labels.jsonl` question text ONLY — **the
consumed B3 test split (`eval/rag/test/labels.jsonl`) must NOT be read
by this subagent at all** (user correction #1: that split is CONSUMED
per `docs/research/B3-CLOSURE-SUMMARY.md` and stays unread by v2
entirely; the brief must say this explicitly as a hard constraint, not
leave it to the subagent's judgment). The brief also instructs the
subagent to: validate with `eval/rag/validate_corpus.py` (making the
same small path-generalization fix this task's earlier draft identified,
if `validate_corpus.py` doesn't already accept an arbitrary
`eval_root`/corpus pair — it currently hardcodes
`SPLITS = ("dev", "calibration", "test")`); commit the result; compute
and write `eval/rag/v2-heldout/manifest.json` with `sha256` (of
`labels.jsonl`'s raw bytes), `build_date`, `build_commit` (git rev-parse
HEAD), `query_count` (~60), `category_counts`, `difficulty_counts`,
`abstain_count` (≥15) — metadata only, never copying
`question`/`answer`/`relevant_note_ids` content into the manifest; and
finally report back **only** a short confirmation (files committed,
manifest written, validation passed) — explicitly instructed not to
quote or summarize any query text, answer, or relevant_note_ids in its
final report back to this session.

- [ ] **Step 2: Read back only the manifest's non-semantic fields**

Once the subagent reports completion, this plan's own session reads
`eval/rag/v2-heldout/manifest.json` directly from disk (not the
subagent's conversational report) and confirms it contains valid
`sha256`/`query_count`/`category_counts`/`difficulty_counts` values.
**Do not open or read `eval/rag/v2-heldout/labels.jsonl` at this point,
or at any later point in this plan.**

- [ ] **Step 3: Confirm the commit exists and matches the manifest's hash**

Run: `sha256sum eval/rag/v2-heldout/labels.jsonl` and compare against
the manifest's own `sha256` field — this confirms the frozen hash
matches the real committed file, the same discipline B3's own freeze
step used, without requiring this session to read the file's content to
do so (hashing is not reading for meaning).

---

### Task 1: Signal analysis — which discriminator separates answerable from abstention best

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
  by Task 2 (human/report-driven threshold proposal — this task
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
frozen-adjacent logic except in Task 3.)

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
file more than Task 3 already requires), computes `QueryScores` for
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

### Task 2: Propose one global threshold from calibration via deterministic sweep — STOP for approval

**Files:**
- Create: `eval/results/B3-TUNE-v2/priority1-threshold-sweep.json`
- Create: `eval/results/B3-TUNE-v2/priority1-threshold-proposal.md`

**Interfaces:**
- Consumes: Task 1's signal analysis conclusion (which signal to
  threshold), the calibration split's real labels/search results (same
  `top_scores` function, rerun against `eval/rag/calibration/labels.jsonl`
  instead of dev), `metrics::recall_at_10`/`ndcg_at_10` (existing,
  unchanged) to score each candidate cutoff, and the official B1 baseline
  numbers in `docs/research/B3-CLOSURE-SUMMARY.md` for the delta column.
- Produces: a full cutoff-by-cutoff sensitivity/Pareto table
  (`priority1-threshold-sweep.json`) and a written proposal grounded in
  it (not a frozen config — this task does not freeze anything, and does
  not auto-select a "best" row) for the user's approval, per the
  two-stop discipline B3.4 used.

- [ ] **Step 1: Rerun the signal analysis binary against calibration**

Add a `--split calibration` argument to
`b3_tune_v2_signal_analysis.rs`'s `main` (defaulting to `dev` to keep
Task 1's existing invocation working unchanged), run it, and write a
second report,
`eval/results/B3-TUNE-v2/priority1-signal-analysis-calibration.json`.

- [ ] **Step 2: Deterministic cutoff sweep over calibration's own distribution**

Per the user's correction #3: do not eyeball a single cutoff or compute
one confusion matrix by hand. Add a `sweep_cutoffs` function (in
`b3_tune_v2_signal_analysis.rs`, or a small sibling module it calls)
that, given the winning signal's calibration values from Step 1's
report: builds the candidate cutoff set as every distinct observed
value of that signal on calibration (the only cutoffs that can change
any query's classification — values between two adjacent observations
all produce the same confusion matrix, so the distinct-value set is
already exhaustive over "sensible" cutoffs, not a sample of them), and
for each candidate cutoff computes: TP/TN/FP/FN (reusing
`metrics::confusion_matrix_metrics`'s tally logic against
`apply_abstention_threshold`-equivalent classification at that cutoff —
duplicate the small amount of logic needed here rather than depending on
Task 3's not-yet-implemented function, since this task runs before
Task 3), abstention precision/recall/F1 (`null` on undefined, same
semantics as `metrics.rs`), the false-abstention rate on answerable rows
(`FP / (FP + TN)`, i.e. the fraction of answerable queries this cutoff
would incorrectly flag as abstain), Recall@10 and nDCG@10 recomputed
under that cutoff's override (reusing `recall_at_10`/`ndcg_at_10` against
the overridden retrieval lists), and the delta of each versus the
official B1 baseline numbers in `docs/research/B3-CLOSURE-SUMMARY.md`.
Write every candidate's full row to
`eval/results/B3-TUNE-v2/priority1-threshold-sweep.json` as a sensitivity/
Pareto table (one row per distinct cutoff, sorted by cutoff value) —
this is the evidence Step 3 reads, not output for its own sake.

- [ ] **Step 3: Run the sweep and inspect the Pareto table**

Run: `cargo run --release -p api --example b3_tune_v2_signal_analysis -- --sweep calibration`
Expected: exits 0, writes `priority1-threshold-sweep.json`, prints its
path and the number of candidate cutoffs evaluated.

- [ ] **Step 4: Propose ONE global threshold with rationale — do not auto-select "best"**

Read the sweep table and propose a single cutoff value for the signal
Task 1 identified, with written rationale referencing specific rows
(e.g. "cutoff X gives abstention recall Y, false-abstention rate Z,
Recall@10 delta W — chosen over neighboring cutoffs because..."). This
is a judgment call informed by the full Pareto table, not a maximization
of any single column — the proposal must name the trade-off it accepts
(e.g. some false-abstention rate in exchange for non-zero abstention
recall) rather than claim one cutoff dominates on every metric. Write
this proposal, with the specific sweep rows it's grounded in, to
`eval/results/B3-TUNE-v2/priority1-threshold-proposal.md`. **This step
proposes; it does not select automatically and does not freeze anything**
— the threshold is not applied anywhere until the user approves this
specific proposal (Step 6).

- [ ] **Step 5: Commit the sweep and proposal documents**

```bash
git add eval/results/B3-TUNE-v2/priority1-signal-analysis-calibration.json eval/results/B3-TUNE-v2/priority1-threshold-sweep.json eval/results/B3-TUNE-v2/priority1-threshold-proposal.md
git commit -m "docs(eval): B3-TUNE-v2 Priority 1 deterministic threshold sweep and proposal from calibration"
```

- [ ] **Step 6: STOP and report the proposal to the user**

Per the design doc's two-stop discipline (same as B3.4): report the
proposed signal, threshold value, and rationale, plus the Pareto
sensitivity table's key rows; do not implement the threshold override in
the runner (Task 3) until the user approves this specific proposal. This
task's own deliverable is the sweep evidence plus the proposal, not a
working override and not an automatically-selected "best" cutoff.

---

### Task 3: Implement the threshold override as a runner option (only after Task 2's approval)

**Files:**
- Modify: `crates/api/src/metrics.rs`
- Modify: `crates/api/examples/b3_metric_runner.rs`

**Interfaces:**
- Consumes: Task 2's approved signal + threshold value.
- Produces: `pub fn apply_abstention_threshold(top1_score: f64, top2_score: Option<f64>, signal: AbstentionSignal, threshold: f64) -> bool`
  (returns `true` if the query should be overridden to `NotPresent`) in
  `metrics.rs`, where `AbstentionSignal` is a small enum
  (`Top1Score`, `Margin`, `Ratio`) matching Task 1's three candidates.
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

(Exact test cases depend on which signal Task 2 proposed; adapt the
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
Task 2's proposal specifies (below threshold → abstain).

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

### Task 4: Run the dev and calibration ablation, log per the experimental rule, propose KEEP/REJECT

**Files:**
- Create: `eval/results/B3-TUNE-v2/priority1-dev-ablation.json`
- Create: `eval/results/B3-TUNE-v2/priority1-calibration-ablation.json`
- Create: `eval/results/B3-TUNE-v2/priority1-ablation-summary.md`

**Interfaces:**
- Consumes: Task 3's `--abstention-threshold` flag.
- Produces: the two ablation report files (same schema as B3's own
  `dev-*.json`/`calibration-*.json` reports plus the
  `abstention_threshold` field) and a written summary comparing this
  ablation against the official B1 baseline, with an explicit KEEP/
  REJECT recommendation — reported to the user, not a unilateral
  decision.

- [ ] **Step 1: Run the runner against dev with the approved threshold**

Run: `cargo run --release -p api --example b3_metric_runner -- dev --abstention-threshold <signal>:<value>`
(using Task 2's approved signal/value). Expected: exits 0, writes a
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
rationale, and explicitly confirm: this plan's own session has still
never read the new v2 held-out final-test's content (Task 0 was
dispatched to a separate subagent precisely so this session couldn't).
Do not run it. Running it against this ablation (or any accepted/
rejected variant) is a separate, later authorization — same two-stop
discipline as B3.5 — and is NOT part of this plan's own scope. It is
also not how the P1→P2→P3→(P4?) stop/proceed decision gets made at all
(design doc correction: that decision comes from dev/calibration/
ablation/resource evidence alone, never from the held-out set).

---

## What this plan stops short of

This plan ends at Task 4's KEEP/REJECT proposal for Priority 1's own
ablation. It does **not** include:
- Running the new v2 held-out final-test (Task 0 only dispatches a
  separate subagent to build and freeze it; this plan's own session
  never reads its content).
- Using the new held-out set's result to decide whether to proceed to
  Priority 2/3/4 — that decision, when it comes, is made from dev/
  calibration/ablation/resource evidence alone (design doc correction).
- Priority 2 (lexical tuning), Priority 3 (reranking), or Priority 4
  (hybrid retrieval) — each remains design-only until separately
  planned and authorized.
- Declaring any v2 configuration the new official baseline — that
  requires the full multi-priority ablation chain and the new held-out
  final-test run, neither of which this plan performs.

## Self-Review Notes

- **Spec coverage**: the design doc's 7 corrections plus the 3 plan-level
  corrections applied directly to this plan (never-reopen-the-consumed-
  B3-test-split in Task 0 Step 1; the ~60-query/≥15-abstention held-out
  size floor in Task 0 Step 1; the deterministic cutoff sweep replacing
  hand-picked thresholds in Task 2 Steps 2-4) map directly to this plan's
  tasks — held-out-first ordering (Task 0), held-out/tuning context
  separation via subagent dispatch (Task 0 itself), held-out never
  gates the P1-P4 stop/proceed decision (explicit in Task 4 Step 5 and
  "What this plan stops short of"), multi-signal measurement before
  tuning (Task 1), global-only threshold (Task 2's single proposed
  value, no per-category logic anywhere), the multi-objective
  trade-off (Task 4's KEEP/REJECT criterion), and the experimental
  logging rule (Task 4 Step 3's explicit field list). Priority 1's own
  two-stop points (Task 2 before implementing, Task 4 before any
  future held-out run) mirror B3.4/B3.5's discipline.
- **Step scan**: Task 1 Step 5 and Task 1 Step 7 both flag real
  uncertainty (whether code can be shared between the two example
  binaries without a refactor; that the "which signal wins" conclusion
  is a judgment call, not an automated decision) and give the
  implementer an explicit, safe default (duplicate rather than
  refactor; write the conclusion as data, not logic) rather than
  assuming either answer.
- **Type consistency**: `QueryScores` is defined once (Task 1);
  `AbstentionSignal` and `apply_abstention_threshold`'s signature are
  defined once (Task 3) and referenced consistently afterward;
  `run_one_query`'s extension to return scores (Task 3 Step 5) is the
  one existing-type change this plan makes, and it's scoped to exactly
  that task.
- **Review Focus**: each of the 5 named risks has an owning
  task/step — full-distribution signal comparison (Task 1 Step 5/7),
  calibration-not-dev threshold verification (Task 2 Step 2's explicit
  "never re-deriving from dev a second time"), held-out/existing-split
  duplication check (Task 0 Step 1, inside the dispatched subagent's
  own brief), the already-NotPresent passthrough and no-second-candidate
  edge cases (Task 3 Step 1's three test cases), and the by_category/
  by_difficulty breakdown requirement (Task 4 Step 3, via the reused B3
  report schema which already includes these breakdowns).
- **Proportion**: 6 tasks (one of them a subagent dispatch, not code),
  mostly small Rust additions (a diagnostic binary, one pure function,
  one CLI flag) plus two measurement/analysis steps that are inherently
  judgment calls rather than code — proportionate to a plan whose main
  risk is measurement/statistical rigor and context isolation
  discipline, not code volume.
