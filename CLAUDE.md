
## CodeGraph

CodeGraph is provided by a third-party CLI (`codegraph`) connected via MCP stdio. When a `.codegraph/` directory exists in the repo root, the agent auto-connects to `codegraph serve --mcp` and exposes `codegraph_node` / `codegraph_explore` as MCP-backed tools.

To set up: install the `codegraph` CLI, then run `codegraph init` in the repo root. The agent handles the rest at startup.
