---
id: code-roles-role-has-permission
schema_version: 1
type: code
status: verified
trust: internal
classification: internal
project: eval-b3
source: controllore@ec274f4680c19f5b335a01a7a563c8f64546e7e2:Access_control/src/roles.py:role_has_permission
created: 2026-09-01
updated: 2026-09-01
event_period: 2026-09-01
relations: {}
---
repository: controllore
commit: ec274f4680c19f5b335a01a7a563c8f64546e7e2
path: Access_control/src/roles.py
symbol: role_has_permission

def role_has_permission(role_name: str, resource_type: str, action: str) -> bool

Looks up the role via get_role(); returns False if the role does not exist (KeyError caught), otherwise checks whether `action` is in role.permissions.get(resource_type, []).
