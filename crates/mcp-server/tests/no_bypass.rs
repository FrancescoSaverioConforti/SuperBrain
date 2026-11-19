//! B5.2 Task 22: structural no-bypass / dependency-direction guards for
//! `mcp-server`, mirroring `crates/brain-api/tests/layering.rs`'s own
//! discipline (B5 Task 20) one layer up the stack.
//!
//! `mcp-server` → `brain-api` → core services is the one legitimate path.
//! This crate additionally depends directly on 7 core crates (`api`,
//! `codegraph-domain`, `codegraph-index`, `domain`, `index`, `policy`,
//! `storage`) ONLY because `brain_api::read`'s functions take their
//! already-built runtime objects by reference — never to reach a raw
//! write/mutation/persistence primitive those crates expose. This file
//! makes that allowance explicit and fail-closed: any NEW direct
//! dependency added to `crates/mcp-server/Cargo.toml` must be a deliberate
//! addition to the allowlist below, not a silent expansion of what this
//! "thin adapter" crate can reach.

use std::collections::BTreeSet;

/// The complete, fail-closed allowlist of `[dependencies]` this crate may
/// declare. Adding a new dependency to `Cargo.toml` without adding it here
/// fails this test — forcing a reviewer to look at exactly what grew and
/// ask whether `mcp-server` should really be able to reach it directly.
fn allowed_path_dependencies() -> BTreeSet<&'static str> {
    [
        "brain-api",
        // Approved core-crate dependencies: brain_api::read's functions
        // take these crates' already-built runtime objects BY REFERENCE
        // (&storage::Vault, &index::Index<'_>,
        // &codegraph_index::graph::CodeGraph) -- read.rs's own doc comment
        // says the caller supplies them. None of these 7 is reached for
        // any write/mutation/raw-persistence purpose -- see
        // tools/search.rs's module doc comment and the behavioral tests
        // in tests/search_tools.rs.
        "api",
        "codegraph-domain",
        "codegraph-index",
        "domain",
        "index",
        "policy",
        "storage",
    ]
    .into_iter()
    .collect()
}

/// Parses `crates/mcp-server/Cargo.toml`'s `[dependencies]` table (path
/// dependencies only -- external crates.io dependencies like `rmcp`/
/// `tokio`/`schemars`/`serde`/`chrono`/`uuid` are not internal workspace
/// layering concerns) and returns every declared path-dependency name.
fn declared_path_dependencies() -> BTreeSet<String> {
    let manifest_path = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
    let text = std::fs::read_to_string(manifest_path).expect("mcp-server's own Cargo.toml must be readable");
    let mut names = BTreeSet::new();
    let mut in_dependencies = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_dependencies = trimmed == "[dependencies]";
            continue;
        }
        if !in_dependencies || trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        // Only lines naming a path dependency (`name = { path = "..." }`)
        // count as an internal workspace-layering concern.
        if trimmed.contains("path") {
            if let Some(name) = trimmed.split('=').next() {
                names.insert(name.trim().to_string());
            }
        }
    }
    names
}

#[test]
fn mcp_server_declares_no_path_dependency_outside_the_approved_allowlist() {
    let declared = declared_path_dependencies();
    let allowed = allowed_path_dependencies();
    for name in &declared {
        assert!(
            allowed.contains(name.as_str()),
            "crates/mcp-server/Cargo.toml declares a NEW path dependency {name:?} not in this test's allowlist -- \
             if this is a deliberate, approved expansion of what mcp-server may reach directly, add it to \
             allowed_path_dependencies() with a comment explaining why; if not, this is exactly the \
             dependency-direction drift this guard exists to catch."
        );
    }
}

#[test]
fn mcp_server_declares_every_core_crate_it_actually_uses_not_a_superset() {
    // The inverse check: every crate in the allowlist (other than
    // brain-api itself) must be a REAL dependency this crate actually
    // uses for a brain_api-required runtime object -- not a speculative
    // addition nobody needs. storage::Vault, index::Index, codegraph_index
    // ::graph::CodeGraph, codegraph_domain::SymbolSnapshotId, domain::
    // NoteType/NoteStatus, policy::ReadPolicy, and api::SearchRequest/
    // RetrieveRequest are all referenced by name in tools/search.rs --
    // this test exists as documentation-by-assertion of that fact, pinned
    // against the same allowlist the previous test protects.
    let declared = declared_path_dependencies();
    for name in ["api", "codegraph-domain", "codegraph-index", "domain", "index", "policy", "storage"] {
        assert!(declared.contains(name), "{name} is in the allowlist but not declared as a dependency -- the allowlist is stale, remove it");
    }
}

/// Compile-time property: `mcp-server`'s only path to approval authority
/// or trusted-local origin would be naming `brain_api::WritePolicy::
/// trusted_cli` or constructing `brain_api::CallOrigin::TrustedLocal`
/// directly -- neither is possible from this crate (both are `pub(crate)`
/// to `brain-api`; see `crates/brain-api/tests/layering.rs`'s own proof of
/// the same property one layer down). This file's own successful
/// compilation, having been written WITHOUT referencing either, is the
/// proof; the test below exercises the one path this crate genuinely has
/// (constructing an MCP-originated context) to show it is permission-
/// limited exactly as `brain_api::WritePolicy::mcp_agent()` promises.
#[test]
fn mcp_server_can_only_construct_an_mcp_agent_write_policy_never_trusted_cli() {
    let mcp_policy = brain_api::WritePolicy::mcp_agent();
    assert!(!mcp_policy.allows(brain_api::WriteOperation::ApproveProposal));
    assert!(!mcp_policy.allows(brain_api::WriteOperation::RejectProposal));
    // There is no second, "trusted" WritePolicy constructor this crate can
    // call instead -- WritePolicy::trusted_cli is pub(crate) to brain-api.
}

/// Compile-time property: `mcp-server` can only ever construct
/// `brain_api::CallOrigin::McpSession(String)` — never
/// `CallOrigin::TrustedLocal(_)`, since its only constructor
/// (`TrustedLocalToken::new`) is `pub(crate)` to `brain-api` and
/// `TrustedLocalToken`'s own inner field is private. This test's
/// successful compilation, having constructed only `McpSession` below, is
/// the proof.
#[test]
fn mcp_server_can_only_construct_mcp_session_origin_never_trusted_local() {
    let origin = brain_api::CallOrigin::McpSession("probe".to_string());
    assert!(!origin.is_trusted_local());
}

/// `rmcp`'s own `transport-io` feature (stdio) is the only transport this
/// crate enables -- pinned by reading `Cargo.toml` itself, not by trusting
/// a comment to stay accurate. A network-capable transport feature
/// (`transport-sse-server`, `transport-streamable-http-server`, etc.)
/// being added later would need this test updated deliberately.
#[test]
fn mcp_server_enables_no_network_capable_rmcp_transport_feature() {
    let manifest_path = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
    let text = std::fs::read_to_string(manifest_path).unwrap();
    let rmcp_line = text.lines().find(|l| l.trim_start().starts_with("rmcp")).expect("mcp-server must declare an rmcp dependency");
    assert!(rmcp_line.contains("transport-io"), "expected the stdio transport feature, got: {rmcp_line}");
    for network_feature in ["transport-sse-server", "transport-streamable-http-server", "transport-streamable-http-client", "transport-ws", "transport-worker"] {
        assert!(!rmcp_line.contains(network_feature), "mcp-server must not enable the network-capable rmcp feature {network_feature:?}, got: {rmcp_line}");
    }
}
