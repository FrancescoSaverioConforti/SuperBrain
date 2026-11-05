//! B2 Task 4: C# symbol extraction, node kinds confirmed against a real
//! parse. `interface_declaration`/`class_declaration`/`method_declaration`
//! all name via a direct `identifier` child. A method is
//! `method_declaration` whether found inside a `class_declaration`'s or
//! an `interface_declaration`'s `declaration_list` — same node kind
//! either way, so `kind_hint` does not itself distinguish "interface
//! method signature" from "class method with a body."

use super::{node_text, ByteSpan};

type Candidate = (String, String, ByteSpan, String);

pub(super) fn extract(root: tree_sitter::Node, source: &[u8]) -> Vec<Candidate> {
    let mut candidates = Vec::new();
    walk(root, source, &mut candidates);
    candidates
}

fn walk(node: tree_sitter::Node, source: &[u8], out: &mut Vec<Candidate>) {
    if matches!(node.kind(), "interface_declaration" | "class_declaration" | "method_declaration") {
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
    use crate::parse::{CSharpParser, LanguageParser};

    fn extract_from(source: &[u8]) -> Vec<Candidate> {
        let tree = CSharpParser.parse(source).unwrap();
        extract(tree.tree().root_node(), tree.source())
    }

    #[test]
    fn a_single_class_is_extracted() {
        let candidates = extract_from(b"class C {}\n");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].0, "C");
        assert_eq!(candidates[0].1, "class_declaration");
    }

    #[test]
    fn multiple_top_level_symbols_are_all_extracted() {
        let source = b"interface I {}\nclass C {}\n";
        let candidates = extract_from(source);
        let names: Vec<&str> = candidates.iter().map(|c| c.0.as_str()).collect();
        assert_eq!(names, vec!["I", "C"]);
    }

    #[test]
    fn a_class_with_a_nested_method_extracts_both() {
        let source = b"class C { public void M() {} }\n";
        let candidates = extract_from(source);
        let names: Vec<&str> = candidates.iter().map(|c| c.0.as_str()).collect();
        assert_eq!(names, vec!["C", "M"]);
    }

    #[test]
    fn an_interface_with_a_method_signature_extracts_both() {
        let source = b"interface I { void M(); }\n";
        let candidates = extract_from(source);
        let names: Vec<&str> = candidates.iter().map(|c| c.0.as_str()).collect();
        assert_eq!(names, vec!["I", "M"]);
    }

    #[test]
    fn a_file_with_no_symbols_yields_nothing() {
        let candidates = extract_from(b"// just a comment\n");
        assert!(candidates.is_empty());
    }

    #[test]
    fn partially_malformed_source_still_extracts_the_well_formed_symbols() {
        // The malformed expression `int x = (;` inside M's body is
        // locally contained by tree-sitter's error recovery (confirmed
        // by inspecting the actual parse tree) — it does not swallow the
        // enclosing class or the next, independent top-level class.
        let source = b"class Broken {\n    void M() { int x = (; }\n}\nclass WellFormed {}\n";
        let candidates = extract_from(source);
        let names: Vec<&str> = candidates.iter().map(|c| c.0.as_str()).collect();
        assert!(names.contains(&"WellFormed"), "expected WellFormed among {names:?}");
    }

    #[test]
    fn multiline_signature_is_captured_verbatim() {
        let source = b"class C {\n    public void M(\n        int a,\n        int b\n    ) {}\n}\n";
        let candidates = extract_from(source);
        let method = candidates.iter().find(|c| c.0 == "M").unwrap();
        assert!(method.3.contains('\n'));
    }

    #[test]
    fn span_matches_the_exact_source_bytes() {
        let source = b"class Exact {}\n";
        let candidates = extract_from(source);
        let (_, _, span, signature_text) = &candidates[0];
        assert_eq!(std::str::from_utf8(&source[span.start..span.end]).unwrap(), signature_text);
    }
}
