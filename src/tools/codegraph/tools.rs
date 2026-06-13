use crate::tools::codegraph::query::QueryEngine;
use crate::tools::codegraph::store::IndexStore;
use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;
use std::sync::{Arc, OnceLock};

/// Lazy-initialized engine — created on first use from cwd.
static ENGINE: OnceLock<Arc<QueryEngine>> = OnceLock::new();

fn get_engine() -> Result<Arc<QueryEngine>, ToolError> {
    if let Some(engine) = ENGINE.get() {
        return Ok(engine.clone());
    }
    let cwd = std::env::current_dir().map_err(|e| ToolError {
        message: format!("Failed to get current directory: {}", e),
        code: Some("cwd_error".to_string()),
    })?;
    let db_path = cwd.join(".codegraph").join("index.db");
    if !db_path.exists() {
        return Err(ToolError {
            message: "No codegraph index found. Run `wgenty-code codegraph index` first.".to_string(),
            code: Some("no_index".to_string()),
        });
    }
    let store = Arc::new(IndexStore::open(&cwd).map_err(|e| ToolError {
        message: format!("Failed to open index: {}", e),
        code: Some("store_error".to_string()),
    })?);
    let engine = Arc::new(QueryEngine::new(store));
    // Ok to ignore error — another thread may have raced us
    let _ = ENGINE.set(engine.clone());
    Ok(engine)
}

/// Tool exposing `codegraph_node` — single symbol lookup.
pub struct CodegraphNodeTool;

impl CodegraphNodeTool {
    pub fn new() -> Self { Self }
}

impl Default for CodegraphNodeTool {
    fn default() -> Self { Self::new() }
}

#[async_trait]
impl Tool for CodegraphNodeTool {
    fn name(&self) -> &str { "codegraph_node" }

    fn is_read_only(&self) -> bool { true }

    fn description(&self) -> &str {
        "Look up a Rust symbol by name. Returns definition location, signature, references, and callers/callees. Requires a codegraph index (run `wgenty-code codegraph index` first)."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "symbol": {
                    "type": "string",
                    "description": "Symbol name to look up (e.g. function name, type name)"
                }
            },
            "required": ["symbol"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let symbol = input["symbol"].as_str().ok_or_else(|| ToolError {
            message: "symbol is required".to_string(),
            code: Some("missing_parameter".to_string()),
        })?;

        let engine = get_engine()?;
        let result = engine.codegraph_node(symbol).map_err(|e| ToolError {
            message: format!("codegraph_node query failed: {}", e),
            code: Some("query_error".to_string()),
        })?;

        let mut lines = Vec::new();
        if !result.found {
            lines.push(format!("Symbol `{}` not found.", symbol));
            if !result.suggestions.is_empty() {
                lines.push("\nSuggestions:".to_string());
                for s in &result.suggestions {
                    lines.push(format!("  - {} ({}) at {}:{}", s.name, s.kind.as_str(), s.file_path, s.line));
                }
            }
        } else {
            for sym in &result.symbols {
                lines.push(format!(
                    "## {} ({})\n  Location: {}:{}\n  Visibility: {}\n  Signature: {}",
                    sym.name,
                    sym.kind.as_str(),
                    sym.file_path,
                    sym.line,
                    sym.visibility.as_str(),
                    sym.signature.as_deref().unwrap_or("N/A"),
                ));
            }
            if !result.references.is_empty() {
                lines.push(format!("\n### References ({})", result.references.len()));
                for r in result.references.iter().take(10) {
                    lines.push(format!("  - {}:{} — {:?}", r.file_path, r.line, r.context));
                }
            }
            if !result.callers.is_empty() {
                lines.push(format!("\n### Callers ({})", result.callers.len()));
                for c in result.callers.iter().take(10) {
                    lines.push(format!("  - {} ({}:{})", c.name, c.file_path, c.line));
                }
            }
            if !result.callees.is_empty() {
                lines.push(format!("\n### Callees ({})", result.callees.len()));
                for c in result.callees.iter().take(10) {
                    lines.push(format!("  - {} ({}:{})", c.name, c.file_path, c.line));
                }
            }
        }

        Ok(ToolOutput {
            output_type: "text".to_string(),
            content: lines.join("\n"),
            metadata: {
                let mut m = std::collections::HashMap::new();
                m.insert("found".to_string(), serde_json::json!(result.found));
                m.insert("count".to_string(), serde_json::json!(result.symbols.len()));
                m
            },
        })
    }
}

/// Tool exposing `codegraph_explore` — symbol exploration with call graph.
pub struct CodegraphExploreTool;

impl CodegraphExploreTool {
    pub fn new() -> Self { Self }
}

impl Default for CodegraphExploreTool {
    fn default() -> Self { Self::new() }
}

#[async_trait]
impl Tool for CodegraphExploreTool {
    fn name(&self) -> &str { "codegraph_explore" }

    fn is_read_only(&self) -> bool { true }

    fn description(&self) -> &str {
        "Explore code symbols and their relationships. Returns relevant symbols and call paths. Requires a codegraph index (run `wgenty-code codegraph index` first)."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Query string to find relevant symbols (e.g. function name, module path)"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let query = input["query"].as_str().ok_or_else(|| ToolError {
            message: "query is required".to_string(),
            code: Some("missing_parameter".to_string()),
        })?;

        let engine = get_engine()?;
        let result = engine.codegraph_explore(query).map_err(|e| ToolError {
            message: format!("codegraph_explore query failed: {}", e),
            code: Some("query_error".to_string()),
        })?;

        let mut lines = Vec::new();
        lines.push(format!(
            "Found {} symbol(s) matching `{}`:\n",
            result.symbols.len(),
            query
        ));
        for sym in &result.symbols {
            lines.push(format!(
                "  - {} ({}) at {}:{}",
                sym.name, sym.kind.as_str(), sym.file_path, sym.line
            ));
        }
        if !result.call_graph.is_empty() {
            lines.push(format!("\n### Call Graph ({})", result.call_graph.len()));
            for entry in result.call_graph.iter().take(20) {
                lines.push(format!(
                    "  - {} → {} (depth {}, {}:{})",
                    entry.relation, entry.symbol_name, entry.depth, entry.file_path, entry.line
                ));
            }
        }

        Ok(ToolOutput {
            output_type: "text".to_string(),
            content: lines.join("\n"),
            metadata: {
                let mut m = std::collections::HashMap::new();
                m.insert("symbol_count".to_string(), serde_json::json!(result.symbols.len()));
                m
            },
        })
    }
}
