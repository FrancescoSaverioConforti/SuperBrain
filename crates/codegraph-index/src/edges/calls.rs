//! B2 Task 7: `Calls` edges. Walks each caller `SymbolSnapshot`'s own
//! `signature_text` (which, per `extract`'s per-language extractors,
//! captures the WHOLE construct — including its body — not just a
//! parameter list) by re-parsing it as a standalone fragment with the
//! same language's `LanguageParser`, and looks for call-expression nodes
//! whose callee name resolves to another snapshot in the given pool.
//!
//! # Resolved vs. heuristic vs. unresolved
//!
//! - A call whose callee name matches EXACTLY ONE other snapshot's name
//!   is `StaticAnalysis`-resolved — not because the whole program's
//!   call graph is statically analyzed (it is not: this pipeline does
//!   not do type inference, overload resolution, or virtual-dispatch
//!   analysis), but because, within the closed set of candidate
//!   snapshots this resolver is given, the name uniquely identifies one
//!   target with no ambiguity left for this resolver to guess at. A
//!   dotted/member call (Python's `self.encoder(x)`, matched by its
//!   final segment `encoder`) is resolved the SAME way once the name
//!   is unique — the pipeline does not need to know `self` is an
//!   instance to usefully record "this method calls something named
//!   `encoder`."
//! - A call whose callee name matches MORE THAN ONE candidate (e.g. two
//!   different types both define a method named `run`) is
//!   `NameHeuristic`, `Confidence::Speculative` — explicitly never
//!   picked arbitrarily. Per the task's own scope, this resolver does
//!   NOT attempt overload/type-based disambiguation; it records the
//!   ambiguity honestly rather than overclaiming a single target.
//! - A call whose callee name matches NO candidate in the given pool
//!   produces no edge at all — the callee is outside what this resolver
//!   was given to work with (e.g. a standard-library function, or a
//!   symbol in a file not included in this resolution pass), not a
//!   relationship this pipeline can honestly assert anything about.
//!
//! # Known gap: calls inside a macro invocation (Rust)
//!
//! A call wrapped inside a Rust macro invocation (e.g.
//! `assert_eq!(add(1, 2), 3)`) is NOT found by this resolver: confirmed
//! empirically against a real parse, `add(1, 2)` inside
//! `assert_eq!(...)`'s argument list is an opaque `token_tree` to
//! tree-sitter, not a real `call_expression` node — tree-sitter does not
//! expand macros, so it has no way to know `assert_eq!`'s arguments are
//! themselves Rust expressions rather than arbitrary tokens. This
//! resolver therefore cannot see calls made from inside macro-heavy test
//! code (a common shape, since `assert_eq!`/`assert!` are exactly where
//! a test's real assertions live) without a macro-aware token-tree
//! heuristic this task's scope does not include. Recorded here as a
//! real, named limitation — not silently masked by a test fixture that
//! happens to avoid it — and a candidate for the real-repository
//! validation pass this plan's section 8b requires before B2's closure.

use codegraph_domain::{Confidence, EdgeKind};

use super::resolution::{heuristic_evidence, resolved_evidence, Edge, EdgeResolutionError, TargetLookup};
use crate::parse::{CSharpParser, CppParser, JavaScriptParser, Language, LanguageParser, PythonParser, RustParser, TsxParser, TypeScriptParser};
use crate::snapshot::SymbolSnapshot;

/// Resolves `Calls` edges from every snapshot in `snapshots` to every
/// other snapshot in the SAME pool whose name matches a call found
/// inside the caller's own signature text. `snapshots` plays both the
/// "source" and "candidate target" role simultaneously — a caller and
/// its potential callees are expected to be given together, unlike
/// Task 6's lineage resolver's `prev`/`next` split (lineage is
/// inherently directional across commits; calls are not).
pub fn resolve_calls(snapshots: &[SymbolSnapshot]) -> Result<Vec<Edge>, EdgeResolutionError> {
    let lookup = TargetLookup::new(snapshots)?;
    let mut edges = Vec::new();

    for caller in lookup.snapshots() {
        let Some(parser) = parser_for(caller.file()) else {
            // This resolver only recognizes languages whose
            // FileIdentity-independent kind it can determine from the
            // caller's own promoted SymbolKind/signature shape — in
            // practice, every language Task 4 extracts already has a
            // LanguageParser; see `parser_for`'s own doc comment for
            // why HTML/CSS snapshots never reach a parser here.
            continue;
        };

        let Ok(tree) = parser.parse(caller.signature().as_str().as_bytes()) else {
            continue;
        };

        for callee_name in call_names_in_tree(&tree) {
            let candidates = lookup.find_by_name(&callee_name, caller.id());
            if candidates.len() == 1 {
                let evidence = resolved_evidence(&format!("call to {callee_name:?} resolves to exactly one candidate in the given snapshot pool"));
                edges.push(Edge {
                    from: caller.id(),
                    to: candidates[0].id(),
                    kind: EdgeKind::Calls,
                    evidence,
                });
                continue;
            }
            // Zero candidates: the callee is outside what this resolver
            // was given to work with (see this module's doc comment) —
            // the loop below naturally produces no edges for an empty
            // `candidates`, with no separate early-return needed (an
            // explicit `0 => continue` arm here would be a redundant,
            // behaviorally-equivalent branch mutation testing already
            // flagged as indistinguishable from this single path, the
            // same class of finding Task 6 surfaced for a different
            // equivalent branch — removed rather than kept as an
            // unprovable distinction). More than one candidate:
            // ambiguous. Per the "no edge beats a false edge" principle,
            // and the task's explicit instruction not to pick
            // arbitrarily, this resolver still records ONE Hypothesized
            // edge per ambiguous candidate (not zero) — unlike Task 6's
            // lineage resolver (which emits NO edge on tie, because a
            // lineage edge claims "this specific prior symbol continues
            // as this one," a 1:1 claim an ambiguous tie cannot honestly
            // make), a `Calls` edge is not 1:1 by nature: real code can
            // and does call one of several same-named overloads, and
            // recording "this caller plausibly calls EACH of these
            // candidates, with low confidence, which one actually is
            // not certain" is a true statement about the available
            // evidence, whereas recording nothing at all would discard
            // the real, if imprecise, signal that a call to this name
            // happened somewhere reachable.
            for candidate in &candidates {
                let (evidence, _confidence) = heuristic_evidence(
                    codegraph_domain::ResolutionMethod::NameHeuristic,
                    Confidence::Speculative,
                    &format!("call to {callee_name:?} is ambiguous among {} same-named candidates in the given pool", candidates.len()),
                );
                edges.push(Edge {
                    from: caller.id(),
                    to: candidate.id(),
                    kind: EdgeKind::Calls,
                    evidence,
                });
            }
        }
    }

    Ok(edges)
}

/// Resolves which `LanguageParser` to re-parse `file`'s owning symbol's
/// signature text with. Returns `None` for a `FileIdentity` this module
/// cannot map to a supported language via its own file-extension-free
/// signal — in practice, every snapshot this resolver receives SHOULD
/// have an unambiguous language from its own extraction pipeline, but
/// `SymbolSnapshot` does not currently carry `codegraph_index::parse::Language`
/// directly (only `codegraph_domain::SymbolKind`, which does not name a
/// language). This resolver infers the language from the file's
/// extension as a pragmatic stand-in — a genuine, if narrow, gap: a
/// future task could close this more robustly by having `SymbolSnapshot`
/// carry its source `Language` directly (see Task 7's own closing report
/// for this noted as a candidate for real-repository-validation-driven
/// follow-up, not implemented speculatively here).
pub(super) fn parser_for(file: &codegraph_domain::FileIdentity) -> Option<&'static dyn LanguageParser> {
    let path = file.relative_path();
    let extension = path.rsplit('.').next()?;
    match extension {
        "rs" => Some(&RustParser),
        "py" => Some(&PythonParser),
        "cs" => Some(&CSharpParser),
        "cpp" | "cc" | "cxx" | "hpp" | "h" => Some(&CppParser),
        "js" | "mjs" | "cjs" => Some(&JavaScriptParser),
        "ts" => Some(&TypeScriptParser),
        "tsx" | "jsx" => Some(&TsxParser),
        _ => None,
    }
}

/// Every callee name found anywhere inside `tree`'s root, by walking for
/// the per-language call-expression node kind and extracting its own
/// callee child's name — `identifier` for a free/simple call, or the
/// LAST segment of an `attribute`/member-access chain for a method-call
/// shape (`self.encoder(x)` -> `"encoder"`, confirmed against this
/// workspace's real `tree_sitter_python` output before writing this
/// logic, not assumed).
pub(super) fn call_names_in_tree(tree: &crate::parse::ParseTree) -> Vec<String> {
    let mut names = Vec::new();
    walk_for_calls(tree.tree().root_node(), tree.source(), tree.language(), &mut names);
    names
}

fn walk_for_calls(node: tree_sitter::Node, source: &[u8], language: Language, out: &mut Vec<String>) {
    let is_call_node = match language {
        Language::Rust => node.kind() == "call_expression",
        Language::Python => node.kind() == "call",
        Language::CSharp => node.kind() == "invocation_expression",
        Language::Cpp => node.kind() == "call_expression",
        Language::JavaScript | Language::TypeScript | Language::Tsx => node.kind() == "call_expression",
        Language::Html | Language::Css => false,
    };
    if is_call_node {
        if let Some(name) = callee_name(node, source) {
            out.push(name);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_for_calls(child, source, language, out);
    }
}

/// `call_node`'s own first named child is the callee expression: an
/// `identifier` for a simple call, or an `attribute`/`field_expression`-
/// shaped member access for a method call, whose own LAST `identifier`/
/// `property_identifier`/`field_identifier` child is the method name
/// actually being called (`self.encoder` -> `"encoder"`, not `"self"`).
fn callee_name(call_node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    let callee = call_node.child(0)?;
    match callee.kind() {
        "identifier" => Some(text_of(callee, source)),
        "attribute" | "field_expression" | "member_expression" => {
            let mut cursor = callee.walk();
            let last_name_child = callee
                .children(&mut cursor)
                .filter(|c| matches!(c.kind(), "identifier" | "property_identifier" | "field_identifier"))
                .last()?;
            Some(text_of(last_name_child, source))
        }
        _ => None,
    }
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
    fn a_call_to_a_unique_candidate_is_resolved() {
        let caller = test_snapshot("src/lib.rs", "foo", "function_item", "fn foo() { helper(); }");
        let callee = test_snapshot("src/lib.rs", "helper", "function_item", "fn helper() {}");
        let snapshots = vec![caller.clone(), callee.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].from, caller.id());
        assert_eq!(edges[0].to, callee.id());
        assert_eq!(edges[0].kind, EdgeKind::Calls);
        assert_eq!(edges[0].evidence.method(), codegraph_domain::ResolutionMethod::StaticAnalysis);
    }

    #[test]
    fn an_ambiguous_call_produces_heuristic_edges_for_every_candidate_not_an_arbitrary_pick() {
        let caller = test_snapshot("src/lib.rs", "foo", "function_item", "fn foo() { run(); }");
        let candidate_a = test_snapshot("src/a.rs", "run", "function_item", "fn run() {}");
        let candidate_b = test_snapshot("src/b.rs", "run", "function_item", "fn run() {}");
        let snapshots = vec![caller.clone(), candidate_a.clone(), candidate_b.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert_eq!(edges.len(), 2, "expected one heuristic edge per ambiguous candidate, got {edges:?}");
        for edge in &edges {
            assert_eq!(edge.from, caller.id());
            assert_ne!(edge.evidence.method(), codegraph_domain::ResolutionMethod::StaticAnalysis, "an ambiguous call must never be StaticAnalysis");
        }
        let target_ids: Vec<_> = edges.iter().map(|e| e.to).collect();
        assert!(target_ids.contains(&candidate_a.id()));
        assert!(target_ids.contains(&candidate_b.id()));
    }

    #[test]
    fn a_call_to_an_unknown_target_produces_no_edge() {
        let caller = test_snapshot("src/lib.rs", "foo", "function_item", "fn foo() { not_in_the_pool(); }");
        let snapshots = vec![caller];

        let edges = resolve_calls(&snapshots).unwrap();

        assert!(edges.is_empty(), "a call to a name absent from the given pool must produce no edge");
    }

    #[test]
    fn a_method_call_resolves_by_its_final_segment_name() {
        // The forward()-calls-encoder() deep-learning-relevant shape:
        // self.encoder(x) must resolve to a snapshot named "encoder",
        // not "self".
        let caller = test_snapshot(
            "src/model.py",
            "forward",
            "function_definition",
            "def forward(self, x):\n    return self.encoder(x)",
        );
        let encoder = test_snapshot("src/model.py", "encoder", "function_definition", "def encoder(x):\n    return x");
        let snapshots = vec![caller.clone(), encoder.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to, encoder.id());
        assert_eq!(edges[0].evidence.method(), codegraph_domain::ResolutionMethod::StaticAnalysis);
    }

    #[test]
    fn a_cross_file_call_is_still_resolved() {
        let caller = test_snapshot("src/a.rs", "foo", "function_item", "fn foo() { helper(); }");
        let callee = test_snapshot("src/b.rs", "helper", "function_item", "fn helper() {}");
        let snapshots = vec![caller.clone(), callee.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to, callee.id());
    }

    #[test]
    fn a_recursive_call_does_not_self_link() {
        let recursive = test_snapshot("src/lib.rs", "factorial", "function_item", "fn factorial(n: u32) -> u32 { factorial(n - 1) }");
        let snapshots = vec![recursive.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert!(edges.is_empty(), "a function calling itself by name must not produce a self-referential edge from this resolver");
    }

    #[test]
    fn a_duplicate_snapshot_id_is_a_typed_rejection() {
        let snapshot = test_snapshot("src/lib.rs", "foo", "function_item", "fn foo() {}");
        let snapshots = vec![snapshot.clone(), snapshot];

        let result = resolve_calls(&snapshots);

        assert_eq!(result.err(), Some(EdgeResolutionError::DuplicateSnapshotId));
    }

    #[test]
    fn a_cpp_call_is_resolved() {
        let caller = test_snapshot("src/a.cpp", "foo", "function_definition", "void foo() { helper(); }");
        let callee = test_snapshot("src/a.cpp", "helper", "function_definition", "void helper() {}");
        let snapshots = vec![caller.clone(), callee.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to, callee.id());
    }

    #[test]
    fn a_header_extension_is_recognized_as_cpp() {
        let caller = test_snapshot("src/a.hpp", "foo", "function_definition", "void foo() { helper(); }");
        let callee = test_snapshot("src/a.hpp", "helper", "function_definition", "void helper() {}");
        let snapshots = vec![caller.clone(), callee.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to, callee.id());
    }

    #[test]
    fn a_csharp_call_is_resolved() {
        let caller = test_snapshot("src/a.cs", "M", "method_declaration", "void M() { Helper(); }");
        let callee = test_snapshot("src/a.cs", "Helper", "method_declaration", "void Helper() {}");
        let snapshots = vec![caller.clone(), callee.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to, callee.id());
    }

    #[test]
    fn a_tsx_call_is_resolved() {
        let caller = test_snapshot("src/a.tsx", "Comp", "function_declaration", "function Comp() { return helper(); }");
        let callee = test_snapshot("src/a.tsx", "helper", "function_declaration", "function helper() { return 1; }");
        let snapshots = vec![caller.clone(), callee.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to, callee.id());
    }

    #[test]
    fn a_jsx_extension_is_recognized_as_tsx() {
        let caller = test_snapshot("src/a.jsx", "Comp", "function_declaration", "function Comp() { return helper(); }");
        let callee = test_snapshot("src/a.jsx", "helper", "function_declaration", "function helper() { return 1; }");
        let snapshots = vec![caller.clone(), callee.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to, callee.id());
    }
}
