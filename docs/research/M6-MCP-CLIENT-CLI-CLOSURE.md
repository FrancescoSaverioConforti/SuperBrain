# M6 — Supervised MCP Client CLI: Closure

**Branch:** `main` (all M6 work committed directly, no feature branch).
**HEAD at closure:** `7930f11`. **Date:** 2026-11-25.
**Milestones covered:** M6.4 (composition root + durable audit),
M6.4-S1 (executable identity / Windows ACL), M6.5 (schema-hash
verification), M6.6 (Windows creation-time Job Object containment),
M6.7 (adversarial security audit), M6.8 (this closure).

**Status: PASS.** The CONDITIONAL PASS this document originally
recorded (at commit `0970a7a`) is resolved: the one outstanding
condition — `windows_job.rs`'s own test-suite flakiness under default
`cargo test` parallelism — has been root-caused and fixed, not merely
mitigated. See [§6a](#6a-resolved-finding-windows_jobrs-test-suite-concurrency-flake)
for the finding and fix, and [§10](#10-closure-decision) for the
updated verdict.

---

## 1. What M6 built

`crates/mcp-client-cli` is a strictly supervised, one-shot composition
root: one process invocation connects to exactly one configured
external MCP server, discovers its tools, verifies one tool's schema
against an operator-pinned hash, authorizes it under an operator
policy, invokes it exactly once, and exits. It introduces no
autonomous execution, no tool chaining, no retry. It has no dependency
on `brain-api`/`mcp-server`/`storage`'s write path — it is a parallel
supervised-use path, not an extension of SuperBrain's own write
pipeline.

```
mcp-client-cli (crates/mcp-client-cli, new in M6.4)
   |-- config.rs: TOML trusted-configuration contract (operator-
   |     authored; command/script hashes, trust boundaries, per-tool
   |     classification/sensitivity/schema pin, approval policy)
   |-- identity.rs: executable/script identity verification
   |     (content-hash + M6.4-S1's Windows ACL trusted-location check)
   |     and schema-hash canonicalization (M6.5)
   |-- windows_acl.rs (Windows only): read-only ACL existence-scan,
   |     M6.4-S1
   |-- adapter.rs: StorageAuditWriterAdapter -- wraps the real
   |     storage::AuditWriterHandle 1:1 into the durable audit sink
   |-- main.rs: the one enforced sequence --
   |     config load -> discovery -> schema verification ->
   |     authorization -> invocation attempt -> invocation outcome
   |
   v
mcp-client (crates/mcp-client, pre-existing B5.8-5.16; M6.6/M6.7
            touched only windows_job.rs inside it)
   |-- registration.rs: connect() -- the ONE entry point to an
   |     external server; global kill-switch, per-registration
   |     enabled flag, env isolation, timeout
   |-- windows_job.rs (Windows only, new in M6.6): creation-time
   |     Job Object process containment, replacing
   |     tokio::process::Command::spawn() for this one call site
   |-- discovery.rs / permission.rs / invoke.rs: tools/list
   |     validation, authorize(), tools/call + response validation
   |     (all pre-M6, unmodified by M6)
   |
   v
mcp-client-audit-worker + audit-chain-core (pre-existing B5.16/B5.17):
   durable, hash-chained, append-only audit log -- the real audit
   trail every M6 phase writes to
```

Nothing above `mcp-client`'s own `connect()` call changed shape across
M6: `main.rs`'s sequencing, `permission.rs`'s `authorize()`, and the
audit event schema are the same code paths M6.4/M6.5 established.
M6.6/M6.7 replaced ONE function call inside `registration.rs`
(`connect_now`'s Windows branch) with a new, self-contained module.

---

## 2. MCP client trust model

This is the frozen, explicit model every later decision in M6 is built
on. None of it changed across M6.4-M6.7; M6.8 states it explicitly in
one place for the first time.

- **The external MCP server is untrusted**, full stop. It is a
  separate process this CLI spawns and fully controls the lifecycle
  of, but its *behavior* once running is never assumed benign. Every
  piece of data it sends (its `tools/list` reply, its `tools/call`
  result) is validated before being trusted with anything, and a
  validation failure fails closed (`McpClientError::MalformedResponse`
  or `PermissionDenied`), never degrades to a partial-trust fallback.
- **Discovery does not imply trust.** `discover_tools` populates a
  local cache of what the server *claims* to offer. Nothing in
  `discovery.rs` or `main.rs` treats a discovered tool as authorized,
  classified, or safe — `permission::authorize` never reads a
  `DiscoveredTool` at all (enforced by
  `authorize_takes_no_discovered_tool_and_derives_nothing_from_server_supplied_metadata`,
  `crates/mcp-client/src/permission.rs`). Trust comes exclusively from
  the operator's own trusted TOML configuration (`config.rs`), never
  from anything the server says about itself.
- **`input_schema_sha256` attests interface shape only, never runtime
  behavior.** This is M6.5's own frozen scope limitation, printed
  verbatim on every real CLI run via
  `identity::SCHEMA_HASH_SCOPE_DISCLOSURE`:
  > "input_schema_sha256 verification proves only that the server's
  > currently-advertised tool interface shape matches what the
  > operator reviewed and pinned. It proves nothing about the server's
  > actual runtime behavior when the tool is invoked — a server could
  > advertise a verbatim-matching schema while behaving arbitrarily
  > differently inside its own tools/call handler."

  M6.7's own audit re-confirmed this is accurate and unchanged — see
  [§6](#6-residual-risks) for why this is an accepted limitation, not
  a gap.
- **Authorization is a separate, local decision from discovery.**
  `permission::authorize(metadata, policy)` is a pure function over
  operator-supplied `ExternalToolMetadata` and an operator-chosen
  `ApprovalPolicy` — no I/O, no network call, nothing read from the
  live connection. A denial happens strictly before `conn`'s transport
  is ever touched for that invocation (`invoke.rs`'s own ordering
  comment, confirmed by
  `a_denied_tool_returns_permission_denied_without_ever_calling_the_real_tool`).
- **Invocation happens only after discovery → schema verification →
  authorization complete, in that order, every time.** This is the
  one ordering M6.5 exists to enforce and M6.6/M6.7 were explicitly
  forbidden from weakening. Proven end-to-end against a real spawned
  server by
  `schema_verification_runs_before_authorization_and_before_any_tools_call_dispatch`
  (`crates/mcp-client-cli/tests/acceptance.rs`).
- **`server.name` (the config label) is never an identity anchor.**
  It is a local, operator-chosen label used only for logging/audit
  grouping. It is never compared against the live server's own
  `serverInfo.name` (M6.5 explicitly rejected adding that check — see
  continuity decision `d316l1`: "User explicitly rejected
  serverInfo.name-based identity checking (self-reported, not trusted
  proof)"). The single-server configuration model means there is no
  binding decision to get wrong today; a future multi-server design
  would need its own explicit identity decision, not an inherited
  assumption from this one.

---

## 3. Environment isolation

Enforced by `registration.rs`'s `isolate_child_environment` (Unix) /
`registration.rs::child_environment_pairs` + `windows_job.rs`'s
`build_environment_block` (Windows) — both read from the SAME fixed
allowlist constant, `CHILD_PROCESS_ENV_ALLOWLIST`.

**Inherited (allowlisted), only if actually set in this process's own
environment — a listed-but-absent variable is never passed, never
passed as an empty string:**

| Variable | Why it is allowlisted |
|---|---|
| `PATH` | Needed one level down — a server that itself shells out to another tool, or is run via a wrapper/interpreter doing its own further `PATH`-based resolution, needs it present in the child's own environment (confirmed by direct reproduction: `cmd /c more` fails resolving `more.com` without it). |
| `SystemRoot` (Windows) | A Python-based server using `asyncio` fails immediately at import time with `WinError 10106` without it (Winsock's own service-provider init depends on it; reproduced directly). |
| `TEMP`/`TMP` (Windows), `TMPDIR` (Unix) | Not required to start, but its absence causes a real server writing temp files to silently fall back to a wrong/non-writable default instead of the user's real temp location. |

**Eliminated — everything else**, including this process's own full
environment, any secret, credential, or API-key-shaped variable the
operator's own shell happens to carry, and anything not on the table
above. `isolate_child_environment`/`child_environment_pairs` start
from a cleared environment (`env_clear()`-equivalent) and add back
only the allowlist, never the reverse (never "clear a denylist").

**Windows-specific behavior (M6.6):** the allowlist is collected as
owned `(name, value)` pairs and encoded by `build_environment_block`
into a `CreateProcessW`-shaped `KEY=VALUE\0...\0\0` wide-character
block — a hand-rolled encoder, because `CreateProcessW` is called
directly (bypassing `tokio::process::Command`, which has no way to
pass `STARTUPINFOEXW`/an attribute list). This is new, Windows-only,
security-sensitive code that M6.6/M6.7 added; see [§6](#6-residual-risks)
for its own residual limitations.

**Known limits (confirmed, not theoretical):**
- `build_environment_block` has no embedded-NUL guard on key/value
  content; an embedded NUL in a value would fracture the block into
  an extra, malformed entry (confirmed structurally by
  `build_environment_block_does_not_let_an_embedded_null_in_a_value_corrupt_the_block_structure`).
  Not reachable today — the only real caller sources values
  exclusively from `std::env::var_os` on the fixed allowlist above,
  and a real OS environment variable cannot itself contain an
  embedded NUL.
- Windows environment-variable name matching is OS-level
  case-insensitive; this crate's own allowlist comparison
  (`std::env::var_os`) matches that OS behavior. Adversarially tested
  (`a_differently_cased_secret_env_var_does_not_leak_through_case_insensitive_matching`,
  M6.7): a secret set under a differently-cased name than any
  allowlist entry does not leak into the spawned server under any
  casing probed.

---

## 4. Windows process containment (M6.6)

### The race this mechanism exists to close

The ordinary pattern — spawn a process (`tokio::process::Command::
spawn()` or a plain `CreateProcessW`), then call
`AssignProcessToJobObject` on the returned handle — has a real window
between the two calls during which the child's own initial thread is
already runnable and can execute attacker-controlled code before any
containment applies. M6.6 was explicitly not authorized to accept that
window as a documented residual risk; it required either closing it
or an explicit STOP.

### Why `CREATE_SUSPENDED` + `ResumeThread` was rejected

That pattern needs the new process's main THREAD handle
(`PROCESS_INFORMATION::hThread`) to resume it. Confirmed by reading
`tokio` 1.54.1's own `src/process/windows.rs` directly:
`tokio::process::Child` (and the `std::process::Child` it wraps)
expose only the PROCESS handle via `AsRawHandle`. The thread handle
`CreateProcessW` returns is created and closed internally inside
`std`'s own spawn implementation and never surfaced to any caller on
stable Rust — `CREATE_SUSPENDED` on that stack would produce a
permanently-suspended, unresumable process.

### The mechanism actually used: creation-time job assignment

`crates/mcp-client/src/windows_job.rs::spawn_contained` replaces
`tokio::process::Command::spawn()` with a raw `CreateProcessW` call
built as follows:

1. `CreateJobObjectW` creates an anonymous job object, then
   `SetInformationJobObject(JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE)`
   configures it so that closing the job handle terminates every
   process still assigned to it — **transitively, the whole process
   tree**, since no `BREAKAWAY_OK`/`SILENT_BREAKAWAY_OK` limit is ever
   set.
2. `CreatePipe` creates the child's stdin/stdout pipes; stderr is an
   inheritable duplicate of this process's own real stderr handle
   (`GetStdHandle(STD_ERROR_HANDLE)` + `DuplicateHandle`), matching
   `Stdio::inherit()`'s real semantic (an M6.7 fix — see
   [§5](#5-m67-findings)).
3. A `STARTUPINFOEXW` is built via `InitializeProcThreadAttributeList`
   / `UpdateProcThreadAttribute` with exactly two attributes:
   - `PROC_THREAD_ATTRIBUTE_JOB_LIST`: the one job handle from step 1.
   - `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`: exactly the three stdio
     handles from step 2 — the child inherits these and nothing
     else, regardless of how many other inheritable handles this
     process holds open (adversarially proven, M6.7 — see
     [§7](#7-security-guarantees-matrix)).
4. `CreateProcessW` is called with `EXTENDED_STARTUPINFO_PRESENT` and
   this attribute list.

**Per Microsoft's own documentation, the kernel assigns the new
process to every job in `PROC_THREAD_ATTRIBUTE_JOB_LIST` as part of
process creation, before the process's initial thread can execute any
code.** There is no separate "assign" step after `CreateProcessW`
returns for an attacker to race against — **the ordinary
`spawn → AssignProcessToJobObject` race this mechanism replaces simply
does not exist on this path**, because there is no window between
"process exists" and "process is in the job" at all.

### Tree-wide containment, proven adversarially

`KILL_ON_JOB_CLOSE` reaches every descendant the contained process
tree spawns, not just the one process this module's own handle refers
to. Proven with a real grandchild process (a Python interpreter
spawning its own child, replacing an earlier `cmd.exe`-scripted
version that turned out never to launch its grandchild at all — see
[§5](#5-m67-findings), finding 4):
`dropping_the_contained_child_terminates_the_whole_process_tree_not_just_the_direct_child`
and `kill_terminates_a_descendant_the_direct_child_already_exited_from`
both confirm the grandchild is terminated before its own 30-second
sleep can complete, whether torn down via `Drop` or via the explicit
async `kill()`.

### Thread/process termination ordering

- `ContainedChild::start_kill` (sync, used from `Drop`): closes the
  job handle directly (firing `KILL_ON_JOB_CLOSE`), then replaces
  `self.job` with a freshly created, never-assigned job object — via
  `mem::replace` + explicit `mem::forget` of the old value, never a
  plain field assignment (which would double-close the handle this
  function just closed itself; an M6.7 fix, finding 3).
- `ContainedChild::kill` (async, used from `McpConnection::close`):
  calls `start_kill()`, then waits (bounded, 5s, never `INFINITE`) for
  the direct process handle to signal via a plain `std::thread::spawn`
  + oneshot channel — deliberately NOT `tokio::task::spawn_blocking`,
  which was found during M6.6's own development to leave tokio's
  blocking pool unable to join cleanly at runtime shutdown on this
  host, even though the blocking call itself always completed
  successfully first (a tokio/Windows-specific interaction, not a bug
  in the wait itself).

---

## 5. M6.7 findings

M6.7 was a no-new-features adversarial audit of M6.6. All fixes stayed
entirely inside `windows_job.rs`; `registration.rs`, `mcp-client-cli/
src`, `mcp-server`, `storage`, `brain-api`, and `audit-chain-core` were
not touched (confirmed by diff at the time, see commit `f3e2ee8`).

**These severities are stated as found, including the Critical one —
a fix being small and locally confined does not retroactively lower
what the bug was.**

1. **Critical — use-after-free.** `PROC_THREAD_ATTRIBUTE_JOB_LIST`'s
   `lpValue` pointed at a temporary `HANDLE` (`&job.raw()`) that was
   already gone by the time `CreateProcessW` read through it.
   Confirmed against Microsoft's own documentation (live research, not
   assumption): `UpdateProcThreadAttribute`'s own reference page states
   `lpValue` "must persist until the attribute list is destroyed using
   the `DeleteProcThreadAttributeList` function" — i.e. the attribute
   list stores the pointer itself, not a copy, and the OS reads
   through it again at `CreateProcessW` time. **Fix:** bind the job
   handle value to a named local that outlives the whole call and the
   attribute list's own `Drop`. **Regression coverage:** the fix is
   exercised by every test that calls `spawn_contained`/
   `spawn_contained_into` (28 of this module's own tests); a targeted
   regression for the exact failure mode was not added as a separate
   test because a use-after-free on a stack slot is not reliably
   observable without external tooling (Application Verifier/a
   sanitizer) — the fix itself is the correction, confirmed by code
   reading against the documented API contract, not by a test that
   could have passed either way.

2. **Medium — broken stderr semantics.** stderr was originally routed
   through a dedicated pipe with the parent's read end closed
   immediately, intended to approximate `Stdio::inherit()` but not
   actually equivalent to it (`inherit()` passes the parent's own real
   stderr handle through; this closed the only reader of a pipe
   instead). The child crashed once its own stderr write filled the
   pipe buffer with no reader — reproduced empirically: a test child
   writing 128KB to stderr then calling `sys.exit(42)` instead
   terminated with exit code 120. **Fix:** an inheritable duplicate of
   `GetStdHandle(STD_ERROR_HANDLE)`, matching real `inherit()`
   semantics (new `Win32_System_Console` Cargo feature). **Regression
   test:** `stderr_writes_do_not_silently_fail_or_hang_the_child_m67_finding`
   — failed before the fix (exit code 120 ≠ 42), passes after.

3. **Low/Medium — double-close.** `ContainedChild::start_kill`'s
   original `self.job = replacement` assignment drops the OLD
   `self.job` value first (Rust's own assignment semantics) — running
   `OwnedJobHandle`'s `Drop`, which calls `CloseHandle` again on a
   handle this function had already closed one statement above. A
   narrow but real wrong-object-closed risk if the handle VALUE had
   been reused by an unrelated `Create*`-style call in the interim
   (documented Windows handle-reuse behavior). **Fix:**
   `mem::replace` + explicit `mem::forget` of the already-closed old
   value, so its own `Drop` never runs. **Regression test:**
   `start_kill_called_twice_does_not_double_close_or_panic_m67_finding`.

4. **Test-validity defect (not a production bug).** Two M6.6-era
   "grandchild containment" tests used a `cmd /c start /b cmd /c
   (more & echo done > "...")` script that — directly confirmed via
   `tasklist` and a throwaway diagnostic test — never actually
   launched `more.com` at all (`cmd.exe`'s own parser rejects the
   parenthesized-group-with-embedded-quotes shape). Both tests were
   therefore passing **vacuously**: "the grandchild's marker never
   appears" was trivially true because there was never a live
   grandchild to kill. This went undetected since M6.6 because the
   resulting `cmd.exe` error landed on stderr, which finding 2's bug
   was silently swallowing at the time. **Fix:** replaced with a
   Python-based script that deterministically spawns a real
   grandchild and writes a start marker immediately, so the test first
   asserts the grandchild genuinely started before asserting anything
   about killing it.

M6.7 also added adversarial coverage with no corresponding bug (the
production code already behaved correctly, confirmed by a passing
test on first write): handle-inheritance restriction against an
artificially-inheritable parent handle, 13-case real argv round-trip
through a spawned child, server-crash-during-discovery/mid-invocation
fail-closed behavior with correct audit evidence, tool-name-casing-
mismatch denial, and 8 unit tests pinning `build_environment_block`'s
exact behavior under adversarial input (Unicode, duplicates, embedded
`=`, embedded NUL).

---

## 6. Residual risks

Each item below is explicitly categorized — **accepted limitation**
(known, will not be fixed without a new design decision),
**deferred future hardening** (a real gap, not required for M6's own
stated scope, left for an explicitly later milestone), or
**unsupported threat model** (outside what this component claims to
defend against at all).

| Risk | Category | Why |
|---|---|---|
| Schema-hash agreement proves interface shape only, never runtime behavior — a malicious server could advertise a matching schema while behaving arbitrarily differently inside its own `tools/call` handler. | **Accepted limitation** | Explicitly disclosed on every real CLI run (`SCHEMA_HASH_SCOPE_DISCLOSURE`). A behavioral-integrity proof would need either sandboxing the tool's actual execution or a formal contract/attestation mechanism neither MCP nor this CLI's own scope provides. Not a gap discovered by M6.7 — re-confirmed accurate and unchanged. |
| Exact-match schema pinning, no rotation or version-negotiation mechanism — a legitimate server updating its own schema requires an operator to re-review and re-pin, with no graceful transition. | **Accepted limitation** | Deliberate, frozen M6.5 design choice (`input_schema_sha256` is exact-match-only by explicit decision). Rotation/negotiation would itself be a new trust decision, not a hardening of the current one. |
| `server.name` is a local operator-chosen label, never checked against the live server's own `serverInfo.name` — carries no identity guarantee. | **Accepted limitation** | Explicitly rejected as a "fix" during M6.5 (self-reported `serverInfo.name` is not trusted proof of anything). The single-server model means nothing today depends on this label disambiguating between two live servers. |
| Unix process containment is NOT equivalent to Windows's Job-Object-based containment — Unix still uses the pre-M6.6 `tokio::process::Command`-based spawn with no process-tree containment (no cgroup/pdeathsig/process-group equivalent added). | **Deferred future hardening** | M6.6 was explicitly scoped to Windows only. A Unix-equivalent mechanism (cgroups, `PR_SET_PDEATHSIG`, or a process group + `setsid`) is a real, separate design decision M6 never made, not a regression from any prior state (there was no containment before M6.6 on either platform). |
| The Windows-specific `CreateProcessW` command-line quoting and environment-block encoding (`quote_arg`/`build_command_line`/`build_environment_block`) are this crate's own hand-rolled reimplementations of MSVCRT/Win32 conventions, not `std::process::Command`'s own battle-tested native argv/env handling. | **Accepted limitation** (with evidence, not blind trust) | Necessary because `CreateProcessW` is called directly (bypassing `tokio::process::Command`, which has no way to pass `STARTUPINFOEXW`). Mitigated, not eliminated: adversarially tested against a real spawned child's own `sys.argv` for 13 shapes (spaces, embedded quotes, trailing backslashes, empty args, Unicode, newlines, option-injection-shaped strings) — all round-trip correctly. Remains security-sensitive custom code that any future change to this module must re-verify against the same real-process round-trip, not just unit-test the encoding function in isolation. |
| `build_environment_block` has no embedded-NUL guard on key/value content — would corrupt the block's own structure (fracture one entry into two, shifting every subsequent variable) if ever fed such a value. | **Unsupported threat model for this function's own current real caller** (would be a real gap for any FUTURE caller with less-trusted input) | Not reachable today: the only real caller (`registration.rs`'s `child_environment_pairs`) sources every value exclusively from `std::env::var_os` against the fixed allowlist in §3, and a real OS environment variable cannot itself contain an embedded NUL. Confirmed structurally corrupting, not guarded, by a dedicated M6.7 unit test — documented rather than defensively patched, per this milestone's own instruction to fix only demonstrated-reachable issues. |
| No CPU/memory/handle-count resource limits are configured on the Job Object beyond `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` (no `JOB_OBJECT_LIMIT_PROCESS_MEMORY`, `ActiveProcessLimit`, CPU rate limit, etc.). | **Deferred future hardening** | M6.6's own milestone text named `KILL_ON_JOB_CLOSE`/tree containment specifically; no other resource limit was named as in-scope, and none was added speculatively. A genuinely malicious or runaway external server can still consume unbounded CPU/memory for as long as it runs (it will still be reliably terminated on connection close/teardown). |

(`windows_job.rs`'s test-suite concurrency flake, previously listed
here as a deferred-hardening residual risk, is now a **resolved**
finding — see [§6a](#6a-resolved-finding-windows_jobrs-test-suite-concurrency-flake)
immediately below, not this table.)

---

## 6a. Resolved finding: `windows_job.rs` test-suite concurrency flake

This finding was open at the previous version of this document (which
recorded a CONDITIONAL PASS over it) and is now closed.

**Original symptom:** `windows_job.rs`'s own test suite was
intermittently flaky under `cargo test`'s default (unbounded)
parallelism on this 20-logical-CPU host — ~25-35% of runs failed with
`ERROR_INVALID_HANDLE` on one or more real-process-spawning tests,
sometimes cascading to several tests in the same run once one failed.

**Investigation path (summarized; the full trail is in the two
relevant commits' own messages):**
1. Bisected across `--test-threads` values: 0/18 failures at
   `--test-threads≤12`, rising to 1/6 at 16 and 2/6 at 20 — a gradual,
   probability-increasing pattern (not a sharp cutoff), consistent
   with a genuine race rather than a hard resource ceiling.
2. Added `SPAWN_SERIALIZATION_LOCK`, reasoning from a real, Microsoft-
   documented Windows handle-table-mutation race class (`CreatePipe`
   creates both pipe ends inheritable, then a later call narrows one
   back — a brief window Raymond Chen's own canonical guidance
   describes). Measured directly: still ~5/15 runs failing with the
   lock in place — a real mitigation for a real but different race,
   contributing nothing measurable to THIS flake.
3. Replaced plain `eprintln!`-based diagnostics (which interleave
   mid-line under real concurrent process output and produced one
   misleading combined-looking line earlier in the investigation)
   with a mutex-protected, thread-and-sequence-tagged log that prints
   each failing Win32 call's own exact handle values atomically.
4. The clean log made the root cause unambiguous: two concurrently-
   running, otherwise-unrelated tests' diagnostic output showed the
   IDENTICAL raw handle value — one attributed to a job handle, the
   other to a stdin pipe handle, from two different threads' own
   calls.

**Root cause:** the test helper
`OwnedJobHandle::already_closed_for_test` created a real job object
via `CreateJobObjectW` and immediately closed it, returning the
now-dangling handle VALUE to simulate "an invalid job handle" for one
specific adversarial test
(`job_setup_failure_in_the_real_spawn_path_prevents_the_untrusted_command_from_ever_running`).
Under heavy concurrent handle churn, a DIFFERENT thread's own
legitimate `CreatePipe`/`CreateJobObjectW` call reliably reused that
exact just-freed handle value for its own, completely unrelated real
object before the first test's own `spawn_contained_into` call ran.
This is real, documented Windows handle-value-reuse behavior
colliding with a test fixture that manufactured a dangling value on
purpose — **not** a bug in `spawn_contained`'s own single-call
correctness, **not** a kernel-level race in `CreateProcessW`/
`PROC_THREAD_ATTRIBUTE_JOB_LIST` itself, and **not** antivirus/EDR
interference (both explicitly considered and measured against before
this root cause was isolated).

**Fix:** `already_closed_for_test` now returns
`OwnedJobHandle::placeholder()` — a `NULL` handle, a value the OS
handle allocator never assigns to a real object — instead of a real
create-then-close round trip. No handle value is ever freed back into
contention. `SPAWN_SERIALIZATION_LOCK` was removed after confirming
(20 consecutive clean runs with it present, 20 more consecutive clean
runs with it absent) that it contributed nothing measurable once the
real bug was fixed — kept, it would have been unexplained,
unjustified serialization of code that must behave identically under
concurrent and sequential callers.

**Scope of the fix:** confined entirely to one test helper's own
handle lifetime inside `crates/mcp-client/src/windows_job.rs`. No
production Job Object mechanism, `CreateProcessW` call shape,
`PROC_THREAD_ATTRIBUTE_JOB_LIST` usage, authorization/schema/audit
semantics, M6.4/M6.5 behavior, or public API changed — confirmed by
`git diff --stat`, one file touched.

**Verification:** 25 consecutive runs of `cargo test -p mcp-client
--lib windows_job::` with NO `--test-threads` override (this host's
own default, unbounded parallelism) — 0 failures. Full workspace test
suite (`cargo test --workspace`, default parallelism) run 4 times
consecutively — 0 failures each time. Zero clippy warnings on
`mcp-client`. No test ignored, no retry logic used to mask a failure,
no arbitrary sleep used as stabilization.

---

## 7. Security guarantees matrix

| Area | Guarantee | Evidence | Residual risk |
|---|---|---|---|
| Discovery | `discover_tools` validates every `tools/list` reply before caching anything — empty name, duplicate name, path-traversal-shaped name, or a wholesale malformed list are all rejected, never partially trusted. A connection/handshake failure or a malformed reply is durably recorded (`DiscoveryOutcomeKind::Failed`/`Timeout`) and the CLI exits 1 before any later phase runs. | `discovery.rs` unit tests (`a_malformed_list_is_rejected_wholesale_not_as_a_partial_subset`, etc.); `mcp-client-cli`'s `a_malformed_tools_list_reply_fails_closed_before_authorization_or_invocation`; M6.7's `a_server_that_crashes_during_discovery_fails_closed_and_records_no_tool_classification_or_authorization` (real process crash mid-handshake). | Discovery proves only "the server answered this one `tools/list` call this way" — a server that answers differently on a hypothetical second discovery call is out of scope (this CLI never makes a second discovery call per run). |
| Schema verification | `input_schema_sha256` is computed from the live, discovered schema (recursive, order-stable canonicalization) and compared exact-match, case-insensitive-hex only, against the operator's pinned hash, strictly before authorization. A mismatch denies before `authorize()` or `tools/call` ever run, with the specific reason in stderr and zero `tools/call` output on stdout. | `identity.rs` hash/canonicalization unit tests; `mcp-client-cli`'s `a_tool_whose_live_schema_differs_from_the_pinned_hash_is_denied_before_tools_call` (real mismatched server) and `schema_verification_runs_before_authorization_and_before_any_tools_call_dispatch`. | See §6: interface-shape proof only, never behavioral proof; exact-match only, no rotation. |
| Authorization | `authorize(metadata, policy)` is pure and local (no I/O); derives nothing from server-supplied/discovered data; a denial under every tested policy variant (`Deny`, `AllowReadOnly`, `AllowScopedExecution`, `AskApprovalRequired`) returns before the transport is touched. | `permission.rs`'s 15 unit tests, including `authorize_takes_no_discovered_tool_and_derives_nothing_from_server_supplied_metadata` and `ask_approval_required_never_succeeds_through_authorize_alone_for_any_classification`; `invoke.rs`'s `a_denied_tool_returns_permission_denied_without_ever_calling_the_real_tool` (real connection, proven no network call happens). | None identified beyond §6's general scope notes — this is the most directly and narrowly tested guarantee in the whole system. |
| Invocation ordering | The full sequence — config load → discovery → schema verification → authorization → invocation attempt → invocation outcome — runs in that exact order every time; a failure at any step prevents every later step's own side effects and audit evidence from ever appearing. | `schema_verification_runs_before_authorization_and_before_any_tools_call_dispatch`; M6.7's two real-crash tests (discovery-phase and invocation-phase) each confirm no evidence of an unreached later phase. | None identified for the single-server, single-invocation-per-run model this CLI implements; a future multi-invocation or re-discovery design would need its own explicit ordering proof. |
| Audit durability | Every phase's outcome is written through the real `storage`-backed, hash-chained, append-only audit log (never an in-memory stand-in for these tests) — including on denial/failure paths, not only on success. A durably-persisted chain verifies cleanly (`scan_chain`) after every scenario tested, including real process crashes and reopening the same vault across two separate CLI process runs. | `mcp-client-cli`'s `reopening_the_same_audit_vault_across_two_cli_runs_preserves_the_verified_hash_chain`, `the_real_vault_backed_writer_preserves_the_same_sequence_and_hash_chain_contract...`, every acceptance test's own `scan_chain` assertion; `audit-chain-core`'s 33 unit tests (tamper/truncation/duplicate-sequence/malformed-entry detection, pre-existing, unmodified by M6, reconfirmed passing). | Crash-during-append (not merely crash-during-invocation with the audit write itself succeeding) is covered at `audit-chain-core`'s own unit level (pre-existing), not by a new M6-specific real-crash-mid-write acceptance test — not newly introduced by M6, and not in M6.7's own explicit scope (M6.7 targeted M6.6's containment code, not the audit pipeline, which M6.6/M6.7 never touch). |
| Environment isolation | Exactly the fixed allowlist (`PATH`/`SystemRoot`/`TEMP`/`TMP`/`TMPDIR`) reaches the spawned server, only if actually set in this process; a real secret-shaped variable (any casing) never does. | `mcp-client`'s `adversarial.rs`: `a_parent_sentinel_env_var_is_not_visible_to_the_spawned_external_server`, `an_allowlisted_bootstrap_variable_remains_visible_to_the_spawned_external_server`, `a_differently_cased_secret_env_var_does_not_leak_through_case_insensitive_matching` (M6.7) — all against a real spawned server via the real M6.6 spawn path. | Embedded-NUL block-structure corruption is unguarded but unreachable today (§6). |
| Handle inheritance | The spawned child inherits exactly its own 3 stdio handles via `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`, never any other handle this process holds open, even one deliberately marked inheritable. | M6.7's `an_artificially_inheritable_handle_open_in_the_parent_is_not_accessible_from_the_child` — a real adversarial probe via `ReadFile` through `ctypes` from inside a real spawned child, against a handle this test deliberately made inheritable, confirming it is NOT accessible. | None identified for the 3-handle, single-process-per-call model this module implements. |
| Windows process containment | The spawned process is assigned to a `KILL_ON_JOB_CLOSE` job object at `CreateProcessW` time itself (no post-spawn `AssignProcessToJobObject` step exists to race). | `the_spawned_process_is_a_member_of_its_own_job` (`IsProcessInJob` against the real job handle); `job_setup_failure_in_the_real_spawn_path_prevents_the_untrusted_command_from_ever_running` (a real injected job-handle failure proven to prevent the untrusted command line from ever executing, via a marker file that never appears). | None identified for the ordering claim itself; see §6 for the Critical use-after-free this guarantee's own implementation originally had (fixed, M6.7) and for resource-limit scope. |
| Process-tree cleanup | Closing the job handle (via explicit `close()`/`kill()` or via `Drop`) terminates the entire contained process tree, including a grandchild the direct child has already exited from — not merely the one process this module's own handle refers to. | `dropping_the_contained_child_terminates_the_whole_process_tree_not_just_the_direct_child` and `kill_terminates_a_descendant_the_direct_child_already_exited_from` — both against a real grandchild process, confirmed to have actually started (M6.7 fix, finding 4) before confirming it was killed. | Unix has no equivalent tree-wide containment (§6); no resource limits beyond kill-on-close (§6). |

---

## 8. Acceptance evidence

All counts below are from real test runs executed during this M6.8
closure session, on this Windows host, with Python available on
`PATH` (confirmed — no test's own `skip_if_no_python`/`no python on
PATH` branch fired; every Python-dependent test genuinely ran). No
number here is carried over from an earlier milestone's own report
without being re-executed now.

| Suite | Tests | Result |
|---|---|---|
| `audit-chain-core` lib | 33 | 33 passed |
| `mcp-client` lib (includes `windows_job`'s 28) | 80 | 80 passed |
| `mcp-client` `tests/adversarial.rs` | 7 | 7 passed |
| `mcp-client` `tests/e2e_external_tool.rs` | 6 | 6 passed |
| `mcp-client` `tests/separation.rs` | 5 | 5 passed |
| `mcp-client-audit-worker` lib | 13 | 13 passed |
| `mcp-client-audit-worker` `tests/dependency_boundary.rs` | 3 | 3 passed |
| `mcp-client-audit-worker` `tests/no_block_on.rs` | 1 | 1 passed |
| `mcp-client-audit-worker` `tests/worker_integration.rs` | 15 | 15 passed |
| `mcp-client-cli` lib (`main.rs` unit tests) | 32 | 32 passed |
| `mcp-client-cli` `tests/acceptance.rs` | 18 | 18 passed |
| **Total** | **213** | **213 passed, 0 failed, 0 ignored** |

Run command for the result above, with NO `--test-threads` override
(this host's own default, unbounded parallelism):
`cargo test -p mcp-client -p mcp-client-cli -p mcp-client-audit-worker
-p audit-chain-core`. The `windows_job.rs` concurrency flake that
previously required a bounded `--test-threads` value for a reliable
result is resolved (§6a) — this run uses the plain default.

Each guarantee in §7 is backed by at least one specific test named in
its own Evidence column, all included in the 213 above.

---

## 9. Final verification

- **Full workspace build:** `cargo build --workspace` — clean, no
  errors, re-run at M6.8 closure time.
- **Full workspace tests:** `cargo test --workspace` (default
  parallelism, no `--test-threads` override anywhere) — run 4 times
  consecutively during this closure update, 0 failures each time. One
  pre-existing, unrelated `#[ignore]`d test found
  (`crates/api/examples/b3_metric_runner.rs`, reads a real held-out
  eval dataset, explicitly self-documented, outside M6's own scope
  entirely) — not a hidden M6 skip. `windows_job`'s own former
  concurrency flake (§6a) is resolved; its own suite was additionally
  run 25 consecutive times in isolation at default parallelism with 0
  failures, for the count in §8.
- **Clippy on touched crates:** `cargo clippy -p mcp-client -p
  mcp-client-cli -p mcp-client-audit-worker -p audit-chain-core
  --all-targets -- -D warnings` — zero warnings.
- **Working tree:** clean immediately before this document's own
  update commit.
- **No unexpected files modified:** confirmed by `git diff --stat`
  scoped per commit — M6.6 touched only `mcp-client/{Cargo.toml,
  src/lib.rs, src/registration.rs, src/windows_job.rs}`; M6.7 touched
  only `mcp-client/{Cargo.toml, src/windows_job.rs, tests/
  adversarial.rs}` and `mcp-client-cli/tests/acceptance.rs`; M6.8's
  two code commits (`10c3757`, `7930f11`) touched only
  `mcp-client/src/windows_job.rs`. `registration.rs`'s non-Windows
  branch, `mcp-client-cli/src/*`, `mcp-server`, `storage`,
  `brain-api`, and `audit-chain-core` were never modified by
  M6.6/M6.7/M6.8.
- **Commit ancestry M6.4 → M6.8**, confirmed present on `main` in
  order via `git log --oneline`:
  ```
  7930f11 fix(mcp-client): M6.8 follow-up - real root cause of windows_job test flakiness found and fixed
  0970a7a docs: M6.8 - M6 closure documentation (trust model, containment mechanism, guarantees matrix, CONDITIONAL PASS)
  10c3757 test(mcp-client): M6.8 - diagnose and document windows_job test-suite concurrency flake
  f3e2ee8 security(mcp-client): M6.7 - adversarial audit of M6.6 Job Object containment, 3 real findings fixed
  debfa65 feat(mcp-client): M6.6 - Windows creation-time Job Object containment for spawned MCP servers
  01c81b4 test(mcp-client-cli): M6.5 - targeted verification of input_schema_sha256 and explicit behavioral-integrity scope disclaimer
  9b12551 fix(mcp-client-cli): M6.4-S1 - executable identity TOCTOU security review and Windows ACL-based trusted-location mitigation
  a1ab624 feat(mcp-client-cli): M6.4 - supervised MCP Client composition root & real durable audit integration
  ```
  (This document's own update commit, created immediately after this
  file, is the next commit in this ancestry.)
- **No security-critical test is `ignored`/conditionally skipped
  without documentation:** confirmed directly — the one workspace
  `#[ignore]` relevant to any M6-adjacent crate is `storage::
  audit_writer`'s `__lock_holder_child_helper`, a documented test
  HELPER (not a skipped assertion) that its own parent test explicitly
  re-invokes as a child process; that parent test itself ran and
  passed in this session's own verification. Every M6-crate test that
  conditionally skips does so only on "no Python interpreter on
  `PATH`" (`skip_if_no_python`), printing an explicit message when it
  does — and none did during this closure's own verification runs
  (confirmed by grepping for "skipping" in the real test output: zero
  matches).
- **Windows-only suite actually executed on Windows:** confirmed — the
  entire session (M6.6 through this closure) ran on this Windows 11
  host; every `windows_job.rs` test, every M6.7 adversarial test, and
  every `mcp-client-cli` acceptance test ran as real `cmd.exe`/
  `python.exe`/`powershell.exe` child processes under real
  `CreateProcessW`/Job Object calls on this real host, not emulated or
  cross-compiled.

---

## 10. Closure decision

### PASS

M6 is complete: all stated acceptance criteria for M6.4-M6.7 are met,
all three M6.7 findings are fixed and covered by regression tests,
the trust model and every frozen invariant are documented and
re-verified intact, and 213 real tests pass with zero clippy warnings
across every M6-related crate — **at this host's own default,
unbounded test parallelism, with no `--test-threads` override
anywhere**, confirmed by 25 consecutive clean runs of `windows_job.rs`
in isolation and 4 consecutive clean full-workspace runs.

The condition recorded in the previous version of this document (a
bounded-concurrency requirement for `windows_job.rs`'s own test
suite) is resolved, not waived: the actual root cause was found (a
test helper manufacturing a dangling handle value that collided with
real concurrent handle allocation — §6a) and fixed at the source, not
worked around with a weaker acceptance bar, a retry, or a sleep.

**No Critical/High finding requiring a redesign of M6.4-M6.6's
architecture emerged during M6.7 or M6.8.** The one Critical finding
(M6.7, §5.1) was a real use-after-free, but its fix was fully local to
`windows_job.rs` and required no change to the creation-time
Job-Object-assignment mechanism itself, the trust model, the
invocation ordering, or any frozen M6.4/M6.5 invariant. The §6a
concurrency finding was a test-infrastructure bug, confined to one
test helper, with zero production-code or public-API change.

---

## 11. Separate proposal: tag/merge (not executed)

No push, tag, merge, or branch change was performed as part of this
closure, per instruction. If the operator wants a durable marker for
this closure point, a lightweight tag at the current `main` HEAD
(`7930f11`) — e.g. `git tag m6-mcp-client-cli-pass 7930f11` — would
mirror this repository's own existing convention
(`b5-brain-api-mcp-complete` exists for B5's own closure). This is a
proposal only; it has not been created, and `main` has not been
pushed as part of this work.
