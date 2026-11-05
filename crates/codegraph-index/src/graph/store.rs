//! `CodeGraph`'s storage: insertion, invariant enforcement, and the
//! lookup/filter primitives built directly on top of the internal
//! indexes. See the module-level doc comment (`graph/mod.rs`) for the
//! full invariant and determinism contract this file implements.

use std::collections::{HashMap, HashSet};

use codegraph_domain::{EdgeKind, RepositoryId, SymbolSnapshotId};
use thiserror::Error;

use crate::edges::Edge;
use crate::graph::traversal::Direction;
use crate::lineage::LineageEdge;
use crate::snapshot::SymbolSnapshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum GraphError {
    /// A `SymbolSnapshot` with this id is already present. Never
    /// silently overwritten or silently skipped — see the module doc
    /// comment for why.
    #[error("a snapshot with this SymbolSnapshotId is already present in the graph")]
    DuplicateSnapshot,
    /// An edge's `from` endpoint does not name any snapshot already in
    /// the graph.
    #[error("edge references an unknown `from` snapshot: insert the snapshot before any edge that references it")]
    DanglingEdgeFrom,
    /// An edge's `to` endpoint does not name any snapshot already in
    /// the graph.
    #[error("edge references an unknown `to` snapshot: insert the snapshot before any edge that references it")]
    DanglingEdgeTo,
    /// The exact same logical edge (same `from`/`to` for a lineage edge;
    /// same `from`/`to`/`kind` for a typed edge) has already been
    /// inserted.
    #[error("this exact edge (same endpoints and kind) is already present in the graph")]
    DuplicateEdge,
    /// `from` and `to` belong to different repositories — nothing in
    /// this pipeline's current scope ever legitimately produces this.
    #[error("edge connects snapshots from two different repositories, which this graph never admits")]
    CrossRepositoryEdge,
}

/// One typed-edge key: `(from, to, kind)`. Two typed edges with this
/// same triple are the same logical edge regardless of their
/// `Evidence`/`Confidence` — see the module doc comment on why evidence
/// is deliberately excluded from the duplicate key.
type TypedEdgeKey = (SymbolSnapshotId, SymbolSnapshotId, EdgeKind);

/// A lineage-edge key: `(from, to)`. A graph never admits two lineage
/// edges between the same ordered pair, regardless of status.
type LineageEdgeKey = (SymbolSnapshotId, SymbolSnapshotId);

/// The in-memory graph of `SymbolSnapshot`s, `LineageEdge`s and typed
/// `Edge`s. See the module-level doc comment (`graph/mod.rs`) for the
/// full invariant, cycle-safety, determinism and evidence-preservation
/// contract. Constructed empty via [`CodeGraph::new`]; every value is
/// added through one of the `insert_*` methods, each of which enforces
/// this graph's invariants and returns a typed [`GraphError`] rather
/// than panicking or silently repairing malformed input.
#[derive(Debug, Default)]
pub struct CodeGraph {
    snapshots: HashMap<SymbolSnapshotId, SymbolSnapshot>,
    lineage_edges: HashMap<LineageEdgeKey, LineageEdge>,
    typed_edges: HashMap<TypedEdgeKey, Edge>,

    /// Outgoing typed-edge keys per source snapshot — an index, not a
    /// second source of truth; every key here also exists in
    /// `typed_edges`.
    outgoing: HashMap<SymbolSnapshotId, HashSet<TypedEdgeKey>>,
    /// Incoming typed-edge keys per target snapshot — same role as
    /// `outgoing`, reversed.
    incoming: HashMap<SymbolSnapshotId, HashSet<TypedEdgeKey>>,
    /// Outgoing lineage-edge keys per `from` snapshot.
    lineage_from: HashMap<SymbolSnapshotId, HashSet<LineageEdgeKey>>,
    /// Incoming lineage-edge keys per `to` snapshot.
    lineage_to: HashMap<SymbolSnapshotId, HashSet<LineageEdgeKey>>,
}

impl CodeGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts `snapshot`. Rejects a duplicate `SymbolSnapshotId`
    /// outright (see the module doc comment) rather than overwriting or
    /// silently accepting it as a no-op.
    pub fn insert_snapshot(&mut self, snapshot: SymbolSnapshot) -> Result<(), GraphError> {
        if self.snapshots.contains_key(&snapshot.id()) {
            return Err(GraphError::DuplicateSnapshot);
        }
        self.snapshots.insert(snapshot.id(), snapshot);
        Ok(())
    }

    /// Inserts `edge` (a cross-commit lineage link). Both endpoints must
    /// already be present ([`GraphError::DanglingEdgeFrom`]/
    /// [`GraphError::DanglingEdgeTo`]); the same `(from, to)` pair must
    /// not already have a lineage edge ([`GraphError::DuplicateEdge`]);
    /// both endpoints must share a `RepositoryId`
    /// ([`GraphError::CrossRepositoryEdge`]).
    pub fn insert_lineage_edge(&mut self, edge: LineageEdge) -> Result<(), GraphError> {
        self.check_endpoints(edge.from, edge.to)?;
        let key = (edge.from, edge.to);
        if self.lineage_edges.contains_key(&key) {
            return Err(GraphError::DuplicateEdge);
        }
        self.lineage_edges.insert(key, edge);
        self.lineage_from.entry(key.0).or_default().insert(key);
        self.lineage_to.entry(key.1).or_default().insert(key);
        Ok(())
    }

    /// Inserts `edge` (a typed, non-lineage edge: `Calls`, `Imports`,
    /// `Implements`, `Extends`, `References`, `TestsFor`, `RenamedTo`).
    /// Both endpoints must already be present; the same `(from, to,
    /// kind)` triple must not already exist; both endpoints must share a
    /// `RepositoryId`.
    pub fn insert_edge(&mut self, edge: Edge) -> Result<(), GraphError> {
        self.check_endpoints(edge.from, edge.to)?;
        let key = (edge.from, edge.to, edge.kind);
        if self.typed_edges.contains_key(&key) {
            return Err(GraphError::DuplicateEdge);
        }
        self.typed_edges.insert(key, edge);
        self.outgoing.entry(key.0).or_default().insert(key);
        self.incoming.entry(key.1).or_default().insert(key);
        Ok(())
    }

    fn check_endpoints(&self, from: SymbolSnapshotId, to: SymbolSnapshotId) -> Result<(), GraphError> {
        let from_snapshot = self.snapshots.get(&from).ok_or(GraphError::DanglingEdgeFrom)?;
        let to_snapshot = self.snapshots.get(&to).ok_or(GraphError::DanglingEdgeTo)?;
        if from_snapshot.file().repository() != to_snapshot.file().repository() {
            return Err(GraphError::CrossRepositoryEdge);
        }
        Ok(())
    }

    /// The number of snapshots currently in the graph.
    pub fn snapshot_count(&self) -> usize {
        self.snapshots.len()
    }

    /// The number of typed edges currently in the graph.
    pub fn edge_count(&self) -> usize {
        self.typed_edges.len()
    }

    /// The number of lineage edges currently in the graph.
    pub fn lineage_edge_count(&self) -> usize {
        self.lineage_edges.len()
    }

    /// Definition lookup: the snapshot with this id, if present.
    pub fn snapshot(&self, id: SymbolSnapshotId) -> Option<&SymbolSnapshot> {
        self.snapshots.get(&id)
    }

    /// Every snapshot in the graph, ordered deterministically by its
    /// `SymbolSnapshotId` bytes.
    pub fn all_snapshots(&self) -> Vec<&SymbolSnapshot> {
        let mut out: Vec<&SymbolSnapshot> = self.snapshots.values().collect();
        sort_snapshots(&mut out);
        out
    }

    /// Every snapshot belonging to `repository`, ordered deterministically.
    pub fn snapshots_in_repository(&self, repository: RepositoryId) -> Vec<&SymbolSnapshot> {
        let mut out: Vec<&SymbolSnapshot> = self.snapshots.values().filter(|s| s.file().repository() == repository).collect();
        sort_snapshots(&mut out);
        out
    }

    /// Lineage lookup: every lineage edge whose `from` is `id`, ordered
    /// deterministically by `to` bytes.
    pub fn lineage_from(&self, id: SymbolSnapshotId) -> Vec<&LineageEdge> {
        let mut out = self.lineage_keys_to_edges(self.lineage_from.get(&id));
        out.sort_by_key(|e| *e.to.as_bytes());
        out
    }

    /// Lineage lookup: every lineage edge whose `to` is `id` (i.e. every
    /// earlier snapshot this one continues from), ordered
    /// deterministically by `from` bytes.
    pub fn lineage_to(&self, id: SymbolSnapshotId) -> Vec<&LineageEdge> {
        let mut out = self.lineage_keys_to_edges(self.lineage_to.get(&id));
        out.sort_by_key(|e| *e.from.as_bytes());
        out
    }

    fn lineage_keys_to_edges(&self, keys: Option<&HashSet<LineageEdgeKey>>) -> Vec<&LineageEdge> {
        keys.into_iter()
            .flatten()
            .filter_map(|key| self.lineage_edges.get(key))
            .collect()
    }

    /// Every typed edge whose `from` is `id`, optionally filtered to one
    /// `EdgeKind`, ordered deterministically by `(to bytes, kind)`.
    pub fn outgoing_edges(&self, id: SymbolSnapshotId, kind: Option<EdgeKind>) -> Vec<&Edge> {
        let mut out = self.typed_keys_to_edges(self.outgoing.get(&id), kind);
        sort_edges_by_other_endpoint(&mut out, Direction::Outgoing);
        out
    }

    /// Every typed edge whose `to` is `id`, optionally filtered to one
    /// `EdgeKind`, ordered deterministically by `(from bytes, kind)`.
    pub fn incoming_edges(&self, id: SymbolSnapshotId, kind: Option<EdgeKind>) -> Vec<&Edge> {
        let mut out = self.typed_keys_to_edges(self.incoming.get(&id), kind);
        sort_edges_by_other_endpoint(&mut out, Direction::Incoming);
        out
    }

    fn typed_keys_to_edges(&self, keys: Option<&HashSet<TypedEdgeKey>>, kind: Option<EdgeKind>) -> Vec<&Edge> {
        keys.into_iter()
            .flatten()
            .filter(|key| kind.is_none_or(|k| key.2 == k))
            .filter_map(|key| self.typed_edges.get(key))
            .collect()
    }

    /// Every typed edge in the graph, ordered deterministically by
    /// `(from bytes, to bytes, kind)`. Used by traversal and by tests
    /// that need to inspect the whole edge set rather than one
    /// snapshot's neighborhood.
    pub fn all_edges(&self) -> Vec<&Edge> {
        let mut out: Vec<&Edge> = self.typed_edges.values().collect();
        out.sort_by(|a, b| a.from.as_bytes().cmp(b.from.as_bytes()).then_with(|| a.to.as_bytes().cmp(b.to.as_bytes())).then_with(|| edge_kind_rank(a.kind).cmp(&edge_kind_rank(b.kind))));
        out
    }
}

pub(super) fn sort_snapshots(snapshots: &mut [&SymbolSnapshot]) {
    snapshots.sort_by_key(|s| *s.id().as_bytes());
}

fn sort_edges_by_other_endpoint(edges: &mut [&Edge], direction: Direction) {
    edges.sort_by(|a, b| {
        let (key_a, key_b) = match direction {
            Direction::Outgoing => (a.to.as_bytes(), b.to.as_bytes()),
            Direction::Incoming => (a.from.as_bytes(), b.from.as_bytes()),
        };
        key_a.cmp(key_b).then_with(|| edge_kind_rank(a.kind).cmp(&edge_kind_rank(b.kind)))
    });
}

/// A fixed, explicit rank per `EdgeKind` variant for deterministic
/// sorting only — `EdgeKind` has no `Ord` impl in `codegraph-domain`,
/// and this graph does not add one there just to sort within this
/// module; the rank is purely a stable tie-breaker, never persisted or
/// compared across process runs with a different build of this crate.
pub(super) fn edge_kind_rank(kind: EdgeKind) -> u8 {
    match kind {
        EdgeKind::Calls => 0,
        EdgeKind::Imports => 1,
        EdgeKind::Implements => 2,
        EdgeKind::Extends => 3,
        EdgeKind::References => 4,
        EdgeKind::TestsFor => 5,
        EdgeKind::RenamedTo => 6,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codegraph_domain::{CommitHash, Evidence, FileIdentity, RepositoryRegistry, RepositoryRoot, ResolutionMethod};

    use crate::extract::{ByteSpan, RawSymbolCandidate};
    use crate::snapshot::promote;

    fn test_snapshot(repo: RepositoryId, commit: &str, path: &str, name: &str, kind_hint: &str, signature: &str) -> SymbolSnapshot {
        let commit = CommitHash::parse(&commit.repeat(40 / commit.len())).unwrap();
        let file = FileIdentity::parse(repo, commit, path).unwrap();
        let candidate = RawSymbolCandidate::new(name.to_string(), kind_hint.to_string(), ByteSpan { start: 0, end: 10 }, signature.to_string()).unwrap();
        promote(&candidate, file).unwrap()
    }

    fn test_repo() -> RepositoryId {
        let mut registry = RepositoryRegistry::new();
        registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap())
    }

    fn resolved_evidence(note: &str) -> Evidence {
        Evidence::new(ResolutionMethod::StaticAnalysis, None, note).unwrap()
    }

    #[test]
    fn a_minimal_graph_with_one_snapshot_and_one_edge_is_correct() {
        let repo = test_repo();
        let caller = test_snapshot(repo, "a", "src/lib.rs", "caller", "function_item", "fn caller() { callee(); }");
        let callee = test_snapshot(repo, "a", "src/lib.rs", "callee", "function_item", "fn callee() {}");
        let mut graph = CodeGraph::new();
        graph.insert_snapshot(caller.clone()).unwrap();
        graph.insert_snapshot(callee.clone()).unwrap();
        graph
            .insert_edge(Edge {
                from: caller.id(),
                to: callee.id(),
                kind: EdgeKind::Calls,
                evidence: resolved_evidence("test"),
            })
            .unwrap();

        assert_eq!(graph.snapshot_count(), 2);
        assert_eq!(graph.edge_count(), 1);
        assert_eq!(graph.snapshot(caller.id()).unwrap().id(), caller.id());
        assert_eq!(graph.outgoing_edges(caller.id(), None).len(), 1);
    }

    #[test]
    fn inserting_a_duplicate_snapshot_id_is_rejected() {
        let repo = test_repo();
        let snapshot = test_snapshot(repo, "a", "src/lib.rs", "foo", "function_item", "fn foo() {}");
        let mut graph = CodeGraph::new();
        graph.insert_snapshot(snapshot.clone()).unwrap();

        let result = graph.insert_snapshot(snapshot);

        assert_eq!(result, Err(GraphError::DuplicateSnapshot));
        assert_eq!(graph.snapshot_count(), 1, "the original snapshot must not be overwritten or duplicated");
    }

    #[test]
    fn inserting_an_edge_with_an_unknown_from_is_rejected() {
        let repo = test_repo();
        let known = test_snapshot(repo, "a", "src/lib.rs", "known", "function_item", "fn known() {}");
        let unknown = test_snapshot(repo, "a", "src/lib.rs", "unknown", "function_item", "fn unknown() {}");
        let mut graph = CodeGraph::new();
        graph.insert_snapshot(known.clone()).unwrap();

        let result = graph.insert_edge(Edge {
            from: unknown.id(),
            to: known.id(),
            kind: EdgeKind::Calls,
            evidence: resolved_evidence("test"),
        });

        assert_eq!(result, Err(GraphError::DanglingEdgeFrom));
    }

    #[test]
    fn inserting_an_edge_with_an_unknown_to_is_rejected() {
        let repo = test_repo();
        let known = test_snapshot(repo, "a", "src/lib.rs", "known", "function_item", "fn known() {}");
        let unknown = test_snapshot(repo, "a", "src/lib.rs", "unknown", "function_item", "fn unknown() {}");
        let mut graph = CodeGraph::new();
        graph.insert_snapshot(known.clone()).unwrap();

        let result = graph.insert_edge(Edge {
            from: known.id(),
            to: unknown.id(),
            kind: EdgeKind::Calls,
            evidence: resolved_evidence("test"),
        });

        assert_eq!(result, Err(GraphError::DanglingEdgeTo));
    }

    #[test]
    fn inserting_the_same_typed_edge_twice_is_rejected_as_duplicate() {
        let repo = test_repo();
        let a = test_snapshot(repo, "a", "src/lib.rs", "a", "function_item", "fn a() {}");
        let b = test_snapshot(repo, "a", "src/lib.rs", "b", "function_item", "fn b() {}");
        let mut graph = CodeGraph::new();
        graph.insert_snapshot(a.clone()).unwrap();
        graph.insert_snapshot(b.clone()).unwrap();
        let make_edge = || Edge {
            from: a.id(),
            to: b.id(),
            kind: EdgeKind::Calls,
            evidence: resolved_evidence("test"),
        };
        graph.insert_edge(make_edge()).unwrap();

        let result = graph.insert_edge(make_edge());

        assert_eq!(result, Err(GraphError::DuplicateEdge));
        assert_eq!(graph.edge_count(), 1);
    }

    #[test]
    fn two_different_edge_kinds_between_the_same_pair_are_both_accepted_not_duplicates() {
        let repo = test_repo();
        let a = test_snapshot(repo, "a", "src/lib.rs", "a", "function_item", "fn a() {}");
        let b = test_snapshot(repo, "a", "src/lib.rs", "b", "function_item", "fn b() {}");
        let mut graph = CodeGraph::new();
        graph.insert_snapshot(a.clone()).unwrap();
        graph.insert_snapshot(b.clone()).unwrap();

        graph
            .insert_edge(Edge {
                from: a.id(),
                to: b.id(),
                kind: EdgeKind::Calls,
                evidence: resolved_evidence("calls"),
            })
            .unwrap();
        graph
            .insert_edge(Edge {
                from: a.id(),
                to: b.id(),
                kind: EdgeKind::References,
                evidence: resolved_evidence("references"),
            })
            .unwrap();

        assert_eq!(graph.edge_count(), 2, "distinct EdgeKinds between the same pair must both be accepted");
    }

    #[test]
    fn a_cross_repository_edge_is_rejected() {
        let mut registry = RepositoryRegistry::new();
        let repo_a = registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap());
        let repo_b = registry.register(RepositoryRoot::parse(&std::env::temp_dir()).unwrap());
        let a = test_snapshot(repo_a, "a", "src/lib.rs", "a", "function_item", "fn a() {}");
        let b = test_snapshot(repo_b, "a", "src/lib.rs", "b", "function_item", "fn b() {}");
        let mut graph = CodeGraph::new();
        graph.insert_snapshot(a.clone()).unwrap();
        graph.insert_snapshot(b.clone()).unwrap();

        let result = graph.insert_edge(Edge {
            from: a.id(),
            to: b.id(),
            kind: EdgeKind::Calls,
            evidence: resolved_evidence("test"),
        });

        assert_eq!(result, Err(GraphError::CrossRepositoryEdge));
    }

    #[test]
    fn incoming_and_outgoing_edges_are_each_correct_and_direction_specific() {
        let repo = test_repo();
        let a = test_snapshot(repo, "a", "src/lib.rs", "a", "function_item", "fn a() { b(); }");
        let b = test_snapshot(repo, "a", "src/lib.rs", "b", "function_item", "fn b() {}");
        let mut graph = CodeGraph::new();
        graph.insert_snapshot(a.clone()).unwrap();
        graph.insert_snapshot(b.clone()).unwrap();
        graph
            .insert_edge(Edge {
                from: a.id(),
                to: b.id(),
                kind: EdgeKind::Calls,
                evidence: resolved_evidence("test"),
            })
            .unwrap();

        assert_eq!(graph.outgoing_edges(a.id(), None).len(), 1);
        assert_eq!(graph.incoming_edges(a.id(), None).len(), 0, "a's own call must not appear as incoming to a");
        assert_eq!(graph.outgoing_edges(b.id(), None).len(), 0);
        assert_eq!(graph.incoming_edges(b.id(), None).len(), 1);
    }

    #[test]
    fn filtering_by_edge_kind_excludes_other_kinds() {
        let repo = test_repo();
        let a = test_snapshot(repo, "a", "src/lib.rs", "a", "function_item", "fn a() {}");
        let b = test_snapshot(repo, "a", "src/lib.rs", "b", "function_item", "fn b() {}");
        let mut graph = CodeGraph::new();
        graph.insert_snapshot(a.clone()).unwrap();
        graph.insert_snapshot(b.clone()).unwrap();
        graph
            .insert_edge(Edge {
                from: a.id(),
                to: b.id(),
                kind: EdgeKind::Calls,
                evidence: resolved_evidence("calls"),
            })
            .unwrap();
        graph
            .insert_edge(Edge {
                from: a.id(),
                to: b.id(),
                kind: EdgeKind::TestsFor,
                evidence: resolved_evidence("tests_for"),
            })
            .unwrap();

        let calls_only = graph.outgoing_edges(a.id(), Some(EdgeKind::Calls));
        assert_eq!(calls_only.len(), 1);
        assert_eq!(calls_only[0].kind, EdgeKind::Calls);

        let implements_only = graph.outgoing_edges(a.id(), Some(EdgeKind::Implements));
        assert!(implements_only.is_empty(), "filtering by a kind with no matching edges must return empty, not all edges");
    }

    #[test]
    fn a_cyclic_calls_relationship_is_accepted_and_both_directions_are_queryable() {
        let repo = test_repo();
        let a = test_snapshot(repo, "a", "src/lib.rs", "a", "function_item", "fn a() { b(); }");
        let b = test_snapshot(repo, "a", "src/lib.rs", "b", "function_item", "fn b() { a(); }");
        let mut graph = CodeGraph::new();
        graph.insert_snapshot(a.clone()).unwrap();
        graph.insert_snapshot(b.clone()).unwrap();
        graph
            .insert_edge(Edge {
                from: a.id(),
                to: b.id(),
                kind: EdgeKind::Calls,
                evidence: resolved_evidence("a calls b"),
            })
            .unwrap();
        graph
            .insert_edge(Edge {
                from: b.id(),
                to: a.id(),
                kind: EdgeKind::Calls,
                evidence: resolved_evidence("b calls a"),
            })
            .unwrap();

        assert_eq!(graph.edge_count(), 2);
        assert_eq!(graph.outgoing_edges(a.id(), None)[0].to, b.id());
        assert_eq!(graph.outgoing_edges(b.id(), None)[0].to, a.id());
    }

    #[test]
    fn lineage_retrieval_preserves_hypothesized_status_and_its_confidence() {
        let repo = test_repo();
        let prev = test_snapshot(repo, "a", "src/lib.rs", "foo", "function_item", "fn foo(x: i32) -> i32 { x + 1 }");
        let next = test_snapshot(repo, "b", "src/lib.rs", "bar", "function_item", "fn bar(x: i32) -> i32 { x + 1 }");
        let mut graph = CodeGraph::new();
        graph.insert_snapshot(prev.clone()).unwrap();
        graph.insert_snapshot(next.clone()).unwrap();

        let evidence = Evidence::new(codegraph_domain::ResolutionMethod::Inferred, None, "possible rename").unwrap();
        let status = codegraph_domain::LineageStatus::hypothesized(evidence, codegraph_domain::Confidence::Likely);
        let lineage_id = codegraph_domain::SymbolLineageId::from_genesis(prev.id());
        graph
            .insert_lineage_edge(LineageEdge {
                from: prev.id(),
                to: next.id(),
                lineage: lineage_id,
                status: status.clone(),
            })
            .unwrap();

        let retrieved = &graph.lineage_from(prev.id())[0];
        assert!(!retrieved.status.is_confirmed(), "Hypothesized must stay Hypothesized after insertion and retrieval");
        assert_eq!(retrieved.status, status);
    }

    #[test]
    fn lineage_retrieval_preserves_confirmed_status_with_static_analysis_evidence() {
        let repo = test_repo();
        let prev = test_snapshot(repo, "a", "src/lib.rs", "foo", "function_item", "fn foo() {}");
        let next = test_snapshot(repo, "b", "src/lib.rs", "foo", "function_item", "fn foo() {}");
        let mut graph = CodeGraph::new();
        graph.insert_snapshot(prev.clone()).unwrap();
        graph.insert_snapshot(next.clone()).unwrap();

        let evidence = resolved_evidence("exact structural match");
        let status = codegraph_domain::LineageStatus::confirmed(evidence).unwrap();
        let lineage_id = codegraph_domain::SymbolLineageId::from_genesis(prev.id());
        graph
            .insert_lineage_edge(LineageEdge {
                from: prev.id(),
                to: next.id(),
                lineage: lineage_id,
                status,
            })
            .unwrap();

        let retrieved = &graph.lineage_from(prev.id())[0];
        assert!(retrieved.status.is_confirmed());
        assert_eq!(retrieved.status.is_confirmed(), true);
        assert_eq!(graph.lineage_to(next.id())[0].from, prev.id());
    }

    #[test]
    fn a_heuristic_typed_edge_retains_its_non_static_analysis_evidence_method() {
        let repo = test_repo();
        let a = test_snapshot(repo, "a", "src/lib.rs", "a", "function_item", "fn a() {}");
        let b = test_snapshot(repo, "a", "src/lib.rs", "b", "function_item", "fn b() {}");
        let mut graph = CodeGraph::new();
        graph.insert_snapshot(a.clone()).unwrap();
        graph.insert_snapshot(b.clone()).unwrap();
        let heuristic = Evidence::new(ResolutionMethod::NameHeuristic, None, "ambiguous").unwrap();
        graph
            .insert_edge(Edge {
                from: a.id(),
                to: b.id(),
                kind: EdgeKind::Calls,
                evidence: heuristic.clone(),
            })
            .unwrap();

        let retrieved = &graph.outgoing_edges(a.id(), None)[0];
        assert_eq!(retrieved.evidence.method(), ResolutionMethod::NameHeuristic);
        assert_eq!(retrieved.evidence, heuristic);
    }

    #[test]
    fn deterministic_ordering_is_stable_across_independently_built_graphs_with_different_insertion_order() {
        let repo = test_repo();
        let a = test_snapshot(repo, "a", "src/lib.rs", "a", "function_item", "fn a() {}");
        let b = test_snapshot(repo, "a", "src/lib.rs", "b", "function_item", "fn b() {}");
        let c = test_snapshot(repo, "a", "src/lib.rs", "c", "function_item", "fn c() {}");

        let mut graph_1 = CodeGraph::new();
        graph_1.insert_snapshot(a.clone()).unwrap();
        graph_1.insert_snapshot(b.clone()).unwrap();
        graph_1.insert_snapshot(c.clone()).unwrap();

        let mut graph_2 = CodeGraph::new();
        graph_2.insert_snapshot(c.clone()).unwrap();
        graph_2.insert_snapshot(a.clone()).unwrap();
        graph_2.insert_snapshot(b.clone()).unwrap();

        let ids_1: Vec<_> = graph_1.all_snapshots().iter().map(|s| s.id()).collect();
        let ids_2: Vec<_> = graph_2.all_snapshots().iter().map(|s| s.id()).collect();
        assert_eq!(ids_1, ids_2, "insertion order must never affect the returned snapshot order");
    }

    #[test]
    fn multiple_repositories_are_each_independently_queryable() {
        let mut registry = RepositoryRegistry::new();
        let repo_a = registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap());
        let repo_b = registry.register(RepositoryRoot::parse(&std::env::temp_dir()).unwrap());
        let a = test_snapshot(repo_a, "a", "src/lib.rs", "foo", "function_item", "fn foo() {}");
        let b = test_snapshot(repo_b, "a", "src/lib.rs", "foo", "function_item", "fn foo() {}");
        let mut graph = CodeGraph::new();
        graph.insert_snapshot(a.clone()).unwrap();
        graph.insert_snapshot(b.clone()).unwrap();

        assert_eq!(graph.snapshots_in_repository(repo_a).len(), 1);
        assert_eq!(graph.snapshots_in_repository(repo_b).len(), 1);
        assert_eq!(graph.snapshot_count(), 2, "the same symbol name in two different repositories must not collapse into one node");
    }

    #[test]
    fn same_symbol_name_in_different_repos_produces_distinct_snapshot_ids() {
        let mut registry = RepositoryRegistry::new();
        let repo_a = registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap());
        let repo_b = registry.register(RepositoryRoot::parse(&std::env::temp_dir()).unwrap());
        let a = test_snapshot(repo_a, "a", "src/lib.rs", "foo", "function_item", "fn foo() {}");
        let b = test_snapshot(repo_b, "a", "src/lib.rs", "foo", "function_item", "fn foo() {}");

        assert_ne!(a.id(), b.id());
    }

    #[test]
    fn lineage_edge_count_reflects_the_actual_number_of_inserted_lineage_edges() {
        let repo = test_repo();
        let prev = test_snapshot(repo, "a", "src/lib.rs", "foo", "function_item", "fn foo() {}");
        let next = test_snapshot(repo, "b", "src/lib.rs", "foo", "function_item", "fn foo() {}");
        let mut graph = CodeGraph::new();
        graph.insert_snapshot(prev.clone()).unwrap();
        graph.insert_snapshot(next.clone()).unwrap();
        assert_eq!(graph.lineage_edge_count(), 0, "an empty graph must report zero lineage edges, not a fixed constant");

        let status = codegraph_domain::LineageStatus::confirmed(resolved_evidence("exact match")).unwrap();
        graph
            .insert_lineage_edge(LineageEdge {
                from: prev.id(),
                to: next.id(),
                lineage: codegraph_domain::SymbolLineageId::from_genesis(prev.id()),
                status,
            })
            .unwrap();

        assert_eq!(graph.lineage_edge_count(), 1, "lineage_edge_count must reflect the one edge actually inserted");
    }

    #[test]
    fn all_snapshots_returns_every_inserted_snapshot_not_an_empty_placeholder() {
        let repo = test_repo();
        let a = test_snapshot(repo, "a", "src/lib.rs", "a", "function_item", "fn a() {}");
        let b = test_snapshot(repo, "a", "src/lib.rs", "b", "function_item", "fn b() {}");
        let mut graph = CodeGraph::new();
        graph.insert_snapshot(a.clone()).unwrap();
        graph.insert_snapshot(b.clone()).unwrap();

        let all = graph.all_snapshots();

        assert_eq!(all.len(), 2, "all_snapshots must return every inserted snapshot, not an empty vec");
        let ids: Vec<_> = all.iter().map(|s| s.id()).collect();
        assert!(ids.contains(&a.id()));
        assert!(ids.contains(&b.id()));
    }

    #[test]
    fn snapshots_in_repository_excludes_snapshots_from_a_different_repository() {
        let mut registry = RepositoryRegistry::new();
        let repo_a = registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap());
        let repo_b = registry.register(RepositoryRoot::parse(&std::env::temp_dir()).unwrap());
        let a = test_snapshot(repo_a, "a", "src/lib.rs", "foo", "function_item", "fn foo() {}");
        let b = test_snapshot(repo_b, "a", "src/lib.rs", "bar", "function_item", "fn bar() {}");
        let mut graph = CodeGraph::new();
        graph.insert_snapshot(a.clone()).unwrap();
        graph.insert_snapshot(b.clone()).unwrap();

        let in_repo_a = graph.snapshots_in_repository(repo_a);

        assert_eq!(in_repo_a.len(), 1, "a repository filter that matched everything (== flipped to !=) would wrongly include repo_b's snapshot too");
        assert_eq!(in_repo_a[0].id(), a.id());
    }

    #[test]
    fn all_edges_returns_every_inserted_typed_edge_not_an_empty_placeholder() {
        let repo = test_repo();
        let a = test_snapshot(repo, "a", "src/lib.rs", "a", "function_item", "fn a() {}");
        let b = test_snapshot(repo, "a", "src/lib.rs", "b", "function_item", "fn b() {}");
        let mut graph = CodeGraph::new();
        graph.insert_snapshot(a.clone()).unwrap();
        graph.insert_snapshot(b.clone()).unwrap();
        graph
            .insert_edge(Edge {
                from: a.id(),
                to: b.id(),
                kind: EdgeKind::Calls,
                evidence: resolved_evidence("test"),
            })
            .unwrap();

        let all = graph.all_edges();

        assert_eq!(all.len(), 1, "all_edges must return the actually inserted edge, not an empty vec");
        assert_eq!(all[0].from, a.id());
        assert_eq!(all[0].to, b.id());
    }

    #[test]
    fn outgoing_edges_to_the_same_target_are_ordered_by_edge_kind_rank_deterministically() {
        // Two edges from `a` to the SAME target `b`, with different
        // EdgeKinds: the (to bytes) key ties, so the order must be
        // decided by edge_kind_rank, not left to insertion/hash order.
        // Pins both `sort_edges_by_other_endpoint`'s actual sorting
        // effect and `edge_kind_rank`'s real, distinct per-variant
        // values (Calls=0 < Extends=3), so Calls must always come first.
        let repo = test_repo();
        let a = test_snapshot(repo, "a", "src/lib.rs", "a", "function_item", "fn a() {}");
        let b = test_snapshot(repo, "a", "src/lib.rs", "b", "function_item", "fn b() {}");
        let mut graph = CodeGraph::new();
        graph.insert_snapshot(a.clone()).unwrap();
        graph.insert_snapshot(b.clone()).unwrap();
        // Inserted in Extends-before-Calls order deliberately, so a
        // passing test cannot be explained by insertion order alone.
        graph
            .insert_edge(Edge {
                from: a.id(),
                to: b.id(),
                kind: EdgeKind::Extends,
                evidence: resolved_evidence("extends"),
            })
            .unwrap();
        graph
            .insert_edge(Edge {
                from: a.id(),
                to: b.id(),
                kind: EdgeKind::Calls,
                evidence: resolved_evidence("calls"),
            })
            .unwrap();

        let outgoing = graph.outgoing_edges(a.id(), None);

        assert_eq!(outgoing.len(), 2);
        assert_eq!(outgoing[0].kind, EdgeKind::Calls, "Calls (rank 0) must sort before Extends (rank 3) when the other endpoint ties");
        assert_eq!(outgoing[1].kind, EdgeKind::Extends);
    }

    #[test]
    fn outgoing_edges_to_different_targets_are_sorted_ascending_by_the_targets_id_bytes() {
        // Three targets, each with a distinct SymbolSnapshotId, inserted
        // in an order that does NOT match ascending id-byte order
        // (insertion/hash order must not be mistaken for the real
        // sort). Pins that `sort_edges_by_other_endpoint` actually
        // reorders by the OTHER endpoint when it differs across edges —
        // a no-op mutant would leave the edges in whatever arbitrary
        // order the internal HashSet iteration produced instead.
        let repo = test_repo();
        let a = test_snapshot(repo, "a", "src/lib.rs", "a", "function_item", "fn a() {}");
        let x = test_snapshot(repo, "a", "src/x.rs", "x", "function_item", "fn x() {}");
        let y = test_snapshot(repo, "a", "src/y.rs", "y", "function_item", "fn y() {}");
        let z = test_snapshot(repo, "a", "src/z.rs", "z", "function_item", "fn z() {}");
        let mut graph = CodeGraph::new();
        graph.insert_snapshot(a.clone()).unwrap();
        graph.insert_snapshot(x.clone()).unwrap();
        graph.insert_snapshot(y.clone()).unwrap();
        graph.insert_snapshot(z.clone()).unwrap();
        for target in [&z, &x, &y] {
            graph
                .insert_edge(Edge {
                    from: a.id(),
                    to: target.id(),
                    kind: EdgeKind::Calls,
                    evidence: resolved_evidence("test"),
                })
                .unwrap();
        }

        let outgoing = graph.outgoing_edges(a.id(), None);

        assert_eq!(outgoing.len(), 3);
        let mut expected_order: Vec<_> = [x.id(), y.id(), z.id()].to_vec();
        expected_order.sort_by_key(|id| *id.as_bytes());
        let actual_order: Vec<_> = outgoing.iter().map(|e| e.to).collect();
        assert_eq!(actual_order, expected_order, "outgoing edges to different targets must be sorted ascending by the target's id bytes, not left in arbitrary order");
    }

    #[test]
    fn edge_kind_rank_assigns_distinct_values_to_every_variant() {
        // Pins edge_kind_rank's actual per-variant mapping directly: a
        // mutant that collapses it to a constant (always 0 or always 1)
        // would make every rank equal, which this test catches even
        // though no sorting is involved.
        let kinds = [
            EdgeKind::Calls,
            EdgeKind::Imports,
            EdgeKind::Implements,
            EdgeKind::Extends,
            EdgeKind::References,
            EdgeKind::TestsFor,
            EdgeKind::RenamedTo,
        ];
        let ranks: Vec<u8> = kinds.iter().map(|k| edge_kind_rank(*k)).collect();
        let mut unique_ranks = ranks.clone();
        unique_ranks.sort_unstable();
        unique_ranks.dedup();
        assert_eq!(unique_ranks.len(), kinds.len(), "every EdgeKind variant must have its own distinct rank");
    }
}
