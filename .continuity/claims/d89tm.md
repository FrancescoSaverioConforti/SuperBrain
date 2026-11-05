---
schema: 1
id: d89tm
type: decision
title: Task 8 (in-memory CodeGraph core) committed at 9031b53 on b2-code-graph
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-11-05T13:40:11.490Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Implemented crates/codegraph-index/src/graph/{mod,store,traversal,deep_learning_fixture}.rs. CodeGraph stores SymbolSnapshot/LineageEdge/typed Edge with HashMap-backed outgoing/incoming and lineage_from/lineage_to indexes. Invariants enforced pre-mutation: DuplicateSnapshot, DanglingEdgeFrom/To, DuplicateEdge (key is from/to/kind, NOT evidence — distinct EdgeKinds between same pair both accepted), CrossRepositoryEdge. reachable_from is a visited-set BFS, provably cycle-safe for any max_depth (caller-supplied, never internally enforced — no Task 13 budget anticipation). Every Vec-returning method sorts explicitly by SymbolSnapshotId/(from,to,kind) bytes — no raw hash-iteration order ever observable. Evidence/Confidence/LineageStatus stored and returned verbatim, never reconstructed. deep_learning_fixture.rs runs real Task 7 resolvers (not hand-built edges) against a PyTorch-shaped fixture (MyModel extends nn.Module, forward calls self.encoder/self.classifier, train's model.step() correctly produces no edge). Mutation testing: 2 rounds found 9 total gaps (count accessors, all_snapshots/all_edges emptiness, repository-filter ==, edge-kind rank/sort including a no-op-sort gap my first fix round missed), all closed; final sweep 51 mutants, 34 caught, 17 unviable, 0 missed. Independent review: APPROVE, zero critical/important findings, three suggestions explicitly deferred as future-task concerns (dangling-index debug_assert, Option-vs-Result asymmetry on unknown traversal start, O(n) repo filter at scale). Query primitives added: snapshot, all_snapshots, snapshots_in_repository, lineage_from/to, outgoing/incoming_edges (EdgeKind-filterable), all_edges, neighbors, callers_of/callees_of/tests_for/subtypes_of, reachable_from. No persistence/API/MCP/Task 9. Task 7's two known gaps (Rust macro-invisible calls, Imports rarely firing on real data) carried forward, not fixed.
