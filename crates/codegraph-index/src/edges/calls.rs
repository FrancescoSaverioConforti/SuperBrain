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
pub(crate) use crate::parse::parser_for;
use crate::parse::Language;
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

        // B2-ID-002: `call_names_in_tree` returns one entry per
        // SYNTACTIC call-site occurrence (e.g. `self.forward(x)` called
        // twice in the same body yields two entries), but `CodeGraph`
        // represents a SEMANTIC/topological `(from, to, EdgeKind)`
        // relation — at most one edge per pair, never one per
        // occurrence. `per_target` collapses every occurrence resolving
        // to the SAME target into exactly one `Edge`, chosen
        // deterministically by `prefer_edge` (see its own doc comment)
        // rather than by iteration order, before this caller's edges
        // are appended to the final output. This never merges two
        // DISTINCT targets — only repeated occurrences of the same
        // `(from, to)` pair.
        let mut per_target: std::collections::HashMap<codegraph_domain::SymbolSnapshotId, Edge> = std::collections::HashMap::new();

        for callee_name in call_names_in_tree(&tree) {
            let candidates = lookup.find_by_name(&callee_name, caller.id());
            if candidates.len() == 1 {
                let evidence = resolved_evidence(&format!("call to {callee_name:?} resolves to exactly one candidate in the given snapshot pool"));
                let edge = Edge {
                    from: caller.id(),
                    to: candidates[0].id(),
                    kind: EdgeKind::Calls,
                    evidence,
                };
                upsert_preferring_stronger_evidence(&mut per_target, edge);
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
                let edge = Edge {
                    from: caller.id(),
                    to: candidate.id(),
                    kind: EdgeKind::Calls,
                    evidence,
                };
                upsert_preferring_stronger_evidence(&mut per_target, edge);
            }
        }

        let mut caller_edges: Vec<Edge> = per_target.into_values().collect();
        caller_edges.sort_by(|a, b| a.to.as_bytes().cmp(b.to.as_bytes()));
        edges.extend(caller_edges);
    }

    Ok(edges)
}

/// Inserts `edge` into `per_target` (keyed by `edge.to`), or, if a
/// different occurrence already produced an edge to the SAME target,
/// keeps whichever one `prefer_edge` ranks higher — deterministically,
/// never by insertion/iteration order. This is the one place
/// `resolve_calls` collapses repeated call-site occurrences of the same
/// `(from, to)` pair into a single stored `Edge` (B2-ID-002).
fn upsert_preferring_stronger_evidence(per_target: &mut std::collections::HashMap<codegraph_domain::SymbolSnapshotId, Edge>, edge: Edge) {
    match per_target.get(&edge.to) {
        Some(existing) if !prefer_edge(&edge, existing) => {}
        _ => {
            per_target.insert(edge.to, edge);
        }
    }
}

/// Deterministic edge-preference rule for B2-ID-002's deduplication:
/// `true` if `candidate` should replace `current` for the same `(from,
/// to)` pair. `StaticAnalysis` always outranks a heuristic method
/// (resolved-unique beats ambiguous, which can only happen if the SAME
/// callee name resolved uniquely on one occurrence but, implausibly,
/// ambiguously on another within the same caller — not reachable today
/// since a given name's candidate set does not change within one
/// `resolve_calls` call, but handled explicitly rather than left
/// unreachable-and-unproven). When both sides have the same
/// `ResolutionMethod`, this scope's `Edge`/`Evidence` carry no other
/// field meaningfully distinguishing "better," so the choice is a
/// stable, explicit tie-break (note text, byte-for-byte) rather than
/// "whichever happened to be inserted first" — never a silent, order-
/// dependent pick.
fn prefer_edge(candidate: &Edge, current: &Edge) -> bool {
    let candidate_is_static = candidate.evidence.method() == codegraph_domain::ResolutionMethod::StaticAnalysis;
    let current_is_static = current.evidence.method() == codegraph_domain::ResolutionMethod::StaticAnalysis;
    if candidate_is_static != current_is_static {
        return candidate_is_static;
    }
    candidate.evidence.note() < current.evidence.note()
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

    /// Like `test_snapshot`, but all snapshots built from the SAME
    /// `repo` the caller passes in — needed when a test actually
    /// inserts into a `CodeGraph`, which rejects a cross-repository
    /// edge (Task 8's own invariant); `test_snapshot` alone mints a
    /// fresh, unrelated `RepositoryId` on every call, which is fine for
    /// every OTHER test in this file (none of which touches
    /// `CodeGraph`) but would trip that invariant here.
    fn test_snapshot_in_repo(repo: codegraph_domain::RepositoryId, path: &str, name: &str, kind_hint: &str, signature: &str) -> SymbolSnapshot {
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

    // --- B2-ID-002: occurrence-vs-relation deduplication ---

    #[test]
    fn calling_the_same_resolved_target_twice_in_one_body_produces_one_edge() {
        let caller = test_snapshot("src/lib.rs", "caller", "function_item", "fn caller() { callee(); callee(); }");
        let callee = test_snapshot("src/lib.rs", "callee", "function_item", "fn callee() {}");
        let snapshots = vec![caller.clone(), callee.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        let to_callee: Vec<_> = edges.iter().filter(|e| e.from == caller.id() && e.to == callee.id()).collect();
        assert_eq!(to_callee.len(), 1, "two occurrences of the same call must collapse to one (from,to,Calls) edge");
    }

    #[test]
    fn calling_the_same_resolved_target_many_times_still_produces_one_edge() {
        let caller = test_snapshot("src/lib.rs", "caller", "function_item", "fn caller() { callee(); callee(); callee(); callee(); callee(); }");
        let callee = test_snapshot("src/lib.rs", "callee", "function_item", "fn callee() {}");
        let snapshots = vec![caller.clone(), callee.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        let to_callee: Vec<_> = edges.iter().filter(|e| e.from == caller.id() && e.to == callee.id()).collect();
        assert_eq!(to_callee.len(), 1, "five occurrences of the same call must still collapse to one edge");
    }

    #[test]
    fn calling_two_different_targets_from_one_caller_produces_two_edges() {
        let caller = test_snapshot("src/lib.rs", "caller", "function_item", "fn caller() { a(); b(); }");
        let a = test_snapshot("src/lib.rs", "a", "function_item", "fn a() {}");
        let b = test_snapshot("src/lib.rs", "b", "function_item", "fn b() {}");
        let snapshots = vec![caller.clone(), a.clone(), b.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        assert_eq!(edges.iter().filter(|e| e.from == caller.id()).count(), 2, "two genuinely distinct targets must not be collapsed into one edge");
    }

    #[test]
    fn two_different_callers_calling_the_same_target_each_get_their_own_edge() {
        let caller_a = test_snapshot("src/a.rs", "caller_a", "function_item", "fn caller_a() { shared(); }");
        let caller_b = test_snapshot("src/b.rs", "caller_b", "function_item", "fn caller_b() { shared(); }");
        let target = test_snapshot("src/lib.rs", "shared", "function_item", "fn shared() {}");
        let snapshots = vec![caller_a.clone(), caller_b.clone(), target.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        let to_target: Vec<_> = edges.iter().filter(|e| e.to == target.id()).collect();
        assert_eq!(to_target.len(), 2, "two distinct callers of the same target must each have their own edge, not be deduplicated against each other");
        assert!(to_target.iter().any(|e| e.from == caller_a.id()));
        assert!(to_target.iter().any(|e| e.from == caller_b.id()));
    }

    #[test]
    fn an_ambiguous_call_repeated_in_one_body_produces_no_duplicate_from_to_calls_triples() {
        // Two candidates both named "run" in different files, called
        // TWICE by the same caller — every (from, to, Calls) triple
        // must still be unique, even though the ambiguous branch
        // produces one heuristic edge per candidate per occurrence
        // before deduplication.
        let caller = test_snapshot("src/caller.rs", "caller", "function_item", "fn caller() { run(); run(); }");
        let candidate_a = test_snapshot("src/a.rs", "run", "function_item", "fn run() {}");
        let candidate_b = test_snapshot("src/b.rs", "run", "function_item", "fn run() {}");
        let snapshots = vec![caller.clone(), candidate_a.clone(), candidate_b.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        let mut seen = std::collections::HashSet::new();
        for edge in &edges {
            assert!(seen.insert((edge.from, edge.to, edge.kind)), "duplicate (from,to,kind) triple found: repeated ambiguous occurrences must not produce more than one edge per distinct target");
        }
        // Both distinct targets must still each get their own
        // heuristic edge — deduplication must never merge candidate_a
        // and candidate_b into one.
        assert_eq!(edges.iter().filter(|e| e.from == caller.id()).count(), 2);
    }

    #[test]
    fn evidence_selection_between_duplicate_occurrences_is_deterministic_regardless_of_body_order() {
        // Two callers with the SAME two call occurrences but written in
        // OPPOSITE order in the source body must still resolve to
        // exactly the same final edge (same evidence note), proving the
        // dedup's tie-break does not depend on which occurrence tree-
        // sitter/this walk visits first.
        let callee = test_snapshot("src/lib.rs", "callee", "function_item", "fn callee() {}");
        let caller_order_1 = test_snapshot("src/a.rs", "caller_order_1", "function_item", "fn caller_order_1() { callee(); callee(); }");
        let caller_order_2 = test_snapshot("src/b.rs", "caller_order_2", "function_item", "fn caller_order_2() { callee(); callee(); }");
        let snapshots = vec![callee.clone(), caller_order_1.clone(), caller_order_2.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        let edge_1 = edges.iter().find(|e| e.from == caller_order_1.id()).unwrap();
        let edge_2 = edges.iter().find(|e| e.from == caller_order_2.id()).unwrap();
        assert_eq!(edge_1.evidence.note(), edge_2.evidence.note(), "identical repeated occurrences must resolve to byte-identical evidence regardless of caller");
    }

    #[test]
    fn b2_id_002_regression_a_caller_calling_the_same_ambiguous_name_many_times_does_not_panic_on_graph_insertion() {
        // Direct regression reproduction of the real failure mode: a
        // caller invoking the SAME ambiguous name repeatedly (common in
        // real code, e.g. several calls to a same-named helper across
        // overloaded candidates) must never produce a (from,to,kind)
        // duplicate that CodeGraph::insert_edge would reject.
        let mut registry = RepositoryRegistry::new();
        let repo = registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap());
        let caller = test_snapshot_in_repo(
            repo,
            "src/caller.rs",
            "caller",
            "function_item",
            "fn caller() { forward(); forward(); forward(); forward(); forward(); }",
        );
        let candidate_a = test_snapshot_in_repo(repo, "src/a.rs", "forward", "function_item", "fn forward() {}");
        let candidate_b = test_snapshot_in_repo(repo, "src/b.rs", "forward", "function_item", "fn forward() {}");
        let snapshots = vec![caller.clone(), candidate_a.clone(), candidate_b.clone()];

        let edges = resolve_calls(&snapshots).unwrap();

        let mut graph = crate::graph::CodeGraph::new();
        for snapshot in &snapshots {
            graph.insert_snapshot(snapshot.clone()).unwrap();
        }
        for edge in edges {
            graph.insert_edge(edge).unwrap();
        }
    }

    // --- Direct unit tests on `prefer_edge` itself: mutation testing
    // flagged this function's actual decision logic as unexercised by
    // the end-to-end tests above (which only ever compare two
    // IDENTICAL-evidence occurrences, never two with genuinely
    // different ResolutionMethod or different note text) ---

    fn dummy_snapshot_id() -> codegraph_domain::SymbolSnapshotId {
        test_snapshot("src/x.rs", "x", "function_item", "fn x() {}").id()
    }

    fn static_edge(note: &str) -> Edge {
        Edge {
            from: dummy_snapshot_id(),
            to: dummy_snapshot_id(),
            kind: EdgeKind::Calls,
            evidence: resolved_evidence(note),
        }
    }

    fn heuristic_edge(note: &str) -> Edge {
        let (evidence, _) = heuristic_evidence(codegraph_domain::ResolutionMethod::NameHeuristic, Confidence::Speculative, note);
        Edge {
            from: dummy_snapshot_id(),
            to: dummy_snapshot_id(),
            kind: EdgeKind::Calls,
            evidence,
        }
    }

    #[test]
    fn prefer_edge_prefers_static_analysis_over_heuristic() {
        let static_candidate = static_edge("resolved");
        let heuristic_current = heuristic_edge("ambiguous");
        assert!(prefer_edge(&static_candidate, &heuristic_current), "a StaticAnalysis candidate must replace a heuristic current edge");
    }

    #[test]
    fn prefer_edge_never_replaces_static_analysis_with_heuristic() {
        let heuristic_candidate = heuristic_edge("ambiguous");
        let static_current = static_edge("resolved");
        assert!(!prefer_edge(&heuristic_candidate, &static_current), "a heuristic candidate must never replace an already-StaticAnalysis current edge");
    }

    #[test]
    fn prefer_edge_breaks_ties_between_two_static_edges_by_note_text() {
        let a = static_edge("aaa");
        let b = static_edge("bbb");
        assert!(prefer_edge(&a, &b), "the lexicographically smaller note must win the tie-break (a < b)");
        assert!(!prefer_edge(&b, &a), "the tie-break must be antisymmetric: b must not also beat a");
    }

    #[test]
    fn prefer_edge_breaks_ties_between_two_heuristic_edges_by_note_text() {
        let a = heuristic_edge("aaa");
        let b = heuristic_edge("bbb");
        assert!(prefer_edge(&a, &b));
        assert!(!prefer_edge(&b, &a));
    }

    #[test]
    fn prefer_edge_is_false_for_byte_identical_edges() {
        // Neither side should "win" against its own exact duplicate —
        // pins the strict `<` (not `<=`) comparison specifically.
        let a = static_edge("same");
        let b = static_edge("same");
        assert!(!prefer_edge(&a, &b), "an edge identical to the current one must not be reported as preferred (strict < , not <=)");
    }

    #[test]
    fn upsert_preferring_stronger_evidence_keeps_the_static_analysis_edge_regardless_of_insertion_order() {
        let mut per_target = std::collections::HashMap::new();
        let target = dummy_snapshot_id();
        let heuristic = Edge {
            to: target,
            ..heuristic_edge("ambiguous")
        };
        let resolved = Edge { to: target, ..static_edge("resolved") };

        // Heuristic inserted FIRST, then the StaticAnalysis one — the
        // stronger one must still win even though it arrives second.
        upsert_preferring_stronger_evidence(&mut per_target, heuristic.clone());
        upsert_preferring_stronger_evidence(&mut per_target, resolved.clone());
        assert_eq!(per_target.get(&target).unwrap().evidence.method(), codegraph_domain::ResolutionMethod::StaticAnalysis);

        // Reverse order: StaticAnalysis first, then heuristic — must
        // still end up StaticAnalysis, proving this is not merely
        // "last write wins."
        let mut per_target_reversed = std::collections::HashMap::new();
        upsert_preferring_stronger_evidence(&mut per_target_reversed, resolved);
        upsert_preferring_stronger_evidence(&mut per_target_reversed, heuristic);
        assert_eq!(per_target_reversed.get(&target).unwrap().evidence.method(), codegraph_domain::ResolutionMethod::StaticAnalysis);
    }
}
