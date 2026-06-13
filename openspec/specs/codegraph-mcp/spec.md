# codegraph-mcp Specification

## Purpose
TBD - created by archiving change code-graph-tool. Update Purpose after archive.
## Requirements
### Requirement: MCP tool listing
The system SHALL expose `codegraph_explore` and `codegraph_node` as MCP tools via the MCP protocol (JSON-RPC 2.0).

#### Scenario: MCP tools/list includes codegraph tools
- **WHEN** an MCP client calls `tools/list`
- **THEN** the response includes `codegraph_explore` and `codegraph_node` with their input schemas

### Requirement: MCP tool invocation
The system SHALL handle MCP `tools/call` requests for codegraph tools and return results in MCP-compliant format.

#### Scenario: codegraph_explore via MCP
- **WHEN** an MCP client calls `tools/call` with `name: "codegraph_explore"` and `arguments: {"query": "Tool trait implementors"}`
- **THEN** the system queries the codegraph index and returns results as MCP `text` content

#### Scenario: codegraph_node via MCP
- **WHEN** an MCP client calls `tools/call` with `name: "codegraph_node"` and `arguments: {"symbol": "ToolRegistry"}`
- **THEN** the system returns the symbol definition, references, and call graph in MCP `text` content

### Requirement: Index freshness check
The system SHALL verify the codegraph index exists before serving MCP requests and return a clear error if no index is found.

#### Scenario: MCP query without index
- **WHEN** an MCP client calls a codegraph tool but `.codegraph/index.db` does not exist
- **THEN** the system returns an error: "No codegraph index found. Run `wgenty-code codegraph index` first."

### Requirement: Built-in tool parity
The MCP codegraph tools SHALL have identical behavior and output format to the built-in `codegraph_explore` and `codegraph_node` tools.

#### Scenario: Same query, same result
- **WHEN** the same `codegraph_node("ToolRegistry")` query is made via built-in tool and via MCP
- **THEN** both return identical structured output

