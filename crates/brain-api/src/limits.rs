//! B5.16 Task 35: resource-limit constants and the enforcement helpers
//! that use them. Per the plan's own instruction, "non serve tuning
//! scientifico, serve impedire resource exhaustion accidentale" —
//! these are deliberately round, conservative numbers, not a
//! scientifically-derived capacity plan. Every constant is documented
//! with a one-line rationale and the exact function(s) that enforce it.
//!
//! **Validation vs. enforcement, kept distinct on purpose**: a few of
//! these constants bound CALLER-SUPPLIED INPUT before an expensive
//! operation ever starts (`MAX_QUERY_LEN`, `MAX_RESULT_COUNT`,
//! `MAX_GRAPH_TRAVERSAL_DEPTH`, `MAX_GRAPH_QUERY_RESULTS`) — these are
//! input validation, rejected as `BrainApiError::InvalidInput` exactly
//! like any other malformed request, never a panic or a silent
//! truncation. The other two (`MCP_MAX_PAYLOAD_BYTES`,
//! `MAX_CONCURRENT_REQUESTS`) bound a RESOURCE the transport/runtime
//! itself consumes regardless of whether any single request's own
//! content is valid — these are resource enforcement, applied at the
//! transport/concurrency layer (`mcp-server`), never inside `brain-api`
//! itself (which has no concept of "a connection" or "a concurrent
//! request slot" — it only ever sees one already-validated call at a
//! time).
//!
//! `DEFAULT_TIMEOUT_MS` is neither of those: it is `brain-api`'s own
//! canonical default for a bounded network-shaped operation, for any
//! future call site in this crate's own call graph or in `mcp-server`
//! (which already depends on `brain-api`).
//!
//! **This constant is NOT shared with `mcp-client`.** `mcp-client` has
//! its own, independently-defined `registration::DEFAULT_CONNECT_TIMEOUT`
//! and `invoke::DEFAULT_INVOKE_TIMEOUT` (both `Duration::from_secs(30)`,
//! unchanged by this task) — `mcp-client` must never depend on
//! `brain-api` at all (the frozen B5.10 trust-boundary separation,
//! verified by `crates/mcp-client/tests/separation.rs`), so importing
//! this constant there is not an option. The two crates' defaults are
//! kept at the SAME VALUE by convention plus a cross-crate regression
//! test (`crates/mcp-client/tests/limits.rs`'s own
//! `mcp_clients_default_timeouts_still_match_brain_apis_canonical_value`),
//! never by a shared import — a deliberate, documented duplication of
//! one literal, not an oversight.

/// Bounds a caller-supplied free-text query (`search`'s `text` field,
/// `get_previous_solution`'s `query` parameter) before it ever reaches
/// `index::search`'s own tokenization/matching work. 4096 bytes is far
/// beyond any realistic human- or agent-authored query while still
/// rejecting an attempt to force a full-corpus linear scan over a
/// multi-megabyte "query" string. Enforced by
/// [`crate::read::search`]/[`crate::memory::get_previous_solution`],
/// both returning [`crate::BrainApiError::InvalidInput`] before calling
/// into `index::search` at all.
pub const MAX_QUERY_LEN: usize = 4096;

/// Bounds the number of results a single `search`/`get_previous_solution`
/// call may request or return. 500 comfortably covers any real usage
/// (an agent paging through more than a few dozen hits in one call is
/// already an unusual pattern) while rejecting a `k`/implicit limit that
/// would force materializing an unbounded result set. Enforced by
/// [`crate::read::search`] (rejects `request.k > MAX_RESULT_COUNT` as
/// `InvalidInput`, before calling `index::search`) and
/// [`crate::memory::get_previous_solution`] (passes this constant as its
/// own internal limit to `index::search`, replacing the prior
/// unconditional `usize::MAX`).
pub const MAX_RESULT_COUNT: usize = 500;

/// Bounds `graph_query`'s `max_depth` parameter. Matches the value
/// already enforced as `brain_api::read::MAX_GRAPH_QUERY_DEPTH` since
/// Task 17 — this module becomes the one canonical definition; `read.rs`
/// re-exports it under its prior name rather than keeping a second,
/// independently-maintained copy of the same number.
pub const MAX_GRAPH_TRAVERSAL_DEPTH: usize = 50;

/// Bounds `graph_query`'s result count. Matches the value already
/// enforced as `brain_api::read::MAX_GRAPH_QUERY_RESULTS` since Task 17 —
/// same consolidation as [`MAX_GRAPH_TRAVERSAL_DEPTH`] above.
pub const MAX_GRAPH_QUERY_RESULTS: usize = 500;

/// Bounds one incoming MCP transport line (one JSON-RPC message) on the
/// server side. 1 MiB is generous for any of the 12 real tool
/// payloads (the largest, `save_memory`'s note body, is still ordinary
/// text) while rejecting a payload shaped to force the transport to
/// buffer an unbounded amount of memory per line before parsing even
/// starts. Smaller than `rmcp`'s own `transport::async_rw::
/// DEFAULT_MAX_LINE_LENGTH` (16 MiB) — this constant is SuperBrain's own,
/// deliberately tighter choice for its own server transport
/// construction; `rmcp`'s 16 MiB upstream default remains what actually
/// governs any transport this crate does not itself construct (see this
/// module's own doc comment on the client-side residual limitation).
/// Enforced by `mcp_server::limited_transport`, which builds
/// `rmcp::transport::async_rw::AsyncRwTransport::with_max_line_length`
/// with this value.
pub const MCP_MAX_PAYLOAD_BYTES: usize = 1_048_576;

/// The canonical default timeout, in milliseconds, for a bounded
/// network-shaped operation within `brain-api`'s own call graph or
/// `mcp-server`'s. See this module's own doc comment for why
/// `mcp-client` keeps its own separate, same-valued constants instead
/// of importing this one.
pub const DEFAULT_TIMEOUT_MS: u64 = 30_000;

/// Bounds how many MCP tool calls `SuperBrainMcpServer` executes at
/// once. 16 is far above any realistic single-user-agent workload
/// (this is a local, single-user tool — see this crate's own `lib.rs`
/// doc comment) while still rejecting an attempt to open unbounded
/// concurrent tool calls against the same server instance and exhaust
/// its underlying `Vault`/index/graph resources. Enforced by
/// `mcp_server::SuperBrainMcpServer`'s own shared `tokio::sync::
/// Semaphore` (one per server instance, shared across every `Clone` —
/// see that struct's own doc comment for why), acquired at the start of
/// every `#[tool]` handler and held for the handler's entire body,
/// released automatically (RAII) on completion, error, or cancellation.
/// This bounds ACTIVE operations, not queued requests: a request beyond
/// the limit waits for a permit rather than being rejected outright
/// (the plan's own "queue or reject" wording — queueing was chosen; see
/// `mcp-server`'s own module doc comment for the full rationale), with
/// no additional acquisition-specific timeout layered on top (an
/// already-cancelled or externally-timed-out caller's own task being
/// dropped is what ends a queued wait, not a timer owned by the
/// semaphore itself).
pub const MAX_CONCURRENT_REQUESTS: usize = 16;

// A configured limit of zero must never silently mean "unlimited" --
// these compile-time assertions are this module's own structural guard
// against that class of mistake (a runtime-configurable limit would
// need its own explicit zero-check; these are fixed constants, so the
// check is simply that none of them was ever written as 0). Checked at
// COMPILE time, unconditionally -- not behind `#[cfg(test)]` -- because
// the fact they assert is itself a compile-time property of these
// `const` definitions, not something that could pass in a test run and
// fail in a release build, or vice versa.
const _: () = assert!(MAX_QUERY_LEN > 0);
const _: () = assert!(MAX_RESULT_COUNT > 0);
const _: () = assert!(MAX_GRAPH_TRAVERSAL_DEPTH > 0);
const _: () = assert!(MAX_GRAPH_QUERY_RESULTS > 0);
const _: () = assert!(MCP_MAX_PAYLOAD_BYTES > 0);
const _: () = assert!(DEFAULT_TIMEOUT_MS > 0);
const _: () = assert!(MAX_CONCURRENT_REQUESTS > 0);

// Documents the relationship this module's own doc comment states:
// SuperBrain's chosen MCP payload limit is deliberately smaller than
// rmcp's 16 MiB built-in default, never larger (a larger value here
// would be a no-op, silently relying on the upstream default instead of
// this crate's own stated choice).
const _: () = assert!(MCP_MAX_PAYLOAD_BYTES < 16 * 1024 * 1024);
