//! B2 Task 4: JavaScript/TypeScript/Tsx symbol extraction, node kinds
//! confirmed against real parses of each.
//!
//! JavaScript's `class_declaration` names via a direct `identifier`
//! child; TypeScript's (and Tsx's, which shares TypeScript's grammar for
//! this construct) names via `type_identifier` instead — a genuine
//! grammar difference, not a naming inconsistency to hide behind one
//! shared function. `function_declaration` names via `identifier` in
//! both. `method_definition` (class methods, and TypeScript's
//! `method_signature` for interface method declarations) names via
//! `property_identifier` in both. An `arrow_function` has NO name of its
//! own in either grammar — it is anonymous at the node level; this
//! extractor recovers a name only for the common `const name = (...) =>
//! ...` shape, by looking at the ENCLOSING `variable_declarator`'s
//! `identifier` sibling. An arrow function anywhere else (e.g. passed
//! directly as a callback argument) has no recoverable name and is
//! correctly omitted — per the "prefer omit over false certainty"
//! policy, this extractor does not invent a synthetic name like
//! `"<anonymous>"`.

use super::{node_text, ByteSpan};

type Candidate = (String, String, ByteSpan, String);

pub(super) fn extract_javascript(root: tree_sitter::Node, source: &[u8]) -> Vec<Candidate> {
    let mut candidates = Vec::new();
    walk(root, source, &mut candidates, "identifier");
    candidates
}

pub(super) fn extract_typescript(root: tree_sitter::Node, source: &[u8]) -> Vec<Candidate> {
    let mut candidates = Vec::new();
    walk(root, source, &mut candidates, "type_identifier");
    candidates
}

/// `class_name_kind` is `"identifier"` for JavaScript and
/// `"type_identifier"` for TypeScript/Tsx — the one real grammar
/// difference this shared walk needs to know about; everything else
/// (function_declaration/method_definition/arrow_function handling) is
/// identical across both.
fn walk(node: tree_sitter::Node, source: &[u8], out: &mut Vec<Candidate>, class_name_kind: &str) {
    match node.kind() {
        "class_declaration" => {
            if let Some(candidate) = named_by_direct_child_kind(node, source, class_name_kind) {
                out.push(candidate);
            }
        }
        "function_declaration" => {
            if let Some(candidate) = named_by_direct_child_kind(node, source, "identifier") {
                out.push(candidate);
            }
        }
        "method_definition" | "method_signature" => {
            if let Some(candidate) = named_by_direct_child_kind(node, source, "property_identifier") {
                out.push(candidate);
            }
        }
        "arrow_function" => {
            if let Some(candidate) = named_arrow_function_from_variable_declarator(node, source) {
                out.push(candidate);
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, source, out, class_name_kind);
    }
}

fn named_by_direct_child_kind(node: tree_sitter::Node, source: &[u8], name_kind: &str) -> Option<Candidate> {
    let mut cursor = node.walk();
    let name_node = node.children(&mut cursor).find(|c| c.kind() == name_kind && !c.is_missing())?;
    let name = node_text(&name_node, source).to_string();
    build_candidate(node, source, name)
}

/// An `arrow_function` node has no name child of its own. The common
/// `const name = (...) => ...` shape puts the arrow function as the
/// `variable_declarator`'s second named child (after `=`), with the
/// name as the declarator's own `identifier` child — so this walks UP
/// one level (the arrow function's parent) rather than down, and omits
/// the candidate entirely if the parent is not a `variable_declarator`
/// with a recoverable `identifier` (e.g. an arrow function passed as a
/// bare callback argument has no name to recover, and none is invented).
fn named_arrow_function_from_variable_declarator(node: tree_sitter::Node, source: &[u8]) -> Option<Candidate> {
    let parent = node.parent()?;
    if parent.kind() != "variable_declarator" {
        return None;
    }
    let mut cursor = parent.walk();
    let name_node = parent.children(&mut cursor).find(|c| c.kind() == "identifier" && !c.is_missing())?;
    let name = node_text(&name_node, source).to_string();
    // The signature/span is the arrow function's OWN extent (its
    // parameters and body), not the whole `variable_declarator` (which
    // would also include the `const`/`=` syntax around it) — this
    // extractor reports the arrow function itself as the construct,
    // merely borrowing its enclosing declarator's name.
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
    use crate::parse::{JavaScriptParser, LanguageParser, TsxParser, TypeScriptParser};

    fn extract_js(source: &[u8]) -> Vec<Candidate> {
        let tree = JavaScriptParser.parse(source).unwrap();
        extract_javascript(tree.tree().root_node(), tree.source())
    }

    fn extract_ts(source: &[u8]) -> Vec<Candidate> {
        let tree = TypeScriptParser.parse(source).unwrap();
        extract_typescript(tree.tree().root_node(), tree.source())
    }

    fn extract_tsx(source: &[u8]) -> Vec<Candidate> {
        let tree = TsxParser.parse(source).unwrap();
        extract_typescript(tree.tree().root_node(), tree.source())
    }

    #[test]
    fn javascript_single_function_is_extracted() {
        let candidates = extract_js(b"function f() {}\n");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].0, "f");
        assert_eq!(candidates[0].1, "function_declaration");
    }

    #[test]
    fn javascript_class_and_method_are_both_extracted() {
        let source = b"class C { method(x) { return x; } }\n";
        let candidates = extract_js(source);
        let names: Vec<&str> = candidates.iter().map(|c| c.0.as_str()).collect();
        assert_eq!(names, vec!["C", "method"]);
    }

    #[test]
    fn javascript_named_arrow_function_assignment_is_extracted() {
        let source = b"const arrow = (x) => x + 1;\n";
        let candidates = extract_js(source);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].0, "arrow");
        assert_eq!(candidates[0].1, "arrow_function");
    }

    #[test]
    fn javascript_arrow_function_assigned_to_a_destructuring_pattern_is_omitted_not_misnamed() {
        // `const {} = () => 1;` has a `variable_declarator` whose first
        // child is `object_pattern`, not `identifier` (confirmed by
        // inspecting the real parse tree) — the real `&&` logic finds no
        // matching child at all and correctly omits the candidate. This
        // is the specific fixture that distinguishes `&&` from `||` in
        // `named_arrow_function_from_variable_declarator`'s find
        // predicate: every OTHER arrow-function fixture in this file has
        // a plain `identifier` as the declarator's first child, which a
        // `||` mutant would coincidentally still match correctly.
        let candidates = extract_js(b"const {} = () => 1;\n");
        assert!(
            candidates.is_empty(),
            "expected the destructuring-pattern-assigned arrow function to be omitted, got {candidates:?}"
        );
    }

    #[test]
    fn javascript_unnamed_arrow_function_as_a_bare_callback_is_omitted() {
        let source = b"setTimeout(() => { doSomething(); }, 10);\n";
        let candidates = extract_js(source);
        assert!(
            candidates.is_empty(),
            "an arrow function with no recoverable name must be omitted, not invented, got {candidates:?}"
        );
    }

    #[test]
    fn javascript_multiple_top_level_symbols_are_all_extracted() {
        let source = b"function f() {}\nclass C {}\nconst arrow = () => 1;\n";
        let candidates = extract_js(source);
        let names: Vec<&str> = candidates.iter().map(|c| c.0.as_str()).collect();
        assert_eq!(names, vec!["f", "C", "arrow"]);
    }

    #[test]
    fn javascript_file_with_no_symbols_yields_nothing() {
        let candidates = extract_js(b"const x = 1;\nconst y = x + 2;\n");
        assert!(candidates.is_empty());
    }

    #[test]
    fn javascript_partially_malformed_source_still_extracts_well_formed_symbols() {
        // The malformed expression `let x = (;` inside broken's body is
        // locally contained by tree-sitter's error recovery (confirmed
        // by inspecting the actual parse tree) — it does not swallow the
        // enclosing function or the next, independent top-level class.
        let source = b"function broken() { let x = (; }\nclass WellFormed {}\n";
        let candidates = extract_js(source);
        let names: Vec<&str> = candidates.iter().map(|c| c.0.as_str()).collect();
        assert!(names.contains(&"WellFormed"), "expected WellFormed among {names:?}");
    }

    #[test]
    fn javascript_multiline_signature_is_captured_verbatim() {
        let source = b"function multi(\n    a,\n    b\n) {\n    return a + b;\n}\n";
        let candidates = extract_js(source);
        assert_eq!(candidates.len(), 1);
        assert!(candidates[0].3.contains('\n'));
    }

    #[test]
    fn typescript_class_names_via_type_identifier_not_identifier() {
        // The one real grammar difference from JavaScript this module
        // exists to NOT hide: confirm TypeScript's class extraction
        // actually works (it would silently fail — zero candidates — if
        // this extractor incorrectly used "identifier" instead of
        // "type_identifier" for TypeScript, since that child kind simply
        // would not match in this grammar).
        let candidates = extract_ts(b"class C {}\n");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].0, "C");
    }

    #[test]
    fn typescript_interface_method_signature_is_extracted() {
        let source = b"interface I { m(): void; }\n";
        let candidates = extract_ts(source);
        let names: Vec<&str> = candidates.iter().map(|c| c.0.as_str()).collect();
        assert!(names.contains(&"m"), "expected interface method m among {names:?}");
    }

    #[test]
    fn typescript_class_method_and_function_are_all_extracted() {
        let source = b"class C implements I { m(): void {} }\nfunction f(a: number): number { return a; }\n";
        let candidates = extract_ts(source);
        let names: Vec<&str> = candidates.iter().map(|c| c.0.as_str()).collect();
        assert_eq!(names, vec!["C", "m", "f"]);
    }

    #[test]
    fn typescript_file_with_no_symbols_yields_nothing() {
        let candidates = extract_ts(b"const x: number = 1;\n");
        assert!(candidates.is_empty());
    }

    #[test]
    fn typescript_partially_malformed_source_still_extracts_well_formed_symbols() {
        let source = b"function f(x: {): numb\nclass WellFormed {}\n";
        let candidates = extract_ts(source);
        let names: Vec<&str> = candidates.iter().map(|c| c.0.as_str()).collect();
        assert!(names.contains(&"WellFormed"), "expected WellFormed among {names:?}");
    }

    #[test]
    fn tsx_function_component_and_arrow_component_are_extracted() {
        let source = b"function Comp() { return <div>hi</div>; }\nconst Arrow = () => <span />;\n";
        let candidates = extract_tsx(source);
        let names: Vec<&str> = candidates.iter().map(|c| c.0.as_str()).collect();
        assert_eq!(names, vec!["Comp", "Arrow"]);
    }

    #[test]
    fn span_matches_the_exact_source_bytes() {
        let source = b"function exact() {}\n";
        let candidates = extract_js(source);
        let (_, _, span, signature_text) = &candidates[0];
        assert_eq!(std::str::from_utf8(&source[span.start..span.end]).unwrap(), signature_text);
    }
}
