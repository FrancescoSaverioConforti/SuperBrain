---
id: code-roles-remove-role-raises-keyerror
schema_version: 1
type: code
status: verified
trust: internal
classification: internal
project: eval-b3
source: controllore@ec274f4680c19f5b335a01a7a563c8f64546e7e2:Access_control/src/roles.py:remove_role
created: 2026-09-01
updated: 2026-09-01
event_period: 2026-09-01
relations: {}
---
repository: controllore
commit: ec274f4680c19f5b335a01a7a563c8f64546e7e2
path: Access_control/src/roles.py
symbol: remove_role

def remove_role(role_name: str) -> None looks up the module-level _registry via _get_registry(), raises KeyError if role_name is absent, otherwise deletes that entry with `del registry[role_name]`.
