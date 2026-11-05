//! B2 Task 4: C++ symbol extraction, node kinds confirmed against a real
//! parse. `class_specifier`/`struct_specifier` name via a direct
//! `type_identifier` child. Both free functions and class methods are
//! `function_definition`, but — unlike Rust/Python/C#, where the name is
//! a direct child of the construct node — a C++ function's name lives
//! one level deeper, inside its `function_declarator` child: a free
//! function's name there is `identifier`, while a method's name there is
//! `field_identifier` (confirmed by parsing a class method and a free
//! function side by side and inspecting both). `kind_hint` is always
//! `"function_definition"` for both; this extractor does not need to
//! distinguish them further to answer "what is this construct's name,"
//! which is this task's only job.

use super::{node_text, ByteSpan};

type Candidate = (String, String, ByteSpan, String);

pub(super) fn extract(root: tree_sitter::Node, source: &[u8]) -> Vec<Candidate> {
    let mut candidates = Vec::new();
    walk(root, source, &mut candidates);
    candidates
}

fn walk(node: tree_sitter::Node, source: &[u8], out: &mut Vec<Candidate>) {
    match node.kind() {
        "class_specifier" | "struct_specifier" => {
            if let Some(candidate) = named_by_direct_child_kind(node, source, "type_identifier") {
                out.push(candidate);
            }
        }
        "function_definition" => {
            if let Some(candidate) = named_function_definition(node, source) {
                out.push(candidate);
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, source, out);
    }
}

fn named_by_direct_child_kind(node: tree_sitter::Node, source: &[u8], name_kind: &str) -> Option<Candidate> {
    let mut cursor = node.walk();
    let name_node = node.children(&mut cursor).find(|c| c.kind() == name_kind && !c.is_missing())?;
    let name = node_text(&name_node, source).to_string();
    build_candidate(node, source, name)
}

/// A `function_definition`'s name lives inside its `function_declarator`
/// child, as either `identifier` (free function) or `field_identifier`
/// (method). Returns `None` (omits the candidate) if there is no
/// `function_declarator` child at all, or neither name-kind is present
/// within it and not missing — e.g. an operator overload or a
/// destructor whose "name" is a different node shape this extractor does
/// not yet recognize is correctly omitted rather than misnamed, per the
/// "prefer omit over false certainty" policy.
fn named_function_definition(node: tree_sitter::Node, source: &[u8]) -> Option<Candidate> {
    let mut cursor = node.walk();
    let declarator = node.children(&mut cursor).find(|c| c.kind() == "function_declarator")?;
    let mut decl_cursor = declarator.walk();
    let name_node = declarator
        .children(&mut decl_cursor)
        .find(|c| matches!(c.kind(), "identifier" | "field_identifier") && !c.is_missing())?;
    let name = node_text(&name_node, source).to_string();
    build_candidate(node, source, name)
}

fn build_candidate(node: tree_sitter::Node, source: &[u8], name: String) -> Option<Candidate> {
    let kind_hint = node.kind().to_string();
    let span = ByteSpan::from_node(&node);
    let signature_text = node_text(&node, source).to_string();
    Some((name, kind_hint, span, signature_text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{CppParser, LanguageParser};

    fn extract_from(source: &[u8]) -> Vec<Candidate> {
        let tree = CppParser.parse(source).unwrap();
        extract(tree.tree().root_node(), tree.source())
    }

    #[test]
    fn a_single_free_function_is_extracted() {
        let candidates = extract_from(b"int f() { return 0; }\n");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].0, "f");
        assert_eq!(candidates[0].1, "function_definition");
    }

    #[test]
    fn multiple_top_level_symbols_are_all_extracted() {
        let source = b"struct S {};\nint f() { return 0; }\n";
        let candidates = extract_from(source);
        let names: Vec<&str> = candidates.iter().map(|c| c.0.as_str()).collect();
        assert_eq!(names, vec!["S", "f"]);
    }

    #[test]
    fn a_class_with_a_nested_method_extracts_both_with_the_right_name() {
        let source = b"class C { public: int method(int x) { return x; } };\n";
        let candidates = extract_from(source);
        let names: Vec<&str> = candidates.iter().map(|c| c.0.as_str()).collect();
        assert_eq!(names, vec!["C", "method"]);
    }

    #[test]
    fn a_file_with_no_symbols_yields_nothing() {
        let candidates = extract_from(b"int x = 1;\n");
        assert!(candidates.is_empty());
    }

    #[test]
    fn partially_malformed_source_still_extracts_the_well_formed_symbols() {
        let source = b"int broken( {\nstruct WellFormed {};\n";
        let candidates = extract_from(source);
        let names: Vec<&str> = candidates.iter().map(|c| c.0.as_str()).collect();
        assert!(names.contains(&"WellFormed"), "expected WellFormed among {names:?}");
    }

    #[test]
    fn multiline_signature_is_captured_verbatim() {
        let source = b"int multi(\n    int a,\n    int b\n) {\n    return a + b;\n}\n";
        let candidates = extract_from(source);
        assert_eq!(candidates.len(), 1);
        assert!(candidates[0].3.contains('\n'));
    }

    #[test]
    fn span_matches_the_exact_source_bytes() {
        let source = b"int exact() { return 0; }\n";
        let candidates = extract_from(source);
        let (_, _, span, signature_text) = &candidates[0];
        assert_eq!(std::str::from_utf8(&source[span.start..span.end]).unwrap(), signature_text);
    }

    #[test]
    fn an_out_of_class_qualified_method_definition_is_omitted_not_misnamed() {
        // `S::method`'s `function_declarator` has a `qualified_identifier`
        // as its first child (confirmed by inspecting the real parse
        // tree), not a direct `identifier`/`field_identifier` — this
        // extractor does not yet recognize that shape and must OMIT the
        // candidate rather than mis-extract "S::method" or any other
        // wrong text from a non-matching child. This is the fixture that
        // actually exercises the `&&` (not `||`) requirement in
        // `named_function_definition`'s find predicate: every OTHER
        // fixture in this file happens to have the real name as the
        // FIRST child of `function_declarator`, so a `||` mutant would
        // coincidentally still find the right node there — only a
        // function_declarator whose first child is neither missing NOR
        // the right kind actually distinguishes the two operators.
        let source = b"int S::method(int x) { return x; }\n";
        let candidates = extract_from(source);
        assert!(
            candidates.is_empty(),
            "expected the qualified out-of-class definition to be omitted, got {candidates:?}"
        );
    }
}
