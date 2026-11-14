---
id: decision-2026-10-26-c2-approval-channel
schema_version: 1
type: decision
status: verified
trust: internal
classification: internal
project: eval-b3
source: docs/Spec_mvp.md#C2
created: 2026-10-26
updated: 2026-10-26
event_period: 2026-10-26
relations: {}
---
In the MVP, the only trusted human-approval channel is a local CLI, `bc approve <proposal_id>`, which shows the diff before confirming. Native MCP client confirmation inside VS Code is allowed only after it is verified that the model cannot generate or forge that confirmation.
