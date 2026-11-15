---
id: code-schedule-lock-guardrail-extract-device-ids
schema_version: 1
type: code
status: verified
trust: internal
classification: internal
project: eval-b3
source: controllore@ec274f4680c19f5b335a01a7a563c8f64546e7e2:app/core/schedule_lock_guardrail.py:extract_trigger_condition_device_ids
created: 2026-09-20
updated: 2026-09-20
event_period: 2026-09-20
relations: {}
---
repository: controllore
commit: ec274f4680c19f5b335a01a7a563c8f64546e7e2
path: app/core/schedule_lock_guardrail.py
symbol: extract_trigger_condition_device_ids, extract_conditional_action_device_ids

extract_trigger_condition_device_ids (line 59) extracts device ids referenced by a trigger's condition group, built on _device_ids_from_condition_group (line 45). extract_conditional_action_device_ids (line 78) does the equivalent for a conditional action. Both feed device ids into evaluate_schedule_lock_guardrail in the same module.
