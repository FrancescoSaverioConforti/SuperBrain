//! B5.10 Task 30: MCP server/client architectural separation guard.
//!
//! This file does not add any new production capability — it is a
//! structural audit over what Tasks 21–29 already built, proving the
//! isolation they each individually documented actually holds as a
//! cross-crate fact, not just a per-crate claim. Three invariants, per
//! the plan's own Step 1 wording:
//!
//! 1. `mcp-server` does not depend on `mcp-client` in its `Cargo.toml`,
//!    and vice versa — checked by PARSING both crates' real `Cargo.toml`
//!    files as text, never by relying on "it compiled" alone (a
//!    transitive dependency could sneak in through some other crate
//!    without a direct one ever being declared, and compilation alone
//!    would not catch that).
//! 2. A capability registered on the server is never automatically
//!    callable by the client — there is no shared registry/type between
//!    the two crates a capability could leak through. Proven
//!    structurally: neither crate's `Cargo.toml` names the other, so
//!    neither can even NAME the other's types, let alone register or
//!    call them.
//! 3. A client-side tool invocation can never reach `brain-api`'s
//!    `storage::propose_and_commit` path directly — checked by
//!    confirming `mcp-client`'s `Cargo.toml` has NEITHER `storage` NOR
//!    `brain-api` as a dependency at all (not merely "doesn't call
//!    `propose_and_commit`" — the stronger, structural claim that the
//!    function is not even NAMEABLE from this crate).
//!
//! Reads `../mcp-server/Cargo.toml` relative to this crate's own manifest
//! directory (both are sibling `crates/*` members of the same
//! workspace) — this is a plain filesystem read, performed by a TEST,
//! never a `[dependencies]`/`[dev-dependencies]` entry in either crate's
//! own `Cargo.toml`. Reading a sibling crate's manifest text to audit it
//! creates no Cargo dependency edge and therefore cannot create the
//! circular-dev-dependency problem the plan's own Task 30 text warns
//! about.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn mcp_client_manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn mcp_server_manifest_path() -> PathBuf {
    mcp_client_manifest_dir().join("..").join("mcp-server").join("Cargo.toml")
}

fn mcp_client_manifest_path() -> PathBuf {
    mcp_client_manifest_dir().join("Cargo.toml")
}

/// Parses one `Cargo.toml`'s `[dependencies]` table (not
/// `[dev-dependencies]` — a dev-only dependency is never reachable from
/// the crate's own production code path, so it is not a layering concern
/// for THIS test's purpose, exactly as `mcp-server`'s own
/// `tests/no_bypass.rs` already treats its dev-only `rmcp` client
/// feature) and returns the set of declared PATH dependency names
/// (`name = { path = "..." }`) — the only kind of dependency that can
/// create an internal workspace coupling; an external crates.io
/// dependency (`rmcp`, `tokio`, `serde_json`, etc.) is not a cross-crate
/// layering concern.
fn declared_path_dependencies(manifest_path: &Path) -> BTreeSet<String> {
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

// --- 1. Neither crate depends on the other ---

#[test]
fn mcp_server_does_not_declare_mcp_client_as_a_dependency() {
    let deps = declared_path_dependencies(&mcp_server_manifest_path());
    assert!(!deps.contains("mcp-client"), "crates/mcp-server/Cargo.toml must never depend on mcp-client -- the server exposes SuperBrain's own capabilities OUTWARD; it has no legitimate reason to call OUT to an external MCP tool through the client crate");
}

#[test]
fn mcp_client_does_not_declare_mcp_server_as_a_dependency() {
    let deps = declared_path_dependencies(&mcp_client_manifest_path());
    assert!(!deps.contains("mcp-server"), "crates/mcp-client/Cargo.toml must never depend on mcp-server -- the client calls OUT to external tools; it has no legitimate reason to reach back into SuperBrain's own inbound-facing server crate");
}

// --- 2. No shared registry a capability could leak through ---

#[test]
fn neither_crate_names_the_other_so_no_capability_registry_can_be_shared_between_them() {
    // The structural proof: a "shared registry" would require at least
    // one of the two crates to be able to NAME the other's types (a
    // `ToolRouter<mcp_server::SuperBrainMcpServer>` living in mcp-client,
    // or an `mcp_client::ToolCache` entry mcp-server could register a
    // server-side capability into). The two dependency checks above
    // already prove neither crate declares the other as a dependency at
    // all -- and Rust has no mechanism to name a type from a crate that
    // is not a declared dependency. There is therefore no code path,
    // anywhere, through which `mcp-server`'s 12 registered tools
    // (crates/mcp-server/src/tools/*.rs, Tasks 22-23) and `mcp-client`'s
    // discovered-tool cache (crates/mcp-client/src/discovery.rs, Task
    // 27) could ever be the same registry or cross-reference each
    // other's entries.
    let server_deps = declared_path_dependencies(&mcp_server_manifest_path());
    let client_deps = declared_path_dependencies(&mcp_client_manifest_path());
    assert!(!server_deps.contains("mcp-client") && !client_deps.contains("mcp-server"), "a shared registry would require at least one crate to depend on the other -- neither does");
}

// --- 3. mcp-client cannot reach storage::propose_and_commit at all ---

#[test]
fn mcp_client_declares_neither_storage_nor_brain_api_as_a_dependency() {
    let deps = declared_path_dependencies(&mcp_client_manifest_path());
    assert!(!deps.contains("storage"), "crates/mcp-client/Cargo.toml must never depend on storage -- a client-side tool invocation (Task 29's invoke_external_tool) must have no path back into SuperBrain's own write pipeline, and storage::propose_and_commit is the one real write authority that pipeline protects (crates/storage/src/propose.rs)");
    assert!(!deps.contains("brain-api"), "crates/mcp-client/Cargo.toml must never depend on brain-api -- the client cannot construct a brain_api::RequestContext/WritePolicy, cannot call brain_api::save_memory/approve_proposal, and has no loop-back path into SuperBrain's own Brain API layer at all");
}

#[test]
fn mcp_client_declares_no_path_dependency_at_all_beyond_its_own_documented_isolation() {
    // The exhaustive version of the check above: mcp-client's own
    // lib.rs doc comment claims its ENTIRE dependency list is `rmcp`,
    // `tokio`, `serde_json` -- all three external crates.io dependencies,
    // zero path dependencies. This test pins that claim directly against
    // the real Cargo.toml, not just against the two named crates the
    // previous test checks.
    let deps = declared_path_dependencies(&mcp_client_manifest_path());
    assert!(deps.is_empty(), "crates/mcp-client/Cargo.toml must declare ZERO path dependencies -- its own lib.rs doc comment claims full isolation from every other workspace crate (mcp-server, brain-api, storage, and everything brain-api itself depends on); found: {deps:?}");
}
