[![Rust](https://img.shields.io/badge/Rust-1.75%2B-orange?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/Platform-Windows%20%7C%20Linux%20%7C%20macOS-lightgrey.svg)]()

# Wgenty Code 🦀

Supports multiple AI providers: **Anthropic (Claude)**, **DeepSeek**, and **DashScope**.

---

## Features

- **Interactive TUI** - turn-based chat, structured plan panel, collapsed tool outputs, agent mode switching (`Normal / Plan / Accept Edits / Yolo`)
- **Plan Mode** - agent explores and proposes a plan before executing any mutations (`Ctrl+P` to toggle)
- **25 built-in tools** - file operations, code search, command execution, web access, and more
- **Multi-provider routing** - automatically detects and routes to the configured AI provider; model aliases like `sonnet`, `haiku`, `opus` are transparently mapped
- **Security by default** - all commands pass through a two-stage guardian review (rule-based + optional LLM review); critical-risk operations are auto-denied; OS-level sandboxing on all platforms
- **Sub-agent delegation** - complex tasks automatically decompose into parallel sub-tasks with recursion control
- **Session management** - save, load, and search past sessions
- **i18n** - 10-language support via Fluent localization
- **MCP support** - connect external MCP servers and use their tools transparently

---

## Quick Start

### Install via npm (recommended)

Requires [Node.js](https://nodejs.org/) 14+. The npm package downloads the correct prebuilt binary for your platform automatically—no Rust toolchain needed.

```bash
npm install -g wgenty-code
wgenty-code --version     # verify installation
```

Supported platforms: `linux-x64`, `linux-arm64`, `darwin-x64` (Intel macOS), `darwin-arm64` (Apple Silicon), `win32-x64`.

### Build from source

Requires **Rust** 1.75+ ([rustup.rs](https://rustup.rs/)) and **Git**.

```bash
git clone https://github.com/zhentingWu-wzt/wgenty-code.git
cd wgenty-code
cargo build --release
```

The binary is at `./target/release/wgenty-code` (`.exe` on Windows).

### Set your API key & run

```bash
# Set your API key (one of the following)
export ANTHROPIC_API_KEY="sk-ant-..."    # Anthropic Claude
# export DEEPSEEK_API_KEY="sk-..."       # DeepSeek
# export DASHSCOPE_API_KEY="sk-..."      # DashScope (Alibaba Cloud)

# Start coding
wgenty-code repl                            # if installed via npm
# ./target/release/wgenty-code repl         # if built from source
```

> Alternatively, set `api_key` in `~/.wgenty-code/settings.json`. Environment variables take priority.

### Docker

```bash
docker build -t wgenty-code:latest .
docker run -it --rm -v ~/.wgenty-code:/root/.wgenty-code wgenty-code:latest repl
```

### Configuration

Settings live in `~/.wgenty-code/settings.json` (auto-generated). Key options:

| Setting | Default | Purpose |
|:--------|:--------|:--------|
| `models.main.name` | `sonnet` | Main model alias (auto-mapped) |
| `models.small.name` | *(none)* | Smaller/cheaper model for delegated sub-tasks |
| `models.planner.name` | *(none)* | Dedicated model for plan generation |
| `models.transport.max_tokens` | `4096` | Max tokens per request |
| `agent.plan_mode` | `false` | Enable plan-before-execute mode |
| `agent.subagent.max_depth` | `3` | Max recursion depth for nested sub-agents |
| `agent.subagent.max_concurrent` | `5` | Max parallel sub-agents |
| `agent.token_budget.main_k` | `0` | Cumulative token limit (0 = unlimited) |
| `integrations.guardian.enabled` | `true` | Toggle command safety review |
| `storage.transcript.max_age_days` | `30` | Days to retain subagent transcripts |

> Use `wgenty-code config set <dotted.path> <value>` to change any setting, e.g. `config set agent.subagent.max_depth 5`.

Environment variable priority: `ANTHROPIC_API_KEY` > `DASHSCOPE_API_KEY` > `DEEPSEEK_API_KEY`. You can also set `api_key` directly in `settings.json`.

---

## CLI at a Glance

```bash
wgenty-code repl                      # Interactive TUI session
wgenty-code query -p "Refactor this"  # One-shot query
wgenty-code config set models.main.name haiku    # Switch models
wgenty-code mcp add --name fs         # Register an MCP server
wgenty-code sandbox status            # Check sandbox state
wgenty-code agent --agent-type plan --prompt "Design an API"
```

Full command reference: `wgenty-code --help`

### REPL Shortcuts

| Key | Action |
|:----|:-------|
| `Ctrl+P` | Toggle plan mode |
| `Ctrl+O` | Expand/collapse tool output |
| `Shift+Enter` | Newline in input |
| `Enter` | Submit input |
| `Ctrl+C` (double) | Quit |

---

## Development

```bash
cargo build                           # Debug build
cargo test --all                      # Full test suite
cargo clippy --all-targets -- -D warnings  # Zero warnings required
cargo fmt                             # Auto-format
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for branch conventions, commit format, and PR workflow.

---

## Documentation

- [QUICKSTART.md](QUICKSTART.md) - Hands-on getting started
- [INSTALL.md](INSTALL.md) - Platform-specific installation
- [PERFORMANCE_BENCHMARKS.md](PERFORMANCE_BENCHMARKS.md) - Full benchmark data
- [MIGRATION_GUIDE.md](MIGRATION_GUIDE.md) - Moving from TypeScript version
- [CHANGELOG.md](CHANGELOG.md) - Release history
- [CONTRIBUTING.md](CONTRIBUTING.md) - How to contribute

---

## License

MIT - see [LICENSE](LICENSE).
