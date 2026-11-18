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
use rmcp::transport::TokioChildProcess;

use crate::McpClientError;

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
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

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
/// handle for the client-role session. Holds no capability/authorization
/// state of its own (Task 28's job) — this is purely "the handshake
/// succeeded and a session exists," nothing about what it may be asked to
/// do.
pub struct McpConnection {
    service: rmcp::service::RunningService<rmcp::RoleClient, ()>,
}

impl McpConnection {
    /// Read-only accessor for later tasks (discovery/invocation) to reach
    /// the underlying session. Not used by this task itself.
    pub fn service(&self) -> &rmcp::service::RunningService<rmcp::RoleClient, ()> {
        &self.service
    }

    /// Closes the connection, shutting down the underlying transport (and,
    /// for a child-process transport, the spawned process).
    pub async fn close(self) -> Result<(), McpClientError> {
        self.service.cancel().await.map_err(|e| McpClientError::ConnectionFailed(e.to_string())).map(|_| ())
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

async fn connect_now(registration: &McpServerRegistration) -> Result<McpConnection, McpClientError> {
    let TransportConfig::ChildProcess { command, args } = &registration.transport;

    let mut cmd = tokio::process::Command::new(command);
    cmd.args(args);
    let transport = TokioChildProcess::new(cmd).map_err(|e| McpClientError::ConnectionFailed(e.to_string()))?;

    let service = ().serve(transport).await.map_err(|e| McpClientError::ConnectionFailed(e.to_string()))?;
    Ok(McpConnection { service })
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
