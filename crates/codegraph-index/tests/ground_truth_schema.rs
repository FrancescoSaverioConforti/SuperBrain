//! Quality & Semantic Reliability Hardening, Phase 2 — retrofits one
//! REAL past evaluation's own already-confirmed ground truth
//! (B2-EVAL-008's Section C, `Extends`/`Implements` edges touching
//! `BulkheadPolicy`/`IBulkheadPolicy`/`Policy`) into the
//! `docs/research/GROUND_TRUTH_SCHEMA.md` format, per that design
//! document's own explicit requirement: "retrofit at least one PAST
//! evaluation's own existing prose sample into this format to prove
//! the schema is actually sufficient in practice, not just on paper."
//!
//! This is NOT a synthetic fixture — every row in
//! `ground_truth_fixtures/b2-eval-008-extends-implements.jsonl` is
//! derived directly from `docs/research/evaluations/B2-EVAL-008.md`'s
//! own Section C, which itself documents these facts as confirmed by
//! direct source reading against the real `App-vNext/Polly` corpus.
//!
//! Row 5 (added Phase 2, deliverable 9) retrofits the generic-arity-
//! collision gap (`Known Semantic Gaps` #5) as an `Ambiguous` row,
//! cross-checked against the real resolver's own output in
//! `golden_corpus.rs`'s
//! `golden_corpus_ground_truth_and_the_real_resolver_agree_on_the_generic_arity_collision_case`.

use codegraph_index::ground_truth::{compute_metrics, parse_jsonl, EvidenceTierField, Status};

#[test]
fn the_real_b2_eval_008_fixture_parses_without_error() {
    let contents = std::fs::read_to_string("tests/ground_truth_fixtures/b2-eval-008-extends-implements.jsonl").expect("fixture file must exist and be readable");
    let rows = parse_jsonl(&contents).expect("a real, well-formed ground-truth fixture must parse cleanly");
    assert_eq!(rows.len(), 5, "the fixture has 5 rows: 4 retrofitted from B2-EVAL-008's own Section C, plus 1 Ambiguous row added in Phase 2 deliverable 9");
}

#[test]
fn the_generic_arity_collision_row_computes_as_ambiguous_correct() {
    let contents = std::fs::read_to_string("tests/ground_truth_fixtures/b2-eval-008-extends-implements.jsonl").unwrap();
    let rows = parse_jsonl(&contents).unwrap();
    let row = rows
        .iter()
        .find(|r| r.resolution_method == Some(EvidenceTierField::Ambiguous))
        .expect("the one Ambiguous row (generic-arity collision) must be present in the real fixture");
    assert_eq!(row.caller, "src/Polly/Bulkhead/BulkheadPolicy.cs::BulkheadPolicy");
    assert_eq!(row.expected_targets.len(), 2, "two equally-valid real candidates sharing one bare name, per the generic-arity-collision gap");
    assert!(row.predicted_targets.is_empty(), "the real resolver correctly declines to guess between them");
    assert_eq!(row.compute_status(), Status::AmbiguousCorrect);
}

#[test]
fn every_row_in_the_real_fixture_shares_the_same_evaluation_and_configuration() {
    let contents = std::fs::read_to_string("tests/ground_truth_fixtures/b2-eval-008-extends-implements.jsonl").unwrap();
    let rows = parse_jsonl(&contents).unwrap();
    assert!(rows.iter().all(|r| r.evaluation_id == "B2-EVAL-008"), "all rows must share one evaluation_id -- the schema's own rule against mixing incomparable samples");
    assert!(rows.iter().all(|r| r.configuration_id == "shipped-post-B2-ID-010"), "all rows must share one configuration_id");
    assert!(rows.iter().all(|r| r.commit == "1f232caf9672e515751ba2df76d82bfbbb62e016"), "all rows must reference the SAME real, cloneable commit this evaluation actually measured against");
}

#[test]
fn the_real_bulkheadpolicy_to_policy_row_computes_as_a_true_positive() {
    let contents = std::fs::read_to_string("tests/ground_truth_fixtures/b2-eval-008-extends-implements.jsonl").unwrap();
    let rows = parse_jsonl(&contents).unwrap();
    let bulkhead_row = rows.iter().find(|r| r.caller == "src/Polly/Bulkhead/BulkheadPolicy.cs::BulkheadPolicy").expect("the headline B2-ID-010 case must be present in the real fixture");
    assert_eq!(bulkhead_row.compute_status(), Status::TruePositive, "BulkheadPolicy -> Policy is a real, confirmed, correctly-resolved relationship per B2-EVAL-008's own Section C");
}

#[test]
fn the_other_real_policy_extending_classes_also_compute_as_true_positives() {
    let contents = std::fs::read_to_string("tests/ground_truth_fixtures/b2-eval-008-extends-implements.jsonl").unwrap();
    let rows = parse_jsonl(&contents).unwrap();
    let timeout_row = rows.iter().find(|r| r.caller.contains("TimeoutPolicy")).expect("one of the 8 other real classes B2-EVAL-008 confirmed must be present");
    assert_eq!(timeout_row.compute_status(), Status::TruePositive);
    let policy_row = rows.iter().find(|r| r.caller == "src/Polly/Policy.cs::Policy").expect("Policy itself extending PolicyBase must be present");
    assert_eq!(policy_row.compute_status(), Status::TruePositive);
}

#[test]
fn computing_metrics_on_this_real_but_undersized_sample_reports_raw_counts_only() {
    // 4 real rows is far below MIN_ROWS_FOR_COMPUTED_METRICS -- this is
    // the schema's own "never compute a percentage from an under-sized
    // sample" rule, demonstrated against genuinely real (not
    // artificially padded) data.
    let contents = std::fs::read_to_string("tests/ground_truth_fixtures/b2-eval-008-extends-implements.jsonl").unwrap();
    let rows = parse_jsonl(&contents).unwrap();
    let metrics = compute_metrics(&rows);
    assert!(metrics.precision.is_none(), "4 real rows must not produce a computed precision percentage");
    assert!(metrics.true_positives >= 3, "at least the 3 unambiguous TruePositive rows must still be counted raw, even without a computed percentage");
}
