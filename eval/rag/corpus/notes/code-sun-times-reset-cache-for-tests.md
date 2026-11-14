---
id: code-sun-times-reset-cache-for-tests
schema_version: 1
type: code
status: verified
trust: internal
classification: internal
project: eval-b3
source: controllore@ec274f4680c19f5b335a01a7a563c8f64546e7e2:app/core/sun_times.py:_reset_cache_for_tests
created: 2026-09-10
updated: 2026-09-10
event_period: 2026-09-10
relations: {}
---
repository: controllore
commit: ec274f4680c19f5b335a01a7a563c8f64546e7e2
path: app/core/sun_times.py
symbol: _reset_cache_for_tests

def _reset_cache_for_tests() -> None clears the module-level _cache_key/_cache_value globals used to memoize get_sun_times_today; any test that mocks the sun calculation must call this first, per the function's own docstring pointing at tests/unit/test_sun_times_consolidation.py.
