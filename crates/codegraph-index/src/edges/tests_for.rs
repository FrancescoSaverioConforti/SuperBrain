//! B2 Task 7: `TestsFor` edges. Deliberately the MOST conservative edge
//! family — per the task's explicit instruction, a function is never
//! linked as `TestsFor` some target merely because its name looks like
//! `test_x`. Every `TestsFor` edge in this resolver requires BOTH:
//!
//! 1. a recognizable test-function naming convention
//!    (`test_<name>`/`Test<Name>`-shaped function/method name — purely
//!    conventional, never treated as certain, for every language this
//!    resolver covers); AND
//! 2. an ACTUAL call from that test function to a candidate matching
//!    the name the convention implies (`test_add` must actually CALL
//!    something named `add` among the given candidates, not merely
//!    share a name-prefix relationship with one).
//!
//! Neither condition alone is sufficient, and the result is ALWAYS
//! `NameHeuristic` evidence with an explicit `Confidence`, NEVER
//! `StaticAnalysis` — this resolver never treats a naming convention,
//! however strong, as proof of which specific target a given test
//! exercises.
//!
//! **Known, explicitly out-of-scope gap for this task:** Rust's real
//! `#[test]` attribute is a genuine, unambiguous syntactic marker that a
//! function IS a test — a stronger signal than any naming convention.
//! This resolver does NOT currently use it: `extract::rust`'s own
//! extraction boundary captures a `function_item`'s signature text
//! starting at `fn`, not at any preceding `#[test]` attribute sibling,
//! so the attribute is not present in the `signature_text` this
//! resolver re-parses. Using it would require changing Task 4's
//! extraction boundary (capturing the attribute alongside the function)
//! or carrying it as a separate field on `RawSymbolCandidate`/
//! `SymbolSnapshot` — both out of THIS task's scope to change, and
//! exactly the kind of coverage gap this task's own closing report
//! flags as a real-repository-validation-driven candidate for later
//! work, not implemented speculatively here. Even if it were available,
//! per the module's own rule, it would still only raise confidence
//! within `Hypothesized`-equivalent (`NameHeuristic`) territory for the
//! TARGET resolution, never promote the edge to `StaticAnalysis` — the
//! attribute proves "this is a test," not "this specific call is the
//! one under test."

use codegraph_domain::{Confidence, EdgeKind, ResolutionMethod};

use super::calls::parser_for;
use super::resolution::{heuristic_evidence, Edge, EdgeResolutionError, TargetLookup};
use crate::snapshot::SymbolSnapshot;

pub fn resolve_tests_for(snapshots: &[SymbolSnapshot]) -> Result<Vec<Edge>, EdgeResolutionError> {
    let lookup = TargetLookup::new(snapshots)?;
    let mut edges = Vec::new();

    for snapshot in lookup.snapshots() {
        let Some(implied_target_name) = test_naming_convention_target(snapshot.name().as_str()) else {
            continue;
        };

        let Some(parser) = parser_for(snapshot.file()) else {
            continue;
        };
        let Ok(tree) = parser.parse(snapshot.signature().as_str().as_bytes()) else {
            continue;
        };

        let called_names = super::calls::call_names_in_tree(&tree);
        if !called_names.iter().any(|name| name == &implied_target_name) {
            // The naming convention alone, with no actual call to the
            // implied target found anywhere in this function's body,
            // is not evidence this resolver accepts — a function named
            // `test_add` that never calls anything named `add` is NOT
            // linked, per the task's explicit "do not infer from naming
            // alone" instruction.
            continue;
        }

        let candidates = lookup.find_by_name(&implied_target_name, snapshot.id());
        if candidates.len() != 1 {
            // Zero or ambiguous: same "no edge beats a false edge"
            // discipline as every other edge family in this module.
            continue;
        }

        let (evidence, _confidence) = heuristic_evidence(
            ResolutionMethod::NameHeuristic,
            Confidence::Speculative,
            &format!("naming convention (no syntactic test marker available through this pipeline's current extraction boundary) plus an actual call to a candidate named {implied_target_name:?}"),
        );
        edges.push(Edge {
            from: snapshot.id(),
            to: candidates[0].id(),
            kind: EdgeKind::TestsFor,
            evidence,
        });
    }

    Ok(edges)
}

/// If `name` matches a recognized test-naming convention, returns the
/// target name it implies — `test_add` / `test_Add` -> `"add"`,
/// `TestAdd` -> `"Add"` (kept as-is, no case-folding: this resolver
/// still requires an EXACT name match against a real candidate, so a
/// convention that implies the wrong case simply will not find one,
/// rather than this function silently normalizing case to manufacture a
/// match). Returns `None` for anything that does not match any
/// recognized shape — this function is intentionally narrow rather than
/// a loose heuristic that would turn "do not infer from naming alone"
/// into "infer from ANY name that looks test-ish."
fn test_naming_convention_target(name: &str) -> Option<String> {
    if let Some(rest) = name.strip_prefix("test_") {
        if !rest.is_empty() {
            return Some(rest.to_string());
        }
    }
    if let Some(rest) = name.strip_prefix("Test") {
        if !rest.is_empty() && rest.chars().next().is_some_and(|c| c.is_uppercase()) {
            return Some(rest.to_string());
        }
    }
    None
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
    fn a_naming_convention_plus_a_real_call_is_linked_as_hypothesized_never_static_analysis() {
        // A plain call, not wrapped in a macro: `assert_eq!(...)`'s
        // arguments are an opaque `token_tree` to tree-sitter (confirmed
        // empirically), so `add(1, 2)` inside one is NOT parsed as a
        // real `call_expression` node this resolver's `calls.rs` walk
        // can see — an honest limitation of the tree-sitter-based
        // approach for macro-wrapped calls, not something this test
        // should paper over by picking a fixture that happens to dodge
        // it silently.
        let target = test_snapshot("src/lib.rs", "add", "function_item", "fn add(a: i32, b: i32) -> i32 { a + b }");
        let test_fn = test_snapshot("src/tests.rs", "test_add", "function_item", "fn test_add() { let result = add(1, 2); }");
        let snapshots = vec![target.clone(), test_fn.clone()];

        let edges = resolve_tests_for(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].from, test_fn.id());
        assert_eq!(edges[0].to, target.id());
        assert_eq!(edges[0].kind, EdgeKind::TestsFor);
        assert_ne!(edges[0].evidence.method(), ResolutionMethod::StaticAnalysis, "TestsFor must never be StaticAnalysis");
    }

    #[test]
    fn a_naming_convention_with_no_actual_call_is_not_linked() {
        // test_add exists, a function named add exists, but test_add's
        // body never actually calls add — naming alone must not be
        // sufficient.
        let target = test_snapshot("src/lib.rs", "add", "function_item", "fn add(a: i32, b: i32) -> i32 { a + b }");
        let test_fn = test_snapshot("src/tests.rs", "test_add", "function_item", "fn test_add() { assert_eq!(1 + 1, 2); }");
        let snapshots = vec![target, test_fn];

        let edges = resolve_tests_for(&snapshots).unwrap();

        assert!(edges.is_empty(), "a test-shaped name with no actual call to the implied target must not be linked");
    }

    #[test]
    fn a_function_without_a_test_naming_convention_is_never_considered() {
        let target = test_snapshot("src/lib.rs", "add", "function_item", "fn add(a: i32, b: i32) -> i32 { a + b }");
        let caller = test_snapshot("src/lib.rs", "compute", "function_item", "fn compute() -> i32 { add(1, 2) }");
        let snapshots = vec![target, caller];

        let edges = resolve_tests_for(&snapshots).unwrap();

        assert!(edges.is_empty(), "an ordinary caller with no test-naming convention must never produce a TestsFor edge");
    }

    #[test]
    fn an_ambiguous_implied_target_produces_no_edge() {
        let candidate_a = test_snapshot("src/a.rs", "add", "function_item", "fn add() {}");
        let candidate_b = test_snapshot("src/b.rs", "add", "function_item", "fn add() {}");
        let test_fn = test_snapshot("src/tests.rs", "test_add", "function_item", "fn test_add() { add(); }");
        let snapshots = vec![candidate_a, candidate_b, test_fn];

        let edges = resolve_tests_for(&snapshots).unwrap();

        assert!(edges.is_empty(), "an ambiguous implied target must never be arbitrarily picked");
    }

    #[test]
    fn a_duplicate_snapshot_id_is_a_typed_rejection() {
        let snapshot = test_snapshot("src/tests.rs", "test_add", "function_item", "fn test_add() {}");
        let snapshots = vec![snapshot.clone(), snapshot];

        let result = resolve_tests_for(&snapshots);

        assert_eq!(result.err(), Some(EdgeResolutionError::DuplicateSnapshotId));
    }

    #[test]
    fn the_test_prefix_convention_with_an_uppercase_follow_on_and_a_real_call_is_linked() {
        // `TestAdd` (C#-style PascalCase convention), distinct from
        // `test_add` — exercises the second convention branch directly.
        let target = test_snapshot("src/lib.cs", "Add", "method_declaration", "int Add(int a, int b) { return a + b; }");
        let test_fn = test_snapshot("src/tests.cs", "TestAdd", "method_declaration", "void TestAdd() { Add(1, 2); }");
        let snapshots = vec![target.clone(), test_fn.clone()];

        let edges = resolve_tests_for(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to, target.id());
    }

    #[test]
    fn a_test_prefix_followed_by_lowercase_does_not_match_the_pascal_case_convention() {
        // "Testing" -> strip_prefix("Test") gives "ing", whose first
        // char is lowercase -> must NOT match the PascalCase
        // convention (which requires an uppercase follow-on, e.g.
        // "TestAdd" -> "Add"). Confirms this is not treated as implying
        // a target named "ing".
        assert_eq!(test_naming_convention_target("Testing"), None);
    }

    #[test]
    fn a_bare_test_prefix_with_nothing_after_it_implies_no_target() {
        assert_eq!(test_naming_convention_target("test_"), None);
        assert_eq!(test_naming_convention_target("Test"), None);
    }
}
