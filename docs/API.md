# API Reference

Wgenty Code provides multiple integration interfaces.

## CLI

```bash
wgenty-code <subcommand> [options]
```

Full CLI reference: `wgenty-code --help`

### Subcommands

| Command | Description |
|:--------|:------------|
| `repl` | Interactive TUI session |
| `query` | One-shot query |
| `config` | Configuration management |
| `mcp` | MCP server management |
| `plugin` | Plugin management |
| `memory` | Memory and session management |
| `skills` | Skills management |
| `sandbox` | Sandbox control |
| `agent` | Run agent subcommand |
| `init` | Initialize project |
| `daemon` | Start HTTP daemon |

## Daemon HTTP API

Start the daemon:

```bash
wgenty-code daemon --port 8371
```

> Detailed API endpoints documentation is being prepared. See [GitHub Issues](https://github.com/zhentingWu-wzt/wgenty-code/issues) for progress.

## Configuration File

Path: `~/.wgenty-code/settings.json` (JSON, auto-generated)

Key sections: `models`, `agent`, `prompt`, `plugins`, `storage`, `integrations`.

See [WGENTY.md](../WGENTY.md#配置) for the full configuration reference.

## Environment Variables

| Variable | Priority | Description |
|:---------|:---------|:------------|
| `ANTHROPIC_API_KEY` | Highest | Anthropic API key |
| `DASHSCOPE_API_KEY` | — | DashScope API key |
| `DEEPSEEK_API_KEY` | — | DeepSeek API key |
| `API_BASE_URL` | — | Override API endpoint |
| `RUST_LOG` | — | Log level (e.g., `wgenty_code=debug`) |
