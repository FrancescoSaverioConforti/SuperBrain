---
id: decision-2026-10-26-faseb-deterministic-baseline
schema_version: 1
type: decision
status: verified
trust: internal
classification: internal
project: eval-b3
source: docs/Spec_mvp.md#Fase-B
created: 2026-10-26
updated: 2026-10-26
event_period: 2026-10-26
relations: {}
---
Fase B must pass using only the deterministic retrieval baseline, trigrams plus BM25. CPU embeddings are introduced only if they measurably improve the benchmark without pushing latency or RAM past the stated limits.
