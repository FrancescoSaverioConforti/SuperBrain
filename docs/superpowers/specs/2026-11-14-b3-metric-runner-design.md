# B3.3/B3.4 — Metric Runner & Calibration: Design

Status: **approved with changes** (see continuity `d202zz`). This
revision incorporates the 6 earlier details plus 3 methodological
corrections from that approval. Nothing built yet.

## Intent and why

B3.1/B3.2 (continuity `m4ku`) produced a frozen corpus, 120 real queries
split dev/calibration/test, and a label schema supporting graded
relevance, abstention, and citation spans. B3.3/B3.4 are now authorized
(continuity `d199pg`) with explicit constraints. This doc covers:

- **B3.3** — a metric runner that calls the real `index::search` /
  `api::search` retrieval path and computes Recall@10, nDCG@10,
  confidence intervals, and category/difficulty/abstention breakdowns.
- **B3.4** — running that runner on dev (development/diagnosis) and
  calibration (where thresholds get chosen and frozen), then freezing a
  configuration before any final-test run.

B3.5 (the actual final-test execution) is explicitly **not** authorized
yet and is not covered by this doc or its plan.

## Architecture (per your decision)

A native Rust binary, not a Python script — the runner must call the
real `index::search` path directly, with no Python↔Rust bridge, so there
is exactly one retrieval implementation. Per your requirement, retrieval
logic is never duplicated in the runner: it calls `api::search` (the same
function `crates/api/src/search.rs:14` exposes to any caller) against a
real `Index` built from a real `Vault`, the same construction pattern
`crates/api/tests/b1_api.rs`'s `Fixture`/`held_out_labels` already use and
that this branch's B3.1/B3.2 work already validated for isolation.

Metrics computation (Recall@10, nDCG@10, CI, breakdowns) lives in
separate, unit-testable Rust modules/functions — pure functions over
`Vec<SearchResult>` plus a label row, with no vault/search dependency —
so they can be tested with synthetic inputs independent of the slow
vault-construction path, per your "logic can live in testable
helpers separate from the part that builds the vault" instruction.

Python (`eval/rag/*.py`) stays exactly as it is — label loading,
`validate_corpus`, `build_manifest` — the runner only *reads* the same
`labels.jsonl` files via a minimal Rust-side JSON Lines parser (no new
Python dependency, no reuse of `runner.py` from Rust).

## Where it lives

```
crates/api/
  src/
    metrics.rs          NEW: pure Recall@10/nDCG@10/CI functions, unit-tested
  examples/
    b3_metric_runner.rs NEW: builds vault, loads labels, calls search,
                         computes metrics via metrics.rs, writes report
eval/
  results/
    B3/
      dev-<date>.json           dev-split run report
      calibration-<date>.json   calibration-split run report
      frozen-config.json        NEW (B3.4 only): the immutable record
                                 your constraint #6 requires, written
                                 once calibration thresholds are chosen
```

Rationale for `src/metrics.rs` + `examples/b3_metric_runner.rs` over a
`tests/` integration test: this runner produces artifacts you read
(JSON reports), not a pass/fail assertion — `cargo run --release
--example b3_metric_runner -- <split>` is the right shape, mirroring the
existing B2 pattern (`crates/codegraph-index/examples/b2_eval_*.rs`) for
exactly this kind of "run and report" tool, not `cargo test`'s pass/fail
contract.

## Label row interpretation

Every label row (per the schema B3.2 built) maps to one of three cases
the runner must handle distinctly, per your abstention-metrics
requirement:

1. **Canary row** (`id: canary-isolation-probe`, the one mechanical
   fixture row): excluded entirely from every metric, by id, per your
   constraint #1. Not loaded into the metrics computation at all — the
   runner filters it out immediately after loading each split's rows,
   before any metric touches it.
2. **Abstention query** (`abstain_expected: true`): not scored for
   Recall@10/nDCG@10 (it has no `relevant_note_ids` to recall). Scored
   separately as an abstention outcome: correct if `index::search`
   returns `SearchOutcome::NotPresent`, incorrect (a false "evidence
   exists" the spec calls out) if it returns `Matches`.
3. **Answerable query** (`abstain_expected: false`): scored for
   Recall@10 and nDCG@10 against its `relevant_note_ids`/
   `graded_relevance`. Also checked for a different abstention failure
   mode — a **false NotPresent** (the query is answerable but search
   returns `NotPresent`) — counted in the same abstention metrics block,
   per the spec's explicit requirement that abstention be measured
   "insieme a recall" (together with recall), never satisfied by
   returning zero results for everything.

## Metrics computed (per your constraint #2)

For each split (dev, calibration — never test, per constraint #3):

- **Recall@10** (corrected — the first draft computed Hit@10 and mislabeled
  it): per query,
  `Recall@10 = |relevant_note_ids ∩ top_10_note_ids| / |relevant_note_ids|`
  — the true fraction of that query's relevant notes retrieved in the
  top 10, not a 0/1 hit indicator. For the ~97 single-relevant-note
  queries this is still 0 or 1 (nothing changes there), but the 3
  two-relevant-note queries (q007, q081, q094) can now score exactly
  0.5 if only one of their two relevant notes is retrieved — the case
  the first draft would have silently scored as a full 1.0. "Top 10" is
  `first` plus the first 9 of `rest` from `SearchOutcome::Matches`,
  already rank-ordered by `index::search`. Aggregate Recall@10 is the
  mean of per-query Recall@10 over answerable queries.
- **Hit@10** (new, optional diagnostic only, per your note that it may
  be added alongside the real metric): 1 if any `relevant_note_ids`
  entry appears in the top 10, else 0 — kept in `per_query` output for
  debugging, but never reported as "Recall@10" and never used for
  thresholds.
- **nDCG@10** — formula frozen explicitly, verbatim, so it cannot drift
  later:
  - `gain(rel) = 2^rel - 1`
  - `DCG@10 = Σ_{i=1}^{10} (2^rel_i - 1) / log2(i + 1)`, where `rel_i` is
    the `graded_relevance` value (0, 1, or 2; 0 for a retrieved note not
    in `graded_relevance` at all) of the note at rank `i` among the top
    10 results.
  - `IDCG@10` = the same DCG formula applied to the query's own
    `graded_relevance` values sorted descending, truncated to the first
    10 (for every query in this corpus that's ≤2 values, so IDCG is
    just gain(2) or gain(2)+gain(1)/log2(3) for the two-note queries —
    never a hypothetical maximum unrelated to this query's actual
    judgments).
  - `nDCG@10 = DCG@10 / IDCG@10`, or exactly `0` when `IDCG@10 = 0`
    (which cannot happen for an answerable query, since
    `relevant_note_ids` is required nonempty — included only for
    completeness of the function's contract, never expected to trigger
    on real B3 data).
  This formula is copied verbatim (not just referenced) into
  `frozen-config.json` at B3.4 freeze time, per your requirement that
  the protocol can't silently change later.
- **Confidence intervals**: a **deterministic** bootstrap CI (resample
  queries with replacement, N=2000 resamples, report 2.5th/97.5th
  percentile) for both aggregate Recall@10 and nDCG@10 — chosen over a
  normal-approximation interval because dev (60) and calibration (30)
  are small samples where a bootstrap is more honest about skew than
  assuming normality, and it requires no new dependency (a simple PRNG
  resample loop). **Deterministic per your requirement**: a fixed seed,
  recorded in both the per-run JSON report and (at B3.4 freeze time) in
  `frozen-config.json`, so any report is exactly reproducible from the
  same inputs. For groups below a practical bootstrap floor (`n < 5`,
  below which a 2000-resample percentile CI is numerically degenerate —
  e.g. a single-element resample can only ever equal that element), the
  CI is still computed (never silently omitted), but the group's entry
  carries `"low_confidence": true` per your requirement not to hide
  uncertainty by omission — the number is reported, flagged, not hidden.
- **Aggregate results**: overall Recall@10/nDCG@10 + CI across all
  answerable queries in the split, with `n` (count of answerable queries
  contributing).
- **Category breakdown**: the same numbers (Recall@10, nDCG@10, CI, `n`)
  computed separately for each of the 6 categories, each flagged
  `low_confidence` if its `n` is small (see threshold above).
- **Difficulty breakdown**: the same numbers, separately for
  easy/medium/hard, same `n` and `low_confidence` flagging.
- **Abstention metrics** (separate block, per constraint #2): modeled as
  a binary classification of "is there evidence for this query",
  ground truth = `abstain_expected`, prediction = whether
  `index::search` returned `NotPresent`. Reports the full confusion
  matrix (TP = abstain-expected and NotPresent returned; TN =
  answerable and Matches returned; FP = answerable but NotPresent
  returned — a missed real answer; FN = abstain-expected but Matches
  returned — a false "evidence exists") plus precision, recall, F1, and
  accuracy derived from it. This subsumes the earlier draft's separate
  true/false abstention rates — they are exactly this confusion
  matrix's recall (true abstention rate = TP/(TP+FN)) and the
  answerable-side miss rate (FP/(FP+TN)) respectively, now reported as
  one coherent block instead of two ad hoc numbers.

  **Zero-denominator semantics (correction)**: `precision = TP/(TP+FP)`,
  `recall = TP/(TP+FN)`, `F1 = 2·precision·recall/(precision+recall)`
  are each `null` (not `0.0`) when their denominator is zero (e.g.
  `precision` is undefined if nothing was predicted abstain, i.e.
  `TP+FP = 0`) — `0.0` would misrepresent "undefined" as "measured and
  bad". `accuracy = (TP+TN)/(TP+TN+FP+FN)` is `null` only if the split
  has zero queries, which cannot happen here.

  For the bootstrap CI on each of precision/recall/F1: a resample is
  only included in that metric's CI if the metric is defined on that
  resample (its own TP/FP/FN/TN counts within that resample yield a
  nonzero denominator). The report records `valid_resamples` alongside
  `total_resamples` (2000) for each of these three metrics. If
  `valid_resamples` falls below a usable floor for a stable
  2.5th/97.5th percentile (the same `n < 5` **valid-resample** floor
  used elsewhere, not the query-count floor), that metric's CI is
  marked `"ci": null, "low_confidence": true` rather than reporting a
  percentile computed from too few valid resamples. `accuracy`'s CI
  never faces this because its denominator can't be zero on any
  nonempty resample.

## Report format

```json
{
  "split": "dev",
  "run_date": "2026-11-14",
  "runner_commit": "<git rev-parse HEAD at run time>",
  "bootstrap_seed": 0,
  "bootstrap_resamples": 2000,
  "corpus_manifest_note_count": 53,
  "query_count": 61,
  "query_count_excluding_canary": 60,
  "aggregate": {"recall_at_10": 0.0, "ndcg_at_10": 0.0, "recall_ci": [0.0, 0.0], "ndcg_ci": [0.0, 0.0], "n": 0, "low_confidence": false},
  "by_category": {"<category>": {"recall_at_10": 0.0, "ndcg_at_10": 0.0, "recall_ci": [0.0, 0.0], "ndcg_ci": [0.0, 0.0], "n": 0, "low_confidence": false}},
  "by_difficulty": {"<difficulty>": {"recall_at_10": 0.0, "ndcg_at_10": 0.0, "recall_ci": [0.0, 0.0], "ndcg_ci": [0.0, 0.0], "n": 0, "low_confidence": false}},
  "abstention": {
    "confusion_matrix": {"tp": 0, "tn": 0, "fp": 0, "fn": 0},
    "precision": null, "precision_ci": null, "precision_valid_resamples": 0,
    "recall": null, "recall_ci": null, "recall_valid_resamples": 0,
    "f1": null, "f1_ci": null, "f1_valid_resamples": 0,
    "accuracy": 0.0, "accuracy_ci": [0.0, 0.0],
    "total_resamples": 2000, "n": 0, "low_confidence": false
  },
  "per_query": [{"id": "...", "category": "...", "difficulty": "...", "abstain_expected": false, "outcome": "Matches|NotPresent", "recall_at_10": 0.0, "hit_at_10": 0, "ndcg_at_10": 0.0, "rank_of_first_relevant": null}]
}
```

`precision`/`recall`/`f1` are `null` above only to show the undefined
case in the schema; a real run with `TP+FP > 0` etc. reports the actual
float. `hit_at_10` is the optional diagnostic from the Recall@10
correction above, always present but never used for thresholds.

`query_count` includes the canary row as loaded from the split's raw
`labels.jsonl`; `query_count_excluding_canary` is what every metric above
is actually computed over — the two numbers are reported side by side so
a reader can see the exclusion happened, not just assume it. `tp`/`fp`
per the Label row interpretation section above: `tp` = abstain-expected
correctly returning `NotPresent`; `fp` = answerable but `NotPresent`
returned (a missed real answer); `tn` = answerable correctly returning
`Matches`; `fn` = abstain-expected but `Matches` returned (a false
"evidence exists").

`per_query` is included so B3.4's calibration/threshold-setting (and any
later debugging) can inspect individual failures without rerunning —
raw measurements, per the spec's evidence requirement, not just
aggregates.

## B3.4 — calibration and freeze

B3.4 is explicitly a two-stop process, per your constraint #5 — nothing
is frozen without an approval round-trip in between:

1. Run the B3.3 runner against **dev** first (diagnosis only — if
   retrieval looks structurally broken, that's a dev-split finding, not
   grounds to touch calibration or test).
2. Run the same runner, same code, against **calibration**.
3. Propose thresholds from the calibration run's numbers (e.g. a minimum
   acceptable Recall@10/nDCG@10, an acceptable abstention-confusion-matrix
   precision/recall) with rationale, in the report this step ends with.
4. **STOP.** No `frozen-config.json` is written yet. You review the
   proposal and either approve it, ask for different thresholds, or ask
   for more runs. This is a real stop, not a formality — the step's own
   deliverable at this point is the proposal, not a frozen artifact.
5. Only after your explicit approval, in a separate follow-up action,
   write `eval/results/B3/frozen-config.json` recording (per constraint
   #6, verbatim): the commit, the corpus manifest's hash/contents, the
   runner's own version (git commit of `metrics.rs`/`b3_metric_runner.rs`
   — same commit, recorded for clarity), the bootstrap seed and resample
   count (per the new determinism requirement), the retrieval
   configuration (there is only one: B1's deterministic trigram+BM25
   baseline, no embeddings — recorded explicitly so a later change is
   visible against this baseline), the approved thresholds, and the
   metrics protocol (this doc, by reference and by copying its metric
   definitions inline so the record doesn't depend on this doc continuing
   to exist unchanged).
6. **Final-test is not run in this step, and nothing in this step reads
   `eval/rag/test/labels.jsonl` for its content** — per your constraint
   #6 ("nessun accesso al final-test durante sviluppo/calibrazione, salvo
   i controlli meccanici già esistenti di isolamento"), the only test-split
   access anywhere in this plan is the existing `held_out_labels` Rust
   test's mechanical isolation check (which reads the canary row to prove
   it can't leak, not to score anything) — nothing B3.3/B3.4 adds touches
   `eval/rag/test/` at all. Running against test is B3.5, requires your
   separate authorization, and any post-hoc threshold change after seeing
   final-test results invalidates that run — recorded as a hard rule in
   `frozen-config.json` itself, not just in this doc.

## Non-goals / anti-goals

- No MCP server (constraint #9).
- No final-test execution (constraint #3, this doc's own scope).
- No merge/tag/push (constraint #10).
- No change to `crates/index`/`crates/storage`/`crates/policy` production
  retrieval logic — the runner calls existing `api::search` unchanged;
  B3 measures the baseline, it doesn't tune it.
- No re-opening the 9 deferred Minor findings from the B3.1/B3.2 review,
  unless computing real metrics reveals one of them materially affects
  results (constraint #8) — e.g., if the near-duplicate dev/test query
  pairs (Minor #9) turn out to produce suspiciously identical scores,
  that would be flagged in the B3.4 report, not silently fixed.

## What's left to the builder

- Exact bootstrap resample count (2000) and CI percentile (2.5/97.5) —
  adjustable; standard defaults, not load-bearing to the design.
- Exact threshold values — explicitly yours to set in B3.4's report, not
  mine to decide.
- Whether `rank_of_first_relevant: null` (no relevant note found in top
  10 or at all) is distinguished from "found at rank 11+" in `per_query`
  — I'll record both (null = not in top 10 at all, capped) distinctly
  since it's useful for debugging and costs nothing.

## Acceptance criteria for this step (B3.3 + B3.4 only)

- `crates/api/src/metrics.rs` exists with unit tests covering: true
  Recall@10 (including a 2-relevant-notes-retrieve-1 case scoring
  exactly 0.5, not 1.0 — the exact case the pre-correction draft got
  wrong), Hit@10 as a separate diagnostic, nDCG@10 against the frozen
  `2^rel - 1` / `log2(i+1)` formula (checked against a hand-computed
  example), deterministic bootstrap CI (same seed -> same interval,
  sane bounds, `low_confidence` flagged below the `n < 5` floor), the
  abstention confusion matrix (TP/TN/FP/FN correctly classified in each
  of the 4 cases), and the zero-denominator case for
  precision/recall/F1 (each returns `null`, not `0.0`, when its
  denominator is zero; the bootstrap CI excludes resamples where the
  metric is undefined and reports `valid_resamples`).
- `crates/api/examples/b3_metric_runner.rs` runs against dev and
  calibration, excludes the canary row, writes a report per split to
  `eval/results/B3/`.
- Reports include aggregate, category breakdown (with `n`), difficulty
  breakdown (with `n`), and abstention confusion-matrix metrics, each
  with a CI and a `low_confidence` flag where applicable.
- `eval/results/B3/frozen-config.json` is NOT created by B3.3/B3.4's own
  tasks — it is written only after you explicitly approve the proposed
  thresholds in a follow-up step, per the two-stop process above.
- No test-split query or label CONTENT is read by any code added in this
  step (the pre-existing `held_out_labels` mechanical isolation check is
  unaffected and unchanged).
- Report back: runner implementation, dev results, calibration results,
  CIs, category/difficulty/abstention breakdowns, proposed thresholds
  and rationale, any blockers, GO/NO-GO recommendation for final-test —
  exactly the structure you asked for. `frozen-config.json`'s contents
  are reported only in a later follow-up, after your approval.
