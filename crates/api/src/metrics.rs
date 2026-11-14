//! B3.3 pure metric functions: Recall@10, Hit@10, nDCG@10, deterministic
//! bootstrap confidence intervals, and the abstention confusion matrix.
//! No vault, search, or IO dependency — every function here takes already-
//! computed inputs (note id lists, graded relevance maps, boolean outcome
//! flags) and returns a number. `crates/api/examples/b3_metric_runner.rs`
//! is the only caller that builds a vault and calls real search.

use std::collections::HashMap;

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
}
