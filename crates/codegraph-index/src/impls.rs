//! B2-ID-008, Task 1: raw `impl` block extraction — a non-symbol
//! "structural fact," following exactly the same architectural pattern
//! `imports.rs` already established for file-level import facts (see
//! `imports.rs`'s own doc comment for the full rationale, which applies
//! here unchanged): a real Rust `impl` block is not "owned" by any one
//! symbol it contains, so it is modeled as its own fact type rather than
//! forced into `Edge { from: SymbolSnapshotId, .. }`.
//!
//! # The exact defect this module fixes
//!
//! `docs/research/findings/B2-ID-008.md` found that Rust `impl` blocks
//! are never extracted as symbols at all (`extract::rust` recognizes
//! `struct_item`/`enum_item`/`trait_item`/`function_item`/
//! `function_signature_item`, but no `impl_item`), so trait-implementation
//! relationships are entirely invisible to this pipeline. This module
//! does not change `extract::rust` or introduce a new `SymbolKind` —
//! that would wrongly claim an `impl` block is itself a named,
//! independently-addressable symbol, which it is not (it has no name of
//! its own; `impl Trait for Type` is a relationship between two other
//! symbols). Instead, [`extract_impl_statements`] extracts `impl_item`
//! nodes directly from the whole-file tree, exactly the same
//! file/module-level granularity `imports.rs` already uses for the same
//! reason (see `imports.rs`'s own "Why a separate model, not an `Edge`"
//! section) — an `impl` block is no more "owned" by one symbol in the
//! file than an import statement is.
//!
//! # Named-field extraction only — never a positional search
//!
//! A grammar probe (confirmed by direct parse against this workspace's
//! `tree_sitter_rust` version, not assumed) showed `impl_item` exposes
//! two named fields: `"trait"` (present only for a trait impl, e.g.
//! `impl Trait for Type`) and `"type"` (always present — the
//! implementing type). The `"type"` field's own node kind varies: a
//! plain type is a `type_identifier` directly; a parameterized type
//! (`impl<T> Container<T>`) is a `generic_type` whose OWN first child is
//! the base type's `type_identifier` (`generic_type`'s text is the full
//! `"Container<T>"`; this module extracts only the base name
//! `"Container"`). This module always reads these two fields via
//! `child_by_field_name`, NEVER via a positional search for a
//! `type_identifier` child — a positional search is exactly the bug
//! class B2-ID-008 is scoped to fix (the same mistake, if made here,
//! would misidentify the TRAIT's own `type_identifier` as the
//! implementing type, or vice versa, whenever both are plain
//! `type_identifier` nodes at the same syntactic depth).
//!
//! # Scope boundary: raw syntax only, Rust only
//!
//! This module is pure syntax extraction, like `extract_import_statements`:
//! no resolution against a symbol-snapshot pool, no `Edge` construction,
//! no `ImplFact` (the resolved/typed counterpart — Task 2's job,
//! dispatched separately after this task is reviewed). Only
//! `Language::Rust` is wired into [`extract_impl_statements`]'s language
//! dispatch; generalizing to another language's own impl/trait-ish
//! syntax is explicitly out of scope for B2-ID-008.

use crate::extract::ByteSpan;
use crate::parse::Language;

/// One syntactically-extracted `impl` block, before any resolution
/// against a symbol-snapshot pool. `self_type_text` is always the base
/// type name only (generic arguments stripped, e.g. `"Container"` for
/// `impl<T> Container<T>`, never the full `"Container<T>"` text).
/// `trait_type_text` is `None` for an inherent impl (`impl Type { .. }`)
/// and `Some` for a trait impl (`impl Trait for Type { .. }`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawImplStatement {
    pub self_type_text: String,
    pub trait_type_text: Option<String>,
    pub has_generics: bool,
    pub has_lifetime_params: bool,
    pub span: ByteSpan,
    pub associated_items: Vec<AssociatedItemFact>,
}

/// One item declared directly inside a specific `impl` block (not
/// resolved against anything — just "this impl block's own
/// `declaration_list` contains a function_item shaped like this").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssociatedItemFact {
    pub name: String,
    pub kind: AssociatedItemKind,
    pub span: ByteSpan,
}

/// Which of the two shapes this plan's scope distinguishes an
/// associated `function_item` as. Deliberately only two variants: see
/// this module's module-level doc comment and the plan's own Task 1
/// brief for why a third "default trait method" variant is explicitly
/// out of scope here (that distinction belongs to the trait's own
/// declaration, not to what one specific impl block declares).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssociatedItemKind {
    /// A `function_item` whose `parameters` child has a `self_parameter`
    /// child — takes a receiver (`&self`, `&mut self`, or by-value
    /// `self`).
    Method,
    /// A `function_item` with a body but no receiver parameter — the
    /// `DirEntry::normal`/`CommandSet::new` constructor shape.
    AssociatedFunction,
}

/// Extracts every `impl` block found anywhere in `tree` (the WHOLE
/// file's own parse tree — not one symbol's signature text; see this
/// module's doc comment for why that granularity matches
/// `extract_import_statements`'s own). Pure syntax extraction: never
/// resolves `self_type_text`/`trait_type_text` against anything: this
/// function alone never consults a symbol-snapshot pool.
pub fn extract_impl_statements(tree: &crate::parse::ParseTree) -> Vec<RawImplStatement> {
    let mut out = Vec::new();
    walk_for_impls(tree.tree().root_node(), tree.source(), tree.language(), &mut out);
    out
}

fn walk_for_impls(node: tree_sitter::Node, source: &[u8], language: Language, out: &mut Vec<RawImplStatement>) {
    if language == Language::Rust {
        collect_rust_impl(node, source, out);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_for_impls(child, source, language, out);
    }
}

fn collect_rust_impl(node: tree_sitter::Node, source: &[u8], out: &mut Vec<RawImplStatement>) {
    if node.kind() != "impl_item" {
        return;
    }
    let Some(type_field) = node.child_by_field_name("type") else {
        // No "type" field at all is not a shape a well-formed
        // `impl_item` can produce (confirmed by direct parse across
        // every fixture this module's own tests cover); prefer omitting
        // the fact over fabricating a self type, mirroring
        // `extract::rust`'s own "prefer omit over false certainty"
        // policy for a missing expected name.
        return;
    };
    let Some(self_type_text) = base_type_name(type_field, source) else {
        return;
    };

    let trait_type_text = node.child_by_field_name("trait").and_then(|trait_field| base_type_name(trait_field, source));

    let type_parameters = node.children(&mut node.walk()).find(|c| c.kind() == "type_parameters");
    let has_generics = type_parameters.is_some();
    let has_lifetime_params = type_parameters.is_some_and(|tp| tp.children(&mut tp.walk()).any(|c| c.kind() == "lifetime_parameter"));

    let associated_items = node
        .child_by_field_name("body")
        .or_else(|| node.children(&mut node.walk()).find(|c| c.kind() == "declaration_list"))
        .map(|body| collect_associated_items(body, source))
        .unwrap_or_default();

    out.push(RawImplStatement {
        self_type_text,
        trait_type_text,
        has_generics,
        has_lifetime_params,
        span: ByteSpan::from_node(&node),
        associated_items,
    });
}

/// Extracts the base type name out of a `type`-field or `trait`-field
/// node: directly for a `type_identifier`, or from its OWN first
/// `type_identifier` child for a `generic_type` (never the full
/// `"Name<Args>"` text). Returns `None` for any other node kind (e.g. a
/// `scoped_type_identifier` or a primitive type used as a trait/self
/// type) — out of this plan's own scope per the brief's named-field
/// instruction, which only describes `type_identifier` and
/// `generic_type` shapes; prefer omitting the fact over guessing at an
/// unconfirmed shape.
fn base_type_name(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    match node.kind() {
        "type_identifier" => Some(text_of(node, source)),
        "generic_type" => {
            let mut cursor = node.walk();
            let base = node.children(&mut cursor).find(|c| c.kind() == "type_identifier")?;
            Some(text_of(base, source))
        }
        _ => None,
    }
}

fn collect_associated_items(declaration_list: tree_sitter::Node, source: &[u8]) -> Vec<AssociatedItemFact> {
    let mut out = Vec::new();
    let mut cursor = declaration_list.walk();
    for child in declaration_list.children(&mut cursor) {
        if child.kind() != "function_item" {
            continue;
        }
        let Some(name_node) = child.child_by_field_name("name") else {
            continue;
        };
        let name = text_of(name_node, source);
        let kind = classify_associated_item(child);
        out.push(AssociatedItemFact {
            name,
            kind,
            span: ByteSpan::from_node(&child),
        });
    }
    out
}

fn classify_associated_item(function_item: tree_sitter::Node) -> AssociatedItemKind {
    let has_receiver = function_item
        .child_by_field_name("parameters")
        .is_some_and(|params| params.children(&mut params.walk()).any(|c| c.kind() == "self_parameter"));
    if has_receiver {
        AssociatedItemKind::Method
    } else {
        AssociatedItemKind::AssociatedFunction
    }
}

fn text_of(node: tree_sitter::Node, source: &[u8]) -> String {
    node.utf8_text(source).unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{LanguageParser, RustParser};

    fn parse_rust(source: &str) -> crate::parse::ParseTree {
        RustParser.parse(source.as_bytes()).unwrap()
    }

    #[test]
    fn trait_impl_is_extracted_with_both_type_names() {
        let tree = parse_rust("trait Trait {} struct Type; impl Trait for Type { fn foo(&self) {} }");
        let facts = extract_impl_statements(&tree);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].trait_type_text, Some("Trait".to_string()));
        assert_eq!(facts[0].self_type_text, "Type");
    }

    #[test]
    fn inherent_impl_has_no_trait_type_text() {
        let tree = parse_rust("struct Type; impl Type { fn new() -> Self { Type } }");
        let facts = extract_impl_statements(&tree);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].trait_type_text, None);
        assert_eq!(facts[0].self_type_text, "Type");
    }

    #[test]
    fn generic_impl_strips_generic_args_from_self_type_text_and_sets_has_generics() {
        let tree = parse_rust("struct Container<T>(T); impl<T> Container<T> { fn get(&self) -> &T { &self.0 } }");
        let facts = extract_impl_statements(&tree);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].self_type_text, "Container");
        assert!(facts[0].has_generics);
        assert!(!facts[0].has_lifetime_params);
    }

    #[test]
    fn impl_with_lifetime_parameter_sets_has_lifetime_params() {
        let tree = parse_rust("struct Holder<'a>(&'a str); impl<'a> Holder<'a> { fn get(&self) -> &'a str { self.0 } }");
        let facts = extract_impl_statements(&tree);
        assert_eq!(facts.len(), 1);
        assert!(facts[0].has_lifetime_params);
        assert!(facts[0].has_generics, "a lifetime parameter is still a type_parameters child");
    }

    #[test]
    fn multiple_impl_blocks_for_the_same_type_are_extracted_as_two_separate_facts() {
        let tree = parse_rust(
            "trait Bar {} struct Foo; impl Foo { fn inherent_method(&self) {} } impl Bar for Foo { fn bar_method(&self) {} }",
        );
        let facts = extract_impl_statements(&tree);
        assert_eq!(facts.len(), 2, "expected two separate RawImplStatements, never merged into one");
        assert!(facts.iter().any(|f| f.trait_type_text.is_none() && f.self_type_text == "Foo"));
        assert!(facts.iter().any(|f| f.trait_type_text == Some("Bar".to_string()) && f.self_type_text == "Foo"));
    }

    #[test]
    fn multiple_traits_implemented_by_the_same_type_across_separate_blocks_each_get_their_own_fact() {
        let tree = parse_rust("trait A {} trait B {} struct Foo; impl A for Foo { fn a(&self) {} } impl B for Foo { fn b(&self) {} }");
        let facts = extract_impl_statements(&tree);
        assert_eq!(facts.len(), 2);
        assert!(facts.iter().any(|f| f.trait_type_text == Some("A".to_string()) && f.self_type_text == "Foo"));
        assert!(facts.iter().any(|f| f.trait_type_text == Some("B".to_string()) && f.self_type_text == "Foo"));
    }

    #[test]
    fn one_trait_implemented_by_multiple_different_types_each_get_their_own_fact() {
        let tree = parse_rust(
            "trait Shape {} struct Circle; struct Square; impl Shape for Circle { fn area(&self) {} } impl Shape for Square { fn area(&self) {} }",
        );
        let facts = extract_impl_statements(&tree);
        assert_eq!(facts.len(), 2);
        assert!(facts.iter().any(|f| f.trait_type_text == Some("Shape".to_string()) && f.self_type_text == "Circle"));
        assert!(facts.iter().any(|f| f.trait_type_text == Some("Shape".to_string()) && f.self_type_text == "Square"));
    }

    #[test]
    fn associated_items_are_correctly_classified_as_method_or_associated_function() {
        let tree = parse_rust("struct S; impl S { fn new() -> Self { S } fn bump(&mut self) {} }");
        let facts = extract_impl_statements(&tree);
        assert_eq!(facts.len(), 1);
        let items = &facts[0].associated_items;
        assert_eq!(items.len(), 2);
        let new_item = items.iter().find(|i| i.name == "new").unwrap();
        assert_eq!(new_item.kind, AssociatedItemKind::AssociatedFunction);
        let bump_item = items.iter().find(|i| i.name == "bump").unwrap();
        assert_eq!(bump_item.kind, AssociatedItemKind::Method);
    }

    #[test]
    fn generic_trait_impl_extracts_both_names_via_named_fields_not_positional_search() {
        // The exact shape the OLD positional-search bug would have
        // gotten wrong: `impl<T: Display> Describe for Wrapper<T>` has
        // a `type_identifier` ("Describe") at the same syntactic depth
        // as `generic_type`'s own `type_identifier` ("Wrapper"). A
        // positional "find the first type_identifier child" search
        // would conflate the trait name with the self type (or vice
        // versa); only `child_by_field_name("trait")` /
        // `child_by_field_name("type")` get both right.
        let tree = parse_rust("use std::fmt::Display; trait Describe { fn describe(&self); } struct Wrapper<T>(T); impl<T: Display> Describe for Wrapper<T> { fn describe(&self) {} }");
        let facts = extract_impl_statements(&tree);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].trait_type_text, Some("Describe".to_string()));
        assert_eq!(facts[0].self_type_text, "Wrapper".to_string());
    }

    #[test]
    fn parsing_the_same_tree_twice_produces_deterministic_identical_facts() {
        let tree = parse_rust("trait Trait {} struct Type; impl Trait for Type { fn foo(&self) {} }\nimpl Type { fn new() -> Self { Type } }");

        let first = extract_impl_statements(&tree);
        let second = extract_impl_statements(&tree);

        assert_eq!(first, second);
    }
}
