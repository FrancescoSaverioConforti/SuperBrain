//! B3.3 pure metric functions: Recall@10, Hit@10, nDCG@10, deterministic
//! bootstrap confidence intervals, and the abstention confusion matrix.
//! No vault, search, or IO dependency — every function here takes already-
//! computed inputs (note id lists, graded relevance maps, boolean outcome
//! flags) and returns a number. `crates/api/examples/b3_metric_runner.rs`
//! is the only caller that builds a vault and calls real search.

use std::collections::HashMap;

/// splitmix64: a small, well-known, non-cryptographic PRNG. Deterministic
/// given the same seed — used only for bootstrap resampling, never for
/// anything adversarial.
pub(crate) struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    pub(crate) fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    pub(crate) fn next(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }

    /// A uniformly distributed index in `[0, bound)`.
    fn next_index(&mut self, bound: usize) -> usize {
        (self.next() % (bound as u64)) as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BootstrapCi {
    pub low: f64,
    pub high: f64,
}

fn percentile_ci(mut sorted_source: Vec<f64>) -> BootstrapCi {
    sorted_source.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = sorted_source.len();
    let low_idx = ((n as f64) * 0.025).floor() as usize;
    let high_idx = (((n as f64) * 0.975).ceil() as usize).min(n - 1);
    BootstrapCi {
        low: sorted_source[low_idx.min(n - 1)],
        high: sorted_source[high_idx],
    }
}

/// Deterministic bootstrap CI over `values` (e.g. per-query Recall@10 or
/// nDCG@10 values for one split/category/difficulty). Panics if `values`
/// is empty — callers never invoke this on an empty group.
pub fn bootstrap_ci(values: &[f64], seed: u64, resamples: u32) -> BootstrapCi {
    assert!(!values.is_empty(), "bootstrap_ci requires a nonempty values slice");
    let mut rng = SplitMix64::new(seed);
    let means: Vec<f64> = (0..resamples)
        .map(|_| {
            let sum: f64 = (0..values.len())
                .map(|_| values[rng.next_index(values.len())])
                .sum();
            sum / values.len() as f64
        })
        .collect();
    percentile_ci(means)
}

/// The `n < 5` floor below which a 2000-resample percentile CI is
/// numerically degenerate — below this, a group's metrics are reported
/// with `low_confidence: true` rather than a CI computed from too few
/// valid resamples.
pub const LOW_CONFIDENCE_FLOOR: usize = 5;

/// A binary classification confusion matrix for the abstention decision:
/// ground truth = `abstain_expected`, prediction = `search` returned
/// `NotPresent`. `fn_` because `fn` is a Rust keyword.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfusionMatrix {
    pub tp: u32,
    pub tn: u32,
    pub fp: u32,
    pub fn_: u32,
}

/// `(precision, recall, f1, accuracy)`. `precision`/`recall`/`f1` are
/// `None` — never `0.0` — exactly when their denominator is zero.
pub fn confusion_matrix_metrics(cm: &ConfusionMatrix) -> (Option<f64>, Option<f64>, Option<f64>, f64) {
    let precision = if cm.tp + cm.fp == 0 {
        None
    } else {
        Some(cm.tp as f64 / (cm.tp + cm.fp) as f64)
    };
    let recall = if cm.tp + cm.fn_ == 0 {
        None
    } else {
        Some(cm.tp as f64 / (cm.tp + cm.fn_) as f64)
    };
    let f1 = match (precision, recall) {
        (Some(p), Some(r)) if p + r > 0.0 => Some(2.0 * p * r / (p + r)),
        (Some(_), Some(_)) => Some(0.0),
        _ => None,
    };
    let total = cm.tp + cm.tn + cm.fp + cm.fn_;
    let accuracy = (cm.tp + cm.tn) as f64 / total as f64;
    (precision, recall, f1, accuracy)
}

fn tally(indices: &[usize], abstain_expected: &[bool], predicted_not_present: &[bool]) -> ConfusionMatrix {
    let mut cm = ConfusionMatrix { tp: 0, tn: 0, fp: 0, fn_: 0 };
    for &i in indices {
        match (abstain_expected[i], predicted_not_present[i]) {
            (true, true) => cm.tp += 1,
            (true, false) => cm.fn_ += 1,
            (false, true) => cm.fp += 1,
            (false, false) => cm.tn += 1,
        }
    }
    cm
}

/// One metric's point estimate (computed once over the full, un-resampled
/// data) plus its bootstrap CI, built from resamples where the metric is
/// defined.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OptionalMetricResult {
    pub point_estimate: Option<f64>,
    pub ci: Option<BootstrapCi>,
    pub valid_resamples: u32,
    pub total_resamples: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConfusionMatrixBootstrap {
    pub confusion_matrix: ConfusionMatrix,
    pub precision: OptionalMetricResult,
    pub recall: OptionalMetricResult,
    pub f1: OptionalMetricResult,
    pub accuracy: OptionalMetricResult,
}

/// Bootstraps the abstention confusion matrix: each resample draws query
/// indices with replacement, rebuilds a real `ConfusionMatrix` over just
/// those indices (via the same `tally` helper the full-data point estimate
/// uses, so the two paths can never drift apart), and calls
/// `confusion_matrix_metrics` on that resample's matrix — never a
/// separate, decoupled numerator/denominator abstraction.
pub fn confusion_matrix_bootstrap(
    abstain_expected: &[bool],
    predicted_not_present: &[bool],
    seed: u64,
    resamples: u32,
) -> ConfusionMatrixBootstrap {
    assert_eq!(abstain_expected.len(), predicted_not_present.len());
    let n = abstain_expected.len();
    let all_indices: Vec<usize> = (0..n).collect();

    let point_cm = tally(&all_indices, abstain_expected, predicted_not_present);
    let (point_precision, point_recall, point_f1, point_accuracy) = confusion_matrix_metrics(&point_cm);

    let mut rng = SplitMix64::new(seed);
    let mut precision_values = Vec::new();
    let mut recall_values = Vec::new();
    let mut f1_values = Vec::new();
    let mut accuracy_values = Vec::new();

    for _ in 0..resamples {
        let indices: Vec<usize> = (0..n).map(|_| rng.next_index(n)).collect();
        let resample_cm = tally(&indices, abstain_expected, predicted_not_present);
        let (p, r, f, a) = confusion_matrix_metrics(&resample_cm);
        if let Some(p) = p {
            precision_values.push(p);
        }
        if let Some(r) = r {
            recall_values.push(r);
        }
        if let Some(f) = f {
            f1_values.push(f);
        }
        accuracy_values.push(a);
    }

    fn build_result(
        point_estimate: Option<f64>,
        values: Vec<f64>,
        total_resamples: u32,
    ) -> OptionalMetricResult {
        let valid_resamples = values.len() as u32;
        let ci = if values.len() < LOW_CONFIDENCE_FLOOR {
            None
        } else {
            Some(percentile_ci(values))
        };
        OptionalMetricResult {
            point_estimate,
            ci,
            valid_resamples,
            total_resamples,
        }
    }

    ConfusionMatrixBootstrap {
        confusion_matrix: point_cm,
        precision: build_result(point_precision, precision_values, resamples),
        recall: build_result(point_recall, recall_values, resamples),
        f1: build_result(point_f1, f1_values, resamples),
        accuracy: build_result(Some(point_accuracy), accuracy_values, resamples),
    }
}

/// True Recall@10: the fraction of a query's relevant notes retrieved in
/// the top 10, never a 0/1 hit indicator. `relevant_note_ids` must be
/// nonempty — guaranteed by the label schema's own invariant for every
/// answerable query (B3.2's `load_labels` already enforces this), not
/// re-checked here since this is a pure function with no IO.
pub fn recall_at_10(relevant_note_ids: &[String], retrieved_note_ids: &[String]) -> f64 {
    let retrieved_count = relevant_note_ids
        .iter()
        .filter(|id| retrieved_note_ids.contains(id))
        .count();
    retrieved_count as f64 / relevant_note_ids.len() as f64
}

/// Hit@10: a diagnostic only, never reported as or substituted for
/// Recall@10. True if any relevant note appears anywhere in
/// `retrieved_note_ids`.
pub fn hit_at_10(relevant_note_ids: &[String], retrieved_note_ids: &[String]) -> bool {
    relevant_note_ids.iter().any(|id| retrieved_note_ids.contains(id))
}

/// nDCG@10, frozen formula: `gain(rel) = 2^rel - 1`,
/// `DCG@10 = Σ_{i=1}^{10} gain(rel_i) / log2(i + 1)`, IDCG from this
/// query's own `graded_relevance` values sorted descending and truncated
/// to 10. A retrieved note absent from `graded_relevance` contributes
/// `gain(0) = 0`. Only the first 10 of `retrieved_note_ids` are
/// considered, even if more are passed.
pub fn ndcg_at_10(retrieved_note_ids: &[String], graded_relevance: &HashMap<String, u8>) -> f64 {
    fn gain(rel: u8) -> f64 {
        2f64.powi(rel as i32) - 1.0
    }
    fn discounted_gain_sum(rels: impl Iterator<Item = u8>) -> f64 {
        rels.take(10)
            .enumerate()
            .map(|(i, rel)| gain(rel) / ((i as f64) + 2.0).log2())
            .sum()
    }

    let dcg = discounted_gain_sum(
        retrieved_note_ids
            .iter()
            .map(|id| graded_relevance.get(id).copied().unwrap_or(0)),
    );

    let mut ideal: Vec<u8> = graded_relevance.values().copied().collect();
    ideal.sort_unstable_by(|a, b| b.cmp(a));
    let idcg = discounted_gain_sum(ideal.into_iter());

    if idcg == 0.0 {
        0.0
    } else {
        dcg / idcg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn recall_and_ndcg_at_10_are_zero_on_an_empty_retrieved_list() {
        // Pins the NotPresent-on-an-answerable-query case: an empty
        // retrieved list must score exactly like "nothing relevant found",
        // never be excluded from aggregation (final review Important #1).
        let relevant = vec!["note-a".to_string()];
        let retrieved: Vec<String> = vec![];
        assert_eq!(recall_at_10(&relevant, &retrieved), 0.0);
        assert!(!hit_at_10(&relevant, &retrieved));

        let mut graded = HashMap::new();
        graded.insert("note-a".to_string(), 2u8);
        assert_eq!(ndcg_at_10(&retrieved, &graded), 0.0);
    }

    #[test]
    fn ndcg_at_10_matches_the_frozen_exponential_gain_formula() {
        // graded_relevance: note-a=2, note-b=1. Retrieved order: note-b, note-a (note-a is NOT first).
        let mut graded = HashMap::new();
        graded.insert("note-a".to_string(), 2u8);
        graded.insert("note-b".to_string(), 1u8);
        let retrieved = vec!["note-b".to_string(), "note-a".to_string()];
        let dcg = 1.0 + 3.0 / (3.0_f64).log2();
        let idcg = 3.0 + 1.0 / (3.0_f64).log2();
        let expected = dcg / idcg;
        assert!((ndcg_at_10(&retrieved, &graded) - expected).abs() < 1e-9);
    }

    #[test]
    fn ndcg_at_10_is_one_when_retrieved_in_ideal_order() {
        let mut graded = HashMap::new();
        graded.insert("note-a".to_string(), 2u8);
        graded.insert("note-b".to_string(), 1u8);
        let retrieved = vec!["note-a".to_string(), "note-b".to_string()];
        assert!((ndcg_at_10(&retrieved, &graded) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn ndcg_at_10_treats_an_unjudged_retrieved_note_as_zero_gain() {
        let mut graded = HashMap::new();
        graded.insert("note-a".to_string(), 2u8);
        let retrieved = vec!["note-unjudged".to_string(), "note-a".to_string()];
        let expected = (3.0 / (3.0_f64).log2()) / 3.0;
        assert!((ndcg_at_10(&retrieved, &graded) - expected).abs() < 1e-9);
    }

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
        let mut rng_a = SplitMix64::new(1);
        let mut rng_b = SplitMix64::new(2);
        let seq_a: Vec<u64> = (0..10).map(|_| rng_a.next()).collect();
        let seq_b: Vec<u64> = (0..10).map(|_| rng_b.next()).collect();
        assert_ne!(seq_a, seq_b);
    }

    #[test]
    fn confusion_matrix_metrics_computes_precision_recall_f1_accuracy() {
        let cm = ConfusionMatrix { tp: 3, tn: 4, fp: 1, fn_: 2 };
        let (precision, recall, f1, accuracy) = confusion_matrix_metrics(&cm);
        assert_eq!(precision, Some(0.75));
        assert_eq!(recall, Some(0.6));
        assert!((f1.unwrap() - (2.0 * 0.75 * 0.6 / (0.75 + 0.6))).abs() < 1e-9);
        assert_eq!(accuracy, 0.7);
    }

    #[test]
    fn confusion_matrix_metrics_precision_is_none_when_nothing_predicted_abstain() {
        let cm = ConfusionMatrix { tp: 0, tn: 5, fp: 0, fn_: 3 };
        let (precision, recall, f1, _accuracy) = confusion_matrix_metrics(&cm);
        assert_eq!(precision, None);
        assert_eq!(recall, Some(0.0));
        assert_eq!(f1, None);
    }

    #[test]
    fn confusion_matrix_metrics_recall_is_none_when_no_true_abstain_queries_exist() {
        let cm = ConfusionMatrix { tp: 0, tn: 5, fp: 2, fn_: 0 };
        let (precision, recall, f1, _accuracy) = confusion_matrix_metrics(&cm);
        assert_eq!(recall, None);
        assert_eq!(precision, Some(0.0));
        assert_eq!(f1, None);
    }

    #[test]
    fn confusion_matrix_bootstrap_reports_valid_resamples_per_metric() {
        // Only 2 of 10 queries are predicted-abstain, so a meaningful share
        // of 500 resamples draw none of them, leaving precision undefined
        // on that resample. valid_resamples must be strictly less than
        // total_resamples — asserting `<= 500` (always true) proves
        // nothing (final review Important #2); this must actually catch a
        // regression that treats an undefined 0/0 resample as 0.0 instead
        // of skipping it.
        let abstain_expected = vec![true, true, true, false, false, false, false, false, false, false];
        let predicted_not_present = vec![true, true, false, false, false, false, false, false, false, false];
        let result = confusion_matrix_bootstrap(&abstain_expected, &predicted_not_present, 123, 500);
        assert!(
            result.precision.valid_resamples < result.precision.total_resamples,
            "expected some resamples to leave precision undefined, got {}/{}",
            result.precision.valid_resamples,
            result.precision.total_resamples
        );
        assert_eq!(result.accuracy.valid_resamples, 500);
    }

    #[test]
    fn confusion_matrix_bootstrap_ci_is_none_when_precision_is_never_defined() {
        // All queries are predicted-NOT-abstain (predicted_not_present all
        // false), so TP+FP = 0 on every possible resample: precision is
        // undefined everywhere, valid_resamples must be exactly 0, and the
        // CI must be None, not computed from zero values.
        let abstain_expected = vec![true, true, false, false, false];
        let predicted_not_present = vec![false, false, false, false, false];
        let result = confusion_matrix_bootstrap(&abstain_expected, &predicted_not_present, 7, 500);
        assert_eq!(result.precision.valid_resamples, 0);
        assert!(result.precision.ci.is_none());
        assert_eq!(result.precision.point_estimate, None);
    }

    #[test]
    fn confusion_matrix_bootstrap_point_estimates_match_confusion_matrix_metrics() {
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
}
