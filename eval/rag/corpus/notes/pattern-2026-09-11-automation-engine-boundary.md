---
id: pattern-2026-09-11-automation-engine-boundary
schema_version: 1
type: pattern
status: verified
trust: internal
classification: internal
project: eval-b3
source: controllore@ec274f4680c19f5b335a01a7a563c8f64546e7e2:app/core/timezone_resolver.py
created: 2026-09-11
updated: 2026-09-11
event_period: 2026-09-11
relations: {}
---
During the "Consolidamento timezone Scheduler/SystemContext" fix on 2026-09-11, it was verified across the whole controllore repo that no module under Automazione/engine/ imports from app/ — this boundary had never been violated. resolve_configured_timezone() was extracted into app/core/timezone_resolver.py specifically to be the one point that builds the tz_provider injected into AutomationEngine/Scheduler, which stay unaware of app/routers/settings_models.
