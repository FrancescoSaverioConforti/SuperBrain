//! B5.2 Task 24: the `ServerHandler` implementation that turns
//! [`SuperBrainMcpServer`]'s already-registered tool routers (Task 22's
//! `read_tools`, Task 23's `graph_tools`/`memory_tools`/`knowledge_tools`,
//! combined once at construction time into `self.tool_router` — see
//! `tools/search.rs`'s `SuperBrainMcpServer::new`) into a real MCP server
//! that a transport can `serve`. Before this task, nothing in this crate
//! implemented `ServerHandler`; `call_tool`/`list_tools` were only
//! reachable by calling a `#[tool]` method directly in-process (exactly
//! what every prior task's own test files already did).
//!
//! `#[tool_handler(router = self.tool_router)]` generates `call_tool`/
//! `list_tools`/`get_tool`/`get_info` from that one field — it is the only
//! dispatch table this crate has, so a real MCP client sees EXACTLY the 12
//! tools Task 22+23's own registration tests already pin, no more, no
//! fewer.
//!
//! `instructions` documents this trust boundary for a real connecting
//! client/operator, not just the crate's own doc comments: a tool caller
//! reading the server's `initialize` response sees, in-band, that write-
//! shaped tools only ever produce a pending proposal.

use rmcp::{ServerHandler, tool_handler};

use super::tools::search::SuperBrainMcpServer;

#[tool_handler(
    router = self.tool_router,
    instructions = "SuperBrain Brain API over MCP. save_memory/update_memory/link_entities never execute a write directly -- each returns a ProposalId for a Pending proposal. Approval/rejection happen only through a separate, trusted local CLI surface never reachable over this transport."
)]
impl ServerHandler for SuperBrainMcpServer {}

#[cfg(test)]
mod tests {
    use rmcp::{ServerHandler, model::ServerConfig};

    use super::*;

    #[test]
    fn get_info_advertises_tools_capability_and_the_documented_write_boundary() {
        let info: ServerConfig = SuperBrainMcpServer::get_info(&test_server());
        assert!(info.capabilities.tools.is_some(), "tools capability must be advertised");
        let instructions = info.instructions.expect("instructions must be present");
        assert!(instructions.contains("ProposalId"), "instructions must document the write-proposal boundary, got: {instructions}");
    }

    fn test_server() -> SuperBrainMcpServer {
        let dir = std::env::temp_dir().join(format!(
            "superbrain-mcp-server-handler-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let vault = storage::Vault::open_with_default_indexer(dir.clone(), &dir.join("journal.sqlite")).unwrap();
        let indexer = std::sync::Arc::new(index::indexer_impl::RealIndexer::new());
        let graph = std::sync::Arc::new(codegraph_index::graph::CodeGraph::new());
        SuperBrainMcpServer::new(vault, indexer, graph, "project-x")
    }
}
