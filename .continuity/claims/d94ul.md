---
schema: 1
id: d94ul
type: decision
title: >-
  B2-ID-001 and B2-ID-002 found and fixed during real-repo smoke test
  (pytorch/examples); SymbolSnapshotId bumped to v2
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-11-05T17:03:06.482Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

B2-ID-001: SymbolSnapshotId v1 (repo,commit,path,kind,name,signature) collided for two distinct methods with identical name+body in different classes in the same file (RNNModel.init_weights vs TransformerModel.init_weights, pytorch/examples commit 2095514...). Fixed by adding ByteSpan (span_start,span_end as u64) to the hash, bumping domain separator to superbrain-symbol-snapshot-v2. derive()'s signature changed: now takes span_start/span_end too. QualifiedName alternative explicitly rejected — Task 4 has no real scope-chain resolver, don't fake one.

B2-ID-002: resolve_calls/resolve_imports/resolve_extends_implements pushed one Edge per syntactic occurrence (not per relation), so a caller calling the same target/ambiguous-name more than once produced literal duplicate (from,to,kind) triples — CodeGraph correctly rejected as DuplicateEdge, crashing the real pipeline (827 duplicate triples found). Fixed with per-resolver dedup keyed by (from,to) or (from,to,kind), deterministic evidence tie-break (StaticAnalysis > heuristic, then evidence-note comparison) in resolve_calls. resolve_tests_for audited, confirmed NOT vulnerable (structurally produces at most one edge per snapshot already), left unchanged. CodeGraph's own duplicate-edge invariant was NOT relaxed — dedup happens in the resolvers, which understand evidence semantics, never silently at insertion.

Both findings fully documented per [[d93lx]]'s policy at docs/research/findings/B2-ID-001.md and B2-ID-002.md, indexed in docs/research/findings.md. Real-repo reproduction confirmed both fixed: zero duplicate snapshot ids, zero duplicate edge triples, on the exact commit that originally failed. Smoke test itself (section B onward per the original checkpoint authorization) was paused mid-flight for these fixes and has not yet resumed — Task 10 remains NO-GO, and the smoke test resumption itself needs explicit GO after this fix's own review completes.
