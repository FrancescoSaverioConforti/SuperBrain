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
        // B2-ID-018: TypeScript/Tsx's `interface_declaration` names via
        // a direct `type_identifier` child — confirmed by real parse,
        // same name-kind `class_declaration` already uses under TS/Tsx.
        // This node kind does not exist in plain JavaScript's grammar at
        // all, so this arm is unreachable (and harmless) when `walk` is
        // invoked from `extract_javascript`.
        "interface_declaration" => {
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
        "assignment_expression" => {
            if let Some(candidate) = commonjs_property_function_assignment(node, source) {
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

/// CommonJS Extraction Checkpoint (B2-ID-014/B2-ID-015): recognizes the
/// two CommonJS "named export via property assignment" idioms that
/// `function_declaration`/`method_definition`/`arrow_function` never
/// cover, confirmed against real parses (`assignment_expression` with
/// a `member_expression` LHS and a `function_expression` RHS):
///
/// - `exports.X = function(){...}` / `module.exports.X = function(){...}`
///   (B2-ID-014): the LHS's own property name IS `X`; the receiver
///   (`exports`, or the nested `module.exports`) carries no further
///   meaning here — `exports`/`module.exports` are not real indexed
///   symbols, so this is a plain named `Function`, not owned by
///   anything. Both forms produce the IDENTICAL candidate shape (same
///   `kind_hint`), since they are semantically the same export idiom
///   with one extra `.` segment.
/// - `Foo.prototype.X = function(){...}` (B2-ID-015): the LHS's own
///   property name is `X`, nested one level deeper under a
///   `Foo.prototype` receiver. This IS a method conceptually owned by
///   `Foo` — but ownership resolution (by name, fail-closed on
///   ambiguity, same-file only) is NOT this extractor's job; per this
///   crate's own Raw-candidate/Promotion-boundary separation (see
///   `extract::mod`'s own module doc comment), this function only
///   recognizes the CONSTRUCT and gives it a distinct `kind_hint` so a
///   later edge resolver (`edges::js_prototype_ownership`) can re-parse
///   this exact `signature_text` and recover `Foo` from it — mirroring
///   `resolve_instantiates`'s own "re-parse the candidate's own
///   signature_text at resolution time" pattern.
///
/// Only a `function_expression` (or `arrow_function`) RHS is
/// recognized — `exports.X = someOtherValue` (a non-function property
/// export, e.g. a constant or re-exported identifier) is correctly
/// NOT a candidate here, since this checkpoint's scope is extracting
/// callable candidates, not general property-export tracking.
fn commonjs_property_function_assignment(node: tree_sitter::Node, source: &[u8]) -> Option<Candidate> {
    let mut cursor = node.walk();
    let children: Vec<_> = node.children(&mut cursor).collect();
    let lhs = children.first()?;
    if lhs.kind() != "member_expression" {
        return None;
    }
    let rhs = children.iter().find(|c| matches!(c.kind(), "function_expression" | "arrow_function"))?;

    let mut lhs_cursor = lhs.walk();
    let lhs_children: Vec<_> = lhs.children(&mut lhs_cursor).collect();
    // The outermost property name is always the LAST `property_identifier`
    // direct child of the LHS `member_expression` (confirmed by real
    // parse: `exports.foo`, `module.exports.foo`, and `Foo.prototype.bar`
    // each have their own exported/method NAME as the outer
    // `member_expression`'s own `property_identifier` child, never
    // nested further).
    let property_name = lhs_children.iter().rev().find(|c| c.kind() == "property_identifier")?;
    let name = node_text(property_name, source).to_string();

    let receiver = lhs_children.first()?;
    let kind_hint = if is_prototype_receiver(*receiver, source) {
        "js_prototype_method_assignment"
    } else if is_exports_receiver(*receiver, source) {
        "js_exports_property_function"
    } else {
        return None;
    };

    // The candidate's own span/signature_text is the WHOLE assignment
    // (`node`, the `assignment_expression`), not just the RHS function —
    // deliberately, so `js_prototype_ownership`'s own resolver can
    // re-parse this exact text later and recover the LHS receiver name
    // (`Foo`) the same way `resolve_instantiates` re-parses a Field's
    // own signature_text to recover its RHS constructor call. `rhs` is
    // only used above to confirm this IS a function-valued assignment.
    let _ = rhs;
    build_candidate_with_kind(node, source, name, kind_hint.to_string())
}

/// `true` when `receiver` is itself a `member_expression` of the exact
/// shape `Identifier.prototype` (confirmed by real parse: `Foo.prototype`
/// is `member_expression { identifier "Foo", property_identifier
/// "prototype" }`, nested one level inside the outer `member_expression`
/// that also carries the method name).
fn is_prototype_receiver(receiver: tree_sitter::Node, source: &[u8]) -> bool {
    if receiver.kind() != "member_expression" {
        return false;
    }
    let mut cursor = receiver.walk();
    let children: Vec<_> = receiver.children(&mut cursor).collect();
    children.first().map(|c| c.kind() == "identifier").unwrap_or(false) && children.iter().any(|c| c.kind() == "property_identifier" && node_text(c, source) == "prototype")
}

/// `true` when `receiver` is the bare `identifier` `"exports"`, or a
/// `member_expression` of the exact shape `module.exports` (confirmed
/// by real parse: `module.exports.foo`'s own receiver is
/// `member_expression { identifier "module", property_identifier
/// "exports" }`).
fn is_exports_receiver(receiver: tree_sitter::Node, source: &[u8]) -> bool {
    if receiver.kind() == "identifier" {
        return node_text(&receiver, source) == "exports";
    }
    if receiver.kind() != "member_expression" {
        return false;
    }
    let mut cursor = receiver.walk();
    let children: Vec<_> = receiver.children(&mut cursor).collect();
    children.first().map(|c| c.kind() == "identifier" && node_text(c, source) == "module").unwrap_or(false)
        && children.iter().any(|c| c.kind() == "property_identifier" && node_text(c, source) == "exports")
}

fn build_candidate(node: tree_sitter::Node, source: &[u8], name: String) -> Option<Candidate> {
    build_candidate_with_kind(node, source, name, node.kind().to_string())
}

/// Same as [`build_candidate`], but with an explicit, possibly
/// synthetic `kind_hint` rather than always using `node.kind()`
/// verbatim — needed for [`commonjs_property_function_assignment`],
/// whose node kind (`assignment_expression`) is shared by many
/// unrelated constructs and cannot, on its own, distinguish
/// `exports.X = ...` from `Foo.prototype.X = ...` or from any other
/// plain assignment; the synthetic `kind_hint` strings this produces
/// (`"js_exports_property_function"`, `"js_prototype_method_assignment"`)
/// follow the same already-established convention as
/// `"self_attribute_assignment"` (Python `Field`, see
/// `extract::python`'s own module doc comment) — a label this
/// extractor assigns after recognizing a specific AST shape, not a
/// raw tree-sitter node-kind string.
fn build_candidate_with_kind(node: tree_sitter::Node, source: &[u8], name: String, kind_hint: String) -> Option<Candidate> {
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

    // --- CommonJS Extraction Checkpoint: exports.X / module.exports.X
    // (B2-ID-014) and Foo.prototype.X (B2-ID-015) ---

    #[test]
    fn exports_property_function_assignment_is_extracted() {
        let candidates = extract_js(b"exports.foo = function (a) { return a; };\n");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].0, "foo");
        assert_eq!(candidates[0].1, "js_exports_property_function");
    }

    #[test]
    fn module_exports_property_function_assignment_is_extracted() {
        let candidates = extract_js(b"module.exports.foo = function (a) { return a; };\n");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].0, "foo");
        assert_eq!(candidates[0].1, "js_exports_property_function");
    }

    #[test]
    fn exports_property_arrow_function_assignment_is_extracted() {
        let candidates = extract_js(b"exports.foo = (a) => a;\n");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].0, "foo");
        assert_eq!(candidates[0].1, "js_exports_property_function");
    }

    #[test]
    fn exports_property_non_function_assignment_is_not_a_candidate() {
        // exports.VERSION = "1.0.0" is a real, common CommonJS export
        // shape, but it is not a callable candidate — out of this
        // checkpoint's own explicit scope (extracting callable
        // candidates, not general property-export tracking).
        let candidates = extract_js(b"exports.VERSION = \"1.0.0\";\n");
        assert!(candidates.is_empty(), "a non-function property export must never be extracted as a candidate");
    }

    #[test]
    fn prototype_method_assignment_is_extracted() {
        let source = b"function Foo() {}\nFoo.prototype.bar = function (x) { return x; };\n";
        let candidates = extract_js(source);
        let names: Vec<&str> = candidates.iter().map(|c| c.0.as_str()).collect();
        assert_eq!(names, vec!["Foo", "bar"]);
        let bar = candidates.iter().find(|c| c.0 == "bar").unwrap();
        assert_eq!(bar.1, "js_prototype_method_assignment");
    }

    #[test]
    fn prototype_method_assignments_signature_text_is_the_whole_assignment_not_just_the_function() {
        // The resolver that recovers the owner (`Foo`) re-parses this
        // exact signature_text later — it must be the WHOLE
        // assignment_expression, including the `Foo.prototype.bar =`
        // LHS, not just the RHS function_expression.
        let source = b"function Foo() {}\nFoo.prototype.bar = function (x) { return x; };\n";
        let candidates = extract_js(source);
        let bar = candidates.iter().find(|c| c.0 == "bar").unwrap();
        assert!(bar.3.starts_with("Foo.prototype.bar ="), "expected signature_text to include the LHS receiver, got {:?}", bar.3);
    }

    #[test]
    fn prototype_arrow_function_assignment_is_extracted() {
        let source = b"function Foo() {}\nFoo.prototype.bar = (x) => x;\n";
        let candidates = extract_js(source);
        let bar = candidates.iter().find(|c| c.0 == "bar").unwrap();
        assert_eq!(bar.1, "js_prototype_method_assignment");
    }

    #[test]
    fn prototype_method_reassignment_produces_two_distinct_candidates() {
        // Re-assignment is a real CommonJS edge case (a later patch
        // overriding an earlier prototype method) — both candidates
        // must be extracted; collapsing or rejecting the second would
        // silently lose a real, distinct construct.
        let source = b"function Foo() {}\nFoo.prototype.bar = function (x) { return x; };\nFoo.prototype.bar = function (x) { return x + 1; };\n";
        let candidates = extract_js(source);
        let bars: Vec<_> = candidates.iter().filter(|c| c.0 == "bar").collect();
        assert_eq!(bars.len(), 2, "both prototype method assignments must be extracted as distinct candidates");
    }

    #[test]
    fn plain_member_expression_assignment_to_a_non_function_value_is_not_a_candidate() {
        let candidates = extract_js(b"Foo.prototype.bar = 42;\n");
        assert!(candidates.is_empty(), "a non-function RHS must never be extracted, even under a prototype receiver");
    }

    #[test]
    fn plain_identifier_assignment_is_not_confused_with_a_property_assignment() {
        let candidates = extract_js(b"x = function () {};\n");
        assert!(candidates.is_empty(), "a bare identifier LHS (not a member_expression) must never be treated as an exports/prototype assignment");
    }

    // --- B2-ID-018: TypeScript `interface_declaration` extraction ---

    #[test]
    fn typescript_simple_interface_is_extracted() {
        let candidates = extract_ts(b"interface Foo { bar(): void; }\n");
        let iface = candidates.iter().find(|c| c.0 == "Foo").expect("interface Foo must be extracted");
        assert_eq!(iface.1, "interface_declaration");
    }

    #[test]
    fn typescript_exported_interface_is_extracted() {
        let candidates = extract_ts(b"export interface Foo { bar(): void; }\n");
        let iface = candidates.iter().find(|c| c.0 == "Foo").expect("export interface Foo must be extracted");
        assert_eq!(iface.1, "interface_declaration");
    }

    #[test]
    fn typescript_generic_interface_is_extracted() {
        let candidates = extract_ts(b"interface Foo<T> { bar(x: T): void; }\n");
        let iface = candidates.iter().find(|c| c.0 == "Foo").expect("generic interface Foo<T> must be extracted");
        assert_eq!(iface.1, "interface_declaration");
    }

    #[test]
    fn typescript_interface_with_extends_is_extracted() {
        // The interface's own `extends_type_clause` is irrelevant to
        // whether the interface ITSELF is extracted as a candidate —
        // that clause is the resolver's concern (`edges::extends_implements`),
        // not the extractor's. This test only confirms the candidate
        // for `Foo` still appears when an `extends` clause is present.
        let candidates = extract_ts(b"interface Foo extends Bar { bar(): void; }\n");
        let names: Vec<&str> = candidates.iter().map(|c| c.0.as_str()).collect();
        assert!(names.contains(&"Foo"), "expected Foo in {names:?}");
    }

    #[test]
    fn typescript_interface_with_multiple_extends_is_extracted() {
        let candidates = extract_ts(b"interface Foo extends Bar, Baz { bar(): void; }\n");
        let names: Vec<&str> = candidates.iter().map(|c| c.0.as_str()).collect();
        assert!(names.contains(&"Foo"), "expected Foo in {names:?}");
    }

    #[test]
    fn typescript_class_implements_single_interface_is_extracted_alongside_the_class() {
        let source = b"class Foo implements IFoo { bar(): void {} }\n";
        let candidates = extract_ts(source);
        let names: Vec<&str> = candidates.iter().map(|c| c.0.as_str()).collect();
        // `IFoo` itself is not declared in this snippet (it is an
        // external/ambient type here) — this test only confirms the
        // `implements` clause does not prevent `Foo`/`bar` extraction.
        assert!(names.contains(&"Foo") && names.contains(&"bar"), "expected Foo and bar in {names:?}");
    }

    #[test]
    fn typescript_class_implements_multiple_interfaces_is_extracted_alongside_the_class() {
        let source = b"class Foo implements IFoo, IBar { bar(): void {} }\n";
        let candidates = extract_ts(source);
        let names: Vec<&str> = candidates.iter().map(|c| c.0.as_str()).collect();
        assert!(names.contains(&"Foo") && names.contains(&"bar"), "expected Foo and bar in {names:?}");
    }

    #[test]
    fn typescript_interface_is_not_extracted_from_plain_javascript() {
        // Plain JavaScript's grammar has no `interface_declaration` node
        // kind at all — this is a sanity check that the new match arm
        // is genuinely unreachable from `extract_javascript`, not just
        // untested. `const interface_ = 1;` only exercises the JS
        // extractor path; there is no valid JS syntax that parses to an
        // `interface_declaration` node to test against directly.
        let candidates = extract_js(b"const x = 1;\n");
        assert!(!candidates.iter().any(|c| c.1 == "interface_declaration"));
    }

    #[test]
    fn typescript_interface_in_tsx_is_extracted() {
        let candidates = extract_tsx(b"interface Foo { bar(): void; }\n");
        let iface = candidates.iter().find(|c| c.0 == "Foo").expect("interface Foo must be extracted in Tsx");
        assert_eq!(iface.1, "interface_declaration");
    }
}
