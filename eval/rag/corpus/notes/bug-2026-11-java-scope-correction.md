---
id: bug-2026-11-java-scope-correction
schema_version: 1
type: bug
status: reviewed
trust: internal
classification: internal
project: eval-b3
source: continuity:d170ij
created: 2026-11-05
updated: 2026-11-05
event_period: 2026-11-05
relations: {}
---
During B2, a correction was made: the Java language-support gap is NOT the same issue as B2-ID-012, which stays assigned to a pre-existing resolve_lineage O(n^2) performance finding. Java was re-logged as a documented future-language-support gap, not a defect in the shipped code graph.
