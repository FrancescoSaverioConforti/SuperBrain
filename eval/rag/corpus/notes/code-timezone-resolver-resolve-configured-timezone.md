---
id: code-timezone-resolver-resolve-configured-timezone
schema_version: 1
type: code
status: verified
trust: internal
classification: internal
project: eval-b3
source: controllore@ec274f4680c19f5b335a01a7a563c8f64546e7e2:app/core/timezone_resolver.py:resolve_configured_timezone
created: 2026-09-11
updated: 2026-09-11
event_period: 2026-09-11
relations: {}
---
repository: controllore
commit: ec274f4680c19f5b335a01a7a563c8f64546e7e2
path: app/core/timezone_resolver.py
symbol: resolve_configured_timezone

def resolve_configured_timezone() -> tzinfo

Reads location.timezone via load_settings() and returns zoneinfo.ZoneInfo(loc.timezone or "UTC"); returns timezone.utc on any exception, logged at debug level, never raised to the caller.
