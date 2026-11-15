---
id: code-roles-config-path-constant
schema_version: 1
type: code
status: verified
trust: internal
classification: internal
project: eval-b3
source: controllore@ec274f4680c19f5b335a01a7a563c8f64546e7e2:Access_control/src/roles.py
created: 2026-09-01
updated: 2026-09-01
event_period: 2026-09-01
relations: {}
---
repository: controllore
commit: ec274f4680c19f5b335a01a7a563c8f64546e7e2
path: Access_control/src/roles.py
symbol: CONFIG_PATH

CONFIG_PATH = Path(__file__).parent / "config" / "roles.json" is the default path every role function reads from unless a caller passes an explicit path override, used as load_roles's and reload_roles's default argument.
