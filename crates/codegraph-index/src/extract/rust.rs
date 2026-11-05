//! B2 Task 4: Rust symbol extraction. Node kinds and name-child shapes
//! below were confirmed by actually parsing real Rust fixtures with this
//! workspace's `tree_sitter_rust` version and dumping the resulting
//! tree, not assumed from memory or from a different grammar version.
//!
//! Recognized constructs: `struct_item`, `enum_item`, `trait_item`
//! (name child: `type_identifier`), `function_item` (free functions AND
//! methods inside an `impl_item`'s `declaration_list` — both are the
//! same node kind in this grammar; this extractor does not distinguish
//! them further since Task 4 has no "is this a method" requirement
//! beyond what `kind_hint` itself conveys, and both report `kind_hint =
//! "function_item"`), and `function_signature_item` (a trait method
//! declaration with no body, e.g. `fn req(&self);`). Name child for
//! both function kinds: `identifier`.

use super::{node_text, ByteSpan};

type Candidate = (String, String, ByteSpan, String);

pub(super) fn extract(root: tree_sitter::Node, source: &[u8]) -> Vec<Candidate> {
    let mut candidates = Vec::new();
    walk(root, source, &mut candidates);
    candidates
}

fn walk(node: tree_sitter::Node, source: &[u8], out: &mut Vec<Candidate>) {
    match node.kind() {
        "struct_item" | "enum_item" | "trait_item" => {
            if let Some(candidate) = named_by_child_kind(node, source, "type_identifier") {
                out.push(candidate);
            }
        }
        "function_item" | "function_signature_item" => {
            if let Some(candidate) = named_by_child_kind(node, source, "identifier") {
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

/// Finds `node`'s first direct child of kind `name_kind` and, if present
/// and not itself a `MISSING` placeholder, builds a candidate from it.
/// Returns `None` (omits the candidate) if no such child exists or the
/// child is `is_missing()` — the "prefer omit over false certainty"
/// policy for a construct whose expected name is itself absent from a
/// partial parse.
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
    use crate::parse::{LanguageParser, RustParser};

    fn extract_from(source: &[u8]) -> Vec<Candidate> {
        let tree = RustParser.parse(source).unwrap();
        extract(tree.tree().root_node(), tree.source())
    }

    #[test]
    fn a_single_free_function_is_extracted() {
        let candidates = extract_from(b"fn foo() {}\n");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].0, "foo");
        assert_eq!(candidates[0].1, "function_item");
    }

    #[test]
    fn multiple_top_level_symbols_in_one_file_are_all_extracted() {
        let source = b"struct S;\nenum E { A }\nfn f() {}\n";
        let candidates = extract_from(source);
        let names: Vec<&str> = candidates.iter().map(|c| c.0.as_str()).collect();
        assert_eq!(names, vec!["S", "E", "f"]);
    }

    #[test]
    fn nested_symbols_a_trait_and_an_impl_method_are_both_extracted() {
        let source = b"trait T { fn req(&self); }\nstruct S;\nimpl S { fn method(&self) -> i32 { 0 } }\n";
        let candidates = extract_from(source);
        let names: Vec<&str> = candidates.iter().map(|c| c.0.as_str()).collect();
        assert_eq!(names, vec!["T", "req", "S", "method"]);
        let req = candidates.iter().find(|c| c.0 == "req").unwrap();
        assert_eq!(req.1, "function_signature_item");
        let method = candidates.iter().find(|c| c.0 == "method").unwrap();
        assert_eq!(method.1, "function_item");
    }

    #[test]
    fn a_file_with_no_symbols_yields_nothing() {
        let candidates = extract_from(b"let x = 1;\nlet y = x + 2;\n");
        assert!(candidates.is_empty());
    }

    #[test]
    fn partially_malformed_source_still_extracts_the_well_formed_symbols() {
        // The first function is syntactically broken (unterminated body);
        // the second, independent struct is well-formed and still a
        // sibling at the file's top level. The extractor must still
        // report the well-formed one.
        let source = b"fn broken( {\nstruct WellFormed;\n";
        let candidates = extract_from(source);
        let names: Vec<&str> = candidates.iter().map(|c| c.0.as_str()).collect();
        assert!(names.contains(&"WellFormed"), "expected WellFormed among {names:?}");
    }

    #[test]
    fn multiline_signature_is_captured_verbatim_in_signature_text() {
        let source = b"fn multi(\n    a: i32,\n    b: i32,\n) -> i32 {\n    a + b\n}\n";
        let candidates = extract_from(source);
        assert_eq!(candidates.len(), 1);
        assert!(candidates[0].3.contains('\n'), "expected signature_text to preserve the multiline signature verbatim");
        assert!(candidates[0].3.starts_with("fn multi("));
    }

    #[test]
    fn span_matches_the_exact_source_bytes_of_the_construct() {
        let source = b"fn exact() {}\n";
        let candidates = extract_from(source);
        let (_, _, span, _) = &candidates[0];
        assert_eq!(&source[span.start..span.end], b"fn exact() {}");
    }
}
