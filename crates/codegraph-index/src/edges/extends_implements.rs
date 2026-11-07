//! B2 Task 7: `Extends`/`Implements` edges. Per-language syntax
//! confirmed against real parses before writing this logic (see each
//! language's own comment below for the exact node shapes).
//!
//! # Syntactically explicit vs. ambiguous-by-syntax languages
//!
//! Python (`class X(Base)`), Rust (`impl Trait for Type`), JavaScript's
//! `extends` and TypeScript/Tsx's `extends`/`implements` each have a
//! syntax that ALREADY distinguishes Extends from Implements (or, for
//! Python and plain JS, has only one construct, always `Extends` — see
//! below) without needing to know anything about the target's own kind.
//! `base_names_in` tags every collected name with that syntactic
//! `EdgeKind` directly (`Some(kind)`) for these languages, and
//! `edge_kind_for` always honors that tag over any kind lookup. These
//! resolve to `StaticAnalysis` evidence directly once the base name
//! matches exactly one candidate.
//!
//! C# (`class Derived : Base, IFoo`) and C++ (`class Derived : public
//! Base`) do NOT distinguish a base class from an implemented interface
//! syntactically — both appear in the same base-list construct. For
//! these two languages ONLY, `base_names_in` tags each name with `None`
//! (defer), and `edge_kind_for` falls back to looking up the base
//! name's `SymbolKind` among the given candidates: a candidate whose
//! kind is `Interface` resolves to `Implements`; any other matched kind
//! (`Class`, `Struct`, ...) resolves to `Extends`. If the base name is
//! not found among the given candidates at all, NEITHER edge kind is
//! guessed — this resolver does not assume "no known kind means it's
//! probably a class," since that would be exactly the kind of overclaim
//! the task forbids; it produces no edge, per "no edge beats a false
//! edge."
//!
//! Python's base-class list CAN syntactically contain more than one
//! base (multiple inheritance), and this resolver treats each
//! comma-separated base identically (one potential `Extends` edge per
//! base), never assuming only the first matters.
//!
//! # B2-ID-007: nested-declaration ownership boundary
//!
//! Each `collect_*` function below walks the ENTIRE re-parsed fragment
//! of one snapshot's own `signature_text` looking for base/heritage
//! clauses — and, per every current extractor's own design, that
//! fragment is the FULL body of whatever declaration the snapshot
//! represents, which can textually contain an unrelated, NESTED type
//! declaration's own base clause too (e.g. Python's `class Outer:
//! class Inner(Base): ...`). Confirmed, via a real-parse reproduction
//! during B2-ID-007's own resolver-ownership audit, that this
//! resolver shared the identical defect [B2-ID-006](../../../../docs/research/findings/B2-ID-006.md)
//! found in `resolve_calls`: without a boundary, `Outer`'s own
//! re-parse would ALSO rediscover `Inner`'s base clause, spuriously
//! producing `Outer -> Base` alongside the correct `Inner -> Base`.
//!
//! The correct ownership rule here is DIFFERENT from `Calls`'s own
//! "nearest executable scope" invariant — inheritance has no
//! executable-scope concept at all; a base/heritage clause belongs to
//! the SPECIFIC type declaration whose own header/clause syntax names
//! it, full stop. `for_each_node_not_inside_a_nested_boundary` (below)
//! implements exactly this: it visits the outermost declaration
//! `collect_*` is being asked about (always visited, regardless of its
//! own node kind), and recurses into its body, but stops the moment it
//! encounters a SECOND node of one of the given `boundary_kinds` —
//! a genuinely nested declaration — visiting neither that node nor
//! anything beneath it. That nested declaration's own base clause is
//! picked up correctly and independently when `resolve_extends_implements`'s
//! own outer loop reaches ITS specific snapshot on a later iteration.
//! See [B2-ID-007](../../../../docs/research/findings/B2-ID-007.md)
//! for the full investigation, the resolver-ownership audit that found
//! this, and why `resolve_instantiates`/`resolve_contains` were
//! confirmed NOT to share this defect.

use codegraph_domain::{EdgeKind, SymbolKind};

use super::resolution::{resolved_evidence, Edge, EdgeResolutionError, TargetLookup};
use crate::edges::calls::parser_for;
use crate::parse::Language;
use crate::snapshot::SymbolSnapshot;

pub fn resolve_extends_implements(snapshots: &[SymbolSnapshot]) -> Result<Vec<Edge>, EdgeResolutionError> {
    let lookup = TargetLookup::new(snapshots)?;
    let mut edges = Vec::new();

    for snapshot in lookup.snapshots() {
        let Some(parser) = parser_for(snapshot.file()) else {
            continue;
        };
        let Ok(tree) = parser.parse(snapshot.signature().as_str().as_bytes()) else {
            continue;
        };

        // B2-ID-002 scope audit: the same base/interface name appearing
        // more than once in one declaration's base/heritage list (a
        // syntactically odd but parseable shape, e.g. `class C(A, A)`)
        // must collapse to ONE edge per `(to, kind)`, never one per
        // occurrence — confirmed present here too by a dedicated
        // regression test, the same occurrence-vs-relation mismatch
        // `resolve_calls` had. Every edge this resolver produces is
        // `StaticAnalysis` (an ambiguous/unresolved base produces no
        // edge at all), so there is no evidence-strength tie-break to
        // make here either.
        let mut seen_targets: std::collections::HashSet<(codegraph_domain::SymbolSnapshotId, EdgeKind)> = std::collections::HashSet::new();

        for (base_name, syntactic_kind) in base_names_in(&tree) {
            let candidates = lookup.find_by_name(&base_name, snapshot.id());
            if candidates.len() != 1 {
                // Zero: base not found among the given pool (e.g. a
                // stdlib/third-party base type) -> no edge. More than
                // one: ambiguous same-named base across files -> no
                // edge either (unlike Calls, inheritance is a 1:1
                // structural claim about THIS declaration's one actual
                // base, so an ambiguous same-name match cannot honestly
                // be split into multiple heuristic edges the way a call
                // site's multiple real overload candidates can).
                continue;
            }
            let target = candidates[0];
            let kind = edge_kind_for(syntactic_kind, target.kind());
            if !seen_targets.insert((target.id(), kind)) {
                continue;
            }
            let evidence = resolved_evidence(&format!(
                "{:?} base/heritage clause names {base_name:?}, resolved to exactly one candidate of kind {:?}",
                tree.language(),
                target.kind()
            ));
            edges.push(Edge {
                from: snapshot.id(),
                to: target.id(),
                kind,
                evidence,
            });
        }
    }

    Ok(edges)
}

/// Picks the final `EdgeKind` for one resolved base/interface name.
/// `syntactic_kind` is `Some(kind)` when the SOURCE SYNTAX ITSELF
/// already said which kind applies — Python's single base-class
/// construct (always `Extends`), JavaScript's `extends` (always
/// `Extends`), and TypeScript/Tsx's `extends` (`Extends`) vs.
/// `implements` (`Implements`), each tagged at the point `base_names_in`
/// collects them — see `collect_js_heritage`. (Rust's own `impl Trait
/// for Type` is NOT one of these languages: B2-ID-008 moved Rust's
/// trait-implementation handling entirely into `impls.rs`'s `ImplFact`
/// mechanism, so `base_names_in` never collects anything for
/// `Language::Rust` at all — see that match arm's own comment below.)
/// It is `None` only for C#/C++, whose `base_list`/`base_class_clause`
/// syntax does NOT distinguish a base class from an implemented
/// interface at all; for those, and ONLY for those, this function falls
/// back to the target's own already-resolved `SymbolKind` (`Interface`
/// -> `Implements`, anything else -> `Extends`). A syntactic tag, when
/// present, is NEVER overridden by the target's kind — the earlier
/// version of this resolver incorrectly routed TypeScript/Tsx through
/// the same kind-based fallback as C#/C++ despite TS/Tsx's `implements`
/// keyword being just as syntactically explicit as Python's/JS's own
/// `extends`; that was a real defect (TS/Tsx's own syntax was being
/// ignored in favor of a lookup that could disagree with it), fixed here
/// by threading the clause tag through instead of discarding it before
/// this function runs.
fn edge_kind_for(syntactic_kind: Option<EdgeKind>, target_kind: SymbolKind) -> EdgeKind {
    syntactic_kind.unwrap_or_else(|| if target_kind == SymbolKind::Interface { EdgeKind::Implements } else { EdgeKind::Extends })
}

/// Every base-class/interface name found in `tree`'s one construct (a
/// `class_definition`/`class_declaration`/`class_specifier` re-parsed
/// standalone from a `SymbolSnapshot`'s own signature text — the same
/// re-parse-the-signature approach `calls.rs` and `imports.rs` use),
/// paired with the `EdgeKind` the SOURCE SYNTAX implies when the
/// language's grammar makes that unambiguous, or `None` when it does
/// not (C#/C++ only — see `edge_kind_for`'s doc comment for what
/// happens then). Rust's own `impl_item` is deliberately NOT one of the
/// node kinds this function (or anything it dispatches to) ever
/// collects from — see the `Language::Rust` match arm below.
fn base_names_in(tree: &crate::parse::ParseTree) -> Vec<(String, Option<EdgeKind>)> {
    let mut names = Vec::new();
    match tree.language() {
        Language::Python => {
            let mut plain_names = Vec::new();
            collect_python_bases(tree.tree().root_node(), tree.source(), &mut plain_names);
            names.extend(plain_names.into_iter().map(|n| (n, Some(EdgeKind::Extends))));
        }
        Language::JavaScript => {
            // JS has only `extends` (no `implements` keyword at all in
            // this grammar) — always Extends.
            let mut plain_names = Vec::new();
            collect_js_heritage(tree.tree().root_node(), tree.source(), &mut plain_names);
            names.extend(plain_names.into_iter().map(|n| (n, Some(EdgeKind::Extends))));
        }
        Language::TypeScript | Language::Tsx => {
            collect_ts_heritage_tagged(tree.tree().root_node(), tree.source(), &mut names);
        }
        Language::CSharp => {
            let mut plain_names = Vec::new();
            collect_csharp_base_list(tree.tree().root_node(), tree.source(), &mut plain_names);
            names.extend(plain_names.into_iter().map(|n| (n, None)));
        }
        Language::Cpp => {
            let mut plain_names = Vec::new();
            collect_cpp_base_clause(tree.tree().root_node(), tree.source(), &mut plain_names);
            names.extend(plain_names.into_iter().map(|n| (n, None)));
        }
        // Rust yields no `Extends`/`Implements` edges from this resolver by
        // design: `impl Trait for Type` is extracted as an `ImplFact` in
        // `impls.rs` (B2-ID-008), not as a base/heritage name here.
        Language::Rust | Language::Html | Language::Css => {}
    }
    names
}

/// B2-ID-007: visits every descendant of `node` (the SAME full-subtree
/// walk every `collect_*` function already performed before this fix),
/// calling `visit` on each one, EXCEPT any node strictly nested inside
/// a DIFFERENT type declaration than the one `node` itself started in
/// — i.e. once the walk passes a node whose own `kind()` is one of
/// `boundary_kinds` (a NEW type declaration starting, e.g. a nested
/// `class_definition`), neither that boundary node NOR anything
/// beneath it is visited. `node` itself is always visited first
/// (matching every existing `collect_*` function's own original
/// behavior of checking the entry node before recursing into it), and
/// the "we have crossed a boundary" state is threaded explicitly
/// through the recursion via `inside_a_nested_boundary`, not
/// re-derived fresh from each step's own immediate parent — re-deriving
/// it fresh was an earlier, incorrect version of this fix: it only
/// checked a node's DIRECT children for a boundary kind, which misses
/// a nested declaration reachable through an intermediate non-boundary
/// WRAPPER node (confirmed concretely: Python's own `class_definition
/// -> block -> class_definition` shape, where `block` sits between an
/// outer class and a nested one and is not itself a boundary kind,
/// let the nested class through unprotected once the recursive call's
/// own locally-recomputed "is a boundary" check reset at that wrapper
/// step). Threading the flag explicitly, rather than recomputing it,
/// fixes that gap: once `true`, it stays `true` for the rest of that
/// subtree, through any number of intermediate wrapper nodes.
fn for_each_node_not_inside_a_nested_boundary(node: tree_sitter::Node, boundary_kinds: &[&str], mut visit: impl FnMut(tree_sitter::Node)) {
    // `already_entered_one_boundary` tracks whether the walk has
    // already passed through (and visited) ONE boundary-kind node —
    // the outermost declaration itself, which must always be visited
    // regardless of its own kind. Only a SECOND (or deeper) boundary-
    // kind node, found AFTER the first one, is a genuinely NESTED
    // declaration and gets skipped. Without this distinction, the
    // entry declaration itself (whose `kind()` IS a boundary kind,
    // e.g. the outermost `class_definition`) would incorrectly skip
    // itself on the very first step.
    fn walk(node: tree_sitter::Node, boundary_kinds: &[&str], already_entered_one_boundary: bool, visit: &mut impl FnMut(tree_sitter::Node)) {
        let this_node_is_a_boundary = boundary_kinds.contains(&node.kind());
        if this_node_is_a_boundary && already_entered_one_boundary {
            return;
        }
        visit(node);
        let next_already_entered_one_boundary = already_entered_one_boundary || this_node_is_a_boundary;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            walk(child, boundary_kinds, next_already_entered_one_boundary, visit);
        }
    }
    walk(node, boundary_kinds, false, &mut visit);
}

fn collect_python_bases(node: tree_sitter::Node, source: &[u8], out: &mut Vec<String>) {
    for_each_node_not_inside_a_nested_boundary(node, &["class_definition"], |n| {
        if n.kind() != "class_definition" {
            return;
        }
        let mut cursor = n.walk();
        let found_args = n.children(&mut cursor).find(|c| c.kind() == "argument_list");
        if let Some(args) = found_args {
            let mut arg_cursor = args.walk();
            for child in args.children(&mut arg_cursor) {
                match child.kind() {
                    "identifier" => out.push(text_of(child, source)),
                    "attribute" => {
                        // `nn.Module` -> use the LAST identifier
                        // ("Module"), confirmed against this
                        // workspace's real tree-sitter-python output:
                        // an `attribute` node's children are
                        // `identifier . identifier`.
                        let mut attr_cursor = child.walk();
                        if let Some(last) = child.children(&mut attr_cursor).filter(|c| c.kind() == "identifier").last() {
                            out.push(text_of(last, source));
                        }
                    }
                    _ => {}
                }
            }
        }
    });
}

fn collect_csharp_base_list(node: tree_sitter::Node, source: &[u8], out: &mut Vec<String>) {
    for_each_node_not_inside_a_nested_boundary(node, &["class_declaration", "interface_declaration"], |n| {
        if n.kind() != "base_list" {
            return;
        }
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            if child.kind() == "identifier" {
                out.push(text_of(child, source));
            }
        }
    });
}

fn collect_cpp_base_clause(node: tree_sitter::Node, source: &[u8], out: &mut Vec<String>) {
    for_each_node_not_inside_a_nested_boundary(node, &["class_specifier", "struct_specifier"], |n| {
        if n.kind() != "base_class_clause" {
            return;
        }
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            if child.kind() == "type_identifier" {
                out.push(text_of(child, source));
            }
        }
    });
}

fn collect_js_heritage(node: tree_sitter::Node, source: &[u8], out: &mut Vec<String>) {
    // Plain JavaScript grammar only ever has a bare `extends identifier`
    // in `class_heritage` — no `implements` keyword exists in this
    // grammar at all, so every name collected here is unconditionally
    // Extends (tagged by the caller in `base_names_in`).
    for_each_node_not_inside_a_nested_boundary(node, &["class_declaration"], |n| {
        if !matches!(n.kind(), "class_heritage") {
            return;
        }
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            if matches!(child.kind(), "identifier" | "type_identifier") {
                out.push(text_of(child, source));
            }
        }
    });
}

/// TypeScript/Tsx's `class_heritage` can carry BOTH a plain `extends
/// identifier` child AND a nested `implements_clause` on the same
/// class. Unlike plain JS, this grammar's `implements` keyword is a
/// real syntactic marker, just as explicit as Python's base-class list
/// (Rust's own trait-vs-type distinction is no longer this file's own
/// concern at all — see the `Language::Rust` match arm's own comment in
/// `base_names_in`) — so each name is tagged with the
/// `EdgeKind` its OWN clause implies, never left to a target-kind
/// fallback. This is the fix for a real defect: an earlier version of
/// this resolver collected every heritage name into one undifferentiated
/// list and then decided Extends vs. Implements by looking up the
/// resolved target's `SymbolKind`, identical to the C#/C++ fallback —
/// which meant a `class X extends Y` where `Y` happened to be an
/// `Interface`-kind snapshot would have been wrongly reported as
/// `Implements`, and `class X implements Z` where `Z` was NOT
/// `Interface`-kind would have been wrongly reported as `Extends`. Both
/// are now impossible: the clause tag is carried from the parse tree
/// straight through to `edge_kind_for`, which never overrides a
/// syntactic tag with a kind lookup.
fn collect_ts_heritage_tagged(node: tree_sitter::Node, source: &[u8], out: &mut Vec<(String, Option<EdgeKind>)>) {
    // Confirmed against this workspace's real tree-sitter-typescript
    // output: `class_heritage`'s direct children are themselves wrapper
    // nodes — `extends_clause` (containing one `identifier`/
    // `type_identifier`) and/or `implements_clause` (containing one or
    // more `type_identifier`s) — never a bare identifier directly.
    for_each_node_not_inside_a_nested_boundary(node, &["class_declaration"], |n| {
        if !matches!(n.kind(), "class_heritage") {
            return;
        }
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            let tag = match child.kind() {
                "extends_clause" => Some(EdgeKind::Extends),
                "implements_clause" => Some(EdgeKind::Implements),
                _ => None,
            };
            if let Some(tag) = tag {
                let mut clause_cursor = child.walk();
                for grandchild in child.children(&mut clause_cursor) {
                    if matches!(grandchild.kind(), "identifier" | "type_identifier") {
                        out.push((text_of(grandchild, source), Some(tag)));
                    }
                }
            }
        }
    });
}

fn text_of(node: tree_sitter::Node, source: &[u8]) -> String {
    node.utf8_text(source).unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use codegraph_domain::{CommitHash, FileIdentity, RepositoryRegistry, RepositoryRoot};

    use crate::extract::{ByteSpan, RawSymbolCandidate};
    use crate::snapshot::promote;

    fn test_snapshot(path: &str, name: &str, kind_hint: &str, signature: &str) -> SymbolSnapshot {
        let mut registry = RepositoryRegistry::new();
        let repo = registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap());
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        let file = FileIdentity::parse(repo, commit, path).unwrap();
        let candidate = RawSymbolCandidate::new(name.to_string(), kind_hint.to_string(), ByteSpan { start: 0, end: 10 }, signature.to_string()).unwrap();
        promote(&candidate, file).unwrap()
    }

    #[test]
    fn python_class_base_resolves_to_extends() {
        let base = test_snapshot("src/base.py", "Module", "class_definition", "class Module:\n    pass");
        let derived = test_snapshot("src/model.py", "MyModel", "class_definition", "class MyModel(Module):\n    pass");
        let snapshots = vec![base.clone(), derived.clone()];

        let edges = resolve_extends_implements(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].from, derived.id());
        assert_eq!(edges[0].to, base.id());
        assert_eq!(edges[0].kind, EdgeKind::Extends);
        assert_eq!(edges[0].evidence.method(), codegraph_domain::ResolutionMethod::StaticAnalysis);
    }

    #[test]
    fn python_dotted_base_resolves_by_its_last_segment() {
        // The exact deep-learning-relevant shape: class MyModel(nn.Module).
        let module = test_snapshot("src/nn.py", "Module", "class_definition", "class Module:\n    pass");
        let model = test_snapshot("src/model.py", "MyModel", "class_definition", "class MyModel(nn.Module):\n    pass");
        let snapshots = vec![module.clone(), model.clone()];

        let edges = resolve_extends_implements(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to, module.id());
        assert_eq!(edges[0].kind, EdgeKind::Extends);
    }

    #[test]
    fn rust_impl_trait_for_type_produces_zero_extends_or_implements_edges_by_design() {
        // B2-ID-008: `impl Trait for Type` is now extracted by `impls.rs`'s
        // `ImplFact` mechanism, not by this resolver. Pins — against a REAL
        // parse of real Rust source, not a placeholder string — that
        // `resolve_extends_implements` deliberately produces no Extends or
        // Implements edge for Rust, so this is a tested fact rather than an
        // accident of deleting `collect_rust_impl_trait`.
        let source = "trait Shape { fn area(&self) -> f64; }\n\nstruct Circle;\n\nimpl Shape for Circle { fn area(&self) -> f64 { 1.0 } }\n";
        let tree = crate::parse::LanguageParser::parse(&crate::parse::RustParser, source.as_bytes()).expect("the fixture must be valid Rust");
        let root = tree.tree().root_node();
        let mut cursor = root.walk();
        assert!(
            root.children(&mut cursor).any(|c| c.kind() == "impl_item"),
            "the real parse must actually contain an impl_item, or this test would prove nothing"
        );

        let repo = {
            let mut registry = RepositoryRegistry::new();
            registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap())
        };
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        let file = FileIdentity::parse(repo, commit, "src/shapes.rs").unwrap();
        let (candidates, rejections) = crate::extract::extract_symbols_reporting_rejections(&tree);
        assert!(rejections.is_empty(), "fixture source must not trigger any extraction rejection: {rejections:?}");
        let snapshots: Vec<SymbolSnapshot> = candidates.into_iter().map(|c| promote(&c, file.clone()).expect("a real extracted candidate must always promote")).collect();

        let edges = resolve_extends_implements(&snapshots).unwrap();

        let structural_edges: Vec<_> = edges.iter().filter(|e| matches!(e.kind, EdgeKind::Extends | EdgeKind::Implements)).collect();
        assert!(structural_edges.is_empty(), "Rust must produce zero Extends/Implements edges from resolve_extends_implements; ImplFact (impls.rs) is the real mechanism");
    }

    #[test]
    fn javascript_extends_resolves_to_extends() {
        let base = test_snapshot("src/base.js", "Base", "class_declaration", "class Base {}");
        let derived = test_snapshot("src/derived.js", "Derived", "class_declaration", "class Derived extends Base {}");
        let snapshots = vec![base.clone(), derived.clone()];

        let edges = resolve_extends_implements(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].kind, EdgeKind::Extends);
        assert_eq!(edges[0].to, base.id());
    }

    #[test]
    fn typescript_implements_resolves_to_implements_via_syntax() {
        let interface = test_snapshot("src/ifoo.ts", "IFoo", "interface_declaration", "interface IFoo {}");
        let implementor = test_snapshot("src/c.ts", "C", "class_declaration", "class C implements IFoo {}");
        let snapshots = vec![interface.clone(), implementor.clone()];

        let edges = resolve_extends_implements(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].kind, EdgeKind::Implements);
        assert_eq!(edges[0].to, interface.id());
    }

    #[test]
    fn typescript_extends_resolves_to_extends_even_when_the_target_kind_is_interface() {
        // The target's resolved SymbolKind happens to be Interface, but
        // the source syntax says `extends`, not `implements` — the
        // clause tag must win over any kind lookup. Before the fix,
        // this would have been wrongly reported as Implements because
        // TS/Tsx was routed through the same kind-based fallback as
        // C#/C++.
        let base_interface_shaped = test_snapshot("src/ifoo.ts", "IFoo", "interface_declaration", "interface IFoo {}");
        let derived = test_snapshot("src/c.ts", "C", "class_declaration", "class C extends IFoo {}");
        let snapshots = vec![base_interface_shaped.clone(), derived.clone()];

        let edges = resolve_extends_implements(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].kind, EdgeKind::Extends, "extends must resolve to Extends regardless of the target's own SymbolKind");
        assert_eq!(edges[0].to, base_interface_shaped.id());
    }

    #[test]
    fn typescript_implements_resolves_to_implements_even_when_the_target_kind_is_not_interface() {
        // The target's resolved SymbolKind is Class (not Interface), but
        // the source syntax says `implements` — the clause tag must win
        // over any kind lookup. Before the fix, this would have been
        // wrongly reported as Extends.
        let base_class_shaped = test_snapshot("src/base.ts", "Base", "class_declaration", "class Base {}");
        let implementor = test_snapshot("src/c.ts", "C", "class_declaration", "class C implements Base {}");
        let snapshots = vec![base_class_shaped.clone(), implementor.clone()];

        let edges = resolve_extends_implements(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].kind, EdgeKind::Implements, "implements must resolve to Implements regardless of the target's own SymbolKind");
        assert_eq!(edges[0].to, base_class_shaped.id());
    }

    #[test]
    fn tsx_extends_and_implements_on_the_same_class_are_both_tagged_correctly() {
        // TS/Tsx grammar allows both clauses on one class declaration:
        // `class X extends Y implements Z`. Confirms both names are
        // collected AND each keeps its own correct tag, not just
        // whichever clause happens to be processed first.
        let base = test_snapshot("src/base.tsx", "Base", "class_declaration", "class Base {}");
        let interface = test_snapshot("src/iface.tsx", "IFoo", "interface_declaration", "interface IFoo {}");
        let derived = test_snapshot("src/derived.tsx", "Derived", "class_declaration", "class Derived extends Base implements IFoo {}");
        let snapshots = vec![base.clone(), interface.clone(), derived.clone()];

        let edges = resolve_extends_implements(&snapshots).unwrap();

        assert_eq!(edges.len(), 2);
        let extends_edge = edges.iter().find(|e| e.kind == EdgeKind::Extends).expect("an Extends edge must exist");
        assert_eq!(extends_edge.to, base.id());
        let implements_edge = edges.iter().find(|e| e.kind == EdgeKind::Implements).expect("an Implements edge must exist");
        assert_eq!(implements_edge.to, interface.id());
    }

    #[test]
    fn csharp_base_list_resolves_implements_vs_extends_by_target_kind() {
        let base_class = test_snapshot("src/base.cs", "Base", "class_declaration", "class Base {}");
        let interface = test_snapshot("src/ifoo.cs", "IFoo", "interface_declaration", "interface IFoo {}");
        let derived = test_snapshot("src/derived.cs", "Derived", "class_declaration", "class Derived : Base, IFoo {}");
        let snapshots = vec![base_class.clone(), interface.clone(), derived.clone()];

        let edges = resolve_extends_implements(&snapshots).unwrap();

        let to_base = edges.iter().find(|e| e.to == base_class.id()).expect("expected an edge to Base");
        assert_eq!(to_base.kind, EdgeKind::Extends);
        let to_interface = edges.iter().find(|e| e.to == interface.id()).expect("expected an edge to IFoo");
        assert_eq!(to_interface.kind, EdgeKind::Implements);
    }

    #[test]
    fn cpp_base_class_clause_resolves_extends() {
        let base = test_snapshot("src/base.hpp", "Base", "class_specifier", "class Base {};");
        let derived = test_snapshot("src/derived.hpp", "Derived", "class_specifier", "class Derived : public Base {};");
        let snapshots = vec![base.clone(), derived.clone()];

        let edges = resolve_extends_implements(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to, base.id());
        assert_eq!(edges[0].kind, EdgeKind::Extends);
    }

    #[test]
    fn cpp_base_naming_an_interface_kind_resolves_implements() {
        // C++ has no syntactic `interface` keyword, but if the resolved
        // base's own SymbolKind happens to be Interface (promoted from
        // some other construct this pipeline mapped to Interface), this
        // resolver still reports Implements for it — the SAME
        // kind-based disambiguation rule C# uses.
        let interface_like = test_snapshot("src/ifoo.hpp", "IFoo", "interface_declaration", "struct IFoo {};");
        let derived = test_snapshot("src/derived.hpp", "Derived", "class_specifier", "class Derived : public IFoo {};");
        let snapshots = vec![interface_like.clone(), derived.clone()];

        let edges = resolve_extends_implements(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to, interface_like.id());
        assert_eq!(edges[0].kind, EdgeKind::Implements);
    }

    #[test]
    fn a_base_not_found_in_the_pool_produces_no_edge() {
        let derived = test_snapshot("src/derived.cs", "Derived", "class_declaration", "class Derived : SomeUnknownBase {}");
        let snapshots = vec![derived];

        let edges = resolve_extends_implements(&snapshots).unwrap();

        assert!(edges.is_empty(), "a base class not found among the given pool must never be guessed at, not even as Extends");
    }

    #[test]
    fn an_ambiguous_base_name_across_files_produces_no_edge() {
        let candidate_a = test_snapshot("src/a.cs", "Base", "class_declaration", "class Base {}");
        let candidate_b = test_snapshot("src/b.cs", "Base", "class_declaration", "class Base {}");
        let derived = test_snapshot("src/derived.cs", "Derived", "class_declaration", "class Derived : Base {}");
        let snapshots = vec![candidate_a, candidate_b, derived];

        let edges = resolve_extends_implements(&snapshots).unwrap();

        assert!(edges.is_empty(), "an ambiguous same-named base across files must never be arbitrarily picked");
    }

    #[test]
    fn python_multiple_inheritance_produces_one_edge_per_base() {
        let base_a = test_snapshot("src/a.py", "A", "class_definition", "class A:\n    pass");
        let base_b = test_snapshot("src/b.py", "B", "class_definition", "class B:\n    pass");
        let derived = test_snapshot("src/c.py", "C", "class_definition", "class C(A, B):\n    pass");
        let snapshots = vec![base_a.clone(), base_b.clone(), derived.clone()];

        let edges = resolve_extends_implements(&snapshots).unwrap();

        assert_eq!(edges.len(), 2);
        let targets: Vec<_> = edges.iter().map(|e| e.to).collect();
        assert!(targets.contains(&base_a.id()));
        assert!(targets.contains(&base_b.id()));
    }

    #[test]
    fn b2_id_002_scope_audit_the_same_base_named_twice_does_not_produce_a_duplicate_edge() {
        // Scope audit for B2-ID-002 (found in resolve_calls, also found
        // in resolve_imports): checks whether resolve_extends_implements
        // has the same occurrence-vs-relation mismatch. A syntactically
        // odd but parseable `class C(A, A)` (the same base named twice)
        // must collapse to ONE Extends edge, not two identical ones.
        let base_a = test_snapshot("src/a.py", "A", "class_definition", "class A:\n    pass");
        let derived = test_snapshot("src/c.py", "C", "class_definition", "class C(A, A):\n    pass");
        let snapshots = vec![base_a.clone(), derived.clone()];

        let edges = resolve_extends_implements(&snapshots).unwrap();

        let to_a: Vec<_> = edges.iter().filter(|e| e.to == base_a.id()).collect();
        assert_eq!(to_a.len(), 1, "the same base named twice must collapse to one edge, not duplicate (from,to,Extends)");
    }

    #[test]
    fn a_duplicate_snapshot_id_is_a_typed_rejection() {
        let snapshot = test_snapshot("src/lib.rs", "Foo", "struct_item", "struct Foo;");
        let snapshots = vec![snapshot.clone(), snapshot];

        let result = resolve_extends_implements(&snapshots);

        assert_eq!(result.err(), Some(EdgeResolutionError::DuplicateSnapshotId));
    }

    // B2-ID-007 regression tests: a nested type declaration's own
    // base/heritage clause must be owned by that declaration alone,
    // never also by an outer ancestor container. Every snapshot below
    // is built from REAL extraction (`extract_symbols_reporting_rejections`
    // + `promote`), not a hand-written placeholder `signature_text` —
    // the same reason B2-ID-006's own regression tests use real
    // extraction: a placeholder body structurally cannot exercise this
    // defect.
    mod b2_id_007_nested_declaration_ownership {
        use super::*;
        use crate::extract::extract_symbols_reporting_rejections;
        use crate::parse::parser_for;

        fn real_snapshots(repo: codegraph_domain::RepositoryId, path: &str, source: &str) -> Vec<SymbolSnapshot> {
            let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
            let file = FileIdentity::parse(repo, commit, path).unwrap();
            let parser = parser_for(&file).expect("a supported extension must have a parser");
            let tree = parser.parse(source.as_bytes()).expect("the hand-written fixture source must parse");
            let (candidates, rejections) = extract_symbols_reporting_rejections(&tree);
            assert!(rejections.is_empty(), "fixture source must not trigger any extraction rejection: {rejections:?}");
            candidates.into_iter().map(|c| promote(&c, file.clone()).expect("a real extracted candidate must always promote")).collect()
        }

        fn find<'a>(snapshots: &'a [SymbolSnapshot], name: &str) -> &'a SymbolSnapshot {
            snapshots.iter().find(|s| s.name().as_str() == name).unwrap_or_else(|| panic!("no snapshot named {name:?} among {:?}", snapshots.iter().map(|s| s.name().as_str()).collect::<Vec<_>>()))
        }

        fn sources_of<'a>(edges: &'a [Edge], target: &SymbolSnapshot, snapshots: &'a [SymbolSnapshot]) -> Vec<&'a str> {
            let mut names: Vec<&str> = edges.iter().filter(|e| e.to == target.id()).filter_map(|e| snapshots.iter().find(|s| s.id() == e.from)).map(|s| s.name().as_str()).collect();
            names.sort_unstable();
            names
        }

        fn repo() -> codegraph_domain::RepositoryId {
            let mut registry = RepositoryRegistry::new();
            registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap())
        }

        #[test]
        fn python_nested_class_extends_is_owned_by_inner_not_outer() {
            let snapshots = real_snapshots(repo(), "src/nested.py", "class Base:\n    pass\n\n\nclass Outer:\n    class Inner(Base):\n        pass\n");
            let base = find(&snapshots, "Base");
            let edges = resolve_extends_implements(&snapshots).unwrap();

            assert_eq!(sources_of(&edges, base, &snapshots), vec!["Inner"], "only Inner extends Base; Outer must not also appear");
        }

        #[test]
        fn python_outer_and_inner_both_extend_different_bases_independently() {
            let snapshots = real_snapshots(
                repo(),
                "src/both.py",
                "class BaseA:\n    pass\n\n\nclass BaseB:\n    pass\n\n\nclass Outer(BaseA):\n    class Inner(BaseB):\n        pass\n",
            );
            let base_a = find(&snapshots, "BaseA");
            let base_b = find(&snapshots, "BaseB");
            let edges = resolve_extends_implements(&snapshots).unwrap();

            assert_eq!(sources_of(&edges, base_a, &snapshots), vec!["Outer"], "Outer's own base must still resolve correctly");
            assert_eq!(sources_of(&edges, base_b, &snapshots), vec!["Inner"], "Inner's own, different base must resolve to Inner alone");
        }

        #[test]
        fn python_sibling_nested_classes_each_own_their_own_base_independently() {
            let snapshots = real_snapshots(
                repo(),
                "src/siblings.py",
                "class Base:\n    pass\n\n\nclass Outer:\n    class InnerA(Base):\n        pass\n\n    class InnerB(Base):\n        pass\n",
            );
            let base = find(&snapshots, "Base");
            let edges = resolve_extends_implements(&snapshots).unwrap();

            assert_eq!(sources_of(&edges, base, &snapshots), vec!["InnerA", "InnerB"], "both siblings own their own edge; Outer must not also appear");
        }

        #[test]
        fn csharp_nested_class_extends_is_owned_by_inner_not_outer() {
            let snapshots = real_snapshots(repo(), "src/Nested.cs", "class Base {}\n\nclass Outer {\n    class Inner : Base {}\n}\n");
            let base = find(&snapshots, "Base");
            let edges = resolve_extends_implements(&snapshots).unwrap();

            assert_eq!(sources_of(&edges, base, &snapshots), vec!["Inner"], "only Inner extends Base in C#; Outer must not also appear");
        }

        #[test]
        fn csharp_nested_interface_implementation_is_owned_by_inner_not_outer() {
            let snapshots = real_snapshots(repo(), "src/NestedIface.cs", "interface IBase {}\n\nclass Outer {\n    class Inner : IBase {}\n}\n");
            let ibase = find(&snapshots, "IBase");
            let edges = resolve_extends_implements(&snapshots).unwrap();

            assert_eq!(sources_of(&edges, ibase, &snapshots), vec!["Inner"]);
            assert_eq!(edges.iter().find(|e| e.to == ibase.id()).unwrap().kind, EdgeKind::Implements, "IBase is Interface-kind, so this must resolve as Implements, not Extends");
        }

        #[test]
        fn cpp_nested_class_extends_is_owned_by_inner_not_outer() {
            let snapshots = real_snapshots(repo(), "src/nested.cpp", "class Base {};\n\nclass Outer {\n    class Inner : public Base {};\n};\n");
            let base = find(&snapshots, "Base");
            let edges = resolve_extends_implements(&snapshots).unwrap();

            assert_eq!(sources_of(&edges, base, &snapshots), vec!["Inner"], "only Inner extends Base in C++; Outer must not also appear");
        }

        #[test]
        fn javascript_nested_class_extends_is_owned_by_inner_not_outer() {
            // JS doesn't have "nested classes" in a block-scoped sense
            // the way Python/C#/C++ do, but a class CAN be declared
            // inside another class's static initializer block or
            // method body in real JS — the simplest real, parseable
            // shape exercising the same AST nesting is a class
            // expression assigned inside a static block. To keep this
            // reproducible against the actual grammar this project
            // targets, this test instead confirms the boundary holds
            // for the one real nested-class shape tree-sitter-javascript
            // parses without error: a class declared inside another
            // class's own class body via a static block.
            let snapshots = real_snapshots(
                repo(),
                "src/nested.js",
                "class Base {}\n\nclass Outer {\n    static {\n        class Inner extends Base {}\n    }\n}\n",
            );
            let base = find(&snapshots, "Base");
            let edges = resolve_extends_implements(&snapshots).unwrap();

            assert_eq!(sources_of(&edges, base, &snapshots), vec!["Inner"], "only Inner extends Base in JS; Outer must not also appear");
        }

        #[test]
        fn typescript_nested_class_extends_is_owned_by_inner_not_outer() {
            // TypeScript's own `interface` declarations are not
            // extracted as symbols by this pipeline's current
            // extractor (no TS `interface_declaration` mapping exists
            // in `snapshot::kind_mapping::map_kind_hint`'s table), so
            // this test uses a class-extends-class shape instead of
            // implements-an-interface, exercising the exact same
            // `class_heritage`/boundary code path
            // `collect_ts_heritage_tagged` shares with the JS
            // collector.
            let snapshots = real_snapshots(
                repo(),
                "src/nested.ts",
                "class Base {}\n\nclass Outer {\n    static {\n        class Inner extends Base {}\n    }\n}\n",
            );
            let base = find(&snapshots, "Base");
            let edges = resolve_extends_implements(&snapshots).unwrap();

            assert_eq!(sources_of(&edges, base, &snapshots), vec!["Inner"], "only Inner extends Base in TS; Outer must not also appear");
        }

        #[test]
        fn result_is_deterministic_regardless_of_snapshot_order() {
            let snapshots = real_snapshots(repo(), "src/order.py", "class Base:\n    pass\n\n\nclass Outer:\n    class Inner(Base):\n        pass\n");
            let base = find(&snapshots, "Base");

            let forward_edges = resolve_extends_implements(&snapshots).unwrap();
            let mut reversed = snapshots.clone();
            reversed.reverse();
            let reversed_edges = resolve_extends_implements(&reversed).unwrap();

            assert_eq!(sources_of(&forward_edges, base, &snapshots), sources_of(&reversed_edges, base, &reversed), "ownership attribution must not depend on snapshot pool ordering");
        }
    }
}
