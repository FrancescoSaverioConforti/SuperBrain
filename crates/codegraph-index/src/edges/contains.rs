//! B2 Task 9.5, Priority 2: `Contains` edges — a structural containment
//! relation read directly off byte-span nesting, never a name-based
//! heuristic. Motivated by `docs/research/evaluations/B2-EVAL-001.md`'s
//! finding that the graph had no way to reconstruct "what does this
//! class own" (e.g. `MyModel contains forward`, `MyModel contains
//! encoder`) even once `Field`/`Attribute` existed as its own
//! `SymbolKind` — knowing a `Field`/`Method` snapshot exists is not the
//! same as knowing WHICH class it belongs to.
//!
//! # Scope: `Class -> Method` and `Class -> Field` only
//!
//! A `Module`/`File -> top-level symbol` containment relation was
//! explicitly considered and deferred — this pipeline has no real,
//! AST-derived `Module`/`File` snapshot today (`SymbolKind::Module`
//! exists in the baseline but no extractor currently produces one), and
//! inventing a synthetic container node just to complete this edge
//! family would be exactly the kind of speculative data-model expansion
//! the B2 plan's section 8b policy forbids. Recorded as a residual
//! limitation of this task, not implemented.
//!
//! # How containment is determined
//!
//! `B` is contained by `A` when: `A` and `B` are in the SAME file;
//! `A.kind() == Class`; `B.kind()` is `Function`, `Method`, or `Field`
//! (`Function` is included because Task 4/5's own Python/C++ extractor
//! does not distinguish a method from a free function at the
//! `SymbolKind` level — both map to `Function` — so excluding it here
//! would silently produce zero `Contains` edges for the most common
//! real case: every Python method in this pipeline's own data today);
//! `B`'s span is strictly nested inside `A`'s span (`A.start <= B.start
//! && B.end <= A.end`, and `B != A`); and `A` is the SMALLEST (innermost)
//! such containing
//! candidate among every `Class` in the given pool — so a method
//! nested inside a class that is itself nested inside an outer class
//! (a real, if less common, Python/JS/TS shape) is attributed to its
//! immediate enclosing class, never the outermost one, without this
//! resolver needing to know anything about scope beyond comparing byte
//! ranges. This is purely a property of `ByteSpan` nesting — every
//! extractor already sets spans correctly for its own language, so
//! this resolver needs no per-language logic at all.
//!
//! Always `StaticAnalysis`: containment is read directly off the AST's
//! own structure (via the span every extractor already produces), never
//! a guess — there is no heuristic/ambiguous case for this edge kind.
//!
//! # Immune to the B2-ID-006/B2-ID-007 nested-scope-ownership defect family
//!
//! [B2-ID-006](../../../../docs/research/findings/B2-ID-006.md) and
//! [B2-ID-007](../../../../docs/research/findings/B2-ID-007.md) found
//! that `resolve_calls`/`resolve_extends_implements`/`resolve_tests_for`
//! could attribute a relation transitively to an OUTER ancestor
//! container, not only to the correct, nearest one — a failure mode
//! this resolver's own design (see "How containment is determined"
//! above) was already built to avoid: explicitly selecting the
//! SMALLEST (innermost) containing `Class` candidate, not merely any
//! containing one, is exactly the fix those two findings needed for
//! `Calls`/`Extends`/`Implements`/`TestsFor`, already present here from
//! this resolver's first version. Confirmed immune by a direct,
//! real-parse reproduction during B2-ID-007's own resolver-ownership
//! audit (`class Outer: class Inner: def method(self): pass` produces
//! exactly `Inner -> method`, never a transitive `Outer -> method`).

use codegraph_domain::{EdgeKind, SymbolKind};

use super::resolution::{resolved_evidence, Edge, EdgeResolutionError, TargetLookup};
use crate::snapshot::SymbolSnapshot;

pub fn resolve_contains(snapshots: &[SymbolSnapshot]) -> Result<Vec<Edge>, EdgeResolutionError> {
    let lookup = TargetLookup::new(snapshots)?;
    let mut edges = Vec::new();

    for contained in lookup.snapshots() {
        // `Function` is included alongside `Method`: Task 4/5's own
        // Python (and C++) extractor does not distinguish a method
        // from a free function at the `kind_hint`/`SymbolKind` level —
        // both map to `SymbolKind::Function` (see `kind_mapping.rs`'s
        // own doc comment) — so a `Function`-kind snapshot nested
        // inside a `Class`'s span IS a method in every real sense this
        // resolver can observe; excluding it here would silently
        // produce zero `Contains` edges for the most common real case
        // (every real Python method in this pipeline's data today).
        if !matches!(contained.kind(), SymbolKind::Function | SymbolKind::Method | SymbolKind::Field) {
            continue;
        }

        let mut innermost: Option<&SymbolSnapshot> = None;
        for candidate_class in lookup.snapshots() {
            if candidate_class.id() == contained.id() || candidate_class.kind() != SymbolKind::Class {
                continue;
            }
            if !same_file(candidate_class, contained) {
                continue;
            }
            if !strictly_contains(candidate_class, contained) {
                continue;
            }
            innermost = Some(match innermost {
                None => candidate_class,
                Some(current) if span_len(candidate_class) < span_len(current) => candidate_class,
                Some(current) => current,
            });
        }

        if let Some(container) = innermost {
            let evidence = resolved_evidence(&format!(
                "{:?} span [{}, {}) is nested inside {:?} span [{}, {})",
                contained.kind(),
                contained.span().start,
                contained.span().end,
                container.kind(),
                container.span().start,
                container.span().end
            ));
            edges.push(Edge {
                from: container.id(),
                to: contained.id(),
                kind: EdgeKind::Contains,
                evidence,
            });
        }
    }

    Ok(edges)
}

fn same_file(a: &SymbolSnapshot, b: &SymbolSnapshot) -> bool {
    a.file().repository() == b.file().repository() && a.file().commit() == b.file().commit() && a.file().relative_path() == b.file().relative_path()
}

/// `true` when `outer`'s span strictly contains `inner`'s span —
/// `inner` must be fully within `outer`'s byte range, and the two
/// spans must not be identical (a symbol never "contains" itself, nor
/// a byte-for-byte coincidental duplicate span).
fn strictly_contains(outer: &SymbolSnapshot, inner: &SymbolSnapshot) -> bool {
    let outer_span = outer.span();
    let inner_span = inner.span();
    outer_span.start <= inner_span.start && inner_span.end <= outer_span.end && (outer_span.start, outer_span.end) != (inner_span.start, inner_span.end)
}

fn span_len(snapshot: &SymbolSnapshot) -> usize {
    snapshot.span().end - snapshot.span().start
}

#[cfg(test)]
mod tests {
    use super::*;
    use codegraph_domain::{CommitHash, FileIdentity, RepositoryRegistry, RepositoryRoot};

    use crate::extract::{ByteSpan, RawSymbolCandidate};
    use crate::snapshot::promote;

    fn test_repo() -> codegraph_domain::RepositoryId {
        let mut registry = RepositoryRegistry::new();
        registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap())
    }

    fn test_snapshot(repo: codegraph_domain::RepositoryId, path: &str, name: &str, kind_hint: &str, signature: &str, start: usize, end: usize) -> SymbolSnapshot {
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        let file = FileIdentity::parse(repo, commit, path).unwrap();
        let candidate = RawSymbolCandidate::new(name.to_string(), kind_hint.to_string(), ByteSpan { start, end }, signature.to_string()).unwrap();
        promote(&candidate, file).unwrap()
    }

    #[test]
    fn class_contains_method() {
        let repo = test_repo();
        let class = test_snapshot(repo, "src/m.py", "MyModel", "class_definition", "class MyModel:\n    def forward(self): pass\n", 0, 100);
        let method = test_snapshot(repo, "src/m.py", "forward", "function_definition", "def forward(self): pass", 20, 90);
        let snapshots = vec![class.clone(), method.clone()];

        let edges = resolve_contains(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].from, class.id());
        assert_eq!(edges[0].to, method.id());
        assert_eq!(edges[0].kind, EdgeKind::Contains);
        assert_eq!(edges[0].evidence.method(), codegraph_domain::ResolutionMethod::StaticAnalysis);
    }

    #[test]
    fn class_contains_field() {
        let repo = test_repo();
        let class = test_snapshot(repo, "src/m.py", "MyModel", "class_definition", "class MyModel:\n    ...\n", 0, 100);
        let field = test_snapshot(repo, "src/m.py", "encoder", "self_attribute_assignment", "self.encoder = Encoder()", 20, 45);
        let snapshots = vec![class.clone(), field.clone()];

        let edges = resolve_contains(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].from, class.id());
        assert_eq!(edges[0].to, field.id());
        assert_eq!(edges[0].kind, EdgeKind::Contains);
    }

    #[test]
    fn a_method_outside_any_class_span_produces_no_contains_edge() {
        let repo = test_repo();
        let class = test_snapshot(repo, "src/m.py", "MyModel", "class_definition", "class MyModel:\n    pass\n", 0, 30);
        let free_function = test_snapshot(repo, "src/m.py", "helper", "function_definition", "def helper(): pass", 50, 80);
        let snapshots = vec![class.clone(), free_function.clone()];

        let edges = resolve_contains(&snapshots).unwrap();

        assert!(edges.is_empty(), "a function outside any class's span must produce no Contains edge");
    }

    #[test]
    fn a_method_in_a_different_file_from_the_class_produces_no_contains_edge() {
        let repo = test_repo();
        let class = test_snapshot(repo, "src/a.py", "MyModel", "class_definition", "class MyModel:\n    pass\n", 0, 100);
        // Same byte range, different file — must not be treated as
        // contained just because the numbers happen to overlap.
        let method = test_snapshot(repo, "src/b.py", "forward", "function_definition", "def forward(self): pass", 20, 90);
        let snapshots = vec![class.clone(), method.clone()];

        let edges = resolve_contains(&snapshots).unwrap();

        assert!(edges.is_empty(), "span overlap across different files must never produce a Contains edge");
    }

    #[test]
    fn a_nested_inner_class_methods_attribute_to_the_immediate_enclosing_class_not_the_outer_one() {
        // Class-contains-class is explicitly OUT of this task's
        // authorized scope (only Class->Method/Function and
        // Class->Field) — this test only checks that a method nested
        // inside Inner, which is itself nested inside Outer, is
        // attributed to Inner (the innermost containing Class), never
        // to Outer, even though Outer's span also structurally
        // contains the method's span.
        let repo = test_repo();
        let outer = test_snapshot(repo, "src/m.py", "Outer", "class_definition", "class Outer:\n    class Inner:\n        def m(self): pass\n", 0, 100);
        let inner = test_snapshot(repo, "src/m.py", "Inner", "class_definition", "class Inner:\n        def m(self): pass", 20, 90);
        let method = test_snapshot(repo, "src/m.py", "m", "function_definition", "def m(self): pass", 50, 80);
        let snapshots = vec![outer.clone(), inner.clone(), method.clone()];

        let edges = resolve_contains(&snapshots).unwrap();

        let method_edge = edges.iter().find(|e| e.to == method.id()).expect("expected a Contains edge targeting the method");
        assert_eq!(method_edge.from, inner.id(), "the method must be attributed to its IMMEDIATE (innermost) enclosing class, not the outer one");
        assert_eq!(edges.len(), 1, "class-contains-class is out of scope: Inner itself must not get a Contains edge from Outer");
    }

    #[test]
    fn a_duplicate_snapshot_id_is_a_typed_rejection() {
        let repo = test_repo();
        let snapshot = test_snapshot(repo, "src/m.py", "MyModel", "class_definition", "class MyModel:\n    pass\n", 0, 30);
        let snapshots = vec![snapshot.clone(), snapshot];

        let result = resolve_contains(&snapshots);

        assert_eq!(result.err(), Some(EdgeResolutionError::DuplicateSnapshotId));
    }

    #[test]
    fn two_unrelated_classes_each_containing_their_own_method_do_not_cross_link() {
        let repo = test_repo();
        let class_a = test_snapshot(repo, "src/m.py", "A", "class_definition", "class A:\n    def f(self): pass\n", 0, 50);
        let method_a = test_snapshot(repo, "src/m.py", "f", "function_definition", "def f(self): pass", 10, 40);
        let class_b = test_snapshot(repo, "src/m.py", "B", "class_definition", "class B:\n    def g(self): pass\n", 100, 150);
        let method_b = test_snapshot(repo, "src/m.py", "g", "function_definition", "def g(self): pass", 110, 140);
        let snapshots = vec![class_a.clone(), method_a.clone(), class_b.clone(), method_b.clone()];

        let edges = resolve_contains(&snapshots).unwrap();

        assert_eq!(edges.len(), 2);
        assert!(edges.iter().any(|e| e.from == class_a.id() && e.to == method_a.id()));
        assert!(edges.iter().any(|e| e.from == class_b.id() && e.to == method_b.id()));
    }

    #[test]
    fn a_non_class_candidate_is_always_skipped_even_if_it_structurally_contains_the_member() {
        // Pins the `candidate_class.kind() != SymbolKind::Class` half
        // of the skip-condition's `||`, isolated from the id-equality
        // half: a Field-kind snapshot ("outer_field") whose span
        // happens to structurally contain a Function's span must NEVER
        // be reported as containing it — only a real Class may be a
        // `Contains` source. (The id-equality half is, by construction,
        // only ever true when the two conditions already coincide —
        // `contained`'s own kind is always Function/Method/Field, never
        // Class, so `candidate_class.id() == contained.id()` implies
        // `candidate_class.kind() != SymbolKind::Class` is ALSO true;
        // there is no real input where the id-equality half is true and
        // the kind half is false, so mutating `||` to `&&` is only
        // observable through the kind-mismatch case this test pins.)
        let repo = test_repo();
        let outer_field = test_snapshot(repo, "src/m.py", "outer_field", "self_attribute_assignment", "self.outer_field = Something()", 0, 100);
        let inner_method = test_snapshot(repo, "src/m.py", "inner_method", "function_definition", "def inner_method(): ...", 10, 50);
        let snapshots = vec![outer_field, inner_method.clone()];

        let edges = resolve_contains(&snapshots).unwrap();

        assert!(edges.is_empty(), "a non-Class candidate must never be treated as a Contains source, regardless of span containment");
    }

    #[test]
    fn resolve_contains_picks_the_first_innermost_class_found_when_a_later_candidate_ties_in_span_length() {
        // Pins the `span_len(candidate_class) < span_len(current)`
        // match guard directly against a TIE: two classes with the
        // SAME span length (50 bytes each) both structurally contain
        // `method` — mutating `<` to `<=` would make the LATER
        // candidate replace the first on a tie; fixing the input order
        // makes the correct (first-wins-on-tie) result deterministic
        // and verifiable.
        let repo = test_repo();
        let first_class = test_snapshot(repo, "src/m.py", "FirstClass", "class_definition", "class FirstClass: ...", 0, 200);
        let second_class = test_snapshot(repo, "src/m.py", "SecondClass", "class_definition", "class SecondClass: ...", 0, 200);
        let method = test_snapshot(repo, "src/m.py", "method", "function_definition", "def method(): ...", 80, 120);
        let snapshots = vec![first_class.clone(), second_class.clone(), method.clone()];

        let edges = resolve_contains(&snapshots).unwrap();

        let method_edge = edges.iter().find(|e| e.to == method.id()).expect("expected a Contains edge targeting the method");
        assert_eq!(method_edge.from, first_class.id(), "the first-encountered class must win a span-length tie, confirming strict `<` rather than `<=` governs replacement");
    }
}
