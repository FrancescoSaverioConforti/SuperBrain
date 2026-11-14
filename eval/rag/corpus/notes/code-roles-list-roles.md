---
id: code-roles-list-roles
schema_version: 1
type: code
status: verified
trust: internal
classification: internal
project: eval-b3
source: controllore@ec274f4680c19f5b335a01a7a563c8f64546e7e2:Access_control/src/roles.py:list_roles
created: 2026-09-01
updated: 2026-09-01
event_period: 2026-09-01
relations: {}
---
repository: controllore
commit: ec274f4680c19f5b335a01a7a563c8f64546e7e2
path: Access_control/src/roles.py
symbol: list_roles, get_role

def list_roles() -> list[str] returns list(_get_registry().keys()). def get_role(role_name: str) -> RoleModel raises KeyError with a message in Italian ("Ruolo '{role_name}' non trovato.") if the role is missing from the registry.
