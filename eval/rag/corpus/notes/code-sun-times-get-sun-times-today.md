---
id: code-sun-times-get-sun-times-today
schema_version: 1
type: code
status: verified
trust: internal
classification: internal
project: eval-b3
source: controllore@ec274f4680c19f5b335a01a7a563c8f64546e7e2:app/core/sun_times.py:get_sun_times_today
created: 2026-09-10
updated: 2026-09-10
event_period: 2026-09-10
relations: {}
---
repository: controllore
commit: ec274f4680c19f5b335a01a7a563c8f64546e7e2
path: app/core/sun_times.py
symbol: get_sun_times_today

def get_sun_times_today(latitude: Optional[float], longitude: Optional[float], tz: tzinfo, today: Optional[_date] = None) -> Optional[dict]

Returns {"sunrise": datetime, "sunset": datetime}, aware, in the given tz, for today at the given location — or None on any failure. The `today` parameter exists only so tests can inject a fixed date; production callers never pass it.
