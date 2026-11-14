---
id: code-automation-rbac-collect-device-targets
schema_version: 1
type: code
status: verified
trust: internal
classification: internal
project: eval-b3
source: controllore@ec274f4680c19f5b335a01a7a563c8f64546e7e2:app/core/automation_rbac.py:collect_device_targets
created: 2026-09-05
updated: 2026-09-05
event_period: 2026-09-05
relations: {}
---
repository: controllore
commit: ec274f4680c19f5b335a01a7a563c8f64546e7e2
path: app/core/automation_rbac.py
symbol: collect_device_targets

Function at line 112 of automation_rbac.py collects the device targets implicated by an automation rule; its output feeds the RBAC check performed by actions_touch_device in the same module.
