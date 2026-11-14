//! B2 Task 4: Python symbol extraction, node kinds confirmed against a
//! real parse. `class_definition` and `function_definition` both name
//! via a direct `identifier` child; a method is a `function_definition`
//! nested inside a `class_definition`'s `block` — the SAME node kind as
//! a top-level function in this grammar, so `kind_hint` does not itself
//! distinguish "method" from "free function" (consistent with Rust's
//! `function_item` not distinguishing methods from free functions
//! either).
//!
//! # Task 9.5: `Field` extraction (`self.X = <expr>`)
//!
//! Confirmed against a real parse (`class M: def __init__(self):
//! self.encoder = TransformerEncoder(a, b)`): the shape is
//! `expression_statement -> assignment -> (attribute, "=", <rhs>)`,
//! where `attribute`'s own children are `(identifier "self", ".",
//! identifier "<field_name>")`. This extractor recognizes EXACTLY that
//! shape — an `assignment` node whose LEFT child is an `attribute` node
//! whose OWN first child is the literal `identifier` text `"self"` —
//! and extracts the attribute's own name (the identifier after the
//! `.`) as a `Field` candidate, with the synthetic `kind_hint`
//! `"self_attribute_assignment"` (no single raw tree-sitter node kind
//! names this construct on its own, since `assignment` is used for
//! every assignment statement in this grammar — same precedent as
//! HTML's `element_with_id`).
//!
//! Deliberately conservative, per the B2 plan's section 8b policy and
//! the real-repository evidence (`docs/research/evaluations/B2-EVAL-001.md`)
//! that motivated this addition:
//!
//! - The right-hand side's shape does NOT matter — `self.encoder =
//!   TransformerEncoder(...)`, `self.ninp = ninp`, and `self.enabled =
//!   False` are ALL `Field`s. Field extraction answers "is this an
//!   explicit instance-attribute assignment," never "does this look
//!   architecturally interesting." (Whether a `Field`'s value is itself
//!   a resolvable constructor call — and therefore also produces an
//!   `Instantiates` edge — is a SEPARATE, later resolution step, not
//!   this extractor's concern.)
//! - A plain local variable (`x = i`), a loop variable (`for i in
//!   ...`), or any assignment whose LEFT side is a bare `identifier`
//!   (not an `attribute`) is never a `Field` — the AST shape itself
//!   excludes these; no name-based heuristic is used anywhere in this
//!   rule.
//! - An `attribute` assignment whose receiver is NOT literally `self`
//!   (e.g. `other.x = 1`, `cls.y = 2`) is never a `Field` here — this
//!   extractor recognizes only the `self`-receiver instance-attribute
//!   shape; a `classmethod`'s `cls.x = ...` or any other receiver is a
//!   separate, NOT-yet-evidence-driven case, deliberately out of this
//!   task's scope rather than guessed at.
//! - `signature_text` for a `Field` candidate is the WHOLE assignment
//!   statement (`self.encoder = TransformerEncoder(a, b)`), the same
//!   "capture the whole construct" convention every other extractor in
//!   this module already follows — this is what lets a later resolver
//!   re-parse it and look for a constructor call on the right-hand
//!   side (the `Instantiates` edge resolver, Task 9.5 Priority 3).

use super::{node_text, ByteSpan};

type Candidate = (String, String, ByteSpan, String);

pub(super) fn extract(root: tree_sitter::Node, source: &[u8]) -> Vec<Candidate> {
    let mut candidates = Vec::new();
    walk(root, source, &mut candidates);
    candidates
}

fn walk(node: tree_sitter::Node, source: &[u8], out: &mut Vec<Candidate>) {
    if matches!(node.kind(), "class_definition" | "function_definition") {
        if let Some(candidate) = named_by_child_kind(node, source, "identifier") {
            out.push(candidate);
        }
    }
    if node.kind() == "assignment" {
        if let Some(candidate) = self_attribute_field_candidate(node, source) {
            out.push(candidate);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, source, out);
    }
}

/// Recognizes `self.<name> = <rhs>` specifically — see this module's
/// doc comment for the exact, conservative AST shape this matches and
/// what it deliberately excludes.
fn self_attribute_field_candidate(assignment_node: tree_sitter::Node, source: &[u8]) -> Option<Candidate> {
    let mut cursor = assignment_node.walk();
    let left = assignment_node.children(&mut cursor).find(|c| c.kind() == "attribute")?;
    // The `attribute` node's children are `(object, ".", attribute_name)`
    // — confirmed against a real parse. Only the FIRST child (the
    // object/receiver) is checked against the literal text "self";
    // this is a text comparison on a specific, narrow AST position
    // (the receiver of a `self.X` attribute chain), never a general
    // name-based heuristic over arbitrary identifiers elsewhere.
    let mut left_cursor = left.walk();
    let mut left_children = left.children(&mut left_cursor);
    let receiver = left_children.next()?;
    if receiver.kind() != "identifier" || node_text(&receiver, source) != "self" {
        return None;
    }
    let field_name_node = left_children.find(|c| c.kind() == "identifier")?;
    let name = node_text(&field_name_node, source).to_string();
    let kind_hint = "self_attribute_assignment".to_string();
    let span = ByteSpan::from_node(&assignment_node);
    let signature_text = node_text(&assignment_node, source).to_string();
    Some((name, kind_hint, span, signature_text))
}

fn named_by_child_kind(node: tree_sitter::Node, source: &[u8], name_kind: &str) -> Option<Candidate> {
    let mut cursor = node.walk();
    let name_node = node.children(&mut cursor).find(|c| c.kind() == name_kind && !c.is_missing())?;
    let name = node_text(&name_node, source).to_string();
    let kind_hint = node.kind().to_string();
    let span = ByteSpan::from_node(&node);
    let signature_text = node_text(&node, source).to_string();
    Some((name, kind_hint, span, signature_text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{LanguageParser, PythonParser};

    fn extract_from(source: &[u8]) -> Vec<Candidate> {
        let tree = PythonParser.parse(source).unwrap();
        extract(tree.tree().root_node(), tree.source())
    }

    #[test]
    fn a_single_function_is_extracted() {
        let candidates = extract_from(b"def f():\n    pass\n");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].0, "f");
        assert_eq!(candidates[0].1, "function_definition");
    }

    #[test]
    fn multiple_top_level_symbols_in_one_file_are_all_extracted() {
        let source = b"def f():\n    pass\n\nclass C:\n    pass\n\ndef g():\n    pass\n";
        let candidates = extract_from(source);
        let names: Vec<&str> = candidates.iter().map(|c| c.0.as_str()).collect();
        assert_eq!(names, vec!["f", "C", "g"]);
    }

    #[test]
    fn a_class_with_a_nested_method_extracts_both() {
        let source = b"class Outer:\n    def method(self):\n        return 1\n";
        let candidates = extract_from(source);
        let names: Vec<&str> = candidates.iter().map(|c| c.0.as_str()).collect();
        assert_eq!(names, vec!["Outer", "method"]);
    }

    #[test]
    fn a_file_with_no_symbols_yields_nothing() {
        let candidates = extract_from(b"x = 1\ny = x + 2\n");
        assert!(candidates.is_empty());
    }

    #[test]
    fn partially_malformed_source_still_extracts_the_well_formed_symbols() {
        let source = b"def broken(\nclass WellFormed:\n    pass\n";
        let candidates = extract_from(source);
        let names: Vec<&str> = candidates.iter().map(|c| c.0.as_str()).collect();
        assert!(names.contains(&"WellFormed"), "expected WellFormed among {names:?}");
    }

    #[test]
    fn multiline_signature_is_captured_verbatim() {
        let source = b"def multi(\n    a,\n    b,\n):\n    return a + b\n";
        let candidates = extract_from(source);
        assert_eq!(candidates.len(), 1);
        assert!(candidates[0].3.contains('\n'));
        assert!(candidates[0].3.starts_with("def multi("));
    }

    #[test]
    fn span_matches_the_exact_source_bytes() {
        let source = b"def exact():\n    pass\n";
        let candidates = extract_from(source);
        let (_, _, span, signature_text) = &candidates[0];
        // Whatever exact end byte tree-sitter chose for this construct
        // (with or without the trailing newline), span.start..span.end
        // sliced from the ORIGINAL source must agree byte-for-byte with
        // signature_text — that is the actual invariant under test, not
        // a specific guessed boundary.
        assert_eq!(std::str::from_utf8(&source[span.start..span.end]).unwrap(), signature_text);
    }

    // --- Task 9.5: Field (`self.X = <expr>`) extraction ---

    #[test]
    fn a_self_attribute_assignment_to_a_constructor_call_is_a_field_candidate() {
        let source = b"class M:\n    def __init__(self):\n        self.encoder = Encoder()\n";
        let candidates = extract_from(source);
        let field = candidates.iter().find(|c| c.0 == "encoder").expect("expected a 'encoder' candidate");
        assert_eq!(field.1, "self_attribute_assignment");
        assert_eq!(field.3, "self.encoder = Encoder()");
    }

    #[test]
    fn a_self_attribute_assignment_to_a_plain_identifier_is_still_a_field_candidate() {
        // self.ninp = ninp: RHS is NOT a call, but this is still a real
        // instance attribute assignment, per Task 9.5's explicit
        // decision that Field extraction never depends on the RHS
        // shape.
        let source = b"class M:\n    def __init__(self, ninp):\n        self.ninp = ninp\n";
        let candidates = extract_from(source);
        let field = candidates.iter().find(|c| c.0 == "ninp").expect("expected a 'ninp' candidate");
        assert_eq!(field.1, "self_attribute_assignment");
    }

    #[test]
    fn a_self_attribute_assignment_to_a_literal_is_still_a_field_candidate() {
        let source = b"class M:\n    def __init__(self):\n        self.enabled = False\n";
        let candidates = extract_from(source);
        let field = candidates.iter().find(|c| c.0 == "enabled").expect("expected an 'enabled' candidate");
        assert_eq!(field.1, "self_attribute_assignment");
    }

    #[test]
    fn a_plain_local_variable_assignment_is_never_a_field_candidate() {
        let source = b"class M:\n    def __init__(self):\n        local = Encoder()\n";
        let candidates = extract_from(source);
        assert!(candidates.iter().all(|c| c.0 != "local"), "a plain local-variable assignment must never be extracted as a Field");
    }

    #[test]
    fn a_loop_variable_is_never_a_field_candidate() {
        let source = b"class M:\n    def __init__(self):\n        for i in range(3):\n            x = i\n";
        let candidates = extract_from(source);
        assert!(candidates.iter().all(|c| c.0 != "i" && c.0 != "x"), "a loop variable or a loop-body local must never be extracted as a Field");
    }

    #[test]
    fn a_non_self_receiver_attribute_assignment_is_never_a_field_candidate() {
        // other.x = 1 / cls.y = 2: a real attribute assignment, but
        // NOT on the `self` receiver this extractor specifically
        // recognizes — deliberately out of scope rather than guessed.
        let source = b"class M:\n    def __init__(self, other):\n        other.x = 1\n";
        let candidates = extract_from(source);
        assert!(candidates.iter().all(|c| c.0 != "x"), "an attribute assignment on a non-self receiver must never be extracted as a Field");
    }

    #[test]
    fn the_same_field_name_in_two_different_classes_produces_two_distinct_candidates() {
        let source = b"class A:\n    def __init__(self):\n        self.encoder = EncoderA()\n\nclass B:\n    def __init__(self):\n        self.encoder = EncoderB()\n";
        let candidates = extract_from(source);
        let encoder_candidates: Vec<_> = candidates.iter().filter(|c| c.0 == "encoder").collect();
        assert_eq!(encoder_candidates.len(), 2, "each class's own self.encoder assignment must be its own candidate");
        assert_ne!(encoder_candidates[0].2.start, encoder_candidates[1].2.start, "the two candidates must have distinct spans (distinct source positions)");
    }

    #[test]
    fn multiple_assignments_to_the_same_self_attribute_each_produce_their_own_candidate() {
        // Reassignment across two branches of the same __init__ — two
        // real, distinct syntactic occurrences, each its own candidate.
        // This is NOT an identity bug to dedup: Task 9.5's own decision
        // is to let SymbolSnapshotId's span-based v2 identity (B2-ID-001)
        // disambiguate them structurally, exactly as it already does
        // for any other same-name/same-file construct.
        let source = b"class M:\n    def __init__(self, flag):\n        if flag:\n            self.encoder = EncoderA()\n        else:\n            self.encoder = EncoderB()\n";
        let candidates = extract_from(source);
        let encoder_candidates: Vec<_> = candidates.iter().filter(|c| c.0 == "encoder").collect();
        assert_eq!(encoder_candidates.len(), 2, "two distinct assignment sites must produce two distinct candidates, never silently deduplicated");
    }

    #[test]
    fn field_signature_text_captures_the_whole_assignment_statement() {
        let source = b"class M:\n    def __init__(self):\n        self.encoder = Encoder(a, b)\n";
        let candidates = extract_from(source);
        let field = candidates.iter().find(|c| c.0 == "encoder").unwrap();
        assert_eq!(field.3, "self.encoder = Encoder(a, b)");
    }
}
