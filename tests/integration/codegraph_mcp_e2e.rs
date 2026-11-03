use std::process::Command;

use serde_json::json;
use wgenty_code::config::McpConfig;
use wgenty_code::mcp::client::McpClientSession;

#[tokio::test]
#[ignore = "requires the third-party codegraph CLI"]
async fn third_party_codegraph_is_discovered_and_called_over_mcp() {
    let project = tempfile::tempdir().expect("create temporary CodeGraph project");
    std::fs::write(
        project.path().join("main.rs"),
        "fn known_symbol() -> &'static str { \"ok\" }\nfn main() { known_symbol(); }\n",
    )
    .expect("write Rust fixture");

    let status = Command::new("codegraph")
        .arg("init")
        .arg(project.path())
        .status()
        .expect("start codegraph init");
    assert!(status.success(), "codegraph init should succeed");

    let config = McpConfig::codegraph_for_project(project.path());
    let session = McpClientSession::spawn(&config)
        .await
        .expect("connect to CodeGraph MCP server");

    let tools = session
        .list_tools()
        .await
        .expect("discover CodeGraph tools");
    // CodeGraph >= 1.4 consolidated its tool surface down to the single
    // `codegraph_explore` tool (older builds exposed codegraph_node/_search/
    // _callers/...). Asserting on the consolidated name keeps this test
    // meaningful across both old and new server versions.
    let explore_tool = tools
        .iter()
        .find(|tool| tool.name == "codegraph_explore")
        .expect("CodeGraph should expose codegraph_explore");
    let symbol_key = if explore_tool.input_schema["properties"]
        .get("query")
        .is_some()
    {
        "query"
    } else {
        "symbol"
    };
    let mut arguments = serde_json::Map::new();
    arguments.insert(symbol_key.to_string(), json!("known_symbol"));
    let result = session
        .call_tool("codegraph_explore", serde_json::Value::Object(arguments))
        .await
        .expect("call codegraph_explore through MCP");
    let rendered = result.to_string();
    assert!(rendered.contains("known_symbol"), "result: {rendered}");
    assert!(rendered.contains("main.rs"), "result: {rendered}");

    session.shutdown().await.expect("stop CodeGraph MCP server");
}
