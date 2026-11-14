//! B2 Task 8: a representative deep-learning-repository fixture,
//! exercised end-to-end through the REAL Task 7 edge resolvers (not
//! hand-built `Edge` values) and then stored in a real `CodeGraph`.
//! This is not a new feature — it is a structural check that nothing
//! about `CodeGraph`'s shape (snapshot storage, typed-edge storage,
//! traversal) assumes anything PyTorch-specific or even
//! class/function-specific; the graph core is adequate for this shape
//! with today's minimal `SymbolKind` baseline.
//!
//! Fixture, matching a real PyTorch module shape:
//!
//! ```python
//! class Module:
//!     pass
//!
//! class MyModel(nn.Module):
//!     def forward(self, x):
//!         self.encoder(x)
//!         self.classifier(x)
//!
//! def encoder(x):
//!     pass
//!
//! def classifier(x):
//!     pass
//!
//! def train(model):
//!     model.step()
//! ```
//!
//! `nn.Module` is represented by a `Module` class snapshot (this
//! pipeline resolves a dotted base like `nn.Module` by its LAST
//! segment — see `extends_implements.rs`'s
//! `python_dotted_base_resolves_by_its_last_segment` test — so a
//! snapshot literally named `Module` is what a real `nn.Module` base
//! resolves against). `encoder`/`classifier` are plain top-level
//! functions, standing in for what would, in a future task, likely be
//! `Field`/`Attribute` snapshots for `self.encoder`/`self.classifier`
//! assigned in `__init__` — NOT added here, per this task's explicit
//! instruction not to anticipate that roadmap. `forward`'s method-call-
//! by-final-segment resolution (`self.encoder(x)` -> `"encoder"`) is
//! exactly the mechanism `calls.rs` already implements for Task 7; this
//! fixture exercises it against a case shaped like the real deep-
//! learning scenario, not just the unit test's minimal one.
//!
//! `train` calling `model.step()` stands in for "a training function
//! calling the model" — `step` is deliberately NOT given its own
//! snapshot here, so this also documents what happens when the DL
//! fixture's target is NOT in the known pool: no edge, per "no edge
//! beats a false edge," not a crash or a placeholder node.

#[cfg(test)]
mod tests {
    use codegraph_domain::{CommitHash, EdgeKind, FileIdentity, RepositoryRegistry, RepositoryRoot};

    use super::super::store::CodeGraph;
    use crate::edges::{resolve_calls, resolve_extends_implements, resolve_tests_for};
    use crate::extract::{ByteSpan, RawSymbolCandidate};
    use crate::snapshot::{promote, SymbolSnapshot};

    fn test_snapshot(repo: codegraph_domain::RepositoryId, path: &str, name: &str, kind_hint: &str, signature: &str) -> SymbolSnapshot {
        let commit = CommitHash::parse(&"a".repeat(40)).unwrap();
        let file = FileIdentity::parse(repo, commit, path).unwrap();
        let candidate = RawSymbolCandidate::new(name.to_string(), kind_hint.to_string(), ByteSpan { start: 0, end: 10 }, signature.to_string()).unwrap();
        promote(&candidate, file).unwrap()
    }

    #[test]
    fn a_pytorch_shaped_repository_round_trips_through_resolvers_and_the_graph() {
        let mut registry = RepositoryRegistry::new();
        let repo = registry.register(RepositoryRoot::parse(&std::env::current_dir().unwrap()).unwrap());

        let module_base = test_snapshot(repo, "src/nn.py", "Module", "class_definition", "class Module:\n    pass");
        let my_model = test_snapshot(
            repo,
            "src/model.py",
            "MyModel",
            "class_definition",
            "class MyModel(nn.Module):\n    def forward(self, x):\n        self.encoder(x)\n        self.classifier(x)",
        );
        let forward = test_snapshot(
            repo,
            "src/model.py",
            "forward",
            "function_definition",
            "def forward(self, x):\n    self.encoder(x)\n    self.classifier(x)",
        );
        let encoder = test_snapshot(repo, "src/model.py", "encoder", "function_definition", "def encoder(x):\n    pass");
        let classifier = test_snapshot(repo, "src/model.py", "classifier", "function_definition", "def classifier(x):\n    pass");
        let train = test_snapshot(repo, "src/train.py", "train", "function_definition", "def train(model):\n    model.step()");
        let test_forward = test_snapshot(
            repo,
            "src/tests.py",
            "test_forward",
            "function_definition",
            "def test_forward():\n    forward(None, None)",
        );

        let snapshots = vec![
            module_base.clone(),
            my_model.clone(),
            forward.clone(),
            encoder.clone(),
            classifier.clone(),
            train.clone(),
            test_forward.clone(),
        ];

        let extends_edges = resolve_extends_implements(&snapshots).unwrap();
        let calls_edges = resolve_calls(&snapshots).unwrap();
        let tests_for_edges = resolve_tests_for(&snapshots).unwrap();

        let mut graph = CodeGraph::new();
        for snapshot in &snapshots {
            graph.insert_snapshot(snapshot.clone()).unwrap();
        }
        for edge in extends_edges {
            graph.insert_edge(edge).unwrap();
        }
        for edge in calls_edges {
            graph.insert_edge(edge).unwrap();
        }
        for edge in tests_for_edges {
            graph.insert_edge(edge).unwrap();
        }

        // MyModel extends (nn.)Module, resolved by last-segment dotted
        // base matching, StaticAnalysis (syntax-certain for Python).
        let extends = graph.outgoing_edges(my_model.id(), Some(EdgeKind::Extends));
        assert_eq!(extends.len(), 1);
        assert_eq!(extends[0].to, module_base.id());
        assert_eq!(extends[0].evidence.method(), codegraph_domain::ResolutionMethod::StaticAnalysis);

        // forward calls encoder and classifier via self.X(...) method-call
        // resolution.
        let callees = graph.callees_of(forward.id());
        assert!(callees.contains(&encoder.id()), "forward must call encoder");
        assert!(callees.contains(&classifier.id()), "forward must call classifier");

        // train calls model.step() — "model" is a parameter name, not a
        // snapshot, and "step" has no snapshot in this pool at all, so
        // this must produce NO edge, not a guess.
        assert!(graph.callees_of(train.id()).is_empty(), "an unresolvable call must never fabricate an edge");

        // test_forward is linked to forward via TestsFor (naming
        // convention + an actual call), never StaticAnalysis.
        let test_edges = graph.incoming_edges(forward.id(), Some(EdgeKind::TestsFor));
        assert_eq!(test_edges.len(), 1);
        assert_eq!(test_edges[0].from, test_forward.id());
        assert_ne!(test_edges[0].evidence.method(), codegraph_domain::ResolutionMethod::StaticAnalysis);

        // The whole fixture is reachable from MyModel by following any
        // typed edge, and termination is guaranteed structurally even
        // though this fixture also happens to be acyclic.
        let reachable = graph.reachable_from(my_model.id(), None, 10_000);
        assert!(reachable.contains(&module_base.id()));
    }
}
