---
id: pattern-2026-09-sun-times-never-raises
schema_version: 1
type: pattern
status: verified
trust: internal
classification: internal
project: eval-b3
source: controllore@ec274f4680c19f5b335a01a7a563c8f64546e7e2:app/core/sun_times.py
created: 2026-09-10
updated: 2026-09-10
event_period: 2026-09-10
relations: {}
---
get_sun_times_today() never propagates an exception to its caller: it returns None if the astral library is not installed, if latitude/longitude are not configured, or if the sun-time computation fails for any other reason — the same never-raise principle already used by both of its historical callers before this consolidation.
