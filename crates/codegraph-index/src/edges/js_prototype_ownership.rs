//! CommonJS Extraction Checkpoint: `Foo.prototype.X = function(){...}`
//! ownership (B2-ID-015). Produces a `Contains` edge from `Foo`'s own
//! `Function`/`Class` snapshot to the `Method` snapshot
//! `extract::javascript`'s `commonjs_property_function_assignment`
//! extracted for `X`, exactly when `Foo` is found, resolvable to
//! EXACTLY ONE candidate, in the SAME FILE as the assignment itself.
//!
//! # Why `Contains`, not a new `EdgeKind`
//!
//! `Contains` (Task 9.5, Priority 2) already means "this `Class`/
//! constructor owns this `Method`/`Field`" everywhere else in this
//! pipeline — `resolve_contains`'s own AST-span-nesting mechanism
//! cannot see this relationship (the method's body is a SIBLING
//! statement, not nested inside the constructor function's own span;
//! see this module's own re-parse-by-name approach below for why), but
//! the semantic claim being made is identical, so reusing `Contains`
//! is the honest choice over inventing a near-duplicate `EdgeKind`
//! that would mean the same thing under a different name.
//!
//! # Why this needs its own resolver, not `resolve_contains`
//!
//! `resolve_contains` determines ownership purely from byte-span
//! nesting: `B` is contained by `A` when `B`'s span is strictly inside
//! `A`'s span, in the same file. For `function Foo() {}` followed by
//! `Foo.prototype.bar = function(x) {...}`, confirmed by direct parse:
//! `bar`'s own candidate span (the whole `assignment_expression`,
//! `Foo.prototype.bar = function(x) {...}`) is a SIBLING top-level
//! statement, NOT nested inside `Foo`'s own `function_declaration`
//! span at all. There is no span-nesting relationship for
//! `resolve_contains` to find here, by construction — this is a
//! fundamentally different ownership mechanism (resolved by NAME,
//! across sibling statements), not a variant of span containment.
//!
//! # Fail-closed on ambiguity — no false ownership
//!
//! Mirrors `resolve_instantiates`'s own "re-parse this candidate's own
//! `signature_text`, resolve the recovered name by exact match, no
//! edge unless EXACTLY ONE candidate" discipline, with one addition:
//! candidates are restricted to the SAME FILE as the assignment
//! (mirroring `resolve_contains`'s own same-file requirement) and to
//! `SymbolKind::Function | SymbolKind::Class` (the only kinds a real
//! constructor/prototype owner can plausibly be). If `Foo` is not
//! found at all, or resolves to 2+ same-named same-file candidates, or
//! the only name-matching candidate lives in a DIFFERENT file, no edge
//! is produced — never an arbitrary pick, never a cross-file guess.
//! This correctly fails closed on the real ambiguous-owner shape
//! (`var Foo = require('./other'); Foo.prototype.bar = function(){};`
//! — `Foo` here is an imported binding, never a `Function`/`Class`
//! snapshot in the given pool, so this resolver produces zero
//! candidates and, correctly, no edge).

use codegraph_domain::{EdgeKind, SymbolKind};

use super::resolution::{resolved_evidence, Edge, EdgeResolutionError, TargetLookup};
use crate::parse::parser_for;
use crate::snapshot::SymbolSnapshot;

pub fn resolve_js_prototype_ownership(snapshots: &[SymbolSnapshot]) -> Result<Vec<Edge>, EdgeResolutionError> {
    let lookup = TargetLookup::new(snapshots)?;
    let mut edges = Vec::new();

    for method in lookup.snapshots() {
        if method.kind() != SymbolKind::Method {
            continue;
        }
        let Some(owner_name) = prototype_owner_name(method) else {
            continue;
        };

        let candidates: Vec<&SymbolSnapshot> = lookup
            .find_by_name(&owner_name, method.id())
            .into_iter()
            .filter(|candidate| matches!(candidate.kind(), SymbolKind::Function | SymbolKind::Class))
            .filter(|candidate| same_file(candidate, method))
            .collect();

        if candidates.len() != 1 {
            // 0: `Foo` is not a real, same-file Function/Class snapshot
            // in the given pool (e.g. it is an imported/external
            // binding, as in the require()-aliased ambiguous-owner
            // case) — no edge. >1: two same-named same-file candidates
            // — genuinely ambiguous, never an arbitrary pick.
            continue;
        }
        let owner = candidates[0];

        let evidence = resolved_evidence(&format!(
            "{:?}.prototype.{:?} = function(...) resolves {owner_name:?} to exactly one same-file Function/Class candidate",
            owner_name,
            method.name().as_str()
        ));
        edges.push(Edge {
            from: owner.id(),
            to: method.id(),
            kind: EdgeKind::Contains,
            evidence,
        });
    }

    Ok(edges)
}

/// Re-parses `method`'s own `signature_text` (the WHOLE
/// `Foo.prototype.bar = function(...) {...}` assignment, exactly as
/// `commonjs_property_function_assignment` captured it — see that
/// function's own doc comment for why the full assignment, not just
/// the RHS, is stored) and recovers `Foo`, the prototype receiver's
/// own name — `None` if `method` is not a `js_prototype_method_assignment`
/// candidate at all (its re-parsed text has no `member_expression`
/// LHS of the `Identifier.prototype.X` shape), or if `method`'s file
/// has no known parser.
fn prototype_owner_name(method: &SymbolSnapshot) -> Option<String> {
    let parser = parser_for(method.file())?;
    let tree = parser.parse(method.signature().as_str().as_bytes()).ok()?;
    let root = tree.tree().root_node();
    let source = tree.source();

    // The re-parsed fragment's own root is a `program` wrapping exactly
    // the one `expression_statement`/`assignment_expression` this
    // candidate's own signature_text consists of — walk down to find
    // it directly, mirroring `resolve_instantiates`'s own
    // `call_names_in_tree`-style "re-parse one candidate's own
    // fragment, extract exactly the one shape expected" approach,
    // rather than a general recursive walk (there is exactly one
    // relevant assignment in this fragment, by construction — this
    // candidate's own signature_text IS that one assignment's text).
    let assignment = find_first_descendant(root, "assignment_expression")?;
    let mut cursor = assignment.walk();
    let lhs = assignment.children(&mut cursor).next()?;
    if lhs.kind() != "member_expression" {
        return None;
    }
    let mut lhs_cursor = lhs.walk();
    let lhs_children: Vec<_> = lhs.children(&mut lhs_cursor).collect();
    let receiver = lhs_children.first()?;
    if receiver.kind() != "member_expression" {
        return None;
    }
    let mut receiver_cursor = receiver.walk();
    let receiver_children: Vec<_> = receiver.children(&mut receiver_cursor).collect();
    let owner_identifier = receiver_children.first()?;
    if owner_identifier.kind() != "identifier" {
        return None;
    }
    let is_prototype = receiver_children.iter().any(|c| c.kind() == "property_identifier" && c.utf8_text(source).unwrap_or("") == "prototype");
    if !is_prototype {
        return None;
    }
    Some(owner_identifier.utf8_text(source).unwrap_or("").to_string())
}

fn find_first_descendant<'a>(node: tree_sitter::Node<'a>, kind: &str) -> Option<tree_sitter::Node<'a>> {
    if node.kind() == kind {
        return Some(node);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(found) = find_first_descendant(child, kind) {
            return Some(found);
        }
    }
    None
}

fn same_file(a: &SymbolSnapshot, b: &SymbolSnapshot) -> bool {
    a.file().repository() == b.file().repository() && a.file().commit() == b.file().commit() && a.file().relative_path() == b.file().relative_path()
}

#[cfg(test)]
mod tests {
    use super::*;
    use codegraph_domain::{CommitHash, FileIdentity, RepositoryRegistry, RepositoryRoot};

    use crate::extract::{extract_symbols, ByteSpan, RawSymbolCandidate};
    use crate::parse::{JavaScriptParser, LanguageParser};
    use crate::snapshot::promote;

    fn test_repo() -> codegraph_domain::RepositoryId {
        let mut registry = RepositoryRegistry::new();
        registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap())
    }

    fn test_file(repo: codegraph_domain::RepositoryId, path: &str) -> FileIdentity {
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        FileIdentity::parse(repo, commit, path).unwrap()
    }

    /// Indexes real JS source through the real extractor
    /// (`extract_symbols`) and promotes every candidate — the same
    /// end-to-end shape `resolve_contains`'s own tests use for
    /// Class->Method, so this resolver's tests exercise the REAL
    /// extraction output, not a hand-built candidate that might not
    /// match what `commonjs_property_function_assignment` actually
    /// produces.
    fn index_js(repo: codegraph_domain::RepositoryId, path: &str, source: &str) -> Vec<SymbolSnapshot> {
        let file = test_file(repo, path);
        let tree = JavaScriptParser.parse(source.as_bytes()).unwrap();
        extract_symbols(&tree).into_iter().map(|candidate| promote(&candidate, file.clone()).unwrap()).collect()
    }

    fn manual_snapshot(repo: codegraph_domain::RepositoryId, path: &str, name: &str, kind_hint: &str, signature: &str, start: usize, end: usize) -> SymbolSnapshot {
        let file = test_file(repo, path);
        let candidate = RawSymbolCandidate::new(name.to_string(), kind_hint.to_string(), ByteSpan { start, end }, signature.to_string()).unwrap();
        promote(&candidate, file).unwrap()
    }

    #[test]
    fn a_real_polly_shaped_prototype_method_resolves_to_its_constructor_owner() {
        let repo = test_repo();
        let snapshots = index_js(repo, "request.js", "function Request(options) {}\nRequest.prototype.init = function (options) { return options; };\n");

        let edges = resolve_js_prototype_ownership(&snapshots).unwrap();

        let owner = snapshots.iter().find(|s| s.name().as_str() == "Request").unwrap();
        let method = snapshots.iter().find(|s| s.name().as_str() == "init").unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].from, owner.id());
        assert_eq!(edges[0].to, method.id());
        assert_eq!(edges[0].kind, EdgeKind::Contains);
        assert_eq!(edges[0].evidence.method(), codegraph_domain::ResolutionMethod::StaticAnalysis);
    }

    #[test]
    fn multiple_prototype_methods_on_the_same_constructor_each_get_their_own_edge() {
        let repo = test_repo();
        let snapshots = index_js(
            repo,
            "request.js",
            "function Request() {}\nRequest.prototype.init = function () {};\nRequest.prototype.start = function () {};\n",
        );

        let edges = resolve_js_prototype_ownership(&snapshots).unwrap();

        let owner = snapshots.iter().find(|s| s.name().as_str() == "Request").unwrap();
        assert_eq!(edges.len(), 2);
        assert!(edges.iter().all(|e| e.from == owner.id() && e.kind == EdgeKind::Contains));
    }

    #[test]
    fn an_owner_not_present_in_the_pool_at_all_produces_no_edge() {
        // `Foo` is referenced but never declared anywhere in this file
        // (e.g. it lives in a file this indexing pass was not given) —
        // zero candidates, no edge, never invented.
        let repo = test_repo();
        let snapshots = index_js(repo, "a.js", "Foo.prototype.bar = function () {};\n");

        let edges = resolve_js_prototype_ownership(&snapshots).unwrap();

        assert!(edges.is_empty(), "an owner with zero real candidates must never produce an edge");
    }

    #[test]
    fn an_ambiguous_owner_required_by_a_name_collision_produces_no_edge() {
        // Two distinct real Function snapshots named `Foo` in the same
        // file (a real, if unusual, shape — e.g. a hoisted redeclaration)
        // must fail closed, never arbitrarily pick one.
        let repo = test_repo();
        let foo_a = manual_snapshot(repo, "a.js", "Foo", "function_declaration", "function Foo() {}", 0, 20);
        let foo_b = manual_snapshot(repo, "a.js", "Foo", "function_declaration", "function Foo() {}", 20, 40);
        let method = manual_snapshot(repo, "a.js", "bar", "js_prototype_method_assignment", "Foo.prototype.bar = function () {};", 40, 76);
        let snapshots = vec![foo_a, foo_b, method];

        let edges = resolve_js_prototype_ownership(&snapshots).unwrap();

        assert!(edges.is_empty(), "an ambiguous same-file owner name must never be arbitrarily picked");
    }

    #[test]
    fn an_owner_whose_only_name_match_lives_in_a_different_file_produces_no_edge() {
        // The real require()-aliased ambiguous-owner shape's cousin:
        // even a real Function named `Foo` existing SOMEWHERE in the
        // pool must not be treated as this assignment's owner unless
        // it is in the SAME file — a same-named Function in another
        // file is not evidence of real ownership here.
        let repo = test_repo();
        let foo_elsewhere = manual_snapshot(repo, "other.js", "Foo", "function_declaration", "function Foo() {}", 0, 20);
        let method = manual_snapshot(repo, "a.js", "bar", "js_prototype_method_assignment", "Foo.prototype.bar = function () {};", 0, 36);
        let snapshots = vec![foo_elsewhere, method];

        let edges = resolve_js_prototype_ownership(&snapshots).unwrap();

        assert!(edges.is_empty(), "a same-named Function in a DIFFERENT file must never be treated as this assignment's real owner");
    }

    #[test]
    fn a_require_aliased_owner_produces_no_edge_the_real_ambiguous_owner_case() {
        // `var Foo = require('./other'); Foo.prototype.bar = function(){};`
        // -- `Foo` here is an imported binding, never extracted as a
        // Function/Class snapshot at all (require() bindings are not
        // symbols this pipeline extracts) — zero candidates, no edge,
        // confirming this resolver fails closed on the real
        // ownership-ambiguous shape from this checkpoint's own
        // directive, not just a synthetic one.
        let repo = test_repo();
        let snapshots = index_js(repo, "a.js", "var Foo = require('./other');\nFoo.prototype.bar = function () {};\n");

        let edges = resolve_js_prototype_ownership(&snapshots).unwrap();

        assert!(edges.is_empty(), "an owner bound only via require() must never be treated as a real Function/Class owner");
    }

    #[test]
    fn a_member_expression_receiver_whose_property_is_not_literally_prototype_is_not_recognized() {
        // `Foo.bar.baz = function(){}` -- the receiver (`Foo.bar`) is a
        // real member_expression with a real identifier first child
        // and a real property_identifier, but that property is "bar",
        // not "prototype". This exact shape is never extracted as a
        // candidate by `commonjs_property_function_assignment` in the
        // first place (its own is_prototype_receiver/is_exports_receiver
        // checks both reject it) -- this test instead hand-builds a
        // Method snapshot whose signature_text directly exercises
        // prototype_owner_name's own fail-closed check, confirming the
        // resolver itself is not fooled by "any property_identifier
        // sibling exists" and specifically requires the text "prototype".
        // Pins the `&&` in `c.kind() == "property_identifier" &&
        // c.utf8_text(source).unwrap_or("") == "prototype"` against a
        // `||` mutant, which would incorrectly treat ANY property name
        // under an identifier receiver as a prototype assignment.
        let repo = test_repo();
        let foo = manual_snapshot(repo, "a.js", "Foo", "function_declaration", "function Foo() {}", 0, 20);
        let method = manual_snapshot(repo, "a.js", "baz", "js_prototype_method_assignment", "Foo.bar.baz = function () {};", 20, 50);
        let snapshots = vec![foo, method];

        let edges = resolve_js_prototype_ownership(&snapshots).unwrap();

        assert!(edges.is_empty(), "a receiver whose property is not literally 'prototype' must never be treated as a prototype-method assignment, even though Foo is a real same-file Function");
    }

    #[test]
    fn a_non_prototype_method_snapshot_is_never_considered_as_a_source() {
        // A real class method_definition (e.g. `class C { bar() {} }`)
        // is also SymbolKind::Method, but its signature_text is not a
        // Foo.prototype.X assignment at all — prototype_owner_name
        // must return None for it, never spuriously matching.
        let repo = test_repo();
        let snapshots = index_js(repo, "a.js", "class C { bar() { return 1; } }\n");

        let edges = resolve_js_prototype_ownership(&snapshots).unwrap();

        assert!(edges.is_empty(), "a real class method_definition must never be mistaken for a prototype-method assignment");
    }

    #[test]
    fn a_duplicate_snapshot_id_is_a_typed_rejection() {
        let repo = test_repo();
        let snapshot = manual_snapshot(repo, "a.js", "Foo", "function_declaration", "function Foo() {}", 0, 20);
        let snapshots = vec![snapshot.clone(), snapshot];

        let result = resolve_js_prototype_ownership(&snapshots);

        assert_eq!(result.err(), Some(EdgeResolutionError::DuplicateSnapshotId));
    }

    #[test]
    fn result_is_deterministic_regardless_of_snapshot_order() {
        let repo = test_repo();
        let snapshots_forward = index_js(repo, "request.js", "function Request() {}\nRequest.prototype.init = function () {};\n");
        let mut snapshots_reversed = snapshots_forward.clone();
        snapshots_reversed.reverse();

        let edges_forward = resolve_js_prototype_ownership(&snapshots_forward).unwrap();
        let edges_reversed = resolve_js_prototype_ownership(&snapshots_reversed).unwrap();

        assert_eq!(edges_forward.len(), 1);
        assert_eq!(edges_forward, edges_reversed);
    }
}
