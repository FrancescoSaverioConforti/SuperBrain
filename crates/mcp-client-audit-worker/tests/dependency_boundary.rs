//! M6.3 Phase 4/scope-and-STOP-rules: this crate's own dependency
//! boundary, checked structurally by parsing its real `Cargo.toml` as
//! text -- the same pattern `crates/mcp-client/tests/separation.rs`
//! already established for `mcp-client` itself (never relying on "it
//! compiled" alone, since a transitive dependency could sneak in
//! through some other crate without a direct one ever being declared).

use std::collections::BTreeSet;
use std::path::PathBuf;

fn this_crate_manifest_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")
}

/// Parses `[dependencies]` only (never `[dev-dependencies]` -- a dev-only
/// dependency is never reachable from this crate's own production code
/// path, matching `mcp-client/tests/separation.rs`'s own documented
/// reasoning) and returns the set of declared PATH dependency names.
fn declared_path_dependencies(manifest_path: &std::path::Path) -> BTreeSet<String> {
    let text = std::fs::read_to_string(manifest_path).unwrap_or_else(|e| panic!("failed to read {manifest_path:?}: {e}"));
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
        if trimmed.contains("path") {
            if let Some(name) = trimmed.split('=').next() {
                names.insert(name.trim().to_string());
            }
        }
    }
    names
}

#[test]
fn this_crate_depends_on_exactly_mcp_client_and_audit_chain_core_as_path_dependencies() {
    let deps = declared_path_dependencies(&this_crate_manifest_path());
    let expected: BTreeSet<String> = ["mcp-client", "audit-chain-core"].into_iter().map(String::from).collect();
    assert_eq!(deps, expected, "mcp-client-audit-worker's own Cargo.toml must declare exactly mcp-client and audit-chain-core as path dependencies -- no more, no fewer; found: {deps:?}");
}

#[test]
fn this_crate_never_declares_brain_api_storage_or_mcp_server_as_a_dependency() {
    let deps = declared_path_dependencies(&this_crate_manifest_path());
    assert!(!deps.contains("brain-api"), "mcp-client-audit-worker must never depend on brain-api -- this task's own event schema is independent of brain_api::AuditEvent");
    assert!(!deps.contains("storage"), "mcp-client-audit-worker must never depend on storage -- the durable writer remains owned by a future composition-root binary via the injected DurableAuditWriter trait, not a direct storage dependency here");
    assert!(!deps.contains("mcp-server"), "mcp-client-audit-worker must never depend on mcp-server -- this task creates no MCP Server tool for external execution");
}

#[test]
fn mcp_client_itself_still_declares_zero_path_dependencies_after_this_crates_addition() {
    // The B5.10 isolation this task must not weaken: mcp-client's own
    // Cargo.toml must remain exactly as isolated as
    // crates/mcp-client/tests/separation.rs already proves -- this new
    // crate depends ON mcp-client, never the reverse, and mcp-client
    // itself was not modified to add this crate as a dependency.
    let mcp_client_manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("mcp-client").join("Cargo.toml");
    let deps = declared_path_dependencies(&mcp_client_manifest);
    assert!(deps.is_empty(), "crates/mcp-client/Cargo.toml must still declare ZERO path dependencies after mcp-client-audit-worker was added -- found: {deps:?}");
}
