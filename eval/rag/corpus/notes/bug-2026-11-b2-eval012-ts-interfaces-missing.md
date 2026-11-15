---
id: bug-2026-11-b2-eval012-ts-interfaces-missing
schema_version: 1
type: bug
status: reviewed
trust: internal
classification: internal
project: eval-b3
source: continuity:d183dc
created: 2026-11-07
updated: 2026-11-07
event_period: 2026-11-07
relations: {}
---
B2-EVAL-012 (TypeScript backend, evaluated against domain-driven-hexagon@5c2d15a) was paused on a new finding: TypeScript interface_declaration nodes were never extracted by the parser — zero of 18 real interfaces in that codebase were modeled, and zero Implements edges were created to any local interface.
