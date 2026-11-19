//! B5.9 Task 27: capability discovery and tool metadata cache.
//!
//! `discover_tools` calls the real MCP `tools/list` method against an
//! already-[`connect`](crate::connect)ed [`McpConnection`] and populates a
//! [`ToolCache`] with what it finds — never a mock, never synthesized
//! data. A server's real `tools/list` reply that is well-formed MCP
//! protocol JSON (already guaranteed by `rmcp`'s own deserialization —
//! see this module's own doc comment on `validate_tools`) but
//! semantically untrustworthy AT THE DOMAIN LEVEL (an empty tool name, a
//! name repeated within the same list, or a name shaped like a path
//! traversal attempt) is rejected OUTRIGHT: nothing from that reply is
//! cached, not even the tools that individually looked fine — a server
//! that can't be trusted to name its OWN tools correctly gets no partial
//! credit.
//!
//! **This task does not classify, authorize, or invoke anything.**
//! [`DiscoveredTool::classification`] is `Option<()>`, always `None` here
//! — a deliberately inert placeholder, not Task 28's real
//! `ToolClassification` enum (which does not exist yet and is not
//! guessed at by this task). `None` carries no security meaning of its
//! own: it is not "safe," not "read-only," not an implicit grant of
//! anything. Task 28 is the only place a tool ever becomes
//! authorizable, and its own `authorize` function must treat every
//! still-`None`/not-yet-classified tool as denied, the same deny-by-default
//! discipline this plan has applied to every other capability.

use std::collections::HashMap;

use crate::{McpClientError, McpConnection};

/// One tool as discovered from a real external MCP server's `tools/list`
/// reply — the real name/description/schema that server sent, never
/// synthesized. `classification` is the Task 28 placeholder described in
/// this module's own doc comment: always `None` here, read by nothing in
/// this task, and carrying no security meaning on its own.
#[derive(Debug, Clone, PartialEq)]
pub struct DiscoveredTool {
    pub server: String,
    pub name: String,
    pub description: Option<String>,
    pub input_schema: serde_json::Map<String, serde_json::Value>,
    /// Always `None` as produced by this task. See this module's own doc
    /// comment: not Task 28's real classification, not a security
    /// decision, not read by `discover_tools` itself.
    pub classification: Option<()>,
}

/// A cache of discovered tools, keyed by `(server, tool_name)` — the same
/// key pair `ExternalToolMetadata` (Task 28) will need to look a specific
/// tool back up by. `discover_tools` is the only way entries are ever
/// added; there is no public method that inserts an entry without having
/// gone through its own validation.
#[derive(Debug, Default)]
pub struct ToolCache {
    entries: HashMap<(String, String), DiscoveredTool>,
}

impl ToolCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, server: &str, tool_name: &str) -> Option<&DiscoveredTool> {
        self.entries.get(&(server.to_string(), tool_name.to_string()))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Every cached tool belonging to `server`, in no particular order —
    /// used by tests and by later tasks to enumerate what one server
    /// offers.
    pub fn tools_for_server(&self, server: &str) -> Vec<&DiscoveredTool> {
        self.entries.values().filter(|t| t.server == server).collect()
    }

    fn insert_all(&mut self, tools: Vec<DiscoveredTool>) {
        for tool in tools {
            self.entries.insert((tool.server.clone(), tool.name.clone()), tool);
        }
    }
}

/// Calls the real `tools/list` method on `conn` and validates the reply
/// before trusting any of it (see `validate_tools`). On success, every
/// validated tool is inserted into `cache` (replacing any prior entry for
/// the same `(server, tool_name)` key — a fresh discovery is authoritative
/// over a stale one) and the newly discovered tools are also returned.
///
/// On a malformed reply, `cache` is left completely UNCHANGED — not even
/// the tools that individually validated are inserted, per this module's
/// own "no partial credit" doc comment — and
/// [`McpClientError::MalformedResponse`] is returned.
pub async fn discover_tools(conn: &McpConnection, server: &str, cache: &mut ToolCache) -> Result<Vec<DiscoveredTool>, McpClientError> {
    let result = conn.service().list_tools(None).await.map_err(|e| McpClientError::ConnectionFailed(e.to_string()))?;

    let tools = validate_tools(server, result.tools)?;
    cache.insert_all(tools.clone());
    Ok(tools)
}

/// Domain-level validation `rmcp`'s own deserialization does not already
/// perform (that layer only guarantees well-formed MCP protocol JSON —
/// see `rmcp::model::Tool`'s own fields, all already real strings/schema
/// objects by the time this function runs; a genuinely malformed JSON-RPC
/// reply never reaches here at all, rejected earlier at the transport
/// layer, per Task 24's own confirmed `rmcp` behavior).
///
/// Rejects the WHOLE list (never a partial subset) if ANY tool in it:
/// - has an empty name (a tool nothing can address by name is not a real,
///   usable tool — the server's own responsibility to never send this);
/// - shares its name with another tool in the SAME reply (an ambiguous
///   server-side naming collision — which one would a later `tools/call`
///   by that name even reach?);
/// - has a name containing `..`, `/`, or `\` (a name shaped like a path
///   traversal attempt has no legitimate reason to exist in a TOOL NAME,
///   which is an identifier, never a path component — rejecting it here
///   is defense-in-depth against a malicious server trying to smuggle a
///   path-like string into SuperBrain's own cache/metadata, even though
///   nothing in this crate yet uses a tool name to construct a real
///   filesystem path).
fn validate_tools(server: &str, tools: Vec<rmcp::model::Tool>) -> Result<Vec<DiscoveredTool>, McpClientError> {
    let mut seen_names = std::collections::HashSet::new();
    for tool in &tools {
        if tool.name.is_empty() {
            return Err(McpClientError::MalformedResponse("tool list contains a tool with an empty name".to_string()));
        }
        if tool.name.contains("..") || tool.name.contains('/') || tool.name.contains('\\') {
            return Err(McpClientError::MalformedResponse(format!("tool name {:?} contains a path-traversal-shaped character", tool.name)));
        }
        if !seen_names.insert(tool.name.clone()) {
            return Err(McpClientError::MalformedResponse(format!("tool list contains a duplicate tool name {:?}", tool.name)));
        }
    }

    Ok(tools
        .into_iter()
        .map(|tool| DiscoveredTool {
            server: server.to_string(),
            name: tool.name.to_string(),
            description: tool.description.map(|d| d.to_string()),
            input_schema: (*tool.input_schema).clone(),
            classification: None,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str) -> rmcp::model::Tool {
        let mut t = rmcp::model::Tool::new(name.to_string(), String::new(), serde_json::Map::new());
        t.description = None;
        t
    }

    // --- validate_tools: the pure validation/mapping logic, no network ---

    #[test]
    fn a_well_formed_list_validates_and_maps_every_field() {
        let mut t = tool("search");
        t.description = Some("searches things".to_string().into());
        let result = validate_tools("server-a", vec![t]).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].server, "server-a");
        assert_eq!(result[0].name, "search");
        assert_eq!(result[0].description, Some("searches things".to_string()));
        assert_eq!(result[0].classification, None, "Task 27 must never set a classification -- that is Task 28's job");
    }

    #[test]
    fn an_empty_tool_name_is_rejected_as_malformed() {
        let result = validate_tools("server-a", vec![tool("")]);
        assert!(matches!(result, Err(McpClientError::MalformedResponse(_))), "expected MalformedResponse, got {result:?}");
    }

    #[test]
    fn a_duplicate_tool_name_within_the_same_list_is_rejected_as_malformed() {
        let result = validate_tools("server-a", vec![tool("search"), tool("search")]);
        assert!(matches!(result, Err(McpClientError::MalformedResponse(_))), "expected MalformedResponse, got {result:?}");
    }

    #[test]
    fn a_path_traversal_shaped_tool_name_is_rejected_as_malformed() {
        for bad_name in ["../etc/passwd", "a/b", "a\\b", "..\\windows"] {
            let result = validate_tools("server-a", vec![tool(bad_name)]);
            assert!(matches!(result, Err(McpClientError::MalformedResponse(_))), "expected MalformedResponse for {bad_name:?}, got {result:?}");
        }
    }

    #[test]
    fn a_malformed_list_is_rejected_wholesale_not_as_a_partial_subset() {
        // "search" alone would validate fine; paired with a duplicate of
        // itself, the WHOLE list must be rejected -- not just the second
        // occurrence silently dropped.
        let result = validate_tools("server-a", vec![tool("search"), tool("search")]);
        assert!(result.is_err());
    }

    #[test]
    fn two_distinct_valid_tool_names_both_validate() {
        let result = validate_tools("server-a", vec![tool("search"), tool("retrieve")]).unwrap();
        assert_eq!(result.len(), 2);
    }

    // --- ToolCache: insertion discipline ---

    #[test]
    fn cache_starts_empty() {
        let cache = ToolCache::new();
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn inserting_validated_tools_populates_the_cache_keyed_by_server_and_name() {
        let mut cache = ToolCache::new();
        let tools = validate_tools("server-a", vec![tool("search")]).unwrap();
        cache.insert_all(tools);

        assert_eq!(cache.len(), 1);
        let found = cache.get("server-a", "search").expect("tool must be findable by (server, name)");
        assert_eq!(found.name, "search");
    }

    #[test]
    fn a_fresh_discovery_replaces_a_stale_entry_for_the_same_server_and_name() {
        let mut cache = ToolCache::new();
        let mut first = tool("search");
        first.description = Some("old description".to_string().into());
        cache.insert_all(validate_tools("server-a", vec![first]).unwrap());

        let mut second = tool("search");
        second.description = Some("new description".to_string().into());
        cache.insert_all(validate_tools("server-a", vec![second]).unwrap());

        assert_eq!(cache.len(), 1, "the same (server, name) key must replace, not duplicate");
        assert_eq!(cache.get("server-a", "search").unwrap().description, Some("new description".to_string()));
    }

    #[test]
    fn tools_from_different_servers_with_the_same_name_do_not_collide() {
        let mut cache = ToolCache::new();
        cache.insert_all(validate_tools("server-a", vec![tool("search")]).unwrap());
        cache.insert_all(validate_tools("server-b", vec![tool("search")]).unwrap());

        assert_eq!(cache.len(), 2, "the cache key includes the server, so identically-named tools on different servers must both be kept");
        assert!(cache.get("server-a", "search").is_some());
        assert!(cache.get("server-b", "search").is_some());
    }

    #[test]
    fn tools_for_server_returns_only_that_servers_entries() {
        let mut cache = ToolCache::new();
        cache.insert_all(validate_tools("server-a", vec![tool("search"), tool("retrieve")]).unwrap());
        cache.insert_all(validate_tools("server-b", vec![tool("other")]).unwrap());

        let a_tools = cache.tools_for_server("server-a");
        assert_eq!(a_tools.len(), 2);
        assert!(a_tools.iter().all(|t| t.server == "server-a"));
    }

    // --- discover_tools: the real end-to-end call, against a real
    //     external MCP server spawned as a real child process ---

    mod end_to_end {
        use super::*;
        use crate::{McpServerRegistration, TransportConfig, connect};

        /// A real, minimal external MCP server: Python's own stdlib is
        /// enough to speak just enough MCP over stdio for `tools/list` to
        /// succeed -- no mock, no fixture framework, a genuinely separate
        /// process SuperBrain's real `mcp-client` connects to exactly as
        /// it would a real third-party MCP server. `python` is tried
        /// before `python3`: on Windows, `python3` often resolves to the
        /// Microsoft Store alias stub (which exits non-zero when no
        /// Store-installed Python is registered) even when a real
        /// `python.exe` is on PATH, so trying `python3` first would
        /// falsely skip this test on a real Windows dev machine that has
        /// Python installed the ordinary way.
        fn python_or_skip() -> Option<&'static str> {
            ["python", "python3"].into_iter().find(|candidate| std::process::Command::new(candidate).arg("--version").output().is_ok_and(|o| o.status.success()))
        }

        const MINIMAL_SERVER_SCRIPT: &str = r#"
import sys, json

def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    req = json.loads(line)
    method = req.get("method")
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": req["id"], "result": {
            "protocolVersion": "2025-11-25",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "minimal-test-server", "version": "0.0.1"},
        }})
    elif method == "notifications/initialized":
        pass
    elif method == "tools/list":
        send({"jsonrpc": "2.0", "id": req["id"], "result": {
            "tools": [
                {"name": "echo", "description": "echoes input", "inputSchema": {"type": "object"}},
            ]
        }})
    elif req.get("id") is not None:
        send({"jsonrpc": "2.0", "id": req["id"], "error": {"code": -32601, "message": "method not found"}})
"#;

        #[tokio::test]
        async fn discover_tools_against_a_real_external_server_populates_the_cache() {
            let Some(python) = python_or_skip() else {
                eprintln!("skipping: no python3/python interpreter found on PATH");
                return;
            };
            let registration = McpServerRegistration {
                name: "minimal-test-server".to_string(),
                transport: TransportConfig::ChildProcess { command: python.to_string(), args: vec!["-c".to_string(), MINIMAL_SERVER_SCRIPT.to_string()] },
                enabled: true,
            };
            let conn = connect(&registration).await.expect("connect to the real minimal server must succeed");

            let mut cache = ToolCache::new();
            let discovered = discover_tools(&conn, "minimal-test-server", &mut cache).await.expect("discover_tools against a real, well-behaved server must succeed");

            assert_eq!(discovered.len(), 1);
            assert_eq!(discovered[0].name, "echo");
            assert_eq!(discovered[0].classification, None);
            assert_eq!(cache.len(), 1);
            assert!(cache.get("minimal-test-server", "echo").is_some());

            conn.close().await.unwrap();
        }

        const MALFORMED_SERVER_SCRIPT: &str = r#"
import sys, json

def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    req = json.loads(line)
    method = req.get("method")
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": req["id"], "result": {
            "protocolVersion": "2025-11-25",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "malformed-test-server", "version": "0.0.1"},
        }})
    elif method == "notifications/initialized":
        pass
    elif method == "tools/list":
        # A real, well-behaved-at-the-protocol-level reply (valid JSON-RPC,
        # valid MCP tools/list shape) that is nonetheless untrustworthy at
        # the domain level this module's own validate_tools checks for: two
        # distinct tool entries sharing the same name.
        send({"jsonrpc": "2.0", "id": req["id"], "result": {
            "tools": [
                {"name": "echo", "description": "first", "inputSchema": {"type": "object"}},
                {"name": "echo", "description": "second, same name", "inputSchema": {"type": "object"}},
            ]
        }})
    elif req.get("id") is not None:
        send({"jsonrpc": "2.0", "id": req["id"], "error": {"code": -32601, "message": "method not found"}})
"#;

        #[tokio::test]
        async fn discover_tools_against_a_real_server_with_a_malformed_tool_list_is_rejected_and_caches_nothing() {
            let Some(python) = python_or_skip() else {
                eprintln!("skipping: no python3/python interpreter found on PATH");
                return;
            };
            let registration = McpServerRegistration {
                name: "malformed-test-server".to_string(),
                transport: TransportConfig::ChildProcess { command: python.to_string(), args: vec!["-c".to_string(), MALFORMED_SERVER_SCRIPT.to_string()] },
                enabled: true,
            };
            let conn = connect(&registration).await.expect("connect to the malformed-reply server must still succeed -- the handshake itself is fine");

            let mut cache = ToolCache::new();
            let result = discover_tools(&conn, "malformed-test-server", &mut cache).await;

            assert!(matches!(result, Err(McpClientError::MalformedResponse(_))), "a real server's duplicate-named tool list must be rejected over the real wire, got {result:?}");
            assert!(cache.is_empty(), "no tool from the rejected list must be cached, not even the ones that individually looked fine");

            conn.close().await.unwrap();
        }
    }
}
