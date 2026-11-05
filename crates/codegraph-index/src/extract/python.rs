//! B2 Task 4: Python symbol extraction, node kinds confirmed against a
//! real parse. `class_definition` and `function_definition` both name
//! via a direct `identifier` child; a method is a `function_definition`
//! nested inside a `class_definition`'s `block` — the SAME node kind as
//! a top-level function in this grammar, so `kind_hint` does not itself
//! distinguish "method" from "free function" (consistent with Rust's
//! `function_item` not distinguishing methods from free functions
//! either).

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
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, source, out);
    }
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
}
