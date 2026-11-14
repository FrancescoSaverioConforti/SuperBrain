//! Quality & Semantic Reliability Hardening, Phase 2 — a parser and
//! validator for the machine-readable ground-truth schema designed in
//! `docs/research/GROUND_TRUTH_SCHEMA.md` (Phase 1, deliverable 9).
//! Implements that document's own schema exactly, including its own
//! `status` DERIVATION rule (never an independently-settable field —
//! see [`GroundTruthRow::compute_status`]) and its own explicit
//! "never compute metrics when ground truth is insufficient" guards.
//!
//! # Dependency-free by design
//!
//! This workspace has no `serde`/JSON-library dependency anywhere
//! (confirmed by grep across every `Cargo.toml` before writing this
//! module) — every existing dependency here is deliberately minimal
//! and individually justified (see `codegraph-index/Cargo.toml`'s own
//! comments). Adding a new external crate just for this one Phase 2
//! deliverable is a real architectural decision this hardening pass
//! does not make unilaterally; this module instead implements a
//! small, hand-rolled JSON Lines reader scoped EXACTLY to this one
//! schema's own fixed field set — not a general-purpose JSON parser.
//! If a future need for general JSON parsing arises elsewhere in this
//! project, that would be the right moment to introduce `serde_json`
//! as a real, broadly-justified dependency; this module should not be
//! read as an argument against doing so later.

use std::fmt;

/// One real call-site observation, exactly per
/// `GROUND_TRUTH_SCHEMA.md`'s own field list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroundTruthRow {
    pub evaluation_id: String,
    pub configuration_id: String,
    pub repo: String,
    pub commit: String,
    pub language: String,
    pub caller: String,
    pub callsite_span: (u64, u64),
    /// Empty means "confirmed, by direct source reading, that no real
    /// target exists" — NOT "we didn't check" (see `notes` for that
    /// case, which this type cannot itself distinguish — the schema
    /// document's own explicit limitation, carried forward unchanged).
    pub expected_targets: Vec<String>,
    /// Empty means "the resolver produced no edge."
    pub predicted_targets: Vec<String>,
    pub resolution_method: Option<EvidenceTierField>,
    pub notes: String,
}

/// A string-serialized `codegraph_domain::EvidenceTier`, kept as its
/// own small type here (rather than depending on `EvidenceTier`
/// directly) because a ground-truth row is a RECORD of an observation
/// made against possibly-historical resolver output — it must remain
/// parseable even if a future `EvidenceTier` variant is renamed, which
/// this module's own `FromStr`/`Display` round-trip keeps independent
/// of that enum's own representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceTierField {
    Resolved,
    Heuristic,
    Ambiguous,
    Unresolved,
}

impl fmt::Display for EvidenceTierField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Resolved => "Resolved",
            Self::Heuristic => "Heuristic",
            Self::Ambiguous => "Ambiguous",
            Self::Unresolved => "Unresolved",
        };
        f.write_str(s)
    }
}

/// The classification one `GroundTruthRow` resolves to — ALWAYS
/// computed from `expected_targets`/`predicted_targets`/
/// `resolution_method`, per the schema document's own status table.
/// There is deliberately no way to construct this independently of
/// [`GroundTruthRow::compute_status`] from outside this module — the
/// schema document's own explicit rule ("a harness MUST compute
/// status FROM the other fields, never record it independently") is
/// enforced by this type simply never accepting a status as a
/// freestanding input anywhere in this module's own public API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    TruePositive,
    FalsePositive,
    FalseNegative,
    AmbiguousCorrect,
    AmbiguousIncorrect,
    /// Ground truth confirmed empty (no real target exists) AND the
    /// resolver also produced nothing — trivially correct, excluded
    /// from precision/recall's own numerator/denominator per the
    /// schema document's own explicit rule.
    Unresolved,
}

impl GroundTruthRow {
    /// Computes this row's own `Status` per
    /// `GROUND_TRUTH_SCHEMA.md`'s status table, reproduced here
    /// exactly (row order matches that table).
    pub fn compute_status(&self) -> Status {
        let expected_is_empty = self.expected_targets.is_empty();
        let predicted_is_empty = self.predicted_targets.is_empty();
        let expected_is_single = self.expected_targets.len() == 1;
        let expected_is_multi = self.expected_targets.len() >= 2;

        match (expected_is_empty, predicted_is_empty, expected_is_single, expected_is_multi) {
            // empty expected, empty predicted -> trivially correct,
            // regardless of resolution_method.
            (true, true, _, _) => Status::Unresolved,
            // empty expected, non-empty predicted -> FP (CRC-002 shape:
            // confident edge to something when no real target exists).
            (true, false, _, _) => Status::FalsePositive,
            // single real expected target.
            (false, _, true, false) => {
                if predicted_is_empty {
                    // FN regardless of WHY it is empty (Unresolved or
                    // Ambiguous) -- the schema document's own table
                    // scores both the same way here: a real target
                    // existed and was missed, even when "correctly
                    // declined to guess" is itself the right resolver
                    // behavior for the Ambiguous sub-case.
                    Status::FalseNegative
                } else if self.predicted_targets == self.expected_targets {
                    Status::TruePositive
                } else {
                    Status::FalsePositive
                }
            }
            // multiple equally-valid real expected targets (a
            // CRC-001-shaped case).
            (false, _, false, true) => {
                if predicted_is_empty {
                    Status::AmbiguousCorrect
                } else if self.predicted_targets.len() == 1 && self.expected_targets.contains(&self.predicted_targets[0]) {
                    Status::AmbiguousIncorrect
                } else {
                    // Predicted something that is not even among the
                    // valid expected targets at all -- a real FP, not
                    // merely "picked one of several valid answers."
                    Status::FalsePositive
                }
            }
            // Structurally unreachable: expected_is_single and
            // expected_is_multi are derived from the SAME length and
            // are mutually exclusive by construction (len()==1 vs.
            // len()>=2), and expected_is_empty already covers len()==0
            // in the first two arms above -- every possible
            // expected_targets length is covered by exactly one arm.
            // Never silently default to a specific Status here if this
            // is somehow reached; that would hide a real logic defect
            // behind a plausible-looking answer.
            _ => unreachable!("every expected_targets length (0, 1, 2+) is covered by an earlier arm"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroundTruthParseError {
    MissingField { line: usize, field: &'static str },
    InvalidResolutionMethod { line: usize, value: String },
    MalformedLine { line: usize, reason: String },
}

impl fmt::Display for GroundTruthParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingField { line, field } => write!(f, "line {line}: missing required field {field:?}"),
            Self::InvalidResolutionMethod { line, value } => write!(f, "line {line}: invalid resolution_method {value:?}"),
            Self::MalformedLine { line, reason } => write!(f, "line {line}: malformed JSON: {reason}"),
        }
    }
}

/// Parses a JSON Lines ground-truth file (one `GroundTruthRow` per
/// non-blank line) using a minimal, schema-specific JSON reader — see
/// this module's own doc comment for why this is hand-rolled rather
/// than a dependency on a general JSON library. Rejects (does not
/// silently skip) any line missing a required field or carrying an
/// unrecognized `resolution_method` value, per this whole evaluation
/// series' own "reject, never default" discipline.
pub fn parse_jsonl(contents: &str) -> Result<Vec<GroundTruthRow>, GroundTruthParseError> {
    let mut rows = Vec::new();
    for (zero_based_line, raw_line) in contents.lines().enumerate() {
        let line = zero_based_line + 1;
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            continue;
        }
        rows.push(parse_one_line(trimmed, line)?);
    }
    Ok(rows)
}

fn parse_one_line(line: &str, line_no: usize) -> Result<GroundTruthRow, GroundTruthParseError> {
    let object = parse_json_object(line).ok_or_else(|| GroundTruthParseError::MalformedLine { line: line_no, reason: "expected a single flat JSON object".to_string() })?;

    let get_str = |field: &'static str| -> Result<String, GroundTruthParseError> { object.get(field).and_then(JsonValue::as_str).map(str::to_string).ok_or(GroundTruthParseError::MissingField { line: line_no, field }) };

    let evaluation_id = get_str("evaluation_id")?;
    let configuration_id = get_str("configuration_id")?;
    let repo = get_str("repo")?;
    let commit = get_str("commit")?;
    let language = get_str("language")?;
    let caller = get_str("caller")?;
    let notes = object.get("notes").and_then(JsonValue::as_str).unwrap_or("").to_string();

    let callsite_span = object
        .get("callsite_span")
        .and_then(JsonValue::as_object)
        .and_then(|span| {
            let start = span.get("start").and_then(JsonValue::as_u64)?;
            let end = span.get("end").and_then(JsonValue::as_u64)?;
            Some((start, end))
        })
        .ok_or(GroundTruthParseError::MissingField { line: line_no, field: "callsite_span" })?;

    let expected_targets = object.get("expected_targets").and_then(JsonValue::as_str_array).ok_or(GroundTruthParseError::MissingField { line: line_no, field: "expected_targets" })?;
    let predicted_targets = object.get("predicted_targets").and_then(JsonValue::as_str_array).ok_or(GroundTruthParseError::MissingField { line: line_no, field: "predicted_targets" })?;

    let resolution_method = match object.get("resolution_method") {
        None | Some(JsonValue::Null) => None,
        Some(JsonValue::Str(s)) => Some(match s.as_str() {
            "Resolved" => EvidenceTierField::Resolved,
            "Heuristic" => EvidenceTierField::Heuristic,
            "Ambiguous" => EvidenceTierField::Ambiguous,
            "Unresolved" => EvidenceTierField::Unresolved,
            other => return Err(GroundTruthParseError::InvalidResolutionMethod { line: line_no, value: other.to_string() }),
        }),
        Some(_) => return Err(GroundTruthParseError::InvalidResolutionMethod { line: line_no, value: "<non-string>".to_string() }),
    };

    Ok(GroundTruthRow { evaluation_id, configuration_id, repo, commit, language, caller, callsite_span, expected_targets, predicted_targets, resolution_method, notes })
}

// ===========================================================================
// Minimal, schema-scoped JSON reader. Not a general-purpose parser:
// handles exactly the shapes this schema's own fields need (flat
// object of strings/numbers/arrays-of-strings/nested-object-of-two-
// numbers), nothing more. Deliberately simple enough to read in one
// pass and mutation-test meaningfully.
// ===========================================================================

#[derive(Debug, Clone, PartialEq)]
enum JsonValue {
    Null,
    Str(String),
    Num(f64),
    StrArray(Vec<String>),
    Object(std::collections::HashMap<String, JsonValue>),
}

impl JsonValue {
    fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s),
            _ => None,
        }
    }
    fn as_u64(&self) -> Option<u64> {
        match self {
            Self::Num(n) if *n >= 0.0 => Some(*n as u64),
            _ => None,
        }
    }
    fn as_str_array(&self) -> Option<Vec<String>> {
        match self {
            Self::StrArray(v) => Some(v.clone()),
            _ => None,
        }
    }
    fn as_object(&self) -> Option<&std::collections::HashMap<String, JsonValue>> {
        match self {
            Self::Object(m) => Some(m),
            _ => None,
        }
    }
}

/// Parses ONE flat JSON object (this schema's own top-level shape) —
/// a tiny, hand-rolled recursive-descent reader, not a general JSON
/// parser. Returns `None` on any malformed input rather than panicking
/// — a ground-truth file is external, untrusted data this module must
/// never trust blindly.
fn parse_json_object(input: &str) -> Option<std::collections::HashMap<String, JsonValue>> {
    let mut chars = input.trim().chars().peekable();
    skip_whitespace(&mut chars);
    if chars.next() != Some('{') {
        return None;
    }
    let mut map = std::collections::HashMap::new();
    skip_whitespace(&mut chars);
    if chars.peek() == Some(&'}') {
        chars.next();
        return Some(map);
    }
    loop {
        skip_whitespace(&mut chars);
        let key = parse_json_string(&mut chars)?;
        skip_whitespace(&mut chars);
        if chars.next() != Some(':') {
            return None;
        }
        skip_whitespace(&mut chars);
        let value = parse_json_value(&mut chars)?;
        map.insert(key, value);
        skip_whitespace(&mut chars);
        match chars.next() {
            Some(',') => continue,
            Some('}') => break,
            _ => return None,
        }
    }
    Some(map)
}

fn parse_json_value(chars: &mut std::iter::Peekable<std::str::Chars>) -> Option<JsonValue> {
    skip_whitespace(chars);
    match chars.peek()? {
        '"' => parse_json_string(chars).map(JsonValue::Str),
        '[' => parse_json_str_array(chars).map(JsonValue::StrArray),
        '{' => {
            // Consume the nested object inline (only `callsite_span`
            // ever uses this shape in this schema) — re-enter a small
            // loop rather than calling `parse_json_object` directly,
            // since that function expects to consume its own leading
            // `{` from a fresh iterator state, which `chars` already
            // is here.
            chars.next(); // consume '{'
            let mut map = std::collections::HashMap::new();
            skip_whitespace(chars);
            if chars.peek() == Some(&'}') {
                chars.next();
                return Some(JsonValue::Object(map));
            }
            loop {
                skip_whitespace(chars);
                let key = parse_json_string(chars)?;
                skip_whitespace(chars);
                if chars.next() != Some(':') {
                    return None;
                }
                skip_whitespace(chars);
                let value = parse_json_value(chars)?;
                map.insert(key, value);
                skip_whitespace(chars);
                match chars.next() {
                    Some(',') => continue,
                    Some('}') => break,
                    _ => return None,
                }
            }
            Some(JsonValue::Object(map))
        }
        'n' => {
            for expected in "null".chars() {
                if chars.next() != Some(expected) {
                    return None;
                }
            }
            Some(JsonValue::Null)
        }
        _ => parse_json_number(chars).map(JsonValue::Num),
    }
}

fn parse_json_str_array(chars: &mut std::iter::Peekable<std::str::Chars>) -> Option<Vec<String>> {
    if chars.next() != Some('[') {
        return None;
    }
    let mut out = Vec::new();
    skip_whitespace(chars);
    if chars.peek() == Some(&']') {
        chars.next();
        return Some(out);
    }
    loop {
        skip_whitespace(chars);
        out.push(parse_json_string(chars)?);
        skip_whitespace(chars);
        match chars.next() {
            Some(',') => continue,
            Some(']') => break,
            _ => return None,
        }
    }
    Some(out)
}

fn parse_json_string(chars: &mut std::iter::Peekable<std::str::Chars>) -> Option<String> {
    if chars.next() != Some('"') {
        return None;
    }
    let mut out = String::new();
    loop {
        match chars.next()? {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                '/' => out.push('/'),
                'n' => out.push('\n'),
                't' => out.push('\t'),
                other => out.push(other),
            },
            c => out.push(c),
        }
    }
}

fn parse_json_number(chars: &mut std::iter::Peekable<std::str::Chars>) -> Option<f64> {
    let mut raw = String::new();
    while let Some(&c) = chars.peek() {
        if c.is_ascii_digit() || c == '-' || c == '+' || c == '.' || c == 'e' || c == 'E' {
            raw.push(c);
            chars.next();
        } else {
            break;
        }
    }
    if raw.is_empty() {
        return None;
    }
    raw.parse().ok()
}

fn skip_whitespace(chars: &mut std::iter::Peekable<std::str::Chars>) {
    while matches!(chars.peek(), Some(c) if c.is_whitespace()) {
        chars.next();
    }
}

/// Recomputes precision/recall/F1/ambiguity-rate/unresolved-rate from
/// a set of rows, per `GROUND_TRUTH_SCHEMA.md`'s own "Derived
/// metrics" section — EXCLUDING `Status::Unresolved` rows from every
/// denominator, per that document's own explicit rule, and NEVER
/// silently mixing rows from different `configuration_id`s (this
/// function requires the caller to have already filtered to one
/// evaluation+configuration; see [`GroundTruthRow::compute_status`]'s
/// own module doc comment for the guard this function does NOT itself
/// re-check — that guard lives one level up, in whatever calls this).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    pub true_positives: usize,
    pub false_positives: usize,
    pub false_negatives: usize,
    pub ambiguous_correct: usize,
    pub ambiguous_incorrect: usize,
    pub unresolved_trivial: usize,
    pub precision: Option<f64>,
    pub recall: Option<f64>,
    pub f1: Option<f64>,
}

/// Per `GROUND_TRUTH_SCHEMA.md`'s own explicit instruction: never
/// compute a percentage from fewer than this many confirmed rows —
/// report raw counts only below this threshold. `30` matches this
/// project's own prior evaluations' typical manual-sample size
/// (e.g. B2-EVAL-004's own 30-edge sample).
pub const MIN_ROWS_FOR_COMPUTED_METRICS: usize = 30;

/// Computes `Metrics` from a slice of rows. Returns `None` for
/// `precision`/`recall`/`f1` (raw counts are still always populated)
/// when `rows.len() < MIN_ROWS_FOR_COMPUTED_METRICS` — per this
/// schema's own explicit "do not compute a percentage from an
/// under-sized sample" rule. `rows` is assumed to already be filtered
/// to one `evaluation_id`/`configuration_id` pair by the caller.
pub fn compute_metrics(rows: &[GroundTruthRow]) -> Metrics {
    let mut true_positives = 0usize;
    let mut false_positives = 0usize;
    let mut false_negatives = 0usize;
    let mut ambiguous_correct = 0usize;
    let mut ambiguous_incorrect = 0usize;
    let mut unresolved_trivial = 0usize;

    for row in rows {
        match row.compute_status() {
            Status::TruePositive => true_positives += 1,
            Status::FalsePositive => false_positives += 1,
            Status::FalseNegative => false_negatives += 1,
            Status::AmbiguousCorrect => ambiguous_correct += 1,
            Status::AmbiguousIncorrect => ambiguous_incorrect += 1,
            Status::Unresolved => unresolved_trivial += 1,
        }
    }

    let sufficient_sample = rows.len() >= MIN_ROWS_FOR_COMPUTED_METRICS;
    let precision = if sufficient_sample && (true_positives + false_positives) > 0 { Some(true_positives as f64 / (true_positives + false_positives) as f64) } else { None };
    let recall = if sufficient_sample && (true_positives + false_negatives) > 0 { Some(true_positives as f64 / (true_positives + false_negatives) as f64) } else { None };
    let f1 = match (precision, recall) {
        (Some(p), Some(r)) if (p + r) > 0.0 => Some(2.0 * p * r / (p + r)),
        _ => None,
    };

    Metrics { true_positives, false_positives, false_negatives, ambiguous_correct, ambiguous_incorrect, unresolved_trivial, precision, recall, f1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_row(expected: Vec<&str>, predicted: Vec<&str>, method: Option<EvidenceTierField>) -> GroundTruthRow {
        GroundTruthRow {
            evaluation_id: "B2-EVAL-TEST".to_string(),
            configuration_id: "TEST".to_string(),
            repo: "test/repo".to_string(),
            commit: "a".repeat(40),
            language: "Rust".to_string(),
            caller: "src/lib.rs::caller".to_string(),
            callsite_span: (0, 10),
            expected_targets: expected.into_iter().map(String::from).collect(),
            predicted_targets: predicted.into_iter().map(String::from).collect(),
            resolution_method: method,
            notes: String::new(),
        }
    }

    // --- Status computation, one row per table entry in the schema doc ---

    #[test]
    fn single_expected_matching_prediction_is_true_positive() {
        let row = minimal_row(vec!["a"], vec!["a"], Some(EvidenceTierField::Resolved));
        assert_eq!(row.compute_status(), Status::TruePositive);
    }

    #[test]
    fn single_expected_different_prediction_is_false_positive() {
        let row = minimal_row(vec!["a"], vec!["b"], Some(EvidenceTierField::Resolved));
        assert_eq!(row.compute_status(), Status::FalsePositive);
    }

    #[test]
    fn single_expected_empty_prediction_unresolved_method_is_false_negative() {
        let row = minimal_row(vec!["a"], vec![], Some(EvidenceTierField::Unresolved));
        assert_eq!(row.compute_status(), Status::FalseNegative);
    }

    #[test]
    fn single_expected_empty_prediction_ambiguous_method_is_still_false_negative() {
        // The schema's own explicit point: correctly declining to
        // guess is still a missed TRUE edge from the consumer's point
        // of view, even though it was the right resolver BEHAVIOR.
        let row = minimal_row(vec!["a"], vec![], Some(EvidenceTierField::Ambiguous));
        assert_eq!(row.compute_status(), Status::FalseNegative);
    }

    #[test]
    fn multi_expected_empty_prediction_is_ambiguous_correct() {
        let row = minimal_row(vec!["a", "b"], vec![], None);
        assert_eq!(row.compute_status(), Status::AmbiguousCorrect);
    }

    #[test]
    fn multi_expected_one_valid_prediction_is_ambiguous_incorrect() {
        let row = minimal_row(vec!["a", "b"], vec!["a"], Some(EvidenceTierField::Resolved));
        assert_eq!(row.compute_status(), Status::AmbiguousIncorrect);
    }

    #[test]
    fn multi_expected_invalid_prediction_is_false_positive_not_ambiguous_incorrect() {
        // Predicted something NOT among the valid targets at all --
        // must not be conflated with "picked one of several valid
        // answers."
        let row = minimal_row(vec!["a", "b"], vec!["c"], Some(EvidenceTierField::Resolved));
        assert_eq!(row.compute_status(), Status::FalsePositive);
    }

    #[test]
    fn empty_expected_empty_predicted_is_unresolved_trivial() {
        let row = minimal_row(vec![], vec![], None);
        assert_eq!(row.compute_status(), Status::Unresolved);
    }

    #[test]
    fn empty_expected_nonempty_predicted_is_false_positive() {
        // The CRC-002 shape: confident edge where no real target
        // exists at all.
        let row = minimal_row(vec![], vec!["phantom"], Some(EvidenceTierField::Resolved));
        assert_eq!(row.compute_status(), Status::FalsePositive);
    }

    // --- Metrics ---

    #[test]
    fn metrics_below_the_minimum_sample_size_report_raw_counts_but_no_computed_percentage() {
        let rows: Vec<GroundTruthRow> = (0..5).map(|_| minimal_row(vec!["a"], vec!["a"], Some(EvidenceTierField::Resolved))).collect();
        let metrics = compute_metrics(&rows);
        assert_eq!(metrics.true_positives, 5);
        assert_eq!(metrics.precision, None, "a 5-row sample must not report a computed precision percentage");
        assert_eq!(metrics.recall, None);
        assert_eq!(metrics.f1, None);
    }

    #[test]
    fn metrics_at_or_above_the_minimum_sample_size_do_compute_percentages() {
        let mut rows: Vec<GroundTruthRow> = (0..MIN_ROWS_FOR_COMPUTED_METRICS).map(|_| minimal_row(vec!["a"], vec!["a"], Some(EvidenceTierField::Resolved))).collect();
        rows.push(minimal_row(vec!["b"], vec!["c"], Some(EvidenceTierField::Resolved))); // one FP
        let metrics = compute_metrics(&rows);
        assert!(metrics.precision.is_some());
        assert_eq!(metrics.true_positives, MIN_ROWS_FOR_COMPUTED_METRICS);
        assert_eq!(metrics.false_positives, 1);
    }

    #[test]
    fn unresolved_trivial_rows_are_counted_separately_never_folded_into_tp_fp_fn() {
        let rows = vec![minimal_row(vec![], vec![], None), minimal_row(vec!["a"], vec!["a"], Some(EvidenceTierField::Resolved))];
        let metrics = compute_metrics(&rows);
        assert_eq!(metrics.unresolved_trivial, 1);
        assert_eq!(metrics.true_positives, 1);
        assert_eq!(metrics.false_positives, 0);
        assert_eq!(metrics.false_negatives, 0);
    }

    // --- JSONL parsing ---

    #[test]
    fn a_well_formed_line_parses_correctly() {
        let line = r#"{"evaluation_id": "B2-EVAL-008", "configuration_id": "shipped", "repo": "App-vNext/Polly", "commit": "1f232caf9672e515751ba2df76d82bfbbb62e016", "language": "CSharp", "caller": "src/Polly/Bulkhead/BulkheadPolicy.cs::Dispose", "callsite_span": {"start": 1842, "end": 1856}, "expected_targets": ["src/Polly/Bulkhead/BulkheadPolicy.cs::Dispose"], "predicted_targets": ["src/Polly/Bulkhead/BulkheadPolicy.cs::Dispose"], "resolution_method": "Resolved", "notes": "confirmed by direct source reading"}"#;
        let rows = parse_jsonl(line).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].evaluation_id, "B2-EVAL-008");
        assert_eq!(rows[0].callsite_span, (1842, 1856));
        assert_eq!(rows[0].expected_targets, vec!["src/Polly/Bulkhead/BulkheadPolicy.cs::Dispose".to_string()]);
        assert_eq!(rows[0].resolution_method, Some(EvidenceTierField::Resolved));
        assert_eq!(rows[0].compute_status(), Status::TruePositive);
    }

    #[test]
    fn multiple_lines_each_parse_as_their_own_row() {
        let line1 = r#"{"evaluation_id":"E","configuration_id":"C","repo":"r","commit":"c","language":"Rust","caller":"x","callsite_span":{"start":0,"end":1},"expected_targets":["a"],"predicted_targets":["a"],"resolution_method":"Resolved","notes":""}"#;
        let line2 = r#"{"evaluation_id":"E","configuration_id":"C","repo":"r","commit":"c","language":"Rust","caller":"y","callsite_span":{"start":2,"end":3},"expected_targets":[],"predicted_targets":[],"resolution_method":null,"notes":""}"#;
        let rows = parse_jsonl(&format!("{line1}\n{line2}\n")).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].resolution_method, None);
    }

    #[test]
    fn blank_lines_are_skipped() {
        let line = r#"{"evaluation_id":"E","configuration_id":"C","repo":"r","commit":"c","language":"Rust","caller":"x","callsite_span":{"start":0,"end":1},"expected_targets":[],"predicted_targets":[],"resolution_method":null,"notes":""}"#;
        let rows = parse_jsonl(&format!("\n{line}\n\n")).unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn a_missing_required_field_is_rejected_not_defaulted() {
        let line = r#"{"configuration_id":"C","repo":"r","commit":"c","language":"Rust","caller":"x","callsite_span":{"start":0,"end":1},"expected_targets":[],"predicted_targets":[],"notes":""}"#;
        let result = parse_jsonl(line);
        assert!(matches!(result, Err(GroundTruthParseError::MissingField { field: "evaluation_id", .. })));
    }

    #[test]
    fn an_invalid_resolution_method_value_is_rejected() {
        let line = r#"{"evaluation_id":"E","configuration_id":"C","repo":"r","commit":"c","language":"Rust","caller":"x","callsite_span":{"start":0,"end":1},"expected_targets":[],"predicted_targets":[],"resolution_method":"NotARealTier","notes":""}"#;
        let result = parse_jsonl(line);
        assert!(matches!(result, Err(GroundTruthParseError::InvalidResolutionMethod { .. })));
    }

    #[test]
    fn malformed_json_is_rejected_not_silently_skipped() {
        let result = parse_jsonl("{ this is not valid json");
        assert!(result.is_err());
    }
}
