# B1 Task 7 - local read-only API

2026-10-30. Baseline 0ce55434d60d21eeadcba6f5425de1b56511e75c, clean.

Implemented the api crate with search/retrieve functions and versioned request
contexts/response envelopes. Correlation UUID is retained in successes and errors.
Every call checks a policy::ReadPolicy operation-specific grant; default denies.
This policy is supplied by the trusted local host, not note/model content, and is
not authentication. No transport or new public storage write path was added.

Search takes text/filters/k and preserves Index's results/context/NotPresent and
Sensitive redaction. Retrieve takes a typed note/chunk target and returns content
byte-identical to Vault::read_note, plus current version/status/provenance and
metadata schema. Chunk ids yield UnsupportedChunk (B2 seam). Sensitive retrieve
preserves the pointer record's full frontmatter/source/relations, as allowed by
the handoff; it never accesses PiiStore plaintext. Storage failures are mapped to
sanitized typed errors; no content/path/raw storage error is echoed.

Eight production-path tests were RED on unimplemented functions, then GREEN.
Coverage includes bytes/version/provenance/status, Sensitive pointer-only content,
actual citation vs topical nonmatch, wrapper redaction/absence, independent
permission checks/default-deny, schema/chunk/missing errors, deletion/corruption,
operational failure and correlation. Independent task7_review verdict GO, no
Critical/Important: personally reran 8/8 API tests and workspace Clippy clean,
and compared every API-table/common-contract row (details below).

| Spec contract | Implementation |
| --- | --- |
| search text, filters, k | SearchRequest fields; direct Index delegation |
| id, excerpt, score, provenance | Existing SearchResponse preserved |
| retrieve note/chunk | RetrieveTarget; chunk explicitly unsupported |
| content/version/provenance/status | One current authoritative read |
| policy every call | check before read, operation-specific default deny |
| versioned schema, correlation | RequestContext and every ApiResponse |
| typed errors, no-secret logs | ApiError, no logging or raw error propagation |
| search time/project/score meaning | Index context preserved; crate docs define score |
| timeout/limits/retry/idempotence | Honest limits documented in api/lib.rs |

Limits: synchronous calls have no deadline, retry scheduler, rate/size limit,
network identity or cache. k limits returned search results only. Read-only retries
may observe newer commits. No cross-note snapshot. Repository/commit provenance
and persisted gate Pending/Resolved remain unavailable (None/NotRevalidated).

Validation: cargo test -p api 8/8, cargo clippy --workspace -- -D warnings clean,
cargo deny check all four categories OK (existing warning categories). Full
workspace initially hit the known pre-B1 concurrent_restore_and_deletion_through_
independent_handles_never_expose_the_deleted_note flake (ReferenceCorrupt).
The isolated rerun passed; complete rerun result recorded below. Storage and
restore/recovery were not modified. Cargo.lock adds only local api; no external
package/version/license change. No MVP/plan changes, merge/tag/push.

Complete rerun with cargo test --workspace --quiet -- --test-threads=1: 487 passed,
0 failed, 0 ignored. Default parallel scheduling had two separate failed runs,
first the concurrent_restore_and_deletion test above, then f1_concurrent_restore_
and_deletion_never_expose_a_deleted_non_sensitive_note (also ReferenceCorrupt).
Both are named in the handoff's pre-existing flake record; no test was changed.
Serial scheduling does not remove the explicit concurrency inside those tests.
