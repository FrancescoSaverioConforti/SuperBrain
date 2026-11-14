---
id: code-api-lib-b1-callable-core
schema_version: 1
type: code
status: verified
trust: internal
classification: internal
project: eval-b3
source: crates/api/src/lib.rs
created: 2026-10-30
updated: 2026-10-30
event_period: 2026-10-30
relations: {}
---
repository: superbrain
commit: e92dc781bcd32fcca5445295221f36aea2fe791e
path: crates/api/src/lib.rs
symbol: module doc

B1's callable core is synchronous, with no timeout, retry scheduler, size/rate cap, or remote identity yet. Chunk ids are explicitly rejected until B2. No backing data is ever fabricated when a field is unavailable.
