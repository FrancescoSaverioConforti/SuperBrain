# B3.3/B3.4 — Metric Runner & Calibration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task (approved execution approach: Native, consistent with B3.1/B3.2's execution — implement in the current session, one independent review of the whole result at the end). Steps use checkbox (`- [ ]`) syntax for tracking.

**Approved with 4 changes** (see continuity `d203xx`), incorporated below: (1) the abstention bootstrap reconstructs a real `ConfusionMatrix` per resample and calls `confusion_matrix_metrics` on it, instead of a separate `predicted_positive`/`correct` abstraction; (2) the "different seeds give different CIs" test is removed (legitimately flaky, two bootstraps can agree); (3) every report is self-identifying (corpus/labels hashes, runner commit, retrieval config, seed/resamples) with a unique filename per run; (4) the runner validates label-row invariants itself before scoring, fail-fast.

**Goal:** Build a native Rust metric runner that calls the real
`api::search` / `index::search` retrieval path against the B3 corpus,
computes true Recall@10, nDCG@10 (frozen formula), confidence intervals,
and category/difficulty/abstention breakdowns, run it on dev then
calibration, and produce a threshold proposal for the user to approve —
stopping before any `frozen-config.json` is written and before any
final-test access.

**Architecture:** `crates/api/src/metrics.rs` holds pure, unit-tested
functions (Recall@10, Hit@10, nDCG@10, deterministic bootstrap CI,
abstention confusion matrix + precision/recall/F1/accuracy with
null-on-undefined semantics) that take `Vec<SearchResult>` + label data,
no vault or search dependency. `crates/api/examples/b3_metric_runner.rs`
builds a real `Vault`/`Index` from the B3 corpus (same construction
pattern as `crates/api/tests/b1_api.rs`'s `Fixture`), loads one split's
`labels.jsonl` via a minimal JSON Lines reader, excludes the canary row
by id, calls `api::search` once per query, and writes a JSON report
using `metrics.rs`. Python (`eval/rag/*.py`) is untouched.

**Tech Stack:** Rust (existing `api`, `index`, `storage`, `domain`,
`policy` crates), `serde`/`serde_json` (new dev-dependency for the
`api` crate — JSON report serialization; `serde` is already in the
workspace via `serde_yaml` in `domain`, `serde_json` is not yet present
anywhere and is added here).

**Spec:** `docs/superpowers/specs/2026-11-14-b3-metric-runner-design.md`
(this plan implements that design's B3.3 + B3.4 sections only; read both
before starting).

## Global Constraints

- Native Rust only: the runner calls `api::search` directly, never
  reimplements retrieval, never bridges to Python for scoring.
- The canary row (`id: canary-isolation-probe`) is excluded from every
  metric by id, before any metric computation sees it.
- Recall@10 is `|relevant_note_ids ∩ top_10| / |relevant_note_ids|`, never
  a 0/1 hit indicator. Hit@10 may exist only as a separate, clearly-named
  diagnostic field, never reported as or substituted for Recall@10.
- nDCG@10 uses exactly: `gain(rel) = 2^rel - 1`,
  `DCG@10 = Σ_{i=1}^{10} gain(rel_i) / log2(i + 1)`, IDCG from the
  query's own `graded_relevance` values sorted descending and truncated
  to 10, `nDCG@10 = DCG@10 / IDCG@10`. This formula is never
  approximated or substituted.
- Every bootstrap CI uses a fixed, recorded seed — same seed, same
  inputs, same CI, always. 2000 resamples unless the plan says
  otherwise. `low_confidence: true` is reported, never silently omitted,
  for any group below a practical confidence floor.
- Abstention `precision`/`recall`/`f1` are `null` when their denominator
  is zero — never `0.0`. Their bootstrap CIs are computed only over
  resamples where they're defined, with `valid_resamples` reported
  alongside `total_resamples`.
- Dev is for diagnosis only. Calibration is where thresholds get
  proposed. **No code in this plan reads `eval/rag/test/labels.jsonl`'s
  content** — the only existing test-split access anywhere stays the
  pre-existing `held_out_labels` Rust test's mechanical isolation check,
  untouched by this plan.
- `frozen-config.json` is NOT written by any task in this plan. B3.4's
  last task ends in a proposal and a STOP; freezing happens only in a
  separate follow-up after the user's explicit approval.
- No MCP server, no `graph_query`/`get_code_context`, no Task 10, no
  merge/tag/push, no change to `crates/index`/`crates/storage`/
  `crates/policy` production retrieval logic.

## Review Focus

- A query with 2 relevant notes where only 1 is retrieved in the top 10
  scoring as Recall@10 = 1.0 instead of 0.5 (the exact bug the
  methodological correction fixed in the design) — Task 1's test suite
  must pin this with a real 2-relevant-note example, not just single-
  relevant-note cases where the bug is invisible.
- `nDCG@10` computed with a different gain function (e.g. linear `rel`
  instead of `2^rel - 1`) or a hardcoded IDCG of 1.0 instead of the
  query's own sorted judgments — Task 1's tests must include a case
  where linear and exponential gain give different numbers, so a
  regression to the wrong formula fails loudly.
- Abstention precision/recall/F1 silently reporting `0.0` instead of
  `null` when `TP+FP=0` or `TP+FN=0` — a real input shape for this
  corpus (e.g. a split with zero predicted-abstain queries) that Task 1
  must construct explicitly, not just assume won't happen.
- The bootstrap CI silently including undefined-metric resamples by
  treating a `0/0` division as `0.0` inside the resample loop, which
  would corrupt `valid_resamples` accounting without ever surfacing an
  error — Task 1 must assert `valid_resamples < total_resamples` in a
  constructed case where some resamples are genuinely degenerate.
- The runner accidentally including the canary row's `category:
  "temporal"` count in the `by_category` breakdown (easy to miss since
  canary exclusion happens once at load time, but a refactor could
  reorder it after breakdown grouping) — Task 3's test must assert the
  `temporal` category's `n` in a real run excludes it.
- The abstention bootstrap's per-resample metric computed from a
  separately-tracked numerator/denominator pair that silently drifts
  from `confusion_matrix_metrics`'s own definition (e.g. a future edit
  changes the zero-denominator rule in one place but not the other) —
  Task 2 Step 9's second test pins the point estimate (computed once,
  unresampled) to exactly equal calling `confusion_matrix_metrics`
  directly, so the two paths can never silently diverge.
- A malformed label row (e.g. `graded_relevance` keys not matching
  `relevant_note_ids`) reaching `metrics.rs`'s functions and producing
  a number that looks plausible but is mathematically meaningless —
  Task 3 Step 5a-5e's `validate_label_rows` must reject it before any
  query runs, not after.
- Two report files from the same day silently overwriting each other
  because the filename only had date granularity — Task 3 Step 18's
  timestamp+commit filename and Task 4 Step 1's expectation of that
  exact naming scheme prevent this.

---

## File Structure

- Create: `crates/api/src/metrics.rs` — pure metric functions, unit
  tested, no vault/search/IO dependency.
- Modify: `crates/api/src/lib.rs` — add `pub mod metrics;`.
- Modify: `crates/api/Cargo.toml` — add `serde`, `serde_json` to
  `[dev-dependencies]` (the report writer lives in `examples/`, a
  dev-only binary).
- Create: `crates/api/examples/b3_metric_runner.rs` — builds the vault,
  loads labels, calls search, writes the report.
- Create: `eval/results/B3/<split>-<timestamp>-<commit>.json` (Task 4's
  real run output; unique per run, never overwritten by a same-day
  rerun) — output directory for JSON reports.

---

### Task 1: Pure metric functions — Recall@10, Hit@10, nDCG@10

**Files:**
- Create: `crates/api/src/metrics.rs`
- Modify: `crates/api/src/lib.rs:21` (add `pub mod metrics;` near the
  existing `pub mod retrieve;` / `pub mod search;` lines)

**Interfaces:**
- Produces: `pub fn recall_at_10(relevant_note_ids: &[String], retrieved_note_ids: &[String]) -> f64`
  (retrieved_note_ids is the top-10 slice, caller's responsibility to
  truncate — this function just computes the ratio over whatever slice
  it's given, so a caller who passes 10 passes correctly and a test can
  pass fewer to check boundary behavior); `pub fn hit_at_10(relevant_note_ids: &[String], retrieved_note_ids: &[String]) -> bool`;
  `pub fn ndcg_at_10(retrieved_note_ids: &[String], graded_relevance: &std::collections::HashMap<String, u8>) -> f64`.
  Consumed by Task 2 (bootstrap CI wraps these), Task 3 (the runner
  calls them per query), and this task's own unit tests.

- [ ] **Step 1: Write the failing test for true Recall@10 on a 2-relevant-note query**

```rust
#[test]
fn recall_at_10_is_fraction_not_a_hit_indicator() {
    let relevant = vec!["note-a".to_string(), "note-b".to_string()];
    let retrieved = vec!["note-a".to_string(), "note-c".to_string()];
    assert_eq!(recall_at_10(&relevant, &retrieved), 0.5);
}

#[test]
fn recall_at_10_is_one_when_all_relevant_notes_retrieved() {
    let relevant = vec!["note-a".to_string(), "note-b".to_string()];
    let retrieved = vec!["note-b".to_string(), "note-a".to_string(), "note-c".to_string()];
    assert_eq!(recall_at_10(&relevant, &retrieved), 1.0);
}

#[test]
fn recall_at_10_is_zero_when_none_retrieved() {
    let relevant = vec!["note-a".to_string()];
    let retrieved = vec!["note-c".to_string()];
    assert_eq!(recall_at_10(&relevant, &retrieved), 0.0);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p api recall_at_10 -- --exact-ish` (or just
`cargo test -p api metrics::` once the module exists as an empty stub
with the three function signatures as `todo!()` bodies — whichever
ordering the implementer prefers, as long as the test is watched to fail
for "not yet implemented," not a compile error unrelated to the logic).
Expected: FAIL (panic on `todo!()` or equivalent), not a type error.

- [ ] **Step 3: Implement `recall_at_10` and `hit_at_10` in `crates/api/src/metrics.rs`**

`recall_at_10`: count how many of `relevant_note_ids` appear in
`retrieved_note_ids` (as strings — note ids, not `NoteId`, since this
function is pure and has no `storage` dependency), divide by
`relevant_note_ids.len()` (never zero for an answerable query, per the
label schema's own invariant — document this as a precondition, not a
runtime check, matching this module's "pure function, no IO" scope).
`hit_at_10`: `true` if the intersection is nonempty.

- [ ] **Step 4: Run tests to verify Recall@10/Hit@10 pass**

Run: `cargo test -p api metrics::recall_at_10 metrics::hit_at_10`
Expected: PASS.

- [ ] **Step 5: Write the failing test for nDCG@10's frozen formula**

```rust
#[test]
fn ndcg_at_10_matches_the_frozen_exponential_gain_formula() {
    // graded_relevance: note-a=2, note-b=1. Retrieved order: note-b, note-a (note-a is NOT first).
    let mut graded = std::collections::HashMap::new();
    graded.insert("note-a".to_string(), 2u8);
    graded.insert("note-b".to_string(), 1u8);
    let retrieved = vec!["note-b".to_string(), "note-a".to_string()];
    // DCG = gain(1)/log2(2) + gain(2)/log2(3) = (2^1-1)/1 + (2^2-1)/log2(3) = 1 + 3/1.58496... = 1 + 1.89279...
    // IDCG = gain(2)/log2(2) + gain(1)/log2(3) = 3/1 + 1/1.58496... = 3 + 0.63093...
    let dcg = 1.0 + 3.0 / (3.0_f64).log2();
    let idcg = 3.0 + 1.0 / (3.0_f64).log2();
    let expected = dcg / idcg;
    assert!((ndcg_at_10(&retrieved, &graded) - expected).abs() < 1e-9);
}

#[test]
fn ndcg_at_10_is_one_when_retrieved_in_ideal_order() {
    let mut graded = std::collections::HashMap::new();
    graded.insert("note-a".to_string(), 2u8);
    graded.insert("note-b".to_string(), 1u8);
    let retrieved = vec!["note-a".to_string(), "note-b".to_string()];
    assert!((ndcg_at_10(&retrieved, &graded) - 1.0).abs() < 1e-9);
}

#[test]
fn ndcg_at_10_treats_an_unjudged_retrieved_note_as_zero_gain() {
    let mut graded = std::collections::HashMap::new();
    graded.insert("note-a".to_string(), 2u8);
    let retrieved = vec!["note-unjudged".to_string(), "note-a".to_string()];
    // DCG = gain(0)/log2(2) + gain(2)/log2(3) = 0 + 3/log2(3)
    // IDCG = gain(2)/log2(2) = 3
    let expected = (3.0 / (3.0_f64).log2()) / 3.0;
    assert!((ndcg_at_10(&retrieved, &graded) - expected).abs() < 1e-9);
}
```

- [ ] **Step 6: Run tests to verify they fail**

Run: `cargo test -p api metrics::ndcg_at_10`
Expected: FAIL (not yet implemented).

- [ ] **Step 7: Implement `ndcg_at_10` using the frozen formula**

`DCG`: for `i` in `1..=10` (1-indexed) over `retrieved_note_ids` (only
the first 10, even if more are passed — truncate inside this function
so callers can't accidentally pass an unbounded list and silently
extend the window), look up `graded_relevance.get(note_id).copied().unwrap_or(0)`,
compute `(2f64.powi(rel as i32) - 1.0) / (i as f64 + 1.0).log2()`, sum.
`IDCG`: take `graded_relevance.values().copied()`, sort descending,
truncate to 10, apply the same gain/discount formula positionally. Return
`if idcg == 0.0 { 0.0 } else { dcg / idcg }`.

- [ ] **Step 8: Run tests to verify nDCG@10 passes**

Run: `cargo test -p api metrics::ndcg_at_10`
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add crates/api/src/metrics.rs crates/api/src/lib.rs
git commit -m "feat(api): pure Recall@10/Hit@10/nDCG@10 metric functions"
```

---

### Task 2: Deterministic bootstrap CI and abstention confusion matrix

**Files:**
- Modify: `crates/api/src/metrics.rs`

**Interfaces:**
- Consumes: nothing from Task 1 directly (this task's functions are
  generic over "a per-query metric value already computed," not tied to
  Recall@10/nDCG@10 specifically — Task 3 is what connects them).
- Produces:
  `pub(crate) struct SplitMix64 { state: u64 }` with `pub(crate) fn new(seed: u64) -> Self`
  and `pub(crate) fn next(&mut self) -> u64` (exposed at `pub(crate)`
  visibility, not fully private, specifically so this module's own test
  in Step 1 can exercise the PRNG sequence directly instead of only
  through `bootstrap_ci`'s aggregate output — the correction's point
  exactly);
  `pub struct BootstrapCi { pub low: f64, pub high: f64 }`;
  `pub fn bootstrap_ci(values: &[f64], seed: u64, resamples: u32) -> BootstrapCi`
  (panics if `values` is empty — a precondition documented in the
  function's doc comment, since the runner never calls this on an empty
  split);
  `pub const LOW_CONFIDENCE_FLOOR: usize = 5;` (the `n < 5` floor from
  the design, exposed so Task 3's report-writing code can reuse the
  exact same constant rather than a second hardcoded `5`);
  `#[derive(Clone, Copy)] pub struct ConfusionMatrix { pub tp: u32, pub tn: u32, pub fp: u32, pub fn_: u32 }`
  (named `fn_` because `fn` is a Rust keyword; serialized as `"fn"` via
  `#[serde(rename = "fn")]` in Task 4's report struct, not here — this
  struct itself has no serde derive, staying dependency-free per this
  module's "pure, no IO" scope; `Clone, Copy` because Step 11's
  per-resample reconstruction builds many of these cheaply);
  `pub fn confusion_matrix_metrics(cm: &ConfusionMatrix) -> (Option<f64>, Option<f64>, Option<f64>, f64)`
  returning `(precision, recall, f1, accuracy)` with the first three as
  `None` exactly when their denominator is zero, per the frozen
  zero-denominator semantics.
  Consumed by Task 3 (per-metric and per-breakdown CI, abstention block).

- [ ] **Step 1: Write the failing test for deterministic bootstrap**

```rust
#[test]
fn bootstrap_ci_is_deterministic_given_the_same_seed() {
    let values = vec![0.0, 0.5, 1.0, 1.0, 0.0, 0.5, 1.0];
    let ci_a = bootstrap_ci(&values, 42, 500);
    let ci_b = bootstrap_ci(&values, 42, 500);
    assert_eq!(ci_a.low, ci_b.low);
    assert_eq!(ci_a.high, ci_b.high);
}

#[test]
fn bootstrap_ci_bounds_are_within_the_input_range() {
    let values = vec![0.2, 0.4, 0.6, 0.8];
    let ci = bootstrap_ci(&values, 7, 500);
    assert!(ci.low >= 0.2 && ci.low <= 0.8);
    assert!(ci.high >= 0.2 && ci.high <= 0.8);
    assert!(ci.low <= ci.high);
}

#[test]
fn the_prng_sequence_itself_differs_for_different_seeds() {
    // Per the review correction: two bootstraps CAN legitimately agree on
    // the same percentile interval even with different seeds, so the CI
    // itself is not a safe thing to assert "differs" on. Test the PRNG's
    // own output sequence instead, which must differ deterministically.
    let mut rng_a = SplitMix64::new(1);
    let mut rng_b = SplitMix64::new(2);
    let seq_a: Vec<u64> = (0..10).map(|_| rng_a.next()).collect();
    let seq_b: Vec<u64> = (0..10).map(|_| rng_b.next()).collect();
    assert_ne!(seq_a, seq_b);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p api metrics::bootstrap_ci`
Expected: FAIL (not yet implemented).

- [ ] **Step 3: Implement `SplitMix64` and `bootstrap_ci`**

No new crate dependency: `SplitMix64::new(seed)` stores `state: seed`;
`next(&mut self) -> u64` does the standard splitmix64 mix-and-advance
step (`self.state = self.state.wrapping_add(0x9E3779B97F4A7C15);` then
the standard xor/multiply finalizer — a well-known ~6-line algorithm,
not original cryptography, safe to hand-write for a non-adversarial
resampling use). `bootstrap_ci`: for each of `resamples` iterations,
draw `values.len()` indices with replacement using `SplitMix64::new(seed)`
advanced once per draw (one generator instance for the whole call, not
reseeded per resample — determinism comes from seeding the call once),
compute the mean of that resample, collect into a `Vec<f64>`, sort, and
take the values at the 2.5th and 97.5th percentile positions (`((resamples
as f64) * 0.025).floor() as usize` and `.ceil()` respectively, clamped to
valid indices) as `low`/`high`.

- [ ] **Step 4: Run tests to verify bootstrap_ci passes**

Run: `cargo test -p api metrics::bootstrap_ci`
Expected: PASS.

- [ ] **Step 5: Write the failing test for the confusion matrix and zero-denominator semantics**

```rust
#[test]
fn confusion_matrix_metrics_computes_precision_recall_f1_accuracy() {
    let cm = ConfusionMatrix { tp: 3, tn: 4, fp: 1, fn_: 2 };
    let (precision, recall, f1, accuracy) = confusion_matrix_metrics(&cm);
    assert_eq!(precision, Some(0.75));       // 3 / (3+1)
    assert_eq!(recall, Some(0.6));            // 3 / (3+2)
    assert!((f1.unwrap() - (2.0 * 0.75 * 0.6 / (0.75 + 0.6))).abs() < 1e-9);
    assert_eq!(accuracy, 0.7);                // (3+4) / 10
}

#[test]
fn confusion_matrix_metrics_precision_is_none_when_nothing_predicted_abstain() {
    let cm = ConfusionMatrix { tp: 0, tn: 5, fp: 0, fn_: 3 };
    let (precision, recall, f1, _accuracy) = confusion_matrix_metrics(&cm);
    assert_eq!(precision, None);
    assert_eq!(recall, Some(0.0));
    assert_eq!(f1, None); // f1 undefined when precision is undefined
}

#[test]
fn confusion_matrix_metrics_recall_is_none_when_no_true_abstain_queries_exist() {
    let cm = ConfusionMatrix { tp: 0, tn: 5, fp: 2, fn_: 0 };
    let (precision, recall, f1, _accuracy) = confusion_matrix_metrics(&cm);
    assert_eq!(recall, None);
    assert_eq!(precision, Some(0.0));
    assert_eq!(f1, None);
}
```

- [ ] **Step 6: Run tests to verify they fail**

Run: `cargo test -p api metrics::confusion_matrix`
Expected: FAIL (not yet implemented).

- [ ] **Step 7: Implement `confusion_matrix_metrics`**

`precision = if tp + fp == 0 { None } else { Some(tp / (tp+fp)) }`;
`recall = if tp + fn_ == 0 { None } else { Some(tp / (tp+fn_)) }`;
`f1 = match (precision, recall) { (Some(p), Some(r)) if p + r > 0.0 => Some(2.0*p*r/(p+r)), (Some(_), Some(_)) => Some(0.0), _ => None }`
(the `p + r > 0.0` guard avoids a NaN from `0.0/0.0` when both are
defined but both zero); `accuracy = (tp+tn) as f64 / (tp+tn+fp+fn_) as f64`.

- [ ] **Step 8: Run tests to verify confusion matrix metrics pass**

Run: `cargo test -p api metrics::confusion_matrix`
Expected: PASS.

- [ ] **Step 9: Write the failing test for confusion-matrix bootstrap CIs built from per-resample reconstruction**

Per the review's correction: the bootstrap must resample the underlying
per-query outcomes, rebuild a real `ConfusionMatrix` for each resample,
and call `confusion_matrix_metrics` on that resample's matrix — never a
separate generic abstraction decoupled from the confusion matrix
definition used elsewhere.

```rust
#[test]
fn confusion_matrix_bootstrap_reports_valid_resamples_per_metric() {
    // Per-query ground truth (abstain_expected) and prediction (NotPresent
    // returned), one entry per query, 10 queries: 3 true abstain-expected
    // (2 correctly predicted NotPresent = TP, 1 incorrectly Matches = FN),
    // 7 answerable (all correctly Matches = TN). A resample that happens to
    // draw zero of the 3 abstain-expected queries makes TP+FP possibly zero
    // for that resample (precision undefined on that resample).
    let abstain_expected = vec![true, true, true, false, false, false, false, false, false, false];
    let predicted_not_present = vec![true, true, false, false, false, false, false, false, false, false];
    let result = confusion_matrix_bootstrap(&abstain_expected, &predicted_not_present, 123, 500);
    assert!(result.precision.valid_resamples <= 500);
    assert!(result.recall.valid_resamples <= 500);
    assert_eq!(result.accuracy.valid_resamples, 500); // accuracy's denominator can't be zero
    if result.precision.valid_resamples < LOW_CONFIDENCE_FLOOR {
        assert!(result.precision.ci.is_none());
    }
}

#[test]
fn confusion_matrix_bootstrap_point_estimates_match_confusion_matrix_metrics() {
    // The un-resampled point estimate (computed once over the full data,
    // not averaged over resamples) must equal calling confusion_matrix_metrics
    // directly on the full-data ConfusionMatrix — the bootstrap only adds a CI
    // around this point estimate, it never redefines it.
    let abstain_expected = vec![true, true, false, false, false];
    let predicted_not_present = vec![true, false, false, false, false];
    let cm = ConfusionMatrix { tp: 1, fn_: 1, tn: 3, fp: 0 };
    let (expected_precision, expected_recall, expected_f1, expected_accuracy) = confusion_matrix_metrics(&cm);
    let result = confusion_matrix_bootstrap(&abstain_expected, &predicted_not_present, 1, 100);
    assert_eq!(result.precision.point_estimate, expected_precision);
    assert_eq!(result.recall.point_estimate, expected_recall);
    assert_eq!(result.f1.point_estimate, expected_f1);
    assert_eq!(result.accuracy.point_estimate, Some(expected_accuracy));
}
```

- [ ] **Step 10: Run the tests to verify they fail**

Run: `cargo test -p api metrics::confusion_matrix_bootstrap`
Expected: FAIL (`confusion_matrix_bootstrap` not yet defined).

- [ ] **Step 11: Implement `confusion_matrix_bootstrap`**

```rust
pub struct OptionalMetricResult {
    pub point_estimate: Option<f64>,
    pub ci: Option<BootstrapCi>,
    pub valid_resamples: u32,
    pub total_resamples: u32,
}
pub struct ConfusionMatrixBootstrap {
    pub confusion_matrix: ConfusionMatrix,
    pub precision: OptionalMetricResult,
    pub recall: OptionalMetricResult,
    pub f1: OptionalMetricResult,
    pub accuracy: OptionalMetricResult, // ci always Some for a nonempty split
}
pub fn confusion_matrix_bootstrap(
    abstain_expected: &[bool],
    predicted_not_present: &[bool],
    seed: u64,
    resamples: u32,
) -> ConfusionMatrixBootstrap
```

First, build the real, un-resampled `ConfusionMatrix` over the full
input (one TP/TN/FP/FN tally per query, per the design's definitions)
and call `confusion_matrix_metrics` on it once — these four values
become each field's `point_estimate` (accuracy wrapped in `Some`, the
other three passed through as-is since `confusion_matrix_metrics`
already returns `Option<f64>` for them).

Then bootstrap: one `SplitMix64` instance seeded once by `seed`, for
each of `resamples` iterations draw `abstain_expected.len()` indices
with replacement, build a **fresh `ConfusionMatrix`** by tallying
TP/TN/FP/FN over exactly those resampled indices (reusing the same
tallying logic the full-data pass used — factor it into a small private
helper `tally(indices, abstain_expected, predicted_not_present) ->
ConfusionMatrix` so the full-data and per-resample paths can never
drift apart), then call `confusion_matrix_metrics` on that resample's
matrix. Push `Some` results into each metric's own value list (skip
`None` for that resample — this is what "skip the resample for that
metric" means now: the metric is computed FROM the matrix, consistent
with `confusion_matrix_metrics`'s own zero-denominator semantics,
rather than a separately-tracked denominator check). After all
resamples: for `accuracy`, always `Some` (its denominator is the full
resample size, never zero for a nonempty split) — sort and take
percentiles as usual. For `precision`/`recall`/`f1`: if the collected
values' count is `< LOW_CONFIDENCE_FLOOR`, that metric's `ci` is `None`;
else sort and take the 2.5th/97.5th percentile. `valid_resamples` is
each metric's own collected-value count; `total_resamples` is the input
`resamples`, shared across all four.

- [ ] **Step 12: Run the tests to verify they pass**

Run: `cargo test -p api metrics::confusion_matrix_bootstrap`
Expected: PASS.

- [ ] **Step 13: Run the full metrics module test suite**

Run: `cargo test -p api metrics::`
Expected: PASS, all tests from Task 1 and Task 2.

- [ ] **Step 14: Commit**

```bash
git add crates/api/src/metrics.rs
git commit -m "feat(api): deterministic bootstrap CI and abstention confusion matrix with null-on-undefined semantics"
```

---

### Task 3: The metric runner binary

**Files:**
- Create: `crates/api/examples/b3_metric_runner.rs`
- Modify: `crates/api/Cargo.toml` (add `serde`, `serde_json` to
  `[dev-dependencies]`)

**Interfaces:**
- Consumes: `metrics::{recall_at_10, hit_at_10, ndcg_at_10, bootstrap_ci,
  confusion_matrix_bootstrap, ConfusionMatrix, confusion_matrix_metrics,
  LOW_CONFIDENCE_FLOOR}` (Tasks 1-2); `api::search::{search, SearchRequest}`,
  `api::{RequestContext, SCHEMA_VERSION}` (existing); `index::{Index,
  indexer_impl::{rebuild_index, RealIndexer}, search::SearchOutcome}`
  (existing); `storage::{propose_and_commit, Vault, Version, NoteId}`
  (existing, same construction pattern as `crates/api/tests/b1_api.rs`'s
  `Fixture`); `policy::{Gate, ReadPolicy, SecretPolicy, ReadOperation}`
  (existing).
- Produces: a `cargo run --release --example b3_metric_runner -- <dev|calibration>`
  CLI that writes `eval/results/B3/<split>-<timestamp>-<commit>.json`.
  Consumed by Task 4 (running it) and the user's final report.

- [ ] **Step 1: Add `serde`/`serde_json`/`sha2` dev-dependencies**

In `crates/api/Cargo.toml`'s `[dev-dependencies]` section, add:
```toml
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sha2 = "0.10"
```

- [ ] **Step 2: Write the failing test for canary-row exclusion**

Since `b3_metric_runner.rs` is a binary (not a lib), put this as a
`#[cfg(test)]` unit test inside the same file, testing a pure helper
function extracted for this purpose:

```rust
#[test]
fn load_split_labels_excludes_the_canary_row_by_id() {
    let jsonl = r#"{"id":"canary-isolation-probe","question":"HELDOUT_QUESTION_CANARY_901","answer":"HELDOUT_ANSWER_CANARY_902","category":"temporal","difficulty":"easy","abstain_expected":true,"relevant_note_ids":[],"graded_relevance":{},"citation_spans":[]}
{"id":"q001","question":"real question","answer":"real answer","category":"bugs-fixes","difficulty":"medium","abstain_expected":false,"relevant_note_ids":["note-a"],"graded_relevance":{"note-a":2},"citation_spans":[]}
"#;
    let rows = parse_label_rows(jsonl);
    let filtered = exclude_canary(rows);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].id, "q001");
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test -p api --example b3_metric_runner load_split_labels_excludes_the_canary_row_by_id`
Expected: FAIL (`parse_label_rows`/`exclude_canary` not yet defined —
note: testing code inside an `examples/` binary requires
`cargo test --example <name>`, confirm this invocation works in this
repo's cargo setup before relying on it for the RED step; if it does
not, move this specific parsing/filtering logic into
`crates/api/src/metrics.rs` instead, where `cargo test -p api` already
works, and have the binary call it from there — a plan-level choice the
implementer rules on and ledgers if the first approach doesn't compile.)

- [ ] **Step 4: Implement `LabelRow` struct, `parse_label_rows`, and `exclude_canary`**

`#[derive(serde::Deserialize)] struct LabelRow { id: String, question: String, answer: String, category: String, difficulty: String, abstain_expected: bool, relevant_note_ids: Vec<String>, graded_relevance: std::collections::HashMap<String, u8>, citation_spans: Vec<serde_json::Value> }`
(`citation_spans` typed as opaque `Value` — the runner never reads
citation content, only needs the row to parse). `parse_label_rows(text:
&str) -> Vec<LabelRow>` splits on newlines, skips empty lines, calls
`serde_json::from_str` per line, panics with a clear message on a
malformed line (this is a dev tool reading its own project's committed
label files, not a boundary needing graceful degradation).
`exclude_canary(rows: Vec<LabelRow>) -> Vec<LabelRow>` filters out
`id == "canary-isolation-probe"`.

- [ ] **Step 5: Run the test to verify it passes**

Run the same command as Step 3. Expected: PASS.

- [ ] **Step 5a: Write the failing tests for label invariant validation**

Per the review's correction: the runner must fail-fast on malformed
rows before any metric touches them, rather than trusting
`eval/rag/*.py`'s own validation was run and never re-checking.

```rust
#[test]
fn validate_label_rows_rejects_answerable_row_with_empty_relevant_note_ids() {
    let row = LabelRow {
        id: "q1".into(), question: "x".into(), answer: "y".into(),
        category: "bugs-fixes".into(), difficulty: "easy".into(),
        abstain_expected: false, relevant_note_ids: vec![],
        graded_relevance: Default::default(), citation_spans: vec![],
    };
    let result = validate_label_rows(&[row]);
    assert!(result.is_err());
}

#[test]
fn validate_label_rows_rejects_abstain_row_with_nonempty_relevant_note_ids() {
    let row = LabelRow {
        id: "q1".into(), question: "x".into(), answer: "".into(),
        category: "bugs-fixes".into(), difficulty: "easy".into(),
        abstain_expected: true, relevant_note_ids: vec!["note-a".into()],
        graded_relevance: Default::default(), citation_spans: vec![],
    };
    let result = validate_label_rows(&[row]);
    assert!(result.is_err());
}

#[test]
fn validate_label_rows_rejects_graded_relevance_keys_not_matching_relevant_note_ids() {
    let mut graded = std::collections::HashMap::new();
    graded.insert("note-b".to_string(), 2u8);
    let row = LabelRow {
        id: "q1".into(), question: "x".into(), answer: "y".into(),
        category: "bugs-fixes".into(), difficulty: "easy".into(),
        abstain_expected: false, relevant_note_ids: vec!["note-a".into()],
        graded_relevance: graded, citation_spans: vec![],
    };
    let result = validate_label_rows(&[row]);
    assert!(result.is_err());
}

#[test]
fn validate_label_rows_rejects_grade_outside_zero_one_two() {
    let mut graded = std::collections::HashMap::new();
    graded.insert("note-a".to_string(), 5u8);
    let row = LabelRow {
        id: "q1".into(), question: "x".into(), answer: "y".into(),
        category: "bugs-fixes".into(), difficulty: "easy".into(),
        abstain_expected: false, relevant_note_ids: vec!["note-a".into()],
        graded_relevance: graded, citation_spans: vec![],
    };
    let result = validate_label_rows(&[row]);
    assert!(result.is_err());
}

#[test]
fn validate_label_rows_rejects_unknown_category() {
    let row = LabelRow {
        id: "q1".into(), question: "x".into(), answer: "y".into(),
        category: "not-a-real-category".into(), difficulty: "easy".into(),
        abstain_expected: false, relevant_note_ids: vec!["note-a".into()],
        graded_relevance: std::collections::HashMap::from([("note-a".to_string(), 2u8)]),
        citation_spans: vec![],
    };
    let result = validate_label_rows(&[row]);
    assert!(result.is_err());
}

#[test]
fn validate_label_rows_rejects_unknown_difficulty() {
    let row = LabelRow {
        id: "q1".into(), question: "x".into(), answer: "y".into(),
        category: "bugs-fixes".into(), difficulty: "impossible".into(),
        abstain_expected: false, relevant_note_ids: vec!["note-a".into()],
        graded_relevance: std::collections::HashMap::from([("note-a".to_string(), 2u8)]),
        citation_spans: vec![],
    };
    let result = validate_label_rows(&[row]);
    assert!(result.is_err());
}

#[test]
fn validate_label_rows_accepts_a_well_formed_row() {
    let row = LabelRow {
        id: "q1".into(), question: "x".into(), answer: "y".into(),
        category: "bugs-fixes".into(), difficulty: "easy".into(),
        abstain_expected: false, relevant_note_ids: vec!["note-a".into()],
        graded_relevance: std::collections::HashMap::from([("note-a".to_string(), 2u8)]),
        citation_spans: vec![],
    };
    assert!(validate_label_rows(&[row]).is_ok());
}
```

- [ ] **Step 5b: Run the tests to verify they fail**

Run: `cargo test -p api --example b3_metric_runner validate_label_rows`
Expected: FAIL (`validate_label_rows` not yet defined).

- [ ] **Step 5c: Implement `validate_label_rows`**

`pub fn validate_label_rows(rows: &[LabelRow]) -> Result<(), String>`
(a plain `String` error is enough for a dev tool that panics on this
error in `main` — no new error-type hierarchy needed). For each row,
check in order and return the first violation found, naming the row's
`id` in the message: (1) `category` is one of the 6 known values
(`notes-decisions`, `bugs-fixes`, `patterns-knowledge`, `temporal`,
`identifier-code`, `cross-file-concept`); (2) `difficulty` is one of
`easy`/`medium`/`hard`; (3) if `abstain_expected`, `relevant_note_ids`
is empty and `graded_relevance` is empty; (4) if not
`abstain_expected`, `relevant_note_ids` is nonempty; (5)
`graded_relevance`'s key set equals `relevant_note_ids`'s set exactly;
(6) every `graded_relevance` value is `0`, `1`, or `2`. This
deliberately mirrors (not duplicates the implementation of, per your
instruction not to reimplement the whole Python validator) the subset
of `eval/rag/runner.py::_validate_extended_schema`'s rules that are
mathematical preconditions for `metrics.rs`'s functions to be
meaningful — citation span content and provenance checks stay Python's
job, out of scope here.

- [ ] **Step 5d: Run the tests to verify they pass**

Run the same command as Step 5b. Expected: PASS.

- [ ] **Step 5e: Wire validation into the loading path**

`parse_label_rows` + `exclude_canary`'s caller (Task 3 Step 16's
`run_split`, written later) must call `validate_label_rows` on the
canary-excluded rows and `panic!` with the returned error message on
`Err` — fail-fast before any query is run, per the review's
requirement. Add this call when `run_split` is implemented in Step 16;
noted here so it isn't forgotten once that step is reached.

- [ ] **Step 6: Write the failing test for building the vault and committing the real corpus**

```rust
#[test]
fn builds_a_vault_and_commits_every_real_corpus_note() {
    let fixture = TestFixture::new();
    let committed = fixture.commit_corpus();
    assert!(committed >= 53, "expected at least the 53 known corpus notes, got {committed}");
}
```

(`TestFixture` is this file's own fixture struct, built the same way
`crates/api/tests/b1_api.rs`'s `Fixture` is, including the `known_ids`
full-set-up-front fix B3.1/B3.2's Task 7 already discovered is
necessary for this corpus's `derived_from`/`supersedes` relations.)

- [ ] **Step 7: Run the test to verify it fails**

Run: `cargo test -p api --example b3_metric_runner builds_a_vault_and_commits_every_real_corpus_note`
Expected: FAIL (`TestFixture` not yet defined).

- [ ] **Step 8: Implement `TestFixture` and corpus-committing logic**

Copy the vault/PiiStore/git-repo construction pattern from
`crates/api/tests/b1_api.rs`'s `Fixture::new()` (same calls: `git2::Repository::init`,
`vault_security::PiiStore::open` + `bootstrap_deletion_log`,
`Vault::open_with_pii_store`). Add `commit_corpus(&self) -> usize`:
reads every `*.md` under `eval/rag/corpus/notes/` relative to
`env!("CARGO_MANIFEST_DIR")` (same `concat!` pattern
`crates/api/tests/b1_api.rs`'s `held_out_labels` test already uses),
collects all note ids into a `known_ids: HashSet<NoteId>` up front
(same fix as B3.1/B3.2's Task 7), commits each via
`storage::propose_and_commit`, returns the count committed.

- [ ] **Step 9: Run the test to verify it passes**

Run the same command as Step 7. Expected: PASS.

- [ ] **Step 10: Write the failing test for per-query search and outcome recording**

```rust
#[test]
fn searches_a_real_corpus_query_and_records_its_outcome() {
    let fixture = TestFixture::new();
    fixture.commit_corpus();
    let idx = fixture.build_index();
    let outcome = run_one_query(&idx, "resolve_configured_timezone", 10);
    assert!(matches!(outcome, QueryOutcome::Matches { .. }));
}

#[test]
fn an_unanswerable_query_returns_not_present() {
    let fixture = TestFixture::new();
    fixture.commit_corpus();
    let idx = fixture.build_index();
    let outcome = run_one_query(&idx, "ZZZ_NO_SUCH_TERM_EVER_QQQ", 10);
    assert!(matches!(outcome, QueryOutcome::NotPresent));
}
```

- [ ] **Step 11: Run the tests to verify they fail**

Run: `cargo test -p api --example b3_metric_runner run_one_query`
Expected: FAIL (`run_one_query`/`QueryOutcome`/`build_index` not yet
defined).

- [ ] **Step 12: Implement `build_index`, `QueryOutcome`, and `run_one_query`**

`fixture.build_index(&self) -> Index<'_>` wraps `rebuild_index` +
`Index::new(&self.vault, &fresh_indexer, "eval-b3", <today's date>)`
(project `"eval-b3"`, matching the corpus notes' real `project:` field —
same fix B3.1/B3.2's Task 7 made to `held_out_labels`, which used
`project-x` against the old single fixture and had to switch to
`eval-b3` for the real corpus). `enum QueryOutcome { NotPresent,
Matches { retrieved_note_ids: Vec<String> } }` (just the ordered note id
strings, truncated to the requested k — everything `metrics.rs`'s pure
functions need, nothing the vault/search types that would couple this
enum back to `index`). `run_one_query(idx: &Index, question: &str, k:
usize) -> QueryOutcome` calls `api::search(idx, &allow_policy(),
request_context(), SearchRequest { text: question.into(), filters:
Default::default(), k })`, matches on the response's `SearchOutcome`,
maps `Matches { first, rest }` to the ordered id list (`first`'s id
then each of `rest`'s ids, each via `.note_id.as_str().to_string()`,
capped at `k`).

- [ ] **Step 13: Run the tests to verify they pass**

Run the same command as Step 11. Expected: PASS.

- [ ] **Step 14: Write the failing test for a full run producing a well-shaped report**

```rust
#[test]
fn a_full_dev_split_run_produces_a_report_with_every_required_field() {
    let report = run_split("dev", /* bootstrap_seed */ 0, /* resamples */ 200);
    assert_eq!(report.split, "dev");
    assert!(report.query_count_excluding_canary > 0);
    assert!(report.by_category.contains_key("bugs-fixes"));
    assert!(report.by_difficulty.contains_key("easy"));
    assert!(report.abstention.confusion_matrix.tp + report.abstention.confusion_matrix.tn
        + report.abstention.confusion_matrix.fp + report.abstention.confusion_matrix.fn_
        == report.query_count_excluding_canary as u32);
    assert!(!report.corpus_manifest_sha256.is_empty());
    assert!(!report.labels_sha256.is_empty());
    assert_eq!(report.retrieval_config, "B1 deterministic trigram+BM25, no embeddings");
    assert_eq!(report.bootstrap_seed, 0);
}
```

(Use a low resample count like 200 in this test for speed; the real CLI
invocation in Task 4 uses the full 2000 per the frozen design.)

- [ ] **Step 15: Run the test to verify it fails**

Run: `cargo test -p api --example b3_metric_runner a_full_dev_split_run_produces_a_report`
Expected: FAIL (`run_split`/report struct not yet defined).

- [ ] **Step 16: Implement the report structs and `run_split`**

Define `#[derive(serde::Serialize)]` structs mirroring the design doc's
JSON report format, extended per the review's self-identifying-report
correction. Fields: `split`, `run_date`, `run_timestamp_utc` (full
ISO-8601 instant, not just a date — needed for the unique filename),
`runner_commit` (git rev-parse HEAD), `retrieval_config` (a fixed
string literal describing the one configuration this baseline runs,
e.g. `"B1 deterministic trigram+BM25, no embeddings"` — recorded so a
future change is visible against this exact string), `corpus_manifest_sha256`
(SHA-256 of `eval/rag/corpus-manifest.json`'s file contents),
`labels_sha256` (SHA-256 of the split's own `labels.jsonl` file
contents, post-exclusion-irrelevant since the hash is of the file as
committed, canary row included — the hash identifies the INPUT file,
exclusion is documented separately via `query_count` vs
`query_count_excluding_canary`), `bootstrap_seed`, `bootstrap_resamples`,
`corpus_manifest_note_count`, `query_count`, `query_count_excluding_canary`,
`aggregate`, `by_category`, `by_difficulty`, `abstention`, `per_query`
— each nested struct matching the design's per-field shapes, including
`precision`/`recall`/`f1` as `Option<f64>` serializing to JSON `null`
when `None`, `valid_resamples`/`total_resamples` on each of
precision/recall/f1/accuracy, and `low_confidence: bool` on every
breakdown entry and on each abstention metric.

No new crate dependency for SHA-256: either hand-roll (more code, no
dependency) or add `sha2` as a dev-dependency alongside `serde`/
`serde_json` from Step 1 — prefer `sha2` (it's a well-known, minimal,
already-indirectly-present dependency: `aes-gcm`/`argon2`'s dependency
tree in this workspace's `Cargo.lock` already pulls in similar RustCrypto
crates for `vault-security`, so `sha2` is a small, low-risk addition,
and hand-rolling SHA-256 correctly is real cryptographic code this
project shouldn't reimplement, unlike the splitmix64 PRNG which is not
expected to resist any adversary).

`run_split(split: &str, seed: u64, resamples: u32) -> Report`: builds
the fixture, commits the corpus, builds the index, loads that split's
raw labels (full file, for hashing) and separately the canary-filtered
rows, calls `validate_label_rows` on the canary-filtered rows per Step
5e and panics with its error on failure, runs `run_one_query` for every
row, computes per-query Recall@10/Hit@10/nDCG@10 for answerable rows
(via Task 1's functions), builds the two parallel
`abstain_expected`/`predicted_not_present` slices for every row, calls
`confusion_matrix_bootstrap` once for the whole split and once per
category and once per difficulty (same function, different row
subsets), uses `bootstrap_ci` for the Recall@10/nDCG@10 breakdowns,
flags `low_confidence` per the `LOW_CONFIDENCE_FLOOR` constant on every
breakdown entry including abstention's own precision/recall/f1 (via
each metric's own `valid_resamples`), fills `runner_commit` via `git
rev-parse HEAD` (`std::process::Command::new("git").args(["rev-parse", "HEAD"]).output()`,
same pattern `eval/rag/build_manifest.py` already uses in Python),
computes `corpus_manifest_sha256`/`labels_sha256` by reading those
files' raw bytes and hashing with `sha2::Sha256`.

- [ ] **Step 17: Run the test to verify it passes**

Run the same command as Step 15. Expected: PASS.

- [ ] **Step 18: Implement the CLI entry point (`fn main`)**

Reads `std::env::args()`, expects exactly one argument (`"dev"` or
`"calibration"` — reject `"test"` explicitly with a clear panic message
citing the Global Constraints rule, as a defense-in-depth check even
though no task calls it with `"test"`), calls `run_split` with seed `0`
and `resamples` `2000` (the frozen defaults), serializes the report with
`serde_json::to_string_pretty`, writes it to
`eval/results/B3/<split>-<run_timestamp_utc, filesystem-safe form e.g.
20261114T153045Z>-<short runner_commit, first 7 chars>.json` — per the
review's correction, never a bare `<split>-<date>.json` that a same-day
rerun would silently overwrite (creating the `B3/` directory if
absent), prints the output path.

- [ ] **Step 19: Commit**

```bash
git add crates/api/examples/b3_metric_runner.rs crates/api/Cargo.toml
git commit -m "feat(api): B3 metric runner — real search, real metrics, dev/calibration only"
```

---

### Task 4: Run dev and calibration, propose thresholds, STOP

**Files:**
- Create: `eval/results/B3/dev-<timestamp>-<commit>.json` (actual run output)
- Create: `eval/results/B3/calibration-<timestamp>-<commit>.json` (actual run output)

**Interfaces:**
- Consumes: Task 3's `b3_metric_runner` binary.
- Produces: the two real report files, consumed by this task's own
  threshold-proposal write-up (reported to the user, not consumed by
  further code in this plan).

- [ ] **Step 1: Run the dev split**

Run: `cargo run --release -p api --example b3_metric_runner -- dev`
Expected: exits 0, prints the path to
`eval/results/B3/dev-<timestamp>-<commit>.json`, which exists and
parses as valid JSON with every field the design's report format
specifies, including the self-identifying fields
(`corpus_manifest_sha256`, `labels_sha256`, `runner_commit`,
`retrieval_config`, `bootstrap_seed`, `bootstrap_resamples`).

- [ ] **Step 2: Inspect the dev report for structural sanity**

Read the written JSON. Confirm: `query_count_excluding_canary` equals
60 (dev's known real-query count); `by_category` has exactly 6 keys
matching the corpus's known categories; `by_difficulty` has exactly 3
keys; the abstention confusion matrix's four counts sum to 60. This is
diagnosis, not a threshold decision — per the Global Constraints, dev
results don't gate anything, they're read to confirm the runner isn't
structurally broken before trusting calibration's numbers.

- [ ] **Step 3: Run the calibration split**

Run: `cargo run --release -p api --example b3_metric_runner -- calibration`
Expected: exits 0, writes
`eval/results/B3/calibration-<timestamp>-<commit>.json`,
`query_count_excluding_canary` equals 30.

- [ ] **Step 4: Read both reports and draft a threshold proposal**

From calibration's aggregate/breakdown/abstention numbers (and dev's, as
a cross-check for consistency — a wildly different dev-vs-calibration
number for the same metric is itself worth flagging, not silently
averaged away), draft a proposed minimum acceptable Recall@10, nDCG@10,
and abstention precision/recall, each with the rationale (e.g. "the
observed calibration Recall@10 is X with CI [Y, Z]; proposing a
threshold of W because..."). This step produces prose for the final
report, not a file — no `frozen-config.json` is written here or
anywhere else in this plan.

- [ ] **Step 5: Commit the two report files**

```bash
git add eval/results/B3/dev-*.json eval/results/B3/calibration-*.json
git commit -m "docs(eval): B3.3/B3.4 dev and calibration metric runs"
```

- [ ] **Step 6: STOP and report**

Per the user's required report structure: runner implementation summary,
dev results, calibration results, CIs, category/difficulty/abstention
breakdowns, proposed thresholds and rationale (from Step 4), any
blockers, GO/NO-GO recommendation for final-test. Do not write
`frozen-config.json`. Do not touch `eval/rag/test/`. Wait for the user's
explicit approval of the proposed thresholds before any further action.

---

## Self-Review Notes

- **Spec coverage**: the design's architecture, label-row interpretation,
  all 3 metric corrections, report format, and the B3.4 two-stop process
  each map to Tasks 1-4. The 4 plan-level review corrections (confusion-
  matrix-based abstention bootstrap, removal of the flaky cross-seed CI
  test, self-identifying reports with unique filenames, runner-side
  label invariant validation) are each in their own step (Task 2 Steps
  9-12, Task 2 Step 1's replaced test, Task 3 Step 16/18, Task 3 Steps
  5a-5e respectively). `frozen-config.json`'s writing is deliberately
  NOT a task in this plan — the design itself says it's a follow-up
  after approval, so its absence here is intentional, not a gap.
- **Step scan**: Task 3 Step 3's note about `cargo test --example`
  support is the one place this plan names a real uncertainty (whether
  tests can live inside an `examples/` binary file in this project's
  Cargo setup) and gives the implementer an explicit fallback with a
  ledger instruction, rather than assuming either answer silently.
- **Type consistency**: `QueryOutcome`, `LabelRow`, `Report`,
  `ConfusionMatrixBootstrap`, `OptionalMetricResult` and their nested
  structs are defined once (in Task 2 or Task 3 as appropriate) and
  referenced by name, consistently, through later steps;
  `LOW_CONFIDENCE_FLOOR` is defined once in Task 2 and reused (not
  redefined) in Task 3; `confusion_matrix_bootstrap` replaces the
  earlier draft's `bootstrap_ci_optional_metric` everywhere it's
  referenced, with no stale mentions of the old name left in this plan.
- **Review Focus**: each of the 8 named risks has an owning test —
  Recall@10's 2-relevant-note case (Task 1 Step 1), nDCG's exponential-
  vs-linear gain distinction (Task 1 Step 5's three-case spread), the
  zero-predicted-abstain precision case (Task 2 Step 5), the bootstrap's
  `valid_resamples` accounting (Task 2 Step 9), canary exclusion from
  breakdowns (Task 3 Step 2, extended by Task 4 Step 2's structural
  sanity check), the point-estimate-matches-direct-call invariant
  guarding against the confusion-matrix bootstrap drifting from
  `confusion_matrix_metrics` (Task 2 Step 9's second test), malformed
  label rows reaching the metrics functions (Task 3 Steps 5a-5e), and
  same-day report overwrites (Task 3 Step 18's filename scheme).
- **Proportion**: four tasks, each producing a small amount of new
  production code (metric functions, a bootstrap module, one new
  example binary, two real report files) with test code roughly
  matching or exceeding the production code in volume, appropriate for
  statistics code where a wrong formula is a silent, hard-to-notice
  defect — the proportion favors tests deliberately here.
