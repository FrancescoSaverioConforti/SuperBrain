//! B2 Task 4: HTML symbol extraction. Semantics explicitly decided (not
//! implied by the plan, which does not define what a "symbol" means for
//! HTML): only an `element` with an explicit `id` attribute is a
//! candidate. An anonymous element (no `id`, even with a `class`) is
//! never a candidate — a `class` attribute names a potential
//! relationship TARGET for a later edge-resolution task, not a DOM-
//! instance identity of its own. This extractor does not resolve any
//! cross-reference (CSS selector <-> this id, JS <-> this id); it only
//! reports the id's existence as a candidate.
//!
//! Node shapes confirmed against a real parse:
//! `element > start_tag > attribute[attribute_name="id"] > quoted_attribute_value > attribute_value`.

use super::{node_text, ByteSpan};

type Candidate = (String, String, ByteSpan, String);

pub(super) fn extract(root: tree_sitter::Node, source: &[u8]) -> Vec<Candidate> {
    let mut candidates = Vec::new();
    walk(root, source, &mut candidates);
    candidates
}

fn walk(node: tree_sitter::Node, source: &[u8], out: &mut Vec<Candidate>) {
    if node.kind() == "element" {
        if let Some(candidate) = named_by_id_attribute(node, source) {
            out.push(candidate);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, source, out);
    }
}

/// Returns `Some` only if `element` has a `start_tag` with an `attribute`
/// whose name is exactly `id` and whose value is present and not
/// `MISSING` — an element with `id=""` (empty but present) or a
/// malformed/incomplete `id` attribute (missing its value) is omitted
/// rather than reported with a guessed/empty name, per the "prefer omit
/// over false certainty" policy: an empty-string identity is not a
/// usable reference target either way.
fn named_by_id_attribute(element: tree_sitter::Node, source: &[u8]) -> Option<Candidate> {
    let mut cursor = element.walk();
    let start_tag = element.children(&mut cursor).find(|c| c.kind() == "start_tag")?;

    let mut tag_cursor = start_tag.walk();
    let id_attribute = start_tag.children(&mut tag_cursor).find(|attr| {
        if attr.kind() != "attribute" {
            return false;
        }
        let mut attr_cursor = attr.walk();
        let is_id = attr
            .children(&mut attr_cursor)
            .any(|c| c.kind() == "attribute_name" && node_text(&c, source) == "id");
        is_id
    })?;

    let mut id_cursor = id_attribute.walk();
    let quoted_value = id_attribute.children(&mut id_cursor).find(|c| c.kind() == "quoted_attribute_value")?;
    let mut value_cursor = quoted_value.walk();
    let value_node = quoted_value
        .children(&mut value_cursor)
        .find(|c| c.kind() == "attribute_value" && !c.is_missing())?;
    let id_value = node_text(&value_node, source);
    if id_value.is_empty() {
        return None;
    }

    let kind_hint = "element_with_id".to_string();
    let span = ByteSpan::from_node(&start_tag);
    let signature_text = node_text(&start_tag, source).to_string();
    Some((id_value.to_string(), kind_hint, span, signature_text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{HtmlParser, LanguageParser};

    fn extract_from(source: &[u8]) -> Vec<Candidate> {
        let tree = HtmlParser.parse(source).unwrap();
        extract(tree.tree().root_node(), tree.source())
    }

    #[test]
    fn an_element_with_an_id_produces_one_symbol() {
        let candidates = extract_from(b"<section id=\"settings\"></section>");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].0, "settings");
        assert_eq!(candidates[0].1, "element_with_id");
    }

    #[test]
    fn a_class_only_attribute_produces_zero_html_symbols() {
        let candidates = extract_from(b"<div class=\"card\"></div>");
        assert!(candidates.is_empty(), "a class attribute alone must never produce an HTML symbol");
    }

    #[test]
    fn hundreds_of_anonymous_divs_produce_zero_symbols() {
        let mut source = String::new();
        for _ in 0..300 {
            source.push_str("<div class=\"card\"></div>");
        }
        let candidates = extract_from(source.as_bytes());
        assert!(candidates.is_empty(), "300 anonymous divs must produce zero HTML symbols, got {}", candidates.len());
    }

    #[test]
    fn multiple_ided_elements_in_one_document_are_all_extracted() {
        let source = b"<html><body><section id=\"settings\"></section><section id=\"profile\"></section></body></html>";
        let candidates = extract_from(source);
        let names: Vec<&str> = candidates.iter().map(|c| c.0.as_str()).collect();
        assert_eq!(names, vec!["settings", "profile"]);
    }

    #[test]
    fn a_nested_ided_element_inside_an_anonymous_wrapper_is_still_extracted() {
        let source = b"<div class=\"card\"><section id=\"settings\"><p>hi</p></section></div>";
        let candidates = extract_from(source);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].0, "settings");
    }

    #[test]
    fn an_id_attribute_with_an_empty_value_is_omitted() {
        let candidates = extract_from(b"<div id=\"\"></div>");
        assert!(candidates.is_empty(), "an empty id value is not a usable identity and must be omitted");
    }

    #[test]
    fn a_file_with_no_elements_at_all_yields_nothing() {
        let candidates = extract_from(b"");
        assert!(candidates.is_empty());
    }

    #[test]
    fn partially_malformed_source_still_extracts_well_formed_ided_elements() {
        let source = b"<div><section id=\"broken\"<p>unclosed\n<section id=\"settings\"></section>";
        let candidates = extract_from(source);
        let names: Vec<&str> = candidates.iter().map(|c| c.0.as_str()).collect();
        assert!(names.contains(&"settings"), "expected the well-formed ided element among {names:?}");
    }

    #[test]
    fn span_matches_the_exact_source_bytes_of_the_start_tag() {
        let source = b"<section id=\"exact\"></section>";
        let candidates = extract_from(source);
        let (_, _, span, signature_text) = &candidates[0];
        assert_eq!(std::str::from_utf8(&source[span.start..span.end]).unwrap(), signature_text);
        assert_eq!(signature_text, "<section id=\"exact\">");
    }
}
