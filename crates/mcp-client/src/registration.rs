//! B5.8 Task 26: MCP client connection lifecycle and server registration.
//!
//! `connect()` is this crate's ONE entry point to reaching an external MCP
//! server at all. It enforces, in this order, before any transport is ever
//! touched:
//!   1. the global kill-switch (`global_client_enabled()`) — short-circuits
//!      every registration regardless of its own `enabled` flag;
//!   2. the registration's own `enabled` flag;
//!   3. a timeout around the whole connect-and-handshake attempt, so a
//!      hung external server returns `McpClientError::Timeout` rather than
//!      blocking forever.
//!
//! What `connect()` does NOT do (deliberately out of this task's scope,
//! left to later tasks): discover the server's tools (Task 27), authorize
//! or classify what it's allowed to do (Task 28), invoke anything on it
//! (Task 29). A successful `connect()` only proves the external process
//! spawned and completed the MCP `initialize` handshake — nothing about
//! what it may be asked to do afterward.

use std::time::Duration;

use rmcp::ServiceExt;
use rmcp::transport::async_rw::AsyncRwTransport;

use crate::McpClientError;

/// B5 remediation M2: bounds a single JSON-RPC line this crate will ever
/// accept from an external MCP server's stdout, before that line is ever
/// parsed or handed to any `brain-api`/policy code. Mirrors
/// `brain_api::limits::MCP_MAX_PAYLOAD_BYTES`'s own documented value
/// (1 MiB) BY CONVENTION, never by a shared import — this crate must
/// never depend on `brain-api` at all (the frozen B5.10 isolation,
/// `tests/separation.rs`), the exact same "same value, independently
/// defined, pinned by a cross-crate regression test" pattern this
/// crate's `lib.rs` already established for `DEFAULT_CONNECT_TIMEOUT`/
/// `DEFAULT_INVOKE_TIMEOUT` against `brain_api::limits::DEFAULT_TIMEOUT_MS`
/// (see `lib.rs`'s own `mcp_clients_default_timeouts_still_match_brain_apis_canonical_value`).
///
/// Without this, `rmcp`'s own upstream default (`AsyncRwTransport`'s
/// built-in 16 MiB `DEFAULT_MAX_LINE_LENGTH`) would apply instead — a
/// malicious or misbehaving external MCP server could otherwise send up
/// to 16 MiB in one line before this crate's own choice ever took
/// effect (the residual limitation `mcp-server`'s own `limits.rs` module
/// doc comment already names from the server side of this same crate
/// boundary).
pub const MCP_MAX_RESPONSE_LINE_BYTES: usize = 1_048_576;

const _: () = assert!(MCP_MAX_RESPONSE_LINE_BYTES > 0);
// Mirrors brain_api::limits's own compile-time assertion that its choice
// is deliberately smaller than rmcp's 16 MiB built-in default, never
// larger (a larger value here would silently fall back to relying on
// the upstream default instead of this crate's own stated choice).
const _: () = assert!(MCP_MAX_RESPONSE_LINE_BYTES < 16 * 1024 * 1024);

/// The env var this crate reads for the global kill-switch, per the design
/// doc's "client deve poter essere disabilitato globalmente" (the client
/// must be globally disableable). Any value other than exactly `"0"` means
/// disabled — including an empty string, `"false"`, or a typo — so an
/// operator's mistake fails CLOSED (no connection attempted) rather than
/// open. Unset means enabled (the default, matching every other
/// capability in this plan's own fail-closed-by-ABSENT-policy convention
/// turned the other way: here, ABSENCE of this switch means "proceed",
/// which is correct because disabling is an explicit operator action, not
/// a permission grant).
const GLOBAL_DISABLE_ENV_VAR: &str = "SUPERBRAIN_MCP_CLIENT_DISABLED";

/// The default timeout for a `connect()` attempt (spawn + MCP `initialize`
/// handshake). Generous enough for a real external MCP server's cold
/// start (e.g. an `npx`-launched one resolving/installing a package) while
/// still bounded — a caller needing a different value uses
/// [`connect_with_timeout`].
pub(crate) const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// Reads the one global kill-switch this crate's `connect()` consults
/// before anything else. A single config read, not cached — re-read on
/// every call, so a change takes effect on the very next connection
/// attempt without requiring a process restart.
pub fn global_client_enabled() -> bool {
    match std::env::var(GLOBAL_DISABLE_ENV_VAR) {
        Ok(value) => value == "0",
        Err(_) => true,
    }
}

/// How to reach one external MCP server. Only one shape exists today:
/// spawning it as a child process and speaking MCP over its stdio (the
/// standard way to run a local MCP server, matching `mcp-server`'s own
/// `transport-io` choice from the other direction, Task 21). A
/// network-reachable transport is deliberately not added here — nothing
/// in this plan's scope calls for SuperBrain to dial an external MCP
/// server over the network, and adding one would need its own explicit
/// decision, not a silent variant addition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportConfig {
    ChildProcess { command: String, args: Vec<String> },
}

/// One external MCP server SuperBrain knows how to reach. `enabled: false`
/// means `connect()` never attempts a connection for this registration at
/// all — not "connects but then refuses to use it," an actual no-op,
/// confirmed by the no-spawn test below.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServerRegistration {
    pub name: String,
    pub transport: TransportConfig,
    pub enabled: bool,
}

/// A live connection to one external MCP server: the `RunningService`
/// handle for the client-role session, plus the spawned child process
/// itself. Holds no capability/authorization state of its own (Task 28's
/// job) — this is purely "the handshake succeeded and a session exists,"
/// nothing about what it may be asked to do.
///
/// Holds `child` directly (B5 remediation M2) rather than relying on
/// `rmcp::transport::TokioChildProcess`'s own cleanup guard, since
/// `connect_now` no longer constructs one (it needs `AsyncRwTransport`'s
/// `with_max_line_length`, which `TokioChildProcess` has no way to
/// expose) — this struct now carries the exact same "kill the spawned
/// process on close" responsibility explicitly that `TokioChildProcess`'s
/// `ChildWithCleanup` carried implicitly before.
pub struct McpConnection {
    // `Option`-wrapped so `close`'s consuming `.cancel().await` can take
    // ownership of the inner `RunningService` while `McpConnection`
    // itself keeps a `Drop` impl (required for the child-process cleanup
    // below) -- a type with a `Drop` impl cannot otherwise be partially
    // moved out of by value.
    service: Option<rmcp::service::RunningService<rmcp::RoleClient, ()>>,
    // M6.6 (Windows only): the spawned external server's process is
    // job-contained (`windows_job::ContainedChild`) rather than a plain
    // `tokio::process::Child` -- see `connect_now`'s own doc comment
    // for why. Unix keeps the prior `tokio::process::Child`-based
    // spawn (Job Objects are a Windows-only mechanism; M6.6 added no
    // equivalent containment for other platforms, by explicit scope).
    #[cfg(windows)]
    child: crate::windows_job::ContainedChild,
    #[cfg(not(windows))]
    child: tokio::process::Child,
}

impl McpConnection {
    /// Read-only accessor for later tasks (discovery/invocation) to reach
    /// the underlying session. Not used by this task itself.
    pub fn service(&self) -> &rmcp::service::RunningService<rmcp::RoleClient, ()> {
        self.service.as_ref().expect("service is only taken by close(), which consumes self")
    }

    /// Closes the connection, shutting down the underlying transport and
    /// killing the spawned child process — the same two-step guarantee
    /// `TokioChildProcess`'s own cleanup previously gave for free.
    pub async fn close(mut self) -> Result<(), McpClientError> {
        let service = self.service.take().expect("service is only taken once, here");
        let cancel_result = service.cancel().await.map_err(|e| McpClientError::ConnectionFailed(e.to_string())).map(|_| ());
        let _ = self.child.kill().await;
        cancel_result
    }
}

impl Drop for McpConnection {
    /// A dropped (never explicitly `close()`d) connection must not leak
    /// its spawned child process — `tokio::process::Child` does NOT kill
    /// its child on drop by default (the exact reason `rmcp`'s own
    /// `TokioChildProcess` wraps it in `ChildWithCleanup` in the first
    /// place). `start_kill()` is synchronous/non-blocking (unlike
    /// `kill().await`), the only option available from a non-async
    /// `Drop::drop`; it sends the kill signal without waiting for exit,
    /// matching `ChildWithCleanup::drop`'s own pattern of spawning a
    /// detached task for the actual async wait rather than blocking drop
    /// itself.
    fn drop(&mut self) {
        // `tokio::process::Child::start_kill` (Unix) returns
        // `io::Result<()>`; `windows_job::ContainedChild::start_kill`
        // (Windows) returns `()` directly -- both discarded here, the
        // same "best-effort, Drop cannot propagate an error" intent on
        // either platform.
        #[cfg(not(windows))]
        let _ = self.child.start_kill();
        #[cfg(windows)]
        self.child.start_kill();
    }
}

/// Connects to `registration`'s external MCP server using
/// [`DEFAULT_CONNECT_TIMEOUT`]. See [`connect_with_timeout`] for the full
/// enforcement order (global switch, then per-registration `enabled`,
/// then timeout).
pub async fn connect(registration: &McpServerRegistration) -> Result<McpConnection, McpClientError> {
    connect_with_timeout(registration, DEFAULT_CONNECT_TIMEOUT).await
}

/// Same as [`connect`], with an explicit timeout instead of the default.
/// Enforcement order, every check running BEFORE the next is ever reached
/// (never racing, never short-circuited out of order):
///   1. [`global_client_enabled`] — `false` denies every registration,
///      regardless of its own `enabled` flag or transport.
///   2. `registration.enabled` — `false` denies this one registration; no
///      process is spawned, no transport is touched.
///   3. The spawn-and-handshake attempt itself, raced against `timeout` —
///      exceeding it returns [`McpClientError::Timeout`], never a hang.
pub async fn connect_with_timeout(registration: &McpServerRegistration, timeout: Duration) -> Result<McpConnection, McpClientError> {
    if !global_client_enabled() {
        return Err(McpClientError::Disabled);
    }
    if !registration.enabled {
        return Err(McpClientError::Disabled);
    }

    match tokio::time::timeout(timeout, connect_now(registration)).await {
        Ok(result) => result,
        Err(_) => Err(McpClientError::Timeout),
    }
}

/// Spawns `registration`'s external MCP server and builds a transport
/// over its real stdio, bounded by [`MCP_MAX_RESPONSE_LINE_BYTES`] on the
/// READ side (what this crate accepts from the external server) — the
/// enforcement point closest to the stream read itself, inside
/// `AsyncRwTransport`'s own line-buffering loop, which never lets a
/// partial line grow past the configured limit before rejecting it (the
/// same mechanism and the same upstream type `mcp-server`'s own
/// `limited_transport` already uses for the reverse direction).
///
/// Deliberately does NOT use `rmcp::transport::TokioChildProcess`/
/// `TokioChildProcessBuilder`: that convenience wrapper builds its own
/// `AsyncRwTransport` internally with no way to configure
/// `max_line_length` (confirmed by reading `rmcp` 3.11.0's own source —
/// the builder's `.stdin()`/`.stdout()`/`.stderr()` methods only
/// configure `std::process::Stdio`, and the constructed `transport`
/// field is private), so this function spawns the child process itself
/// and builds the SAME transport type directly, with the one additional
/// call `TokioChildProcess` has no way to expose. This loses no
/// functionality this crate's existing code relies on: `McpConnection`
/// never calls `TokioChildProcess::graceful_shutdown()`/`.id()` today
/// (`close()` uses the underlying `RunningService::cancel()` only), and
/// `AsyncRwTransport<RoleClient, _, _>` satisfies the same
/// `IntoTransport`/`Transport<RoleClient>` bound `ServiceExt::serve`
/// already requires, via `rmcp`'s own blanket impl.
///
/// Also applies [`isolate_child_environment`] (B5 remediation M3) to
/// `cmd` before spawning: the external server's child process gets
/// [`CHILD_PROCESS_ENV_ALLOWLIST`] only, never this process's full
/// environment.
/// B5 remediation M3: the bootstrap environment variables this crate
/// passes to every spawned external MCP server — an explicit, minimal
/// allowlist, never the parent process's full environment. Each entry
/// is here because its ABSENCE causes a real, reproducible failure or
/// degradation for a realistic external server (verified directly on
/// this project's supported Windows target, never assumed):
///
/// - `SystemRoot` (Windows only): without it, a Python-based external
///   server using `asyncio` (the standard way to implement concurrent
///   I/O, and a realistic shape for any real MCP server implementation)
///   fails immediately at import time with `OSError: [WinError 10106]`
///   — Winsock's own service-provider initialization depends on it.
///   Reproduced directly: `python -c "import asyncio; asyncio.run(...)"`
///   under a fully cleared environment fails with exactly this error;
///   re-adding only `SystemRoot` fixes it.
/// - `TEMP`/`TMP` (Windows; `TMPDIR` is the POSIX equivalent, included
///   for the same reason on Unix even though this crate's own test host
///   is Windows-only): not required for a spawned server to start at
///   all, but its absence causes a real external server that writes
///   temporary files (e.g. via Python's `tempfile` module) to silently
///   fall back to a hardcoded, possibly-wrong or non-writable default
///   directory instead of the user's real temp location — a functional
///   degradation this allowlist avoids rather than accepting.
/// - `PATH`: NOT needed for THIS crate's own top-level executable
///   resolution (`std::process::Command`, which `tokio::process::
///   Command` wraps, resolves the first executable using the CALLING
///   process's own `PATH`, confirmed by direct reproduction — a bare
///   command name still resolves correctly under an otherwise fully
///   cleared environment). It IS needed one level down: reproduced
///   directly that `cmd /c more` — `cmd.exe`'s OWN internal resolution
///   of `more` as `more.com` — fails ("'more' non è riconosciuto...")
///   without `PATH` present in the CHILD's environment, because that
///   resolution is `cmd.exe`'s own process doing a `PATH`-based lookup,
///   a different step from the one that found `cmd.exe` itself. Any
///   real external MCP server that itself shells out to another tool,
///   or is launched via a wrapper/interpreter that does its own further
///   `PATH`-based resolution, needs this for the same reason.
///
/// This list is deliberately small and general-purpose (process
/// bootstrap only) — it is NOT a per-server configuration mechanism and
/// must not become one without a new, explicit design decision; no
/// credential or secret-shaped variable is ever a candidate for this
/// list, by construction (this crate reads no such variable from any
/// external server's needs today).
const CHILD_PROCESS_ENV_ALLOWLIST: &[&str] = &["PATH", "SystemRoot", "TEMP", "TMP", "TMPDIR"];

/// Configures `cmd` to run with an explicit, minimal environment instead
/// of inheriting the parent process's own — every spawned external MCP
/// server gets exactly [`CHILD_PROCESS_ENV_ALLOWLIST`]'s variables (only
/// the ones actually set in THIS process's own environment; a listed but
/// absent variable is simply not passed, never passed as an empty
/// string), nothing else. This never reads, logs, or prints any
/// variable's VALUE outside of passing it directly to the child process
/// argument list `std::process::Command` itself manages.
///
/// Unix-only (M6.6): the Windows `connect_now` below uses
/// `windows_job::spawn_contained`/`child_environment_pairs` instead,
/// which needs the same allowlisted values collected as owned pairs
/// rather than applied to a `tokio::process::Command` builder.
#[cfg(not(windows))]
fn isolate_child_environment(cmd: &mut tokio::process::Command) {
    cmd.env_clear();
    for name in CHILD_PROCESS_ENV_ALLOWLIST {
        if let Some(value) = std::env::var_os(name) {
            cmd.env(name, value);
        }
    }
}

/// M6.6 (Windows only): the env allowlist as owned `(name, value)` pairs,
/// the shape `windows_job::spawn_contained` takes — same values
/// [`isolate_child_environment`] passes to `tokio::process::Command` on
/// every other platform, just collected instead of applied to a
/// `Command` builder (which `connect_now`'s Windows path does not use).
#[cfg(windows)]
fn child_environment_pairs() -> Vec<(String, String)> {
    CHILD_PROCESS_ENV_ALLOWLIST.iter().filter_map(|name| std::env::var_os(name).map(|value| (name.to_string(), value.to_string_lossy().into_owned()))).collect()
}

#[cfg(windows)]
async fn connect_now(registration: &McpServerRegistration) -> Result<McpConnection, McpClientError> {
    let TransportConfig::ChildProcess { command, args } = &registration.transport;

    // M6.6: creation-time Job Object containment replaces
    // `tokio::process::Command::spawn()` on Windows -- see
    // `windows_job`'s own module doc comment for why the ordinary
    // spawn-then-assign pattern (and the `CREATE_SUSPENDED` alternative)
    // were both rejected in favor of `PROC_THREAD_ATTRIBUTE_JOB_LIST`.
    // `cwd` is passed explicitly as this process's own current directory
    // to match `tokio::process::Command`'s default (it inherits the
    // parent's cwd when none is set via `.current_dir()`, which this
    // crate's prior Windows behavior never called).
    let cwd = std::env::current_dir().map_err(|e| McpClientError::ConnectionFailed(e.to_string()))?;
    let env = child_environment_pairs();
    let (mut child, stdio) = crate::windows_job::spawn_contained(std::path::Path::new(command), args, &cwd, &env).map_err(|e| McpClientError::ConnectionFailed(e.to_string()))?;
    let (stdin, stdout) = stdio.into_async_io();

    let transport = AsyncRwTransport::new_client(stdout, stdin).with_max_line_length(MCP_MAX_RESPONSE_LINE_BYTES);

    let service = match ().serve(transport).await {
        Ok(service) => service,
        Err(err) => {
            // The handshake never completed -- this connection attempt
            // is abandoned, so the spawned child (and, per M6.6, its
            // whole contained process tree) must not be left running.
            child.start_kill();
            return Err(McpClientError::ConnectionFailed(err.to_string()));
        }
    };
    // The handshake succeeded. `McpConnection` must keep `child` alive
    // for the connection's whole lifetime and kill it on close/drop.
    Ok(McpConnection { service: Some(service), child })
}

#[cfg(not(windows))]
async fn connect_now(registration: &McpServerRegistration) -> Result<McpConnection, McpClientError> {
    let TransportConfig::ChildProcess { command, args } = &registration.transport;

    let mut cmd = tokio::process::Command::new(command);
    cmd.args(args);
    isolate_child_environment(&mut cmd);
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::inherit());

    let mut child = cmd.spawn().map_err(|e| McpClientError::ConnectionFailed(e.to_string()))?;
    let stdout = child.stdout.take().ok_or_else(|| McpClientError::ConnectionFailed("child process stdout was not piped".to_string()))?;
    let stdin = child.stdin.take().ok_or_else(|| McpClientError::ConnectionFailed("child process stdin was not piped".to_string()))?;

    let transport = AsyncRwTransport::new_client(stdout, stdin).with_max_line_length(MCP_MAX_RESPONSE_LINE_BYTES);

    let service = match ().serve(transport).await {
        Ok(service) => service,
        Err(err) => {
            // The handshake never completed -- this connection attempt
            // is abandoned, so the spawned child must not be left
            // running/zombied. `AsyncRwTransport`'s own `Drop` closes the
            // piped stdin/stdout handles, but does not kill the process
            // itself (unlike `TokioChildProcess`'s own cleanup guard,
            // which this function intentionally does not use, since it
            // has no way to configure `max_line_length`) -- killing
            // explicitly here preserves the same "a failed connect leaves
            // nothing behind" guarantee the previous `TokioChildProcess`-
            // based implementation provided for free via its own
            // `ChildWithCleanup::drop`.
            let _ = child.start_kill();
            return Err(McpClientError::ConnectionFailed(err.to_string()));
        }
    };
    // The handshake succeeded. `McpConnection` must keep `child` alive
    // for the connection's whole lifetime and kill it on close/drop --
    // the exact responsibility `TokioChildProcess`'s own
    // `ChildWithCleanup` carried before this change, now made explicit
    // here instead of implicit inside the bypassed convenience wrapper.
    Ok(McpConnection { service: Some(service), child })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serializes every test in this module that touches the global env
    /// var: `std::env::set_var`/`remove_var` are process-global, and
    /// `cargo test`'s default multi-threaded runner would otherwise race
    /// two tests setting different values concurrently. A plain
    /// `std::sync::Mutex` guard held only around a SYNCHRONOUS closure is
    /// not enough for an `async fn` test: the actual work that reads
    /// `global_client_enabled()` happens later, inside the test's own
    /// `.await`ed body, after a synchronous setup closure has already
    /// returned and released the lock. `EnvGuard` instead holds the lock
    /// for its own lifetime (released on `Drop`, at the end of the test
    /// function's scope), so the env var stays pinned to the value this
    /// test set for the ENTIRE test body, including every `.await` point.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    // The `MutexGuard` is held only for its lifetime/`Drop` ordering (it
    // keeps the lock until this guard itself drops, at the end of the
    // test's scope) -- its value is never read, by design.
    struct EnvGuard<'a>(#[allow(dead_code)] std::sync::MutexGuard<'a, ()>);

    impl Drop for EnvGuard<'_> {
        fn drop(&mut self) {
            unsafe { std::env::remove_var(GLOBAL_DISABLE_ENV_VAR) };
        }
    }

    fn global_disabled() -> EnvGuard<'static> {
        let guard = ENV_LOCK.lock().unwrap();
        unsafe { std::env::set_var(GLOBAL_DISABLE_ENV_VAR, "1") };
        EnvGuard(guard)
    }

    fn global_enabled() -> EnvGuard<'static> {
        let guard = ENV_LOCK.lock().unwrap();
        unsafe { std::env::set_var(GLOBAL_DISABLE_ENV_VAR, "0") };
        EnvGuard(guard)
    }

    fn env_unset() -> EnvGuard<'static> {
        let guard = ENV_LOCK.lock().unwrap();
        unsafe { std::env::remove_var(GLOBAL_DISABLE_ENV_VAR) };
        EnvGuard(guard)
    }

    fn a_registration(enabled: bool) -> McpServerRegistration {
        McpServerRegistration { name: "test-server".to_string(), transport: TransportConfig::ChildProcess { command: "this-binary-does-not-exist-12345".to_string(), args: vec![] }, enabled }
    }

    /// `McpConnection` wraps `rmcp::service::RunningService`, which
    /// implements neither `Debug` nor `PartialEq` -- this extracts just
    /// the `Err` side so this module's tests can assert on it directly,
    /// the same shape `mcp-server`'s own `expect_mcp_error` test helper
    /// uses for the same underlying reason.
    fn expect_err(result: Result<McpConnection, McpClientError>) -> McpClientError {
        match result {
            Err(e) => e,
            Ok(_) => panic!("expected an error, got a live connection"),
        }
    }

    // --- global kill-switch ---

    #[test]
    fn global_client_enabled_defaults_to_true_when_unset() {
        let _guard = env_unset();
        assert!(global_client_enabled());
    }

    #[test]
    fn global_client_enabled_is_false_when_the_env_var_is_set_to_anything_other_than_zero() {
        let _guard = ENV_LOCK.lock().unwrap();
        for value in ["1", "true", "yes", ""] {
            unsafe { std::env::set_var(GLOBAL_DISABLE_ENV_VAR, value) };
            assert!(!global_client_enabled(), "value {value:?} must disable the client (fail closed on any non-\"0\" value)");
        }
        unsafe { std::env::remove_var(GLOBAL_DISABLE_ENV_VAR) };
    }

    #[test]
    fn global_client_enabled_is_true_only_when_the_env_var_is_exactly_zero() {
        let _guard = global_enabled();
        assert!(global_client_enabled());
    }

    // --- registration.enabled: false never attempts a connection ---

    #[tokio::test]
    async fn a_disabled_registration_never_attempts_connection_and_returns_disabled_immediately() {
        let _guard = global_enabled(); // ensure the global switch itself isn't the reason
        let registration = a_registration(false);
        // The command named in this registration does not exist -- if
        // connect() ever attempted to spawn it, the error would be
        // ConnectionFailed (spawn failure), not Disabled. Getting
        // Disabled here is itself the proof no spawn was attempted.
        let result = connect(&registration).await;
        assert_eq!(expect_err(result), McpClientError::Disabled);
    }

    #[tokio::test]
    async fn an_enabled_registration_with_global_switch_off_is_still_denied_without_a_connection_attempt() {
        let _guard = global_disabled();
        let registration = a_registration(true);
        let result = connect(&registration).await;
        assert_eq!(expect_err(result), McpClientError::Disabled, "the global kill-switch must short-circuit regardless of the registration's own enabled flag");
    }

    #[tokio::test]
    async fn the_global_disable_switch_denies_every_registration_regardless_of_individual_enabled_flags() {
        let _guard = global_disabled();
        for enabled in [true, false] {
            let registration = a_registration(enabled);
            let result = connect(&registration).await;
            assert_eq!(expect_err(result), McpClientError::Disabled, "enabled={enabled} must still be denied while the global switch is off");
        }
    }

    // --- timeout: a connection attempt that cannot complete in time fails
    //     closed as Timeout, never hangs ---

    #[tokio::test]
    async fn a_connection_attempt_exceeding_its_timeout_returns_timeout_not_a_hang() {
        let _guard = global_enabled();
        // A command that starts successfully (so this exercises the
        // TIMEOUT path, not a spawn failure) but never speaks MCP on its
        // stdio, so the initialize handshake never completes -- exactly
        // the "hung external server" case this task's own Step 1 names.
        // Unix: `cat` with no args blocks reading stdin forever and never
        // writes anything unprompted. Windows: `more` (no file argument)
        // reads from stdin and blocks identically, run via `cmd /c` since
        // it is a cmd.exe builtin, not a standalone executable.
        #[cfg(unix)]
        let registration = McpServerRegistration { name: "hangs".into(), transport: TransportConfig::ChildProcess { command: "cat".into(), args: vec![] }, enabled: true };
        #[cfg(windows)]
        let registration = McpServerRegistration { name: "hangs".into(), transport: TransportConfig::ChildProcess { command: "cmd".into(), args: vec!["/c".into(), "more".into()] }, enabled: true };

        let result = connect_with_timeout(&registration, Duration::from_millis(500)).await;
        assert_eq!(expect_err(result), McpClientError::Timeout);
    }

    // --- a genuinely nonexistent command is a connection failure, not a
    //     silent success and not confused with Disabled/Timeout ---

    #[tokio::test]
    async fn spawning_a_nonexistent_command_is_a_connection_failure() {
        let _guard = global_enabled();
        let registration = a_registration(true);
        let result = expect_err(connect(&registration).await);
        assert!(matches!(result, McpClientError::ConnectionFailed(_)), "expected ConnectionFailed for a nonexistent command, got {result:?}");
    }
}
