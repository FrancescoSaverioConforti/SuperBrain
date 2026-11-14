---
id: code-search-provenance-withheld-for-sensitive
schema_version: 1
type: code
status: verified
trust: internal
classification: internal
project: eval-b3
source: crates/index/src/search.rs
created: 2026-10-30
updated: 2026-10-30
event_period: 2026-10-30
relations: {}
---
repository: superbrain
commit: see corpus-manifest.json build_commit
path: crates/index/src/search.rs
symbol: Provenance

The Provenance struct's source field is withheld on the search surface for Sensitive notes, including in its Debug output — repository_id and commit stay None because B1 note records carry no repository/commit provenance and the code never guesses it.
