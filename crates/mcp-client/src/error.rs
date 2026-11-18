//! `McpClientError`: the error vocabulary for `mcp-client`'s own
//! operations (connecting to, then later invoking, an external MCP
//! server) — distinct from `brain_api::BrainApiError` (which covers
//! SuperBrain's own 12 capabilities) and from whatever error type the
//! external server itself returns over the wire. No variant carries a
//! filesystem path, raw process spawn detail, or the external server's own
//! internal error text verbatim — only a caller-facing category.

/// The frozen `mcp-client` error vocabulary. Grows only with later tasks
/// in this plan (B5.9's `authorize` denial) — Task 26 defined connection
/// lifecycle's own two variants; Task 27 (this one) adds exactly the one
/// `MalformedResponse` variant its own scope (rejecting an external
/// server's untrustworthy `tools/list` reply) needs, matching the exact
/// name the plan's own Task 29 text already uses for the same category of
/// failure — not a new invention, the vocabulary this crate was always
/// going to need for "the external server answered, but what it sent
/// cannot be trusted."
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpClientError {
    /// The global client kill-switch (`global_client_enabled`) is off, or
    /// this specific registration has `enabled: false`. Checked before any
    /// connection attempt — never a value a connection attempt itself
    /// discovers partway through.
    Disabled,
    /// The connection attempt did not complete within its timeout —
    /// returned instead of hanging forever, never silently retried.
    Timeout,
    /// Spawning the external server's transport (e.g. the child process)
    /// or completing the MCP `initialize` handshake failed. Carries a
    /// caller-facing reason, never the raw `std::io::Error`/`rmcp` error
    /// text verbatim (which may contain a filesystem path or command
    /// line).
    ConnectionFailed(String),
    /// The external server answered, but its response cannot be trusted
    /// as-is: a malformed `tools/list` reply (Task 27 — an empty tool
    /// name, a tool name repeated within the same list, or a name
    /// containing a path-traversal-shaped character), and later (Task 29)
    /// a malformed `tools/call` result or an envelope that claims to be
    /// from a different tool than the one invoked. The response is
    /// rejected outright, never partially trusted or passed through —
    /// carries a caller-facing reason, never the raw untrusted payload
    /// verbatim.
    MalformedResponse(String),
}

impl std::fmt::Display for McpClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => f.write_str("MCP client is disabled"),
            Self::Timeout => f.write_str("connection attempt timed out"),
            Self::ConnectionFailed(reason) => write!(f, "connection failed: {reason}"),
            Self::MalformedResponse(reason) => write!(f, "malformed response from external server: {reason}"),
        }
    }
}

impl std::error::Error for McpClientError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_variants() -> Vec<McpClientError> {
        vec![McpClientError::Disabled, McpClientError::Timeout, McpClientError::ConnectionFailed("x".into()), McpClientError::MalformedResponse("x".into())]
    }

    #[test]
    fn every_variant_has_a_stable_nonempty_display_message() {
        for case in all_variants() {
            assert!(!case.to_string().is_empty(), "{case:?} produced an empty Display message");
        }
    }

    #[test]
    fn display_messages_are_distinct_across_variants() {
        let messages: Vec<String> = all_variants().iter().map(|e| e.to_string()).collect();
        let unique: std::collections::HashSet<&String> = messages.iter().collect();
        assert_eq!(messages.len(), unique.len(), "two variants produced the same Display text: {messages:?}");
    }

    #[test]
    fn implements_std_error() {
        fn assert_is_error<E: std::error::Error>(_: &E) {}
        assert_is_error(&McpClientError::Timeout);
    }
}
