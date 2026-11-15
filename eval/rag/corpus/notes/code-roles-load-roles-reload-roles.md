---
id: code-roles-load-roles-reload-roles
schema_version: 1
type: code
status: verified
trust: internal
classification: internal
project: eval-b3
source: controllore@ec274f4680c19f5b335a01a7a563c8f64546e7e2:Access_control/src/roles.py:load_roles
created: 2026-09-01
updated: 2026-09-01
event_period: 2026-09-01
relations: {}
---
repository: controllore
commit: ec274f4680c19f5b335a01a7a563c8f64546e7e2
path: Access_control/src/roles.py
symbol: load_roles, reload_roles

def load_roles(path: Path = CONFIG_PATH) -> RoleRegistry parses roles.json via RoleRegistry.model_validate. def reload_roles(path) -> None rebuilds the module-level cache from a fresh parse, used to pick up config changes without restarting the process.
