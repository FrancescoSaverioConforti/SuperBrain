# M5 — Durable Audit Subsystem: Closure

**Branch:** `main` (no dedicated feature branch was used for M5).
**HEAD at closure:** `6ed5fd2`. **Date:** 2026-11-20.
**Local-vs-remote:** 38 commits ahead of `origin/main`, 0 behind. Not pushed.
**Status: IMPLEMENTATION SCOPE COMPLETE (Tasks 1-8B done).
CONDITIONALLY CLOSED — strictly for M5's own stated scope (a durable,
fail-closed audit trail for MCP tool calls and trusted CLI approval/
rejection). NOT a declaration that SuperBrain's overall security
posture is production-ready, and NOT a declaration that unsupervised
agentic autonomy is safe.**

---

## 1. Scope, objectives, non-goals

M5 is the remediation series that followed the 2026-11-19 Data
Governance/Security/Trust architectural audit (R7: "non-durable,
non-tamper-evident audit" named as a blocker for agentic autonomy,
though not for current supervised use). Its objective: replace B5.12's
`AuditEvent`/`AuditSink` mechanism — explicitly disclaimed there as
"not persisted, not signed, not append-only, not tamper-evident" — with
a real, durable, hash-chained, crash-consistent audit log for the two
places SuperBrain actually performs a trusted action: an MCP tool call,
and a trusted-CLI approval or rejection of a write proposal.

**Non-goals, explicit from Task 1 onward and never silently expanded:**
- Not a general-purpose append-only log facility — exactly one fixed
  resource per vault (`.audit/audit.log` + `.audit/head.ref`).
- Not MCP Client (`mcp-client`) integration — that crate's own B5.10
  isolation (zero dependency on `brain-api`) is untouched.
- Not automatic reconciliation, log rotation, or retention policy.
- Not a defense against a privileged/Administrator-level attacker, or
  against a coordinated replay of a genuinely-valid old `(audit.log,
  head.ref)` pair (see Section 8).
- Not a wiring task for MCP approval/rejection — the trust boundary that
  only a local CLI surface may approve/reject a proposal is preserved
  exactly as B5 froze it; no task in this series added, widened, or
  relaxed that boundary.

## 2. Architecture implemented

```
Trusted CLI (crates/brain-api/examples/approve_cli.rs, composition root)
   |-- opens a SECOND, independent storage::Vault handle for the worker
   |-- spawn_worker(...) -- fails closed, before any command runs
   |-- run_cli_durable(args, vault, now, sink, approval_hooks, rejection_hooks)
   |     |-- run_approve_core: ApprovalAttempted -> approve_proposal -> CommitOutcome
   |     `-- run_reject_core:  RejectionAttempted -> reject_proposal -> RejectionOutcome
   |-- sync/async bridge: spawn_blocking + bounded mpsc -> one async bridge task
   `-- bounded shutdown (DEFAULT_TIMEOUT_MS), 3 honest outcomes

MCP Server (crates/mcp-server/examples/serve_stdio.rs, composition root)
   |-- opens a SECOND, independent storage::Vault handle for the worker
   |-- spawn_worker(...) -- fails closed, before any command runs
   |-- SuperBrainMcpServer::new(...).with_audit_sink(audit_worker.clone())
   |-- every one of the 12 tools: record_tool_call (best-effort, via AuditSink::record)
   `-- bounded shutdown, 3 honest outcomes

brain-api-audit-worker (crates/brain-api-audit-worker)
   |-- spawn_worker: one dedicated writer task per call, strict
   |     sequential processing, no mutex (mpsc channel IS the serialization)
   |-- AuditWorkerHandle::record_durable: async, awaits full
   |     append+fsync+reference-update before resolving
   |-- AuditWorkerHandle::shutdown: graceful FIFO drain, true-exit await
   `-- implements brain_api::AuditSink (best-effort path: try_send, never blocks)

storage::audit_writer (crates/storage/src/audit_writer.rs)
   |-- Vault::open_audit_writer: exclusive OS-level lock (std::fs::File::
   |     try_lock(), cross-platform by construction since M5 Task 8A) +
   |     startup reference-agreement verification
   |-- AuditWriterHandle::append: write_all -> sync_all -> DPAPI head-
   |     reference update, in that exact order, under the one lock
   `-- reuses vault_security::DpapiTrustedReferenceStore/handle_safety
        unmodified -- no new cryptographic or containment code

brain-api::audit_chain + audit_record (crates/brain-api/src/{audit_chain,audit_record}.rs)
   |-- pure SHA-256 hash-chain primitives (compute_record_hash, verify_link, scan_chain)
   `-- the concrete, versioned JSON line format (serialize_record/parse_line/parse_event)

brain-api::audit (crates/brain-api/src/audit.rs) -- the schema
   AuditEventKind: 7 frozen (B5.12) + ApprovalAttempted + CommitOutcome
   (M5 Task 5) + RejectionAttempted + RejectionOutcome (M5 Task 8B)
```

## 3. Commit history (derived from `git log`, not assumed)

```
e42a315  M5 Task 1  - pure audit hash-chain and recovery-scan primitives
5b37ab9  M5 Task 2  - durable audit filesystem writer
a091965  M5 Task 3  - audit head reference, integrity verification, crash consistency
b453085  M5 Task 4  - cancellation-safe durable audit writer worker
0b8143d  M5 Task 5  - ApprovalAttempted/CommitOutcome audit schema
7fd3f9e  (fix)      - remove unsatisfiable genesis-generation defense-in-depth check
a9b916d  M5 Task 6  - production audit worker wiring in serve_stdio
4045e6b  M5 Task 7  - durable approval/commit audit integration
99edab3  M5 Task 8A - cross-platform audit writer lock (P0 fix)
5ac937c  M5 Task 8B - durable rejection audit (closes P1)
```

10 substantive commits, `e42a315..5ac937c` (plus `continuity: record
decision`/`next_action` commits interleaved throughout, none touching
production code). Task 8 itself (the verification pass that found P0/
P1) produced no commit of its own — report only, per its own
read-only instruction.

Milestones:
- Tasks 1-4: pure primitives, durable filesystem writer, DPAPI head
  reference, cancellation-safe single-owner worker — the storage
  foundation, with no production wiring yet.
- Task 5: the `ApprovalAttempted`/`CommitOutcome` schema extension,
  additive, frozen-7-untouched.
- Task 6: first production wiring — the MCP stdio launcher.
- Task 7: second production wiring — the trusted CLI, plus the
  `run_cli_durable`/`DurableApprovalHooks` architecture (resolved after
  an explicit STOP: exporting `trusted_ctx()`/`parse_id()` would have
  widened the trust boundary `tests/layering.rs` freezes — see
  Continuity decision `d286gw`).
- Task 8: read-only verification, found 2 defects (P0, P1).
- Task 8A: fixed P0 (cross-platform compile break).
- Task 8B: fixed P1 (rejection had zero audit trail), the exact
  structural mirror of Task 7's own approval design.

## 4. Acceptance-criteria matrix

| # | Criterion | Evidence | Verdict |
|---|---|---|---|
| 1 | Audit log uses a real, verifiable SHA-256 hash chain, not a flat append log | `crates/brain-api/src/audit_chain.rs` (`compute_record_hash`, `verify_link`, `scan_chain`); `crates/storage/src/audit_writer.rs` tests, 24/24 pass | **PASS** |
| 2 | Writer ownership is single, OS-enforced, crash-safe | `audit_writer.rs:404-430` (`acquire_writer_lock` via `std::fs::File::try_lock()`); tests `a_second_handle_to_the_same_audit_log_is_rejected_while_the_first_is_still_open`, `killing_the_process_holding_the_lock_releases_it_for_a_fresh_acquire`, `a_second_process_is_excluded_for_the_full_duration_the_first_holds_its_writer_handle` | **PASS** |
| 3 | Append is fsync-ordered before the caller's in-memory state advances | `audit_writer.rs::append` (`write_all` -> `sync_all` -> reference write, in that order); `durable_ack_is_only_returned_after_the_record_is_actually_on_disk` | **PASS** |
| 4 | Exactly one writer task, bounded queue, cancellation-safe | `crates/brain-api-audit-worker/src/worker.rs:23` (`QUEUE_CAPACITY = 256`), `:79` (one `mpsc::channel`), `:81` (one `tokio::spawn` per `spawn_worker` call); `cancelling_a_caller_while_its_durable_write_is_in_flight_causes_no_duplicate_or_corruption` | **PASS** |
| 5 | Durable approval: intent registered, acknowledged, before commit; outcome recorded after, correlated | `crates/brain-api/src/cli.rs:245-259` (`run_approve_core`); `cli_approval_e2e.rs`/`approve_cli_durable_e2e.rs`, correlation + distinct-attempt-id tests | **PASS** |
| 6 | Durable rejection: same contract as approval, structurally | `cli.rs:273-287` (`run_reject_core`, textually mirrors `run_approve_core`); `DurableRejectionHooks` has no `storage::CommitResult`-shaped field at all (`cli.rs:101-117`) | **PASS** |
| 7 | Failed pre-registration prevents the real action (fail-closed) | `cli.rs:251-253` and `:279-281` (`if let Err(reason) = (hooks.pre_register)(...) { return ... }`, before `approve_proposal`/`reject_proposal` is ever called) | **PASS** |
| 8 | A failed outcome-write never falsifies a real, successful result | `cli.rs` (`formatted`/`audit_outcome_write` are two independent return values; `DurableCliOutcome` struct); `run_cli_durable_reports_a_successful_commit_truthfully_even_when_the_outcome_write_itself_fails`, `..._rejection_...` | **PASS** |
| 9 | `CommitOutcome` is exclusive to approval; rejection can never emit it | `audit.rs:76` (`AuditEventKind`); structural (not just tested): `run_reject_core`'s only hook parameter is `DurableRejectionHooks`, which has no `CommitOutcomeKind`-shaped field | **PASS** |
| 10 | MCP tool calls are audited (best-effort) via the same worker | `crates/mcp-server/examples/serve_stdio.rs:122` (`spawn_worker`), `.with_audit_sink(...)`; `tests/launcher_audit.rs`, 8/8 pass | **PASS** |
| 11 | No production fallback to `NullAuditSink` in either composition root | Read both `serve_stdio.rs` and `approve_cli.rs` in full: both fail closed (non-zero exit, no stdout) if `spawn_worker` errs; neither constructs `NullAuditSink` for the durable path | **PASS** |
| 12 | MCP cannot reach trusted approval/rejection authority | `crates/mcp-server/tests/{launcher,no_bypass}.rs`, 11/11 pass; `trusted_ctx()`/`parse_id()` remain `fn` (not `pub`) in `cli.rs` | **PASS** |
| 13 | Frozen Brain API contracts untouched by M5 | `git log -p` across `0b8143d..HEAD` on `approval.rs`/`error.rs`: zero diff hits on `approve_proposal`/`reject_proposal`/`BrainApiError`'s variant set | **PASS** |
| 14 | Cross-platform compile safety for the audit writer | `audit_writer.rs`: zero `windows::`/`std::os::windows` references in production code (the one remaining Windows-only call is a test, correctly `#[cfg(windows)]`/`#[cfg(unix)]`-split) | **PASS (by construction)** — see Section 9 for the honest Unix-runtime caveat |
| 15 | Full Unix build and test run | Not performed — no C cross-toolchain available in this environment for `storage`'s `git2` dependency | **NOT VERIFIED** (named explicitly, not silently assumed) |

## 5. Verified security and durability guarantees

- **Single-writer exclusion is real and OS-enforced**, not an
  in-process convention: a second process (or a second handle within
  the same process) cannot acquire the lock while the first holds it,
  proven by real child-process spawn/kill reproduction, on both the
  storage layer (`audit_writer.rs`) and across the full M5 Task 7/8B
  CLI acceptance suite.
- **Durable acknowledgment is genuine**: `AuditWorkerHandle::
  record_durable` only resolves after `write_all` + `sync_all` + the
  DPAPI reference update all completed; a caller that awaits it and
  gets `Ok` has a provably on-disk record, confirmed by reading the
  real bytes back independently in the same test.
- **Fail-closed pre-registration is structural, not advisory**: both
  `run_approve_core` and `run_reject_core` call the hook and check its
  result with an early `return` — `approve_proposal`/`reject_proposal`
  are lexically unreachable past a pre-registration failure in the
  same function body.
- **Truthful result reporting is structural**: the real command result
  and the outcome-write's own success/failure are two independent
  fields on `DurableCliOutcome`; nothing in `cli.rs` or either
  composition-root binary folds one into the other.
- **Cancellation safety**: a caller's own task being aborted mid-await
  never un-enqueues or duplicates a command already accepted by the
  worker — the worker always finishes whatever it dequeued. Verified
  by direct process-level reproduction (M5 Task 4), re-confirmed
  unmodified through Tasks 7/8B.

## 6. Explicit limits of best-effort MCP auditing

`brain_api::AuditSink::record(&self, event: AuditEvent)` returns `()`.
The `AuditWorkerHandle` implementation of it (used by both composition
roots for the per-tool-call `ToolCall`/`PermissionDecision`/
`WriteMemory` events) calls `try_send` and silently discards a
`QueueFull`/`WorkerGone` outcome — **there is no caller-visible signal,
ever, that a best-effort event was dropped.** This is not a bug: it is
the documented, load-bearing contract of the `AuditSink` trait itself
(a synchronous, infallible-from-the-caller's-perspective signature that
predates M5, from B5.12). M5 did not, and could not without changing
that frozen trait, make MCP tool-call auditing durable — only the
trusted CLI's approval/rejection path reaches the genuinely durable
`record_durable` API. **This closure document does not claim MCP
auditing is lossless, and no code or test anywhere in this series makes
that claim either.**

## 7. Crash-consistency guarantees and remaining uncertainty

| Crash window | Classification | Evidence |
|---|---|---|
| Before/after approval- or rejection-intent fsync | TESTED (general ack-after-fsync contract) + uncontrolled-timing kill tests | `durable_ack_is_only_returned_after_the_record_is_actually_on_disk`; `killing_the_cli_process_mid_flight_...` (both approval and rejection variants) |
| During commit execution (`approve_proposal`/`propose_and_commit`) | STRUCTURALLY VERIFIED | Pre-existing A2 journal crash-consistency machinery, outside M5's own scope, unmodified |
| After commit, before proposal-status persistence | STRUCTURALLY VERIFIED | `approval.rs` orders `execute()` then `transition_to`/`save_proposal`; inherited from Task 14, not re-tested by M5 |
| After proposal-status persistence, before audit-outcome fsync | **NOT VERIFIED (deterministically)** | Only reachable by chance via the uncontrolled-timing kill tests; no fault-injection seam exists to target this exact window on demand |
| During audit-log append | TESTED | `a_write_failure_poisons_the_handle_and_refuses_all_further_appends` (real I/O failure via a read-only-opened proxy file) |
| During DPAPI head-reference update | TESTED | T3-4/T3-5 tests in `audit_writer.rs` simulate this exact window directly |
| During worker shutdown | STRUCTURALLY VERIFIED, not directly tested | Relies on Rust `Drop` + OS lock release (proven generally, not at the exact shutdown-call boundary) |

The one genuinely open gap is the "status-persisted, outcome-fsync
pending" window. The smallest closing step (not implemented, scope
explicitly excluded by Task 8's own instructions): a `#[cfg(test)]`-only
fault-injection hook in `AuditWriterHandle::append`, mirroring the
`FaultInjector` pattern `storage/src/fault.rs` already uses for the A2
journal, to force a deterministic mid-step interruption.

## 8. DPAPI replay/rollback limitation

**Unchanged since before M5** (originally characterized for
`vault_security::deletion_log`, continuity decision `d279ua`; M5's own
audit-specific restatement is decision `d280gi`). The DPAPI-wrapped head
reference detects **forgery** (bytes that were never legitimately
produced by this mechanism) and **a single incomplete update** (log
exactly one generation ahead of its reference — the one safe
roll-forward pattern). **It does NOT detect a coordinated replay of a
genuinely-valid OLD `(audit.log, head.ref)` pair restored together** —
this produces the identical "one generation ahead, chain-consistent"
shape a real crash would, and the mechanism cannot distinguish them.
This is proven directly, not merely asserted: the test
`replaying_an_old_genuinely_valid_log_and_reference_pair_together_is_not_detected_accepted_limitation`
(`crates/storage/src/audit_writer.rs:1213`) demonstrates the limitation
by actually performing the replay and asserting it is accepted. **This
document does not claim anti-rollback protection.** The DPAPI wrapping
provides forgery-resistance only.

## 9. Platform support and the outstanding Unix-build verification

M5 Task 8A fixed a confirmed, build-breaking defect: `storage::
audit_writer` previously imported the `windows` crate's raw FFI
(`LockFileEx`) unconditionally, with no `#[cfg(windows)]` gate anywhere
in the module — `storage`, and therefore every crate depending on it,
could not compile on a non-Windows target at all. The fix replaced this
with `std::fs::File::try_lock()`, a stabilized standard-library API
that is genuinely cross-platform **by construction** (the same call
compiles and provides the identical exclusive-lock semantics on both
platforms — `LockFileEx` under the hood on Windows, `flock`/`fcntl` on
Unix) — not a `#[cfg]`-gated stub, not an `UnsupportedPlatform`
fallback. Confirmed:
- Zero `windows::`/`std::os::windows` references remain in `audit_writer.rs`
  production code (the one remaining Windows-specific API use, in a
  symlink test, is correctly paired with a `#[cfg(unix)]` equivalent).
- The module's own API usage (`std::fs::File::try_lock`/
  `TryLockError`) was independently type-checked against the real
  `x86_64-unknown-linux-gnu` target in an isolated probe crate with no
  native dependencies — it compiles cleanly there.

**What remains explicitly NOT VERIFIED, and is not upgraded to
"fixed" by this document**: a full `cargo build`/`cargo test` run of
the `storage` crate (and everything depending on it) on an actual Unix
host or CI environment. This was not attempted to completion in this
environment because `storage`'s own `git2` dependency requires a real
C cross-compiling toolchain (`x86_64-linux-gnu-gcc` or equivalent),
which is not installed here — an environment limitation unrelated to
the fix itself, not evidence the fix is incomplete. **This document
does not claim full Unix support.** The closure condition is a real
build-and-test run on a genuine Unix target.

## 10. SQLite cross-process contention

**Pre-existing, unchanged, accepted MVP limitation.** `crates/storage/
src/journal.rs` configures no `busy_timeout` on its SQLite connection.
Two independent OS processes, each opening their own connection to the
same journal file, can legitimately race at the SQLite layer itself —
discovered and documented during M5 Task 7's own acceptance testing
(not introduced by it). `crates/brain-api/tests/layering.rs`'s own
pre-existing concurrent-approval test only ever exercises two
**threads sharing one `Vault` instance** (where the in-process per-note
mutex fully serializes correctly) — it never claimed, and was never
meant to prove, a stronger guarantee across two separate OS processes.
M5's own test suite (`approve_cli_durable_e2e.rs`) reflects this
honestly: `concurrent_in_process_approval_attempts_...` proves the real
CAS guarantee in-process; `cross_process_concurrent_approvals_...`
documents, rather than over-asserts past, the weaker cross-process
reality (at most one commit succeeds, the log is never corrupted,
regardless of how the race resolves).

## 11. Incomplete-attempt reconciliation limitations

**No reconciliation tool or automatic recovery mechanism exists.** An
`ApprovalAttempted`/`RejectionAttempted` record with no matching
outcome record (because the process crashed, was killed, or the
durable outcome write itself failed and nobody retried) is an honestly
unresolved entry in the audit log — the log correctly represents this
as "unknown," never as a fabricated success or failure (Section 7's
crash-window tests prove this directly) — but nothing in this codebase
automatically detects, flags, or repairs such an entry. A human
inspecting the log, or a future tool built specifically for this
purpose, is the only path to resolving it today. Explicitly out of
scope for every M5 task (Task 7's and 8B's own instructions both
excluded "automatic reconciliation, recovery repair, or log rotation").

## 12. Audit retention, rotation and growth

**No mechanism exists.** The audit log grows without bound; every
`spawn_worker` call rescans the entire log from byte zero via
`recover_tail`'s own startup chain-verification. At current, low-volume
supervised usage this is not a correctness concern, only a future
operational one. No size cap, time-based rotation, archival, or
multi-segment log format exists anywhere in `brain-api-audit-worker` or
`storage`. **Blocks any claim of sustained, high-volume, or
long-running unattended production use** until a retention policy is
designed and implemented — explicitly a future milestone's
responsibility, not attempted here.

## 13. MCP Client external execution audit gap

**Unchanged since before M5, by frozen architectural design.**
`crates/mcp-client` has, and must keep, zero dependency on `brain-api`
(the B5.10 isolation, enforced by its own `tests/separation.rs`) — it
therefore cannot construct a real `brain_api::AuditEvent` itself.
`mcp-client::InvocationLogEntry`/`InvocationLogSink` remain local,
diagnostic, non-persisted, exactly as B5.12 originally disclaimed them.
**No task in M5 touched this boundary, and none was asked to.** A
future milestone that wants external-tool invocations (SuperBrain
calling OUT to other MCP servers) in the same durable audit trail would
need either a new, deliberately-designed bridge across this frozen
isolation, or a second, independent durable log for that direction —
not a trivial extension of the mechanism this document closes.

## 14. Security threat-model boundaries

M5's own guarantees hold against: an unprivileged local process or
thread racing another unprivileged local process/thread for the same
resource; a process crash or kill at an arbitrary point; a single
transient I/O failure during append or reference update; a forged
(never-legitimately-written) head reference. M5's own guarantees do
**NOT** hold against: an Administrator/SYSTEM-level attacker (DPAPI's
own user-scoping is the only defense in that threat class, unchanged
from `vault_security`'s pre-existing posture); a coordinated replay/
rollback of a genuinely-valid old log+reference pair (Section 8); an
attacker with the ability to race two OS processes against the SQLite
journal with no `busy_timeout` (Section 10, a pre-existing `storage`
property); sustained high-volume production load with no retention
policy (Section 12). This is the same threat-model framing the B5
closure document and the 2026-11-19 audit both already established —
M5 narrows the specific "non-durable audit" gap that audit named,
nothing broader.

## 15. Readiness verdicts

- **MCP Server operational audit: CONDITIONAL.** The trust boundary
  (exactly 12 tools, approve/reject unreachable) and the worker's own
  durability guarantees are solid; every per-tool-call event is
  best-effort only, with no caller-visible loss signal (Section 6).
  Acceptable for supervised, low-stakes diagnostic use; not a
  complete-audit-coverage guarantee.
- **Trusted CLI approval audit: READY.** Both `approve` and `reject`
  are genuinely durable, fail-closed, and truthfully reported, with
  independently correlated attempt IDs for each.
- **MCP Client external execution audit: NOT READY.** Unintegrated by
  frozen design (Section 13); unchanged since before M5.
- **Unsupervised agentic autonomy: NOT READY.** Best-effort MCP
  auditing, no Unix verification, no log rotation, and no MCP Client
  integration are all real blockers for any autonomous/unsupervised
  use case — correctly out of this series' own scope throughout.

## 16. Residual risk register

| # | Item | Severity | Classification | Ownership | Exit criterion |
|---|---|---|---|---|---|
| 1 | Full Unix compilation/runtime unverified | Medium (no current non-Windows deployment target; would block one if added) | NOT VERIFIED | Unowned — needs a real Unix host or CI | A real `cargo build`/`cargo test -p storage` (and dependents) pass on an actual Unix target |
| 2 | MCP best-effort event loss, no caller-visible signal | Medium (observability, not correctness) | ACCEPTED MVP LIMITATION | Unowned | Either accept permanently for supervised use, or add an observable drop-rate metric at the composition-root level |
| 3 | MCP Client external execution audit not integrated | High for autonomy, zero for current supervised scope | FUTURE AUTONOMY BLOCKER | Unowned — must be explicitly assigned to a future milestone | A deliberately-designed bridge across the B5.10 isolation, or an independent durable log for that direction |
| 4 | No deterministic crash-window fault injection for the one untested window (Section 7) | Low (structural guarantees + general ack-after-fsync tests already cover the load-bearing property) | ACCEPTED MVP LIMITATION | Unowned | A `#[cfg(test)]`-only fault-injection hook, mirroring `storage::fault::FaultInjector` |
| 5 | SQLite cross-process contention, no `busy_timeout` | Low-Medium (pre-existing `storage::Vault` property, not audit-specific) | ACCEPTED MVP LIMITATION | Unowned | An explicit decision to add `busy_timeout`/WAL mode to `journal.rs`, if cross-process write concurrency becomes a real requirement |
| 6 | No audit-log retention/rotation/growth policy | High for sustained production volume, zero at current scale | FUTURE AUTONOMY BLOCKER | Unowned — must be explicitly assigned to a future milestone | A designed rotation/retention policy, including how `scan_chain`/`recover_tail` handle a multi-segment log |
| 7 | No reconciliation tool for incomplete attempts | Low (the log never lies about this — an unresolved entry is honestly unresolved) | ACCEPTED MVP LIMITATION | Unowned | A human-operated or automated reconciliation procedure, if unattended recovery is ever required |
| 8 | DPAPI replay/rollback not detected | Medium (requires filesystem-level access to exploit; pre-existing, honestly documented) | ACCEPTED MVP LIMITATION (pre-existing, `d279ua`) | Unowned | None planned — would require a fundamentally different reference mechanism (e.g., a monotonic external counter) |

## 17. Final verdict

**CONDITIONALLY CLOSED.**

M5's own implementation scope — a durable, hash-chained, crash-
consistent audit trail for MCP tool calls (best-effort) and trusted
CLI approval/rejection (fully durable, fail-closed) — is complete and
verified against its own stated acceptance criteria (Section 4, 15/15
PASS or honestly NOT VERIFIED, zero silent gaps). This conditional
closure requires that items 1, 3, 4, and 6 of Section 16 remain
explicitly tracked in Continuity as named, owned limitations, not
silently dropped, and that any future milestone claiming "unsupervised
agentic autonomy is ready" must first explicitly close items 1, 3, and
6 at minimum.

**This is a narrower claim than "SuperBrain's security posture is
production-ready" or "unsupervised agentic autonomy is safe."** Those
are separate, broader claims this document does not make and this
series was never scoped to establish.

## 18. Recommended Git sequence for eventual closure (NOT executed)

1. `git log --oneline e42a315..5ac937c` — confirm the final commit
   list matches Section 3 exactly before any push.
2. Review this closure document's own diff once more against the
   actual current source (Section 19).
3. `git push origin main` — publishes the 38 currently-unpushed local
   commits (including this closure document's own commit).
4. Optionally, `git tag -a m5-durable-audit-closure -m "M5: durable
   audit subsystem, conditionally closed"` then `git push origin
   m5-durable-audit-closure` — only if the user wants a durable
   reference point distinct from `main`'s own moving HEAD.
5. No branch merge is needed: M5 was developed directly on `main`,
   unlike B5's dedicated feature branch.

## 19. STOP

Per this task's own explicit instruction: **this closure document does
not push, tag, merge, or create a new branch.** The only Git mutation
performed by this task is the one commit adding this document itself.
The sequence in Section 18 is reporting, not execution.
