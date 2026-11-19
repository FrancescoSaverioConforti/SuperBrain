# B5 — Brain API + MCP Integration: Closure

**Branch:** `b5-brain-api-mcp`, created from `main` at `b58820b`.
**HEAD at closure:** `c4f2551`. **Date:** 2026-11-19.
**Status: FEATURE-COMPLETE (Tasks 1-37 done). CONDITIONALLY READY for
merge. NOT READY for unsupervised autonomous production use. Not
merged into `main`.**

---

## 1. Architecture implemented

Per the design doc (`docs/superpowers/specs/2026-11-15-b5-brain-api-mcp-design.md`),
B5 adds a new orchestration layer between SuperBrain's existing
read-only core and two new transport crates:

```
MCP server (crates/mcp-server, new)
   |
   v
crates/brain-api (new) -- the Brain API's public contract
   |-- delegates reads to crates/api as-is
   |-- proposal creation (save_memory/update_memory/link_entities)
   |-- proposal inspection, approval/rejection (CLI-only origin)
   |-- approved-write execution (calls storage::propose_and_commit)
   |-- WritePolicy/WriteOperation enforcement
   |-- frozen typed errors (BrainApiError, 10 variants)
   `-- content-free audit event emission (AuditEvent)
   |
   v
crates/api / crates/storage / crates/index / crates/policy / crates/domain
   (crates/api's "B1 callable core: local single-user read-only
   functions, no transport" contract unmodified throughout)

crates/mcp-client (new) -- the reverse direction: SuperBrain calling
OUT to external MCP tools under its own permission gate. Zero
dependency on brain-api/storage/mcp-server (B5.10, enforced
structurally and by a dedicated separation test).
```

`crates/api` was never modified beyond what B5.0 explicitly
authorized (no write/proposal semantics entered it). `storage` gained
one closed security fix (B5.SEC, below) plus a narrow, confined
`.proposals/` persistence surface (`Vault::save_proposal`/
`load_proposal`/`list_proposal_ids`), never a general write
primitive.

### B5.SEC (prerequisite phase, Tasks 1-6)

`crates/storage`'s pre-existing TOCTOU gap (SEC-001/002/003 — a path
string canonicalized once, then re-resolved separately at I/O time)
was closed before any B5 proposal/MCP code was built on top of it, per
the design's own hard gate. `crates/storage/src/handle_safety.rs`
(new) opens a candidate path without following a reparse
point/symlink at the final component (`O_NOFOLLOW`/
`FILE_FLAG_OPEN_REPARSE_POINT`), then verifies the identity of the
**open handle itself** (regular file, single hard link, resolves
inside the vault root) — never re-deriving trust from a path string
after the open. All 12 real production call sites in `vault.rs`/
`recovery.rs`/`restore.rs` were rewired to this primitive. SEC-001/
002/003 are recorded CHIUSO (closed) in
`docs/security/security-audit.md` rows FS-02/FS-03, against the B5
threat model (an unprivileged local process with filesystem access;
not Administrator/SYSTEM), with one documented residual: a
microsecond-scale window between the pre-rename re-verification and
`std::fs::rename` itself (Approach A, not a true directory-handle-
relative rename — a real handle-relative rename primitive was
attempted in both Rust and an independent C reference against real
Windows SDK headers and fails with `ERROR_INVALID_PARAMETER` on this
build; accepted as residual risk, not represented as race-free).

## 2. Deviations from the original design

- **The "20 acceptance criteria" the plan's own Task 38 instruction
  references as living in "the design doc's source brief" are not
  present in this repository as a separate, enumerated artifact.**
  The design doc (`docs/superpowers/specs/2026-11-15-b5-brain-api-mcp-design.md`)
  has Goals (5), Non-goals (4), Anti-goals (4), and frozen Constraints
  (5) — 18 items, not 20, and not framed as a single acceptance list.
  No other file in `docs/` contains a 20-item brief. Section 9 below
  self-derives a 20-item acceptance table from these sections plus the
  plan's own explicit per-phase gates (B5.SEC closure, the B5.1
  layering test list), the same way the B4 closure doc
  (`docs/research/B4-DENSE-RETRIEVAL-PRODUCTION-CLOSURE.md`) derived
  its own 15-item table directly from its plan rather than from a
  separate file. This is a documentation gap worth flagging honestly,
  not a scope deviation in the implementation itself.
- **`get_rules`/`get_regulations`/`get_canon`'s precedence/scope**,
  flagged in the design doc as needing "one more grounding pass" (line
  152/223), was resolved pragmatically at Task 19: `get_canon` always
  returns an empty list (no canonization mechanism exists yet in this
  codebase), `get_rules`/`get_regulations` delegate to whatever
  `crates/api` already exposes for these two scopes. This is a
  narrower, more conservative resolution than the design doc
  anticipated, not a departure from its intent.
- **No other deviation from the approved design was found.** Every
  frozen constraint (`c1p2`, `c7ec`, `d9mm`, `c29zz`/`c30so`) is intact
  (verified in Section 9); the crate boundary, the proposal lifecycle
  shape, and the MCP server/client separation all match the design
  doc's own diagrams exactly.

## 3. Tasks B5.SEC, B5.0-B5.18 (Tasks 1-37)

All 37 tasks complete across 44 commits from `main`'s `b58820b` to
`c4f2551` (67 files changed, +17917/-231 lines). Full list in `git log
--oneline b58820b..c4f2551`; not re-pasted here. Task-to-commit
mapping, for traceability:

| Tasks | Scope | Key commits |
|---|---|---|
| 1-6 | B5.SEC: handle-based identity verification, close SEC-001/002/003 | `2167ba8`..`f6eb5cd` |
| 7-20 | B5.0/B5.1: frozen contract, `brain-api` scaffold, proposal lifecycle, 12 capabilities, layering tests | `ade28a8`..`60b6597` |
| 21-25 | B5.2/B5.7: MCP server, 12 tools wired, integration tests, deny-by-default audit | `a6d0d61`..`5cefa1e` |
| 26-30 | B5.8/B5.9/B5.10: MCP client foundation, discovery, authorization, invocation, separation test | `fb94216`..`9bdc31d` |
| 31-32 | E2E flows 1 and 2 | `79c8a34`, `3a3aaad` |
| 33 | B5.12: structured, content-free audit events | `8330ae3` |
| 34 | B5.13: security/adversarial suite | `daa0f8a` |
| 35 | B5.16: resource limits | `b3f1a22` |
| 36 | B5.17: mutation testing pass | `c4f2551` |
| 37 | B5.18: final verification sweep | (report only, no commit — see Section 8) |

## 4. Brain API contract (B5.0)

Frozen in `docs/superpowers/specs/2026-11-15-b5-0-brain-api-contract-freeze.md`
(Task 7). 12 capabilities, each with a stable name, typed input/output,
a permission requirement drawn from the one frozen `BrainApiError`
vocabulary (`NotFound`/`InvalidInput`/`PermissionDenied`/
`RuntimeUnavailable`/`DenseUnavailable`/`ExternalToolUnavailable`/
`Timeout`/`Conflict`/`CorruptData`/`UnsupportedOperation` — exactly
10, Task 11), a determinism statement, and documented empty/not-found
semantics:

| # | Capability | Shape | Permission |
|---|---|---|---|
| 1 | `search` | read, direct | `ReadOperation::Search` |
| 2 | `retrieve` | read, direct | `ReadOperation::Retrieve` |
| 3 | `graph_query` | read, direct, bounded traversal | `ReadOperation::GraphQuery` |
| 4 | `get_entity` | read, direct | `ReadOperation::GetEntity` |
| 5 | `get_code_context` | read, direct | `ReadOperation::GetCodeContext` |
| 6 | `get_previous_solution` | read, direct, provenance-linked | `ReadOperation::GetPreviousSolution` |
| 7 | `get_rules` | read, direct | `ReadOperation::GetRules` |
| 8 | `get_regulations` | read, direct | `ReadOperation::GetRegulations` |
| 9 | `get_canon` | read, direct (always empty today) | `ReadOperation::GetCanon` |
| 10 | `save_memory` | proposal-creating | `WriteOperation::ProposeWrite` |
| 11 | `update_memory` | proposal-creating | `WriteOperation::ProposeWrite` |
| 12 | `link_entities` | proposal-creating | `WriteOperation::ProposeWrite` |

Plus the CLI-only, never-MCP-reachable execution pair
`approve_proposal`/`reject_proposal` (`WriteOperation::ApproveProposal`/
`RejectProposal`). Verified at closure: all 12 signatures match the
freeze doc exactly, no drift (Section 9, item 1).

## 5. MCP server tools (Tasks 21-25)

`crates/mcp-server` registers exactly these 12 capabilities as MCP
tools — no more, no fewer — confirmed both structurally
(`exactly_the_3_task_22_tools_are_registered_no_capability_creep` and
equivalents) and over a real wire connection
(`real_client_lists_exactly_the_12_registered_tools_over_the_wire`).
`approve_proposal`/`reject_proposal` are never registered (Task 23's
own dedicated test). Every `RequestContext` an MCP request builds uses
`CallOrigin::McpSession`/`WritePolicy::mcp_agent()` — the only two
values this crate's public surface can construct at all.

## 6. MCP client (Tasks 26-30)

`crates/mcp-client` connects to external MCP servers under its own
permission gate (`ApprovalPolicy`: `Deny`/`AllowReadOnly`/
`AskApprovalRequired`/`AllowScopedExecution` — no "allow everything"
variant exists, enforced by an exhaustive-match compile-time test).
Deny-by-default: an unregistered/unclassified tool, or a policy that
doesn't cover a tool's real classification, is denied before any
network call. `ExternalToolMetadata`/`ToolClassification` are
caller-supplied and never auto-derived from server-reported discovery
data (`DiscoveredTool::classification` is a permanently-`None`
placeholder). Zero dependency on `brain-api`/`storage`/`mcp-server`
(B5.10), verified both by a dedicated architecture test
(`tests/separation.rs`, 5 tests) and independently by `cargo tree -p
mcp-client` at this closure (no match for any of the three).

## 7. Permission model (B5.7/B5.9)

Deny-by-default end to end:
- `RequestContext.read_policy: None` denies every read operation
  (`brain_api::read::require_read_policy`).
- `RequestContext.write_policy: None` denies every write operation.
- `WritePolicy::trusted_cli()` and `CallOrigin::TrustedLocal`'s only
  constructor (`TrustedLocalToken::new`) are both `pub(crate)` to
  `brain-api` — structurally unreachable from `mcp-server`, which can
  only ever construct `WritePolicy::mcp_agent()` (approval rights
  always `false`).
- Permission checks run before a proposal/resource is loaded or its
  existence is revealed (`PermissionDenied` never leaks into
  `NotFound`-shaped information).
- `crates/brain-api/tests/permission_model.rs` (Task 25): an explicit
  deny-by-default matrix across all 12 capabilities, denied both with
  no policy at all and with the most restrictive real policy, and
  confirming no capability implicitly grants another.

## 8. E2E flows (Tasks 31-32)

- **Flow 1** (external agent -> MCP server -> Brain API -> DenseV2
  search): one real test, a real external MCP client driving `search`
  over a real wire connection against a real `DenseScorer`, confirming
  the semantic match ranks first — `e2e_agent_search.rs`.
- **Flow 2** (Brain service -> MCP client -> mock external tool): 6
  real tests against genuine child-process Python MCP servers (no
  mocks of this crate's own logic) — happy path, denial, unavailable
  server, timeout, malformed response, mid-session permission
  revocation — `e2e_external_tool.rs`.

Both re-confirmed passing in a full-workspace build at this closure
(Section 10).

## 9. Acceptance criteria — exact PASS/FAIL

As noted in Section 2, the plan's own Task 38 text asks for a table
against "the 20 acceptance criteria in the design doc's source
brief" — that enumerated brief is not a file in this repository. The
20 items below are self-derived from the design doc's Goals/
Non-goals/Anti-goals/frozen-Constraints sections and the plan's own
explicit phase gates, numbered for a verdict table exactly as the B4
closure precedent did from its own plan. Each item cites the real
evidence verified at Task 37's sweep, not re-asserted from memory.

1. Frozen, documented 12-capability Brain API contract, one service
   layer for both internal and MCP callers — **PASS** (Section 4;
   `crates/brain-api/src/{read,graph,memory,knowledge}.rs`, no drift
   from the freeze doc).
2. Real MCP server exposing 9 read-shaped capabilities directly and 3
   write-shaped capabilities only as proposal-creation — **PASS**
   (Section 5; `save_memory`/`update_memory`/`link_entities` all
   return `ProposalId`, never a committed result, confirmed by direct
   test and by the real-wire integration test).
3. `PendingWriteProposal` mechanism: only a trusted human approval
   surface can approve a proposal; approval triggers execution through
   the existing storage/policy path — **PASS**
   (`approve_proposal`/`reject_proposal` are CLI-only-origin,
   unreachable via MCP; execution calls `storage::propose_and_commit`,
   never around it).
4. Minimal MCP client foundation, permission-gated, cannot bypass
   Brain API/policy — **PASS** (Section 6).
5. SEC-001/002/003 closed before any proposal storage or MCP read
   surface was built on top — **PASS** (Section 1; chronologically
   enforced — Tasks 1-6 preceded Task 7 in the actual commit history).
6. No MCP tool can execute a write without a human approval step,
   however indirect — **PASS** (no MCP tool calls
   `approve_proposal`/`reject_proposal` or anything that reaches
   `propose_and_commit` directly; the only path to committed bytes
   from an MCP-originated call is through a `Pending` proposal plus a
   separate, CLI-only approval action).
7. No code path other than the trusted approval surface can set a
   proposal's `status` to `Approved` — **PASS**
   (`PendingWriteProposal.status` is private; its only mutation path
   is `transition_to`, validated by `valid_proposal_transition`'s
   explicit pairwise table; `brain_api_exposes_no_raw_proposal_persistence_functions`
   confirms no raw save/load function is `pub`).
8. No new proposal file I/O inherits the pre-B5.SEC TOCTOU gap —
   **PASS** (`.proposals/` persistence reuses the same
   `handle_safety`-verified primitives via `Vault::verified_open_relative`,
   never a fresh unchecked path join).
9. No silent capability creep: every MCP tool traces 1:1 to a named
   Brain API operation — **PASS**
   (`every_registered_mcp_tool_maps_1to1_to_a_named_brain_api_operation`,
   and the real-wire test confirming exactly 12 tools, no more).
10. `storage::propose_and_commit` remains the only function committing
    Note bytes; the approval execution path calls into it, never
    around it (`c1p2`) — **PASS** (Section 9 preamble; only real
    production call site is `crates/brain-api/src/approval.rs:71`).
11. MVP perimeter frozen; the 8-capability extension and write-proposal
    trust model are the only authorized exceptions (`c7ec`) — **PASS**
    (`crates/api` unmodified beyond what B5.0 authorized; no other
    perimeter change found).
12. Read paths never fall back silently on hash mismatch (`d9mm`) —
    **PASS** (`a_hash_mismatch_on_the_underlying_read_never_falls_back_silently_through_brain_api`,
    `retrieve` surfaces `CorruptData`, never a before-image or other
    substitute).
13. Agent side can never self-approve, impersonate the approval
    surface, escalate read-to-write, bypass approval via another API
    call, or mutate proposal status directly in storage
    (`c29zz`/`c30so`) — **PASS** (Section 7; `WritePolicy::trusted_cli`/
    `TrustedLocalToken::new` both `pub(crate)`, structurally
    unreachable from any MCP-facing code).
14. No SuperBrain UI built in B5 (B6's job); CLI approval path only,
    sufficient for E2E testing — **PASS** (`crates/brain-api::cli`,
    a 3-command `list`/`approve`/`reject` surface, nothing more).
15. No generalization of `Note`'s persistence into a shared typed-record
    framework; proposals get their own dedicated storage module —
    **PASS** (`crates/brain-api/src/proposal_store.rs`, a confined,
    proposal-id-validated surface, not a general write primitive).
16. No reopening of B3/B4/DenseV2 behavior; no change to the frozen
    MVP perimeter beyond the authorized extension — **PASS** (DenseV2
    retrieval path untouched; `search`/`get_previous_solution` delegate
    to the same `index::search` DenseV2 mode B4 already froze).
17. Security audit items AP-02/03/04/05 (flagged "P1 before MCP") left
    addressed for the surfaces B5 actually ships — **PASS**
    (deny-by-default permission model, Section 7; local transport only,
    no sandboxed external tool execution introduced, matching the
    constraint's own stated scope limit).
18. Full workspace test suite passes — **PASS** (Section 10, 1889
    tests, zero failures).
19. Security/adversarial test suite passes (Task 34) — **PASS**
    (Section 10, 14 tests across `mcp-server`/`mcp-client`).
20. Mutation testing closes every genuine gap in the plan's named
    permission-gate/proposal-lifecycle/tool-mapping files (Task 36) —
    **PASS**, with one documented residual-verification gap, not a
    failure (Section 10's mutation subsection; the 12 Unix-cfg mutants
    require a Unix-hosted run to get real signal, tracked as an open
    item, not swept under "equivalent").

**19 of 20 PASS outright; 1 (item 20) PASS with a documented residual
verification gap that does not itself constitute a failure** (the
plan's own `MUTATION_TESTING_POLICY.md` gate — zero genuine missed —
is met for every mutant this Windows host could actually exercise;
what remains unexercised is a tooling-platform limitation, not a
discovered defect).

## 10. Tests, mutation, and adversarial results

**Full workspace**: `cargo test --workspace --all-features` — **1889
tests, 0 failed, 0 ignored** (re-confirmed at Task 37's sweep, after
Task 36's commit).

**Per B5-relevant crate** (whole-crate totals, including each crate's
own pre-existing tests where the crate predates B5 — `storage` only):

| Crate | Tests | Failed |
|---|---|---|
| `storage` | 291 | 0 |
| `brain-api` (new) | 206 | 0 |
| `mcp-server` (new) | 88 | 0 |
| `mcp-client` (new) | 64 | 0 |

**Named suites re-run explicitly in a full-workspace build at Task
37** (per the plan's own Step 4): Task 24's `server_integration.rs`
(5), Task 30's `separation.rs` (5), Task 31's `e2e_agent_search.rs`
(1), Task 32's `e2e_external_tool.rs` (6), Task 34's two
`adversarial.rs` files (6 + 2 = 8) — **25/25 pass**.

**Mutation testing (`cargo mutants`, Task 36)**, scoped per the
plan's named files, per `docs/research/MUTATION_TESTING_POLICY.md`'s
standard command:

| File | Caught | Unviable | Missed (genuine) | Missed (documented, not genuine) |
|---|---|---|---|---|
| `mcp-client/src/permission.rs` | 5 | 0 | 0 | 0 |
| `brain-api/src/{policy_write,proposal,approval,proposal_store}.rs` | 31 | 24 | 0 | 2 (structural equivalence, argued inline) |
| `storage/src/handle_safety.rs` | 20 | 6 | 0 | 12 (Windows-host tooling artifact on `#[cfg(unix)]` code — see Section 11) |

**0 genuine missed mutants in every scoped file.** The 2 documented-
equivalent mutants in `proposal_store.rs` (a `PathEscapesVaultRoot`
arm that can never fire because every id this module constructs is a
well-formed UUID, a strict subset of the validator's accepted
grammar; a `Conflict` arm that can never fire because none of the
three `storage::Vault` methods this module calls ever runs the
version-check code path that produces it) carry the structural
argument the project's own mutation policy requires, documented
inline in the source, not merely asserted here.

**Security/adversarial suite (Task 34)**: 6 new `mcp-server` tests
(oversized payload handled without crash/hang; wrong-JSON-type
schema mismatch; structural proof no tool schema accepts a raw
filesystem path; injection-shaped note body treated as inert data;
`retrieve`'s not-found path never leaks another note's content —
twice, end-to-end and structurally) + 2 new `mcp-client` tests
(oversized external-tool response handled without hang; injection-
shaped tool argument forwarded as inert data). Every item on the
plan's own adversarial checklist not covered by these 8 new tests was
independently confirmed already covered by Tasks 6/9/12/23/24/27/
28/29/30/32 (malformed JSON-RPC, unregistered tool names, malformed
`tools/list`/`tools/call` envelopes, replay/double-approve) —
documented, not silently assumed.

## 11. Known limitations / technical debt (named, not resolved)

Ten items tracked from prior task reports, each classified at Task
37's sweep by whether it blocks B5 closure, blocks merge, or only
blocks unattended production use. Full evidence trail in the Task 37
conversation turn; summarized here for closure-document permanence
(see also the Continuity record, decision `d275pv`):

1. **12 Unix-cfg mutants unverified** (`storage/handle_safety.rs`) —
   documented inline (lines ~104-122, ~313-320) as a Windows-host
   `cargo-mutants` tooling artifact (the mutated `#[cfg(unix)]`
   function never compiles into a Windows test binary, so every test
   trivially "passes" against dead code). **Does not block closure or
   merge.** Closure condition: a Unix-hosted `cargo mutants` run
   against the same file scope.
2. **No production composition-root binary calls
   `mcp_server::limited_transport`** — the function exists
   (`crates/mcp-server/src/limits.rs:36`) and is exercised only by
   tests; no `fn main`/`[[bin]]` target exists in `mcp-server` at all.
   **Does not block closure or merge** (B5's scope is library/API, not
   a launcher — consistent with every task's own doc comments).
   **Blocks autonomous production use**: `MCP_MAX_PAYLOAD_BYTES` is
   inert in any real deployment until a binary wires it in. Closure
   condition: a real stdio-server launcher binary (out of B5's scope,
   likely B6 or a dedicated follow-up).
3. **Concurrency semaphore's queue has no bound of its own** — an
   unlimited number of callers can simultaneously wait for one of the
   16 active permits (`acquire_concurrency_permit`,
   `crates/mcp-server/src/tools/search.rs`). Intentional design choice
   (queue, not reject, per explicit user decision in Task 35). **Does
   not block closure or merge. Blocks unattended production use under
   adversarial/high-concurrency load** (no backpressure on queue
   depth itself). Closure condition: an explicit decision to add a
   queue-depth cap, if ever needed.
4. **No client-side override of `rmcp`'s 16 MiB line-length default**
   — `crates/mcp-client/src/registration.rs`'s `connect_now` passes a
   raw `TokioChildProcess` with no `limited_transport`-equivalent
   override, unlike the server side. **Does not block closure or
   merge. Blocks autonomous use against untrusted external MCP
   servers** (a malicious external tool can still send up to 16 MiB
   per line). Closure condition: `rmcp`'s own `TokioChildProcessBuilder`
   would need a public hook to override `AsyncRwTransport`'s default,
   which does not exist in `rmcp` 3.11.0 as used.
5. **`AuditEvent` is not persistent, not append-only, not
   tamper-evident** — explicitly and prominently disclaimed in
   `crates/brain-api/src/audit.rs`'s own module doc comment. **Does
   not block closure or merge** (self-aware, documented design, not an
   oversight). **Blocks use as a compliance/forensic audit trail**
   until a real persistent `AuditSink` is built (explicitly deferred).
6. **`NullAuditSink` is `SuperBrainMcpServer`'s default** — documented,
   intentional (`Self::new`). **Does not block closure or merge.
   Blocks production observability** until a caller opts in via
   `.with_audit_sink(...)` — nothing in this repo does so outside
   tests today.
7. **`mcp-client`'s `InvocationLogEntry`/`InvocationLogSink` remain
   diagnostic/provisional**, never correlated into the real
   `AuditEvent` trail — an intentional consequence of the frozen B5.10
   isolation (`mcp-client` cannot depend on `brain-api` at all, so it
   cannot construct a real `AuditEvent` itself). **Does not block
   closure or merge** (the boundary is a frozen architectural decision,
   not a bug). **Limits production observability** of external-tool
   invocations specifically.
8. **`AuditEventKind::McpConnection`/`ExternalExecution` have no real
   call site** — both exist only in `audit.rs`'s own definition and
   unit tests; confirmed by grep across `mcp-server/src/` and
   `brain-api/src/`. **Does not block closure or merge** (no task
   through 36 required emitting them). **Limits audit completeness**:
   2 of 7 `AuditEventKind` variants are dead code today.
9. **Pre-existing `clippy::manual_contains` lint**,
   `crates/codegraph-index/src/agent_policy.rs:132,135` — confirmed
   present on `main` before the B5 branch existed (commits `87aab4b`/
   `8584cb1`, both ancestors of the B5 branch point). **Does not block
   closure or merge.** Explicitly out of scope for every B5 task.
10. **Pre-existing project-wide rustfmt drift** — `cargo fmt --check`
    on the full workspace shows diffs in ~150 files across every
    crate, including files B5 never touched
    (`codegraph-index/examples/*`, `vault-security/src/lib.rs`).
    Confirmed a toolchain/config mismatch pre-dating B5, not
    introduced by it. **Does not block closure or merge.** Every file
    this B5 session newly authored (`limits.rs` ×2, `adversarial.rs`
    ×2) is itself `cargo fmt`-clean; only edits to already-drifted
    pre-existing files inherit that file's existing drift, per
    explicit instruction not to reformat unrelated code.

## 12. Merge recommendation

**Recommend: merge `b5-brain-api-mcp` into `main`, once the user
explicitly authorizes it** (not done as part of this closure, and not
to be done automatically per the plan's own explicit STOP instruction
on Task 38).

Rationale: every architectural invariant verifiable from this
repository — the 12 frozen capability contracts, deny-by-default on
both read and write axes, the impossibility of an MCP-originated
caller reaching `trusted_cli()`/`TrustedLocal`, the single real
`propose_and_commit` call site, the `mcp-client`/`brain-api`/`storage`/
`mcp-server` separation (both by architecture test and by `cargo
tree`), and B5.SEC's closure — holds with zero exceptions. All
mechanical gates (fmt/clippy/test, scoped to B5's own touched code)
pass; 1889 workspace tests are green; the 5 named integration/E2E/
adversarial suites (Tasks 24/30/31/32/34) re-confirmed passing in a
full-workspace build; mutation testing (Task 36) found zero genuine
coverage gaps in every file it could actually scope.

This is **merge-ready and feature-complete**, which is a narrower
claim than **production-ready for unsupervised autonomous use**:
Section 11's items 1-4 and 8 are real, unclosed gaps (not merely
accepted tradeoffs) that a production deployment running without a
human in the loop would need closed first — a Unix-hosted mutation
run, a real composition-root binary enforcing `MCP_MAX_PAYLOAD_BYTES`
and a client-side payload limit, and either closing or deliberately
re-accepting the unbounded semaphore queue and the two dead
`AuditEventKind` variants. None of these gaps touches a frozen
contract, a trust boundary, or the deny-by-default model itself —
they are operational hardening work for a later phase, not evidence
that B5's own design or implementation is unsound.

## 13. STOP

Per Task 38's own instruction and the user's explicit standing
request: **this closure document does not merge, tag, push, or
delete the worktree.** No git state beyond this one commit (adding
the closure document itself) was changed by this task. The exact
sequence that would be needed to actually close B5 — merge, tag, push,
worktree cleanup — is listed in the accompanying turn report, for the
user's explicit review and authorization before any of it runs.
