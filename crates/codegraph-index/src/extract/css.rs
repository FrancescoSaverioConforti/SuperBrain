//! B2 Task 4: CSS symbol extraction. Semantics explicitly decided (not
//! implied by the plan): a `rule_set` is a candidate only if its
//! `selectors` text contains at least one referenceable selector form
//! (`#id`, `.class`, a tag selector, or a custom-element selector),
//! including composite selectors that contain one of those alongside
//! other combinators. A bare property/declaration inside a rule's body
//! is never, on its own, a candidate. A NAMED at-rule with its own
//! identity is also a candidate, specifically `@keyframes <name>`
//! (`keyframes_statement`, name child `keyframes_name`) — `@media` and
//! other at-rules are containers this extractor still walks INTO (their
//! nested `rule_set`s are still candidates), but are not themselves
//! candidates, since they have no referenceable name of their own (an
//! `@media` query condition is not an identity).
//!
//! `signature_text` for a `rule_set` is bounded to its `selectors` node
//! alone — never the rule's full declaration block — per the approved
//! semantics ("Il signature_text della regola deve preservare il
//! selector bounded, non l'intero corpo delle dichiarazioni").

use super::{node_text, ByteSpan};

type Candidate = (String, String, ByteSpan, String);

pub(super) fn extract(root: tree_sitter::Node, source: &[u8]) -> Vec<Candidate> {
    let mut candidates = Vec::new();
    walk(root, source, &mut candidates);
    candidates
}

fn walk(node: tree_sitter::Node, source: &[u8], out: &mut Vec<Candidate>) {
    match node.kind() {
        "rule_set" => {
            if let Some(candidate) = named_by_referenceable_selector(node, source) {
                out.push(candidate);
            }
        }
        "keyframes_statement" => {
            if let Some(candidate) = named_keyframes(node, source) {
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

/// A selector is "referenceable" if it is (or contains, for a composite
/// selector) an id selector, class selector, tag/type selector, or a
/// custom-element selector (a tag selector whose name contains a
/// hyphen — the standard custom-element naming convention; this grammar
/// does not expose a distinct node kind for "custom element" versus an
/// ordinary tag selector, so this extractor recognizes it by that
/// convention rather than inventing a node-kind distinction tree-sitter
/// itself does not make).
fn is_referenceable_selector_kind(kind: &str) -> bool {
    matches!(kind, "id_selector" | "class_selector" | "tag_name")
}

fn contains_referenceable_selector(node: tree_sitter::Node) -> bool {
    if is_referenceable_selector_kind(node.kind()) {
        return true;
    }
    let mut cursor = node.walk();
    let found = node.children(&mut cursor).any(contains_referenceable_selector);
    found
}

fn named_by_referenceable_selector(rule_set: tree_sitter::Node, source: &[u8]) -> Option<Candidate> {
    let mut cursor = rule_set.walk();
    let selectors = rule_set.children(&mut cursor).find(|c| c.kind() == "selectors")?;
    if !contains_referenceable_selector(selectors) {
        return None;
    }
    let name = node_text(&selectors, source).to_string();
    let kind_hint = "rule_set".to_string();
    // The selector's own span/text, NOT the whole rule_set's — the
    // bounded-selector signature_text requirement.
    let span = ByteSpan::from_node(&selectors);
    let signature_text = name.clone();
    Some((name, kind_hint, span, signature_text))
}

fn named_keyframes(node: tree_sitter::Node, source: &[u8]) -> Option<Candidate> {
    let mut cursor = node.walk();
    let name_node = node.children(&mut cursor).find(|c| c.kind() == "keyframes_name" && !c.is_missing())?;
    let name = node_text(&name_node, source).to_string();
    let kind_hint = "keyframes_statement".to_string();
    let span = ByteSpan::from_node(&node);
    let signature_text = node_text(&node, source).to_string();
    Some((name, kind_hint, span, signature_text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{CssParser, LanguageParser};

    fn extract_from(source: &[u8]) -> Vec<Candidate> {
        let tree = CssParser.parse(source).unwrap();
        extract(tree.tree().root_node(), tree.source())
    }

    #[test]
    fn an_id_selector_rule_produces_one_symbol() {
        let candidates = extract_from(b"#header { color: red; }\n");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].0, "#header");
        assert_eq!(candidates[0].1, "rule_set");
    }

    #[test]
    fn a_universal_selector_rule_produces_zero_symbols() {
        // `*` is a `universal_selector` — the one real negative case that
        // distinguishes a correct referenceable-selector check from a
        // mutant that always returns true: every OTHER test in this
        // file only exercises selectors that ARE referenceable, so a
        // vacuously-true check would still pass them all.
        let candidates = extract_from(b"* { box-sizing: border-box; }\n");
        assert!(candidates.is_empty(), "a universal selector alone is not referenceable and must not be a symbol");
    }

    #[test]
    fn a_class_selector_rule_produces_one_symbol() {
        let candidates = extract_from(b".card { padding: 1px; }\n");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].0, ".card");
    }

    #[test]
    fn a_bare_property_declaration_produces_zero_autonomous_symbols() {
        // `color: red;` alone (not inside a rule) is not valid
        // standalone CSS — the actual guarantee under test is that a
        // rule's declaration body contributes zero symbols of its own,
        // exercised via a real rule.
        let candidates = extract_from(b".a { color: red; padding: 2px; margin: 1px; }\n");
        assert_eq!(candidates.len(), 1, "only the rule itself is a symbol, never its individual declarations");
    }

    #[test]
    fn multiple_rules_in_one_file_are_all_extracted() {
        let source = b"#header { color: red; }\n.card { padding: 1px; }\n";
        let candidates = extract_from(source);
        let names: Vec<&str> = candidates.iter().map(|c| c.0.as_str()).collect();
        assert_eq!(names, vec!["#header", ".card"]);
    }

    #[test]
    fn a_named_keyframes_at_rule_is_extracted() {
        let source = b"@keyframes fade { from { opacity: 0; } to { opacity: 1; } }\n";
        let candidates = extract_from(source);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].0, "fade");
        assert_eq!(candidates[0].1, "keyframes_statement");
    }

    #[test]
    fn a_rule_nested_inside_a_media_query_is_still_extracted() {
        let source = b"@media screen { .a { color: blue; } }\n";
        let candidates = extract_from(source);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].0, ".a");
    }

    #[test]
    fn a_file_with_no_symbols_yields_nothing() {
        let candidates = extract_from(b"");
        assert!(candidates.is_empty());
    }

    #[test]
    fn partially_malformed_source_still_extracts_well_formed_rules() {
        let source = b".broken { color: red; .nested {\n#header { color: blue; }\n";
        let candidates = extract_from(source);
        let names: Vec<&str> = candidates.iter().map(|c| c.0.as_str()).collect();
        assert!(names.contains(&"#header"), "expected the well-formed rule among {names:?}");
    }

    #[test]
    fn signature_text_is_bounded_to_the_selector_not_the_full_declaration_block() {
        let source = b"#header { color: red; padding: 10px; margin: 5px; border: 1px solid black; }\n";
        let candidates = extract_from(source);
        assert_eq!(candidates[0].3, "#header");
        assert!(!candidates[0].3.contains("color"), "signature_text must not include the declaration block");
    }

    #[test]
    fn span_matches_the_exact_source_bytes_of_the_selector() {
        let source = b"#exact { color: red; }\n";
        let candidates = extract_from(source);
        let (_, _, span, signature_text) = &candidates[0];
        assert_eq!(std::str::from_utf8(&source[span.start..span.end]).unwrap(), signature_text);
    }
}
