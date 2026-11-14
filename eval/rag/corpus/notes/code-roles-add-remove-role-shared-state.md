---
id: code-roles-add-remove-role-shared-state
schema_version: 1
type: code
status: verified
trust: internal
classification: internal
project: eval-b3
source: controllore@ec274f4680c19f5b335a01a7a563c8f64546e7e2:Access_control/src/roles.py:add_role
created: 2026-09-01
updated: 2026-09-01
event_period: 2026-09-01
relations: {}
---
repository: controllore
commit: ec274f4680c19f5b335a01a7a563c8f64546e7e2
path: Access_control/src/roles.py
symbol: add_role, remove_role

add_role(role_name, description, permissions) raises ValueError if the role already exists in the module-level _registry dict; remove_role(role_name) raises KeyError if it does not exist. Both mutate the same dict returned by _get_registry(), the shared state also read by role_has_permission and list_roles.
