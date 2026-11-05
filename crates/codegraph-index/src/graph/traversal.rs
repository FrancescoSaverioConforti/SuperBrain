//! Cycle-safe traversal primitives built on top of `CodeGraph`'s
//! indexed lookups (`store.rs`). These are the internal building blocks
//! a future `get_code_context`/`graph_query` (out of this task's scope)
//! will call; this task does not expose them through any API or MCP
//! surface, and does not add Task 13's final traversal budgets
//! (depth/node-count limits for a public query surface) — only the
//! structural guarantee that a cycle can never cause non-termination.

use std::collections::HashSet;

use codegraph_domain::{EdgeKind, SymbolSnapshotId};

use super::store::CodeGraph;
use crate::edges::Edge;

/// Which side of a typed edge to look from: `Outgoing` edges start at
/// the snapshot of interest; `Incoming` edges end at it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Outgoing,
    Incoming,
}

impl CodeGraph {
    /// Every typed edge touching `id` in `direction`, optionally
    /// filtered to one `EdgeKind`. A thin, direction-generic wrapper
    /// over [`CodeGraph::outgoing_edges`]/[`CodeGraph::incoming_edges`]
    /// for callers that pick the direction dynamically (e.g. a future
    /// `graph_query` answering "who calls this?" vs. "what does this
    /// call?" from the same code path).
    pub fn neighbors(&self, id: SymbolSnapshotId, direction: Direction, kind: Option<EdgeKind>) -> Vec<&Edge> {
        match direction {
            Direction::Outgoing => self.outgoing_edges(id, kind),
            Direction::Incoming => self.incoming_edges(id, kind),
        }
    }

    /// "Who calls this symbol?" — every snapshot id with an outgoing
    /// `Calls` edge to `id`, deduplicated and ordered deterministically.
    pub fn callers_of(&self, id: SymbolSnapshotId) -> Vec<SymbolSnapshotId> {
        edge_other_ids(self.incoming_edges(id, Some(EdgeKind::Calls)), Direction::Incoming)
    }

    /// "What does this method call?" — every snapshot id `id` has an
    /// outgoing `Calls` edge to, deduplicated and ordered
    /// deterministically.
    pub fn callees_of(&self, id: SymbolSnapshotId) -> Vec<SymbolSnapshotId> {
        edge_other_ids(self.outgoing_edges(id, Some(EdgeKind::Calls)), Direction::Outgoing)
    }

    /// "Which tests are linked to this symbol?" — every snapshot id with
    /// an outgoing `TestsFor` edge to `id` (i.e. every test that targets
    /// it), deduplicated and ordered deterministically.
    pub fn tests_for(&self, id: SymbolSnapshotId) -> Vec<SymbolSnapshotId> {
        edge_other_ids(self.incoming_edges(id, Some(EdgeKind::TestsFor)), Direction::Incoming)
    }

    /// "Which symbols extend or implement this type?" — every snapshot
    /// id with an outgoing `Extends` or `Implements` edge to `id`,
    /// deduplicated and ordered deterministically.
    pub fn subtypes_of(&self, id: SymbolSnapshotId) -> Vec<SymbolSnapshotId> {
        let mut ids = edge_other_ids(self.incoming_edges(id, Some(EdgeKind::Extends)), Direction::Incoming);
        ids.extend(edge_other_ids(self.incoming_edges(id, Some(EdgeKind::Implements)), Direction::Incoming));
        ids.sort_by_key(|snapshot_id| *snapshot_id.as_bytes());
        ids.dedup();
        ids
    }

    /// Every snapshot reachable from `start` by following OUTGOING
    /// typed edges, optionally restricted to one `EdgeKind`, up to
    /// `max_depth` hops (`0` returns just `start`, if `start` itself is
    /// a snapshot in this graph). Cycle-safe: a visited set ensures
    /// every snapshot is expanded at most once regardless of how many
    /// cycles the edge set contains, so `A calls B` + `B calls A`
    /// always terminates. Returns snapshot ids in BFS discovery order
    /// (deterministic: at each depth, candidates are visited in the
    /// same sorted order `outgoing_edges` already returns them in) —
    /// `start` itself is included first if it is a known snapshot.
    pub fn reachable_from(&self, start: SymbolSnapshotId, kind: Option<EdgeKind>, max_depth: usize) -> Vec<SymbolSnapshotId> {
        let mut visited: HashSet<SymbolSnapshotId> = HashSet::new();
        let mut order = Vec::new();

        if self.snapshot(start).is_none() {
            return order;
        }

        visited.insert(start);
        order.push(start);
        let mut frontier = vec![start];

        for _ in 0..max_depth {
            let mut next_frontier = Vec::new();
            for current in &frontier {
                for edge in self.outgoing_edges(*current, kind) {
                    if visited.insert(edge.to) {
                        order.push(edge.to);
                        next_frontier.push(edge.to);
                    }
                }
            }
            if next_frontier.is_empty() {
                break;
            }
            frontier = next_frontier;
        }

        order
    }
}

fn edge_other_ids(edges: Vec<&Edge>, direction: Direction) -> Vec<SymbolSnapshotId> {
    let mut ids: Vec<SymbolSnapshotId> = edges
        .into_iter()
        .map(|edge| match direction {
            Direction::Outgoing => edge.to,
            Direction::Incoming => edge.from,
        })
        .collect();
    ids.sort_by_key(|id| *id.as_bytes());
    ids.dedup();
    ids
}

#[cfg(test)]
mod tests {
    use codegraph_domain::{CommitHash, Evidence, FileIdentity, RepositoryId, RepositoryRegistry, RepositoryRoot, ResolutionMethod};

    use super::*;
    use crate::extract::{ByteSpan, RawSymbolCandidate};
    use crate::snapshot::{promote, SymbolSnapshot};

    fn test_repo() -> RepositoryId {
        let mut registry = RepositoryRegistry::new();
        registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap())
    }

    fn test_snapshot(repo: RepositoryId, path: &str, name: &str, kind_hint: &str, signature: &str) -> SymbolSnapshot {
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        let file = FileIdentity::parse(repo, commit, path).unwrap();
        let candidate = RawSymbolCandidate::new(name.to_string(), kind_hint.to_string(), ByteSpan { start: 0, end: 10 }, signature.to_string()).unwrap();
        promote(&candidate, file).unwrap()
    }

    fn resolved_evidence(note: &str) -> Evidence {
        Evidence::new(ResolutionMethod::StaticAnalysis, None, note).unwrap()
    }

    fn edge(from: SymbolSnapshotId, to: SymbolSnapshotId, kind: EdgeKind) -> Edge {
        Edge {
            from,
            to,
            kind,
            evidence: resolved_evidence("test"),
        }
    }

    #[test]
    fn a_two_cycle_terminates_and_visits_each_node_once() {
        let repo = test_repo();
        let a = test_snapshot(repo, "src/lib.rs", "a", "function_item", "fn a() { b(); }");
        let b = test_snapshot(repo, "src/lib.rs", "b", "function_item", "fn b() { a(); }");
        let mut graph = CodeGraph::new();
        graph.insert_snapshot(a.clone()).unwrap();
        graph.insert_snapshot(b.clone()).unwrap();
        graph.insert_edge(edge(a.id(), b.id(), EdgeKind::Calls)).unwrap();
        graph.insert_edge(edge(b.id(), a.id(), EdgeKind::Calls)).unwrap();

        let reached = graph.reachable_from(a.id(), Some(EdgeKind::Calls), 10_000);

        assert_eq!(reached.len(), 2, "a two-node cycle must visit each node exactly once, not loop forever");
        assert!(reached.contains(&a.id()));
        assert!(reached.contains(&b.id()));
    }

    #[test]
    fn a_self_loop_terminates() {
        let repo = test_repo();
        let a = test_snapshot(repo, "src/lib.rs", "a", "function_item", "fn a() { a(); }");
        let mut graph = CodeGraph::new();
        graph.insert_snapshot(a.clone()).unwrap();
        graph.insert_edge(edge(a.id(), a.id(), EdgeKind::Calls)).unwrap();

        let reached = graph.reachable_from(a.id(), Some(EdgeKind::Calls), 10_000);

        assert_eq!(reached, vec![a.id()]);
    }

    #[test]
    fn reachable_from_an_unknown_start_is_empty() {
        let repo = test_repo();
        let a = test_snapshot(repo, "src/lib.rs", "a", "function_item", "fn a() {}");
        let graph = CodeGraph::new();
        let unknown_id = a.id();

        assert!(graph.reachable_from(unknown_id, None, 10).is_empty());
    }

    #[test]
    fn max_depth_zero_returns_only_the_start_node() {
        let repo = test_repo();
        let a = test_snapshot(repo, "src/lib.rs", "a", "function_item", "fn a() { b(); }");
        let b = test_snapshot(repo, "src/lib.rs", "b", "function_item", "fn b() {}");
        let mut graph = CodeGraph::new();
        graph.insert_snapshot(a.clone()).unwrap();
        graph.insert_snapshot(b.clone()).unwrap();
        graph.insert_edge(edge(a.id(), b.id(), EdgeKind::Calls)).unwrap();

        assert_eq!(graph.reachable_from(a.id(), None, 0), vec![a.id()]);
    }

    #[test]
    fn callers_of_and_callees_of_are_inverse_views_of_the_same_calls_edges() {
        let repo = test_repo();
        let caller = test_snapshot(repo, "src/lib.rs", "caller", "function_item", "fn caller() { callee(); }");
        let callee = test_snapshot(repo, "src/lib.rs", "callee", "function_item", "fn callee() {}");
        let mut graph = CodeGraph::new();
        graph.insert_snapshot(caller.clone()).unwrap();
        graph.insert_snapshot(callee.clone()).unwrap();
        graph.insert_edge(edge(caller.id(), callee.id(), EdgeKind::Calls)).unwrap();

        assert_eq!(graph.callers_of(callee.id()), vec![caller.id()]);
        assert_eq!(graph.callees_of(caller.id()), vec![callee.id()]);
        assert!(graph.callers_of(caller.id()).is_empty());
        assert!(graph.callees_of(callee.id()).is_empty());
    }

    #[test]
    fn tests_for_returns_the_test_snapshots_targeting_a_symbol() {
        let repo = test_repo();
        let target = test_snapshot(repo, "src/lib.rs", "add", "function_item", "fn add(a: i32, b: i32) -> i32 { a + b }");
        let test_fn = test_snapshot(repo, "src/tests.rs", "test_add", "function_item", "fn test_add() { add(1, 2); }");
        let mut graph = CodeGraph::new();
        graph.insert_snapshot(target.clone()).unwrap();
        graph.insert_snapshot(test_fn.clone()).unwrap();
        graph.insert_edge(edge(test_fn.id(), target.id(), EdgeKind::TestsFor)).unwrap();

        assert_eq!(graph.tests_for(target.id()), vec![test_fn.id()]);
    }

    #[test]
    fn subtypes_of_merges_extends_and_implements_without_duplicates() {
        let repo = test_repo();
        let base = test_snapshot(repo, "src/base.py", "Module", "class_definition", "class Module:\n    pass");
        let model = test_snapshot(repo, "src/model.py", "MyModel", "class_definition", "class MyModel(Module):\n    pass");
        let mut graph = CodeGraph::new();
        graph.insert_snapshot(base.clone()).unwrap();
        graph.insert_snapshot(model.clone()).unwrap();
        graph.insert_edge(edge(model.id(), base.id(), EdgeKind::Extends)).unwrap();

        assert_eq!(graph.subtypes_of(base.id()), vec![model.id()]);
    }

    #[test]
    fn neighbors_matches_outgoing_and_incoming_edges_by_direction() {
        let repo = test_repo();
        let a = test_snapshot(repo, "src/lib.rs", "a", "function_item", "fn a() { b(); }");
        let b = test_snapshot(repo, "src/lib.rs", "b", "function_item", "fn b() {}");
        let mut graph = CodeGraph::new();
        graph.insert_snapshot(a.clone()).unwrap();
        graph.insert_snapshot(b.clone()).unwrap();
        graph.insert_edge(edge(a.id(), b.id(), EdgeKind::Calls)).unwrap();

        assert_eq!(graph.neighbors(a.id(), Direction::Outgoing, None).len(), 1);
        assert_eq!(graph.neighbors(b.id(), Direction::Incoming, None).len(), 1);
        assert_eq!(graph.neighbors(a.id(), Direction::Incoming, None).len(), 0);
    }

    #[test]
    fn a_longer_chain_is_fully_reachable_within_sufficient_depth() {
        let repo = test_repo();
        let a = test_snapshot(repo, "src/lib.rs", "a", "function_item", "fn a() { b(); }");
        let b = test_snapshot(repo, "src/lib.rs", "b", "function_item", "fn b() { c(); }");
        let c = test_snapshot(repo, "src/lib.rs", "c", "function_item", "fn c() {}");
        let mut graph = CodeGraph::new();
        graph.insert_snapshot(a.clone()).unwrap();
        graph.insert_snapshot(b.clone()).unwrap();
        graph.insert_snapshot(c.clone()).unwrap();
        graph.insert_edge(edge(a.id(), b.id(), EdgeKind::Calls)).unwrap();
        graph.insert_edge(edge(b.id(), c.id(), EdgeKind::Calls)).unwrap();

        let reached = graph.reachable_from(a.id(), Some(EdgeKind::Calls), 10);

        assert_eq!(reached, vec![a.id(), b.id(), c.id()]);
    }

    #[test]
    fn depth_limit_stops_traversal_before_reaching_the_far_end_of_a_chain() {
        let repo = test_repo();
        let a = test_snapshot(repo, "src/lib.rs", "a", "function_item", "fn a() { b(); }");
        let b = test_snapshot(repo, "src/lib.rs", "b", "function_item", "fn b() { c(); }");
        let c = test_snapshot(repo, "src/lib.rs", "c", "function_item", "fn c() {}");
        let mut graph = CodeGraph::new();
        graph.insert_snapshot(a.clone()).unwrap();
        graph.insert_snapshot(b.clone()).unwrap();
        graph.insert_snapshot(c.clone()).unwrap();
        graph.insert_edge(edge(a.id(), b.id(), EdgeKind::Calls)).unwrap();
        graph.insert_edge(edge(b.id(), c.id(), EdgeKind::Calls)).unwrap();

        let reached = graph.reachable_from(a.id(), Some(EdgeKind::Calls), 1);

        assert_eq!(reached, vec![a.id(), b.id()], "depth 1 must reach b but not c");
    }
}
