//! B2 Task 9.5, Priority 3: `Instantiates` edges — a `Field`'s own
//! right-hand side is a resolvable constructor/call expression naming
//! another known symbol. Stricter than `Field` extraction itself (which
//! never depends on the RHS shape, per Task 9.5's own decision): not
//! every `Field` gets an `Instantiates` edge — only when the RHS is
//! genuinely a call AND that call's callee name resolves to EXACTLY ONE
//! candidate in the given pool.
//!
//! # Reuse, not reinvention
//!
//! This resolver re-parses a `Field` snapshot's own `signature_text`
//! (the WHOLE assignment statement, e.g. `self.encoder =
//! TransformerEncoder(a, b)`) and reuses `calls::call_names_in_tree` —
//! the SAME call-site walk `resolve_calls` already uses — to find the
//! constructor name. This deliberately does not duplicate call-
//! extraction logic: a `self.encoder = TransformerEncoder(a, b)`
//! statement's one real call expression is extracted exactly the same
//! way a `forward`'s call site would be, since both are, syntactically,
//! just a `call` node.
//!
//! # No invented targets for external constructors
//!
//! If the constructor name resolves to ZERO candidates (the overwhelmingly
//! common real case — `nn.Linear`, `nn.Dropout`, and most other
//! constructors in real deep-learning code are external/third-party,
//! never indexed in this pipeline's own pool) or to MORE THAN ONE
//! candidate (ambiguous — two different classes share that constructor
//! name in the given pool), NO edge is produced. "No edge beats a false
//! edge" applies here exactly as it does for every other edge kind in
//! this project; this resolver never guesses which external type a
//! name "probably" refers to.
//!
//! Always `StaticAnalysis` when an edge IS produced: a uniquely-resolved
//! constructor call is syntactically unambiguous evidence, not a
//! heuristic guess — the same reasoning `resolve_calls` already applies
//! to its own uniquely-resolved case.
//!
//! # Immune to the B2-ID-006/B2-ID-007 nested-scope-ownership defect family
//!
//! [B2-ID-006](../../../../docs/research/findings/B2-ID-006.md) and
//! [B2-ID-007](../../../../docs/research/findings/B2-ID-007.md) found
//! that `resolve_calls`/`resolve_extends_implements`/`resolve_tests_for`
//! could attribute a relation to an ANCESTOR container snapshot (e.g. a
//! `Class`) merely because that container's own re-parsed
//! `signature_text` happens to textually contain a nested symbol's own
//! relevant construct. This resolver's own outer loop
//! (`if field.kind() != SymbolKind::Field { continue; }`, below) is
//! structurally immune to that entire defect family: a `Class`/
//! `Struct`/`Interface`/etc. snapshot is never even considered as a
//! candidate "from" side here, independent of any span-nesting check —
//! confirmed by a direct, real-parse reproduction during B2-ID-007's
//! own resolver-ownership audit (`class Outer: def __init__(self):
//! self.x = Foo()` produces exactly `x -> Foo`, never `Outer -> Foo`).
//! This is not an accident of this resolver's current behavior; it
//! follows directly from a `Field` (`self.X = <rhs>`) only ever
//! existing inside a method body in the first place (`self` is only
//! ever bound as a parameter inside one), so there is no ancestor
//! container this resolver's own kind filter would ever need to
//! exclude separately.

use codegraph_domain::{EdgeKind, SymbolKind};

use super::calls::call_names_in_tree;
use super::resolution::{resolved_evidence, Edge, EdgeResolutionError, TargetLookup};
use crate::parse::parser_for;
use crate::snapshot::SymbolSnapshot;

pub fn resolve_instantiates(snapshots: &[SymbolSnapshot]) -> Result<Vec<Edge>, EdgeResolutionError> {
    let lookup = TargetLookup::new(snapshots)?;
    let mut edges = Vec::new();

    for field in lookup.snapshots() {
        if field.kind() != SymbolKind::Field {
            continue;
        }
        let Some(parser) = parser_for(field.file()) else {
            continue;
        };
        let Ok(tree) = parser.parse(field.signature().as_str().as_bytes()) else {
            continue;
        };

        // B2-ID-002-shaped occurrence-vs-relation note: a Field's RHS
        // can genuinely contain MORE THAN ONE call expression (e.g.
        // `self.mlp = nn.Sequential(nn.Linear(...), nn.Linear(...))`
        // — two real, distinct `Linear(...)` call sites), and
        // `call_names_in_tree` correctly returns one entry per
        // occurrence, not per distinct relation. If more than one
        // occurrence resolves to the SAME target (confirmed to happen
        // on real data: `pytorch/examples`' `nn.Sequential(nn.Linear,
        // nn.Linear, ...)` pattern repeatedly instantiating the same
        // internally-defined class under an ambiguous ``nn.Linear``-
        // shaped name), this must collapse to ONE edge per `(from,
        // to)`, never one per occurrence — the exact same mismatch
        // B2-ID-002 found and fixed in `resolve_calls`, confirmed here
        // in this resolver too by a dedicated regression test.
        let mut seen_targets = std::collections::HashSet::new();
        for constructor_name in call_names_in_tree(&tree) {
            let candidates = lookup.find_by_name(&constructor_name, field.id());
            if candidates.len() != 1 {
                // 0: external/unresolvable constructor — no edge.
                // >1: ambiguous same-named constructor across the
                // given pool — no edge either; unlike Calls (where a
                // real call site genuinely could invoke any of several
                // overloads), an instantiation names exactly one type,
                // so an ambiguous match is not evidence this resolver
                // can honestly split across multiple heuristic edges.
                continue;
            }
            let target = candidates[0];
            if !seen_targets.insert(target.id()) {
                continue;
            }
            let evidence = resolved_evidence(&format!("field {:?}'s assignment calls {constructor_name:?}, resolving to exactly one candidate in the given snapshot pool", field.name().as_str()));
            edges.push(Edge {
                from: field.id(),
                to: target.id(),
                kind: EdgeKind::Instantiates,
                evidence,
            });
        }
    }

    Ok(edges)
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

    fn test_snapshot(repo: codegraph_domain::RepositoryId, path: &str, name: &str, kind_hint: &str, signature: &str) -> SymbolSnapshot {
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        let file = FileIdentity::parse(repo, commit, path).unwrap();
        let candidate = RawSymbolCandidate::new(name.to_string(), kind_hint.to_string(), ByteSpan { start: 0, end: 10 }, signature.to_string()).unwrap();
        promote(&candidate, file).unwrap()
    }

    #[test]
    fn a_field_assigned_a_resolvable_constructor_call_produces_an_instantiates_edge() {
        let repo = test_repo();
        let encoder_field = test_snapshot(repo, "src/model.py", "encoder", "self_attribute_assignment", "self.encoder = TransformerEncoder(a, b)");
        let encoder_class = test_snapshot(repo, "src/model.py", "TransformerEncoder", "class_definition", "class TransformerEncoder:\n    pass");
        let snapshots = vec![encoder_field.clone(), encoder_class.clone()];

        let edges = resolve_instantiates(&snapshots).unwrap();

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].from, encoder_field.id());
        assert_eq!(edges[0].to, encoder_class.id());
        assert_eq!(edges[0].kind, EdgeKind::Instantiates);
        assert_eq!(edges[0].evidence.method(), codegraph_domain::ResolutionMethod::StaticAnalysis);
    }

    #[test]
    fn a_field_assigned_an_unresolvable_external_constructor_produces_no_edge() {
        // nn.Linear is never indexed in this pool — no target must be
        // invented for it.
        let repo = test_repo();
        let field = test_snapshot(repo, "src/model.py", "classifier", "self_attribute_assignment", "self.classifier = nn.Linear(ninp, ntoken)");
        let snapshots = vec![field.clone()];

        let edges = resolve_instantiates(&snapshots).unwrap();

        assert!(edges.is_empty(), "an external/unresolvable constructor must never produce an invented target");
    }

    #[test]
    fn a_field_assigned_a_plain_scalar_produces_no_instantiates_edge() {
        let repo = test_repo();
        let field = test_snapshot(repo, "src/model.py", "ninp", "self_attribute_assignment", "self.ninp = ninp");
        let snapshots = vec![field.clone()];

        let edges = resolve_instantiates(&snapshots).unwrap();

        assert!(edges.is_empty(), "a scalar/non-call assignment must never produce an Instantiates edge, even though it is a valid Field");
    }

    #[test]
    fn an_ambiguous_constructor_name_produces_no_edge() {
        let repo = test_repo();
        let field = test_snapshot(repo, "src/model.py", "encoder", "self_attribute_assignment", "self.encoder = Encoder()");
        let candidate_a = test_snapshot(repo, "src/a.py", "Encoder", "class_definition", "class Encoder:\n    pass");
        let candidate_b = test_snapshot(repo, "src/b.py", "Encoder", "class_definition", "class Encoder:\n    pass");
        let snapshots = vec![field.clone(), candidate_a.clone(), candidate_b.clone()];

        let edges = resolve_instantiates(&snapshots).unwrap();

        assert!(edges.is_empty(), "an ambiguous constructor name must never be arbitrarily picked");
    }

    #[test]
    fn a_non_field_snapshot_is_never_considered_as_a_source() {
        let repo = test_repo();
        let function = test_snapshot(repo, "src/model.py", "forward", "function_definition", "def forward(self): return Encoder()");
        let encoder_class = test_snapshot(repo, "src/model.py", "Encoder", "class_definition", "class Encoder:\n    pass");
        let snapshots = vec![function.clone(), encoder_class.clone()];

        let edges = resolve_instantiates(&snapshots).unwrap();

        assert!(edges.is_empty(), "Instantiates only ever originates from a Field snapshot, never a Function/Method, even if its body happens to call a constructor");
    }

    #[test]
    fn a_duplicate_snapshot_id_is_a_typed_rejection() {
        let repo = test_repo();
        let field = test_snapshot(repo, "src/model.py", "encoder", "self_attribute_assignment", "self.encoder = Encoder()");
        let snapshots = vec![field.clone(), field];

        let result = resolve_instantiates(&snapshots);

        assert_eq!(result.err(), Some(EdgeResolutionError::DuplicateSnapshotId));
    }

    #[test]
    fn a_field_calling_the_same_resolvable_constructor_more_than_once_produces_one_edge_not_a_duplicate() {
        // Real-repository regression (found during Task 9.5's own
        // smoke-test reproduction against pytorch/examples):
        // `self.mlp = Sequential(Linear(), Linear())` — TWO real,
        // distinct call sites for the same constructor name inside one
        // Field's RHS. Both resolving to the SAME target must collapse
        // to ONE Instantiates edge, never two identical (from, to,
        // kind) triples (which CodeGraph::insert_edge would otherwise
        // reject as DuplicateEdge).
        let repo = test_repo();
        let field = test_snapshot(repo, "src/model.py", "mlp", "self_attribute_assignment", "self.mlp = Sequential(Linear(), Linear())");
        let linear_class = test_snapshot(repo, "src/model.py", "Linear", "class_definition", "class Linear:\n    pass");
        let snapshots = vec![field.clone(), linear_class.clone()];

        let edges = resolve_instantiates(&snapshots).unwrap();

        let to_linear: Vec<_> = edges.iter().filter(|e| e.to == linear_class.id()).collect();
        assert_eq!(to_linear.len(), 1, "two occurrences of the same resolvable constructor must collapse to one Instantiates edge");
    }

    #[test]
    fn a_field_calling_two_different_resolvable_constructors_produces_two_edges() {
        let repo = test_repo();
        let field = test_snapshot(repo, "src/model.py", "mlp", "self_attribute_assignment", "self.mlp = Sequential(Linear(), Dropout())");
        let linear_class = test_snapshot(repo, "src/model.py", "Linear", "class_definition", "class Linear:\n    pass");
        let dropout_class = test_snapshot(repo, "src/model.py", "Dropout", "class_definition", "class Dropout:\n    pass");
        let snapshots = vec![field.clone(), linear_class.clone(), dropout_class.clone()];

        let edges = resolve_instantiates(&snapshots).unwrap();

        assert_eq!(edges.len(), 2, "two genuinely distinct constructors must not be collapsed into one edge");
        assert!(edges.iter().any(|e| e.to == linear_class.id()));
        assert!(edges.iter().any(|e| e.to == dropout_class.id()));
    }
}
