---
schema: 1
id: d95ey
type: decision
title: >-
  B2-ID-001 and B2-ID-002 fixes committed at 47abdf3 on b2-code-graph;
  independent review APPROVE for both
status: accepted
confidence: confirmed
provenance:
  origin: auto
  created: '2026-11-05T17:22:54.897Z'
supersedes: []
superseded_by: null
depends_on: []
tags: []
---

Supersedes/completes [[d94ul]]. Both findings fixed, documented at docs/research/findings/{B2-ID-001,B2-ID-002}.md, indexed at docs/research/findings.md. Mutation testing: snapshot_id.rs 6/6 (0 missed); calls.rs+imports.rs+extends_implements.rs 96/96 after closing an 11-gap first pass on prefer_edge tie-break (added 6 white-box unit tests). Independent review (agent-protocols:code-reviewer) APPROVE for both combined — traced hash encoding and dedup logic by hand, confirmed all 30 real derive() call sites updated, confirmed no overreach (CodeGraph invariants/Edge shape/SymbolKind variants all untouched), found one doc-freshness nit (stale test counts) which was fixed before commit. Full workspace tests green, clippy/deny clean. Real-repo reproduction against the exact originally-failing pytorch/examples commit confirmed both bugs gone.

SymbolSnapshotId::derive signature is now (file, kind, name, signature, span_start: u64, span_end: u64) — 6 params, v2 domain separator. Any future code constructing a SymbolSnapshotId must use this signature.

Smoke test harness (crates/codegraph-index/examples/b2_smoke_test.rs) committed as throwaway investigative tooling, not a permanent feature — reusable for future real-repo validation passes (Rust/TS/JS/C#/C++ per [[d92ou]]).

Smoke test itself (the original B2 checkpoint authorization: section B onward — pipeline results, symbol coverage, edge quality, DL usefulness analysis, incremental A->B on real commits, performance, final report at docs/eval/b2-real-repo-smoke-python-dl.md) has NOT resumed yet — only the blocking bugs were fixed so far. Needs explicit user GO to resume. Task 10 remains NO-GO.
