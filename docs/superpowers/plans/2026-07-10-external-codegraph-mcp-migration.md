# External CodeGraph MCP Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the built-in CodeGraph engine with third-party CodeGraph exposed through a reusable MCP stdio client, while preserving the existing `Tool` abstraction used by main agents and subagents.

**Architecture:** Add a generic MCP stdio session that performs `initialize`, `notifications/initialized`, `tools/list`, and `tools/call`. Wrap discovered remote tools in `McpToolProxy` implementations and register them in the existing `crate::tools::ToolRegistry`; the model-facing agent loop remains unchanged. Once an external CodeGraph server is proven discoverable and callable, remove the duplicate in-process indexer, its CLI commands, dedicated dependencies, and old `.codegraph/index.db` guidance.

**Tech Stack:** Rust 2021, Tokio process/stdin/stdout, JSON-RPC 2.0, MCP stdio transport, serde/serde_json, async-trait, existing `Tool` and guardian policy APIs.

---

### Task 1: MCP stdio protocol session

**Files:**
- Create: `src/mcp/client.rs`
- Modify: `src/mcp/mod.rs`
- Test: `src/mcp/client.rs`

- [ ] **Step 1: Write failing protocol tests**

Add tests using a small shell-backed fake MCP server. Verify that the client sends `initialize`, accepts the server response, sends `notifications/initialized`, discovers `tools/list`, and forwards `tools/call` arguments.

- [ ] **Step 2: Run the focused tests and verify RED**

Run: `cargo test mcp::client::tests -- --nocapture`

Expected: compilation or assertion failure because `McpClientSession` does not exist.

- [ ] **Step 3: Implement the minimal session**

Implement:

```rust
pub struct McpClientSession {
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    stdout: Mutex<BufReader<ChildStdout>>,
    next_id: AtomicI64,
}

impl McpClientSession {
    pub async fn spawn(config: &McpConfig) -> anyhow::Result<Arc<Self>>;
    pub async fn list_tools(&self) -> anyhow::Result<Vec<McpRemoteTool>>;
    pub async fn call_tool(&self, name: &str, arguments: Value) -> anyhow::Result<Value>;
    pub async fn shutdown(&self) -> anyhow::Result<()>;
}
```

Use one request at a time per session so responses cannot be consumed by the wrong caller. Ignore JSON-RPC notifications while waiting for the matching response ID. Pipe stdin/stdout, inherit or pipe stderr for diagnostics, apply configured `cwd` and `env`, and attach context to spawn/read/write/JSON failures.

- [ ] **Step 4: Run focused tests and verify GREEN**

Run: `cargo test mcp::client::tests -- --nocapture`

Expected: all MCP client tests pass.

### Task 2: Remote tools as native agent tools

**Files:**
- Create: `src/mcp/proxy.rs`
- Modify: `src/mcp/mod.rs`
- Modify: `src/tools/mod.rs`
- Test: `src/mcp/proxy.rs`

- [ ] **Step 1: Write failing proxy tests**

Verify that a discovered MCP tool preserves its name, description, and input schema; `execute()` sends the same arguments to the remote server; MCP text content becomes `ToolOutput.content`; and remote protocol errors become actionable `ToolError` values.

- [ ] **Step 2: Run tests and verify RED**

Run: `cargo test mcp::proxy::tests -- --nocapture`

Expected: failure because `McpToolProxy` and external registration do not exist.

- [ ] **Step 3: Implement proxy and registry insertion**

Add:

```rust
pub struct McpToolProxy {
    exposed_name: String,
    remote_name: String,
    description: String,
    input_schema: Value,
    server_name: String,
    session: Arc<McpClientSession>,
    read_only: bool,
}
```

Expose a `ToolRegistry::register_external()` path using the existing duplicate-name replacement semantics. Prefix only on collision (`<server>__<tool>`), leaving standard CodeGraph names such as `codegraph_node` unchanged. Treat remote tools as write-capable by default; mark the known CodeGraph query tools read-only through explicit server/tool classification.

- [ ] **Step 4: Run proxy and registry tests**

Run: `cargo test mcp::proxy::tests tools::tests -- --nocapture`

Expected: all focused tests pass.

### Task 3: Runtime MCP server lifecycle and discovery

**Files:**
- Modify: `src/mcp/mod.rs`
- Modify: `src/daemon/state.rs`
- Modify: `src/state/mod.rs`
- Modify: `src/daemon/handlers.rs`
- Test: `src/mcp/mod.rs`

- [ ] **Step 1: Write failing manager tests**

Verify `McpManager::connect_configured()` starts enabled servers, records real tool counts, returns discovered proxies, continues when one optional server fails, and shuts children down on explicit stop.

- [ ] **Step 2: Run tests and verify RED**

Run: `cargo test mcp::tests -- --nocapture`

Expected: tests fail because the manager currently only spawns processes and never initializes them.

- [ ] **Step 3: Implement connection management**

Replace raw child-only connections with `Arc<McpClientSession>` plus discovered tool metadata. Add an async startup method that accepts the application `ToolRegistry`, discovers tools, and installs `McpToolProxy` values. Preserve per-server status, start time, and last error.

- [ ] **Step 4: Wire startup into agent runtime**

Initialize configured MCP servers before model tool definitions are assembled. Keep server startup outside registry locks and avoid holding async locks across process I/O. Ensure the daemon list-tools endpoint and subagent tool list include remote tools.

- [ ] **Step 5: Run manager and agent integration tests**

Run: `cargo test mcp daemon:: -- --nocapture`

Expected: configured fake MCP tools appear in the same registry as built-ins and can be executed through the normal tool executor.

### Task 4: Third-party CodeGraph integration

**Files:**
- Modify: `src/config/mcp_config.rs`
- Modify: `src/config/services.rs`
- Modify: `src/prompts/base.md`
- Modify: `README.md`
- Test: MCP integration tests

- [ ] **Step 1: Add a failing CodeGraph configuration test**

Verify that a default or generated CodeGraph MCP configuration uses:

```text
command = "codegraph"
args = ["serve", "--mcp"]
```

and that absence of the executable produces a non-fatal server error while grep/LSP tools remain available.

- [ ] **Step 2: Run the test and verify RED**

Run: `cargo test codegraph_mcp -- --nocapture`

Expected: failure because no external CodeGraph configuration helper exists.

- [ ] **Step 3: Add configuration and prompt guidance**

Document installation and project initialization separately from agent execution. Prompt guidance must refer to MCP tools first and use `codegraph init`/`codegraph sync`, not `wgenty-code codegraph index`.

- [ ] **Step 4: Run a real end-to-end check**

In an isolated temporary repository/index, launch `codegraph serve --mcp`, discover `codegraph_node`, call it through `McpToolProxy`, and assert that the result contains a known symbol and source location.

### Task 5: Remove the built-in CodeGraph implementation

**Files:**
- Delete: `src/tools/codegraph/`
- Modify: `src/tools/mod.rs`
- Modify: `src/cli/mod.rs`
- Modify: `src/cli/args.rs`
- Modify: `src/permissions/policy.rs`
- Modify: `src/tui/components/chat.rs`
- Modify: `src/tui/util.rs`
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: CodeGraph benchmark scripts and documentation

- [ ] **Step 1: Add/adjust tests for the external-only contract**

Assert that no built-in CodeGraph tools are registered without MCP configuration, that external CodeGraph tools retain their familiar names, and that the rest of the tool registry remains functional when CodeGraph is unavailable.

- [ ] **Step 2: Remove built-in registrations and CLI commands**

Delete the in-process engine and `wgenty-code codegraph index/query/clean`. Remove stale references to `.codegraph/index.db`, `call_path`, `symbol_batch`, and `module_summary` where they are no longer supplied by the default external MCP tool set.

- [ ] **Step 3: Remove dedicated dependencies**

Remove `tree-sitter`, `tree-sitter-rust`, `tree-sitter-java`, and `tree-sitter-python`. Retain shared crates such as `rusqlite`, `walkdir`, `notify`, and `sha2` where other modules still use them. Regenerate `Cargo.lock` with Cargo.

- [ ] **Step 4: Run focused migration tests**

Run: `cargo test mcp tools permissions -- --nocapture`

Expected: all tests pass with external MCP tools and without the built-in engine.

### Task 6: Verification and review

**Files:**
- Modify: `CHANGELOG.md`
- Modify: `WGENTY.md`

- [ ] **Step 1: Format**

Run: `cargo fmt`

- [ ] **Step 2: Lint**

Run: `cargo clippy --all-targets -- -D warnings`

- [ ] **Step 3: Test**

Run: `cargo test --all`

- [ ] **Step 4: Verify release build and binary size**

Run: `cargo build --release && ls -lh target/release/wgenty-code`

- [ ] **Step 5: Review the final diff**

Check error context, child-process cleanup, lock duration, read-only classification, cross-platform command handling, and prompt/docs consistency. Confirm no existing unrelated workspace changes were reverted.
