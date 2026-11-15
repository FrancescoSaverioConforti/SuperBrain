---
id: pattern-2026-09-11-timezone-fallback-reuse
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
resolve_configured_timezone() falls back to timezone.utc if settings are unavailable or invalid, reusing the same try/except pattern _check_programs() in watchdogs.py already used. It deliberately leaves _check_sun_events()'s slightly different variant untouched — no "UTC" fallback there, no try/except, an early return instead when lat/lon aren't configured.
