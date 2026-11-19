//! B5.16 Task 35: the server-side transport payload-size enforcement
//! point for `brain_api::limits::MCP_MAX_PAYLOAD_BYTES`.
//!
//! No real composition-root binary exists in this crate yet (every
//! existing `.serve(...)` call site is a test, constructing a
//! `SuperBrainMcpServer` directly against an in-memory duplex stream or
//! a real stdio pipe — see `tools/search.rs`'s own module doc comment on
//! why a real launcher binary stays out of B5's scope). Passing a raw
//! `AsyncRead + AsyncWrite` pair to `rmcp`'s `ServiceExt::serve` builds
//! an `rmcp::transport::async_rw::AsyncRwTransport` INTERNALLY, with
//! its own upstream default of 16 MiB
//! (`rmcp::transport::async_rw::DEFAULT_MAX_LINE_LENGTH`) — there is no
//! way to override that default through the raw-pair path.
//!
//! [`limited_transport`] is the one function in this crate that builds
//! an `AsyncRwTransport` explicitly, with
//! [`brain_api::limits::MCP_MAX_PAYLOAD_BYTES`] instead of `rmcp`'s own
//! default, so a caller passes ITS RESULT to `.serve(...)` (never the
//! raw `(reader, writer)` pair) to get this crate's own, tighter limit
//! enforced at the transport layer, before any line is ever handed to
//! this server's JSON-RPC/tool-dispatch logic. A line exceeding the
//! limit is dropped by `AsyncRwTransport` itself (never buffered
//! unboundedly, never a panic) and the connection stays open — `rmcp`'s
//! own documented behavior for `with_max_line_length`, unchanged here.

use rmcp::transport::async_rw::AsyncRwTransport;

/// Builds a [`rmcp::service::RoleServer`]-side transport over `(reader,
/// writer)`, configured with [`brain_api::limits::MCP_MAX_PAYLOAD_BYTES`]
/// as its maximum single JSON-RPC line length — SuperBrain's own,
/// deliberately tighter choice than `rmcp`'s 16 MiB upstream default
/// (see this module's own doc comment). Pass the RETURNED value to
/// `ServiceExt::serve`/`serve_with_ct`, never the raw `(reader, writer)`
/// pair, to get this limit enforced; passing the raw pair instead
/// silently falls back to `rmcp`'s own unconfigured default.
pub fn limited_transport<R, W>(reader: R, writer: W) -> AsyncRwTransport<rmcp::RoleServer, R, W>
where
    R: tokio::io::AsyncRead + Unpin + Send,
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    AsyncRwTransport::new(reader, writer)
        .with_max_line_length(brain_api::limits::MCP_MAX_PAYLOAD_BYTES)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limited_transport_builds_without_panicking_on_an_in_memory_pair() {
        // Structural smoke test: the generic bounds/feature wiring
        // compiles and constructs successfully. The REAL end-to-end
        // proof that an oversized line is actually rejected lives in
        // tests/limits.rs, against a genuine `.serve()`d connection --
        // this module's own unit test only confirms the builder itself
        // is reachable and does not require anything beyond a plain
        // AsyncRead/AsyncWrite pair to construct.
        let (reader, writer) = tokio::io::duplex(1024);
        let (read_half, _write_half_unused) = tokio::io::split(reader);
        let _transport = limited_transport(read_half, writer);
    }
}
