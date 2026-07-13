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

    let mut config = McpConfig::codegraph();
    config.cwd = Some(project.path().to_path_buf());
    let session = McpClientSession::spawn(&config)
        .await
        .expect("connect to CodeGraph MCP server");

    let tools = session
        .list_tools()
        .await
        .expect("discover CodeGraph tools");
    let node_tool = tools
        .iter()
        .find(|tool| tool.name == "codegraph_node")
        .expect("CodeGraph should expose codegraph_node");
    let symbol_key = if node_tool.input_schema["properties"].get("symbol").is_some() {
        "symbol"
    } else {
        "name"
    };
    let mut arguments = serde_json::Map::new();
    arguments.insert(symbol_key.to_string(), json!("known_symbol"));
    let result = session
        .call_tool("codegraph_node", serde_json::Value::Object(arguments))
        .await
        .expect("call codegraph_node through MCP");
    let rendered = result.to_string();
    assert!(rendered.contains("known_symbol"), "result: {rendered}");
    assert!(rendered.contains("main.rs"), "result: {rendered}");

    session.shutdown().await.expect("stop CodeGraph MCP server");
}
