//! B5.2 Task 31: E2E flow 1 — a real external agent (a genuine MCP
//! client over a real transport, same pattern as `tests/
//! server_integration.rs`'s own `connected_client`) connects to the real
//! `mcp-server`, calls `search` with a query, and receives real,
//! correctly-ranked results from the real `RetrievalMode::DenseV2`
//! path — the real `ProductionDenseScorer`/`EmbeddingModel` (B4), the
//! real hash-verified ONNX artifacts, the real `index::Index::search`
//! scoring. Nothing in this chain is mocked.
//!
//! Compiled and run ONLY under `--features dense-retrieval` (this
//! crate's own Task 31 addition, forwarding `brain-api`'s, forwarding
//! `api`'s B4.1 feature) — a build without the feature has none of
//! `api::dense`/`api::dense_lifecycle`'s types available to name at all,
//! identical to every other B4-gated caller in this workspace.
//!
//! Skips cleanly (prints why, does not fail) if the pinned model
//! artifacts are not present on this machine and cannot be fetched --
//! this file never substitutes a mock/sparse fallback for the real
//! DenseV2 path; the acceptance criterion this task owns is that the
//! REAL path works, not that *some* path works.

#![cfg(feature = "dense-retrieval")]

use std::sync::Arc;

use api::dense::{self, EmbeddingModel};
use api::dense_lifecycle::ProductionDenseScorer;
use codegraph_index::graph::CodeGraph;
use index::indexer_impl::{RealIndexer, rebuild_index};
use index::search::DenseScorer;
use mcp_server::SuperBrainMcpServer;
use rmcp::{ServiceExt, model::CallToolRequestParams, object};
use storage::Vault;

fn unique_test_dir(label: &str) -> std::path::PathBuf {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "superbrain-mcp-server-e2e-dense-{label}-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A STABLE, shared cache directory outside this crate's own build tree
/// — reused across repeated test runs so only the FIRST run on a given
/// machine performs the real network fetch, matching `crates/api/
/// examples/b4_production_performance_smoke`'s own established
/// convention of checking a persistent cache location rather than a
/// fresh-per-run temp directory. `dense::setup`'s own cache layout is
/// scoped per-"vault root" it's given, so this path doubles as that
/// vault root argument purely for cache-location purposes — nothing
/// else in this file treats it as a real vault.
fn shared_artifact_cache_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join(".e2e-dense-model-cache")
}

/// Ensures the pinned, hash-verified ONNX+tokenizer artifacts exist
/// under the shared cache root, fetching them via `api::dense::setup`
/// (the ONE function in this codebase that performs this network
/// fetch — B4.1's own documented, explicit, user-visible step) only if
/// not already present there. Returns `None` (never panics) if the
/// fetch itself fails for any reason — a real, legitimate "skip, don't
/// fail the suite" case (no network access in this CI/dev environment),
/// distinct from this test's own assertions about the DenseV2 PATH once
/// artifacts genuinely are available.
fn ensure_model_artifacts() -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    let cache_root = shared_artifact_cache_root();
    match dense::setup(&cache_root) {
        Ok(paths) => Some(paths),
        Err(e) => {
            eprintln!("SKIP: could not obtain the pinned DenseV2 model artifacts ({e}) -- this test requires real network access to huggingface.co on first run on this machine.");
            None
        }
    }
}

/// Real fixture: a `Vault` with several real committed notes whose
/// bodies are deliberately semantically related to the test's own query
/// but share few/no exact lexical tokens with it — exactly the shape a
/// correctly-working DenseV2 (semantic) path should surface that a
/// purely lexical/BM25 path would not reliably rank as the top hit,
/// proving this test genuinely exercises semantic scoring, not an
/// accidental keyword match.
struct Fixture {
    vault: Vault,
    indexer: Arc<RealIndexer>,
    scorer: Arc<ProductionDenseScorer>,
}

impl Fixture {
    fn new(onnx_path: &std::path::Path, tokenizer_path: &std::path::Path) -> Self {
        let root = unique_test_dir("fixture");
        {
            let repo = git2::Repository::init(&root).unwrap();
            let sig = git2::Signature::now("Test", "test@localhost").unwrap();
            let oid = repo.index().unwrap().write_tree().unwrap();
            let tree = repo.find_tree(oid).unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[]).unwrap();
        }
        let vault = Vault::open_with_default_indexer(root.clone(), &root.join("journal.sqlite")).unwrap();
        let indexer = Arc::new(RealIndexer::new());

        let notes: &[(&str, &str)] = &[
            ("note-target", "The kettle began to whistle as steam rose from its spout, signaling the water was ready for tea."),
            ("note-distractor-a", "quarterly revenue projections for the northern sales region exceeded expectations"),
            ("note-distractor-b", "a reference implementation of a red-black tree in the storage layer"),
            ("note-distractor-c", "notes on configuring the continuous integration pipeline for the release branch"),
        ];
        for (id, body) in notes {
            let text = format!(
                "---\nid: {id}\nschema_version: 1\ntype: research\nstatus: reviewed\ntrust: official\nclassification: internal\nproject: project-x\nsource: agent:coder@prompt-v3\ncreated: 2026-11-18\nupdated: 2026-11-18\nevent_period: 2026-11-18\nrelations: {{}}\n---\n{body}\n"
            );
            let gate = policy::Gate::new(policy::SecretPolicy::new(1));
            let result = storage::propose_and_commit(&vault, &text, storage::Version(0), uuid::Uuid::new_v4(), &std::collections::HashSet::new(), &std::collections::HashSet::new(), &gate, None).unwrap();
            assert!(matches!(result, storage::CommitResult::Committed(_) | storage::CommitResult::GitPending { .. }));
        }
        rebuild_index(&vault, &indexer).unwrap();

        let env = dense::shared_environment();
        let model = EmbeddingModel::load(env, onnx_path, tokenizer_path).expect("hash-verified real artifacts must load");
        let dense_index_path = root.join(format!("e2e-dense-index-{}.bin", uuid::Uuid::new_v4()));
        let scorer = Arc::new(ProductionDenseScorer::new(model, dense_index_path));
        scorer.rebuild(&vault).expect("rebuild against the real vault must succeed");

        Self { vault, indexer, scorer }
    }

    fn server(self) -> SuperBrainMcpServer {
        SuperBrainMcpServer::new(self.vault, self.indexer, Arc::new(CodeGraph::new()), "project-x").with_dense_retrieval(self.scorer)
    }
}

async fn connected_client(server: SuperBrainMcpServer) -> (rmcp::service::RunningService<rmcp::RoleClient, ()>, rmcp::service::RunningService<rmcp::RoleServer, SuperBrainMcpServer>) {
    let (server_transport, client_transport) = tokio::io::duplex(8192);
    let (server_result, client_result) = tokio::join!(server.serve(server_transport), ().serve(client_transport));
    (client_result.expect("client should initialize"), server_result.expect("server should initialize"))
}

#[tokio::test]
async fn a_real_external_agent_search_call_hits_the_real_densev2_path_and_ranks_the_semantic_match_first() {
    let Some((onnx_path, tokenizer_path)) = ensure_model_artifacts() else {
        return;
    };

    let fixture = Fixture::new(&onnx_path, &tokenizer_path);
    // Confirm, directly against the real scorer this test just built,
    // that the dense path is genuinely available before trusting the
    // MCP-level result to mean anything -- a structurally-unavailable
    // dense index (B4.0's own frozen fallback) would make this whole
    // test pass vacuously via the Lexical fallback instead, which is
    // NOT what Task 31 asks for.
    assert!(fixture.scorer.candidate_ids().is_some(), "the real dense index must be available after a successful rebuild -- if this fails, the DenseV2 path silently fell back to Lexical and the rest of this test would be meaningless");

    let (client, server) = connected_client(fixture.server()).await;

    // Deliberately shares almost no lexical tokens with "note-target"'s
    // real body ("kettle"/"whistle"/"steam"/"spout"/"tea" vs. this
    // query's own words) -- a query a purely lexical/BM25 engine would
    // have little to latch onto, but one DenseV2's real semantic
    // embedding should still recognize as closely related (boiling
    // water for a hot beverage).
    let query = "boiling water for a hot beverage in the kitchen";
    let params = CallToolRequestParams::new("search").with_arguments(object!({ "query": query, "k": 10 }));
    let result = client.call_tool(params).await.expect("tools/call search should succeed over the real transport");

    assert_ne!(result.is_error, Some(true), "search must not be reported as a tool-level error, got {result:?}");
    let structured = result.structured_content.expect("search's Json<SearchOutput> must produce structured_content");
    let results = structured["results"].as_array().expect("structured_content.results must be an array");

    assert!(!results.is_empty(), "DenseV2 must surface at least one real, non-empty result for this query, got {structured:?}");
    assert_eq!(results[0]["note_id"], "note-target", "the real DenseV2 path must rank the semantically-related note first, even though it shares almost no lexical tokens with the query -- got {structured:?}");

    client.cancel().await.expect("client should cancel cleanly");
    server.cancel().await.expect("server should cancel cleanly");
}
