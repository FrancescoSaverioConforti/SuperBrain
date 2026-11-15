---
id: code-schedule-lock-guardrail-evaluate
schema_version: 1
type: code
status: verified
trust: internal
classification: internal
project: eval-b3
source: controllore@ec274f4680c19f5b335a01a7a563c8f64546e7e2:app/core/schedule_lock_guardrail.py:evaluate_schedule_lock_guardrail
created: 2026-09-20
updated: 2026-09-20
event_period: 2026-09-20
relations: {}
---
repository: controllore
commit: ec274f4680c19f5b335a01a7a563c8f64546e7e2
path: app/core/schedule_lock_guardrail.py
symbol: evaluate_schedule_lock_guardrail

Function at line 185 evaluates whether a schedule change is blocked by a lock guardrail, walking condition trees via the internal _walk helper (line 102) and the _LockEvent NamedTuple (line 96) defined earlier in the same module.
