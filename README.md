[![Rust](https://img.shields.io/badge/Rust-1.75%2B-orange?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/Platform-Windows%20%7C%20Linux%20%7C%20macOS-lightgrey.svg)]()

# Wgenty Code 🦀

> A high-performance AI coding agent that lives in your terminal. Explore, edit, search, and refactor entire codebases through natural language — with fast startup, a tiny binary, and zero runtime dependencies.

Wgenty Code is an LLM-driven coding assistant built in Rust. Instead of copy-pasting snippets into a chat box, you point it at a real project: it reads files, runs searches, executes commands, applies edits, and iterates until the task is done — all from a single self-contained binary with no Node.js or Python runtime required.

It ships with **25 built-in tools** (filesystem, code search, command execution, web access, …), a **two-stage command guardian**, and **OS-level sandboxing** on every platform, so the agent can act autonomously while staying safe by default. It supports multiple AI providers with automatic routing — **Anthropic (Claude)**, **OpenAI**, **DeepSeek**, and any OpenAI-compatible endpoint (DashScope, Ollama, vLLM, …) — and model aliases like `sonnet`, `haiku`, `opus` are mapped transparently.

[中文文档](README.zh.md)

---

## Features

- **Interactive TUI** — turn-based chat, a structured plan panel, collapsible tool output, and agent mode switching (`Normal / Plan / Accept Edits / Yolo`)
- **Plan Mode** — the agent explores the codebase and proposes a plan *before* executing any mutations (`Ctrl+P` to toggle); nothing changes until you approve
- **25 built-in tools** — file read/write/edit, code search (grep/glob/LSP), command execution, web search/fetch, and more
- **Multi-provider routing** — auto-detects the provider from the base URL; switch between Claude, OpenAI, DeepSeek, or self-hosted endpoints by changing one setting
- **Security by default** — every command passes a two-stage guardian review (rule-based + optional LLM review); critical-risk operations are auto-denied; OS-level sandboxing on macOS (Seatbelt), Linux (seccomp-bpf), and Windows (Job Objects)
- **Sub-agent delegation** — complex tasks automatically decompose into parallel sub-tasks with recursion control (RLM pipeline: Planner → Executor → Aggregator)
- **Session & memory management** — save, load, and search past sessions; project-scoped + global memory with TF-IDF recall
- **MCP support** — connect external MCP servers and use their tools transparently inside the agent loop
- **i18n** — 10-language support via Fluent localization

---

## Why Rust?

The original TypeScript implementation carried an entire Node.js runtime — 164 MB of dependencies, 100 MB idle memory, and JIT warm-up latency on every call. The Rust rewrite eliminates all of that:

| Metric | Rust | TypeScript | Improvement |
|:-------|:----:|:----------:|:-----------:|
| Cold start | **58 ms** | 152 ms | **2.6× faster** |
| Binary size | **5 MB** | 164 MB | **97% smaller** |
| Idle memory | **10 MB** | 100 MB | **90% less** |
| Config read | **6 ms** | 150 ms | **25× faster** |
| REPL key latency | **<1 ms** | 100 ms | **instant** |

Beyond the numbers, Rust's ownership model eliminates whole classes of bugs — no null-pointer exceptions, no data races, no GC pauses. The compiler proves memory and thread safety at build time.

See [PERFORMANCE_BENCHMARKS.md](PERFORMANCE_BENCHMARKS.md) for full data.

---

## How It Works

### 🔒 Secure by default

Every command the agent wants to run goes through a **two-stage Guardian review**:

1. **Rule filtering** — static patterns block obviously dangerous operations (e.g. `rm -rf /`, `curl | sh`)
2. **LLM review** *(optional)* — a model evaluates ambiguous commands and classifies risk as `Low / Medium / High / Critical`

Critical-risk operations are auto-denied. The execution surface is further isolated by an **OS-level sandbox** (macOS Seatbelt, Linux seccomp-bpf, Windows Job Objects), degrading gracefully to a no-op when kernel support is unavailable.

### 🧩 25 tools, one abstraction

All agent capabilities — file ops, code search, command execution, web access — implement a single `Tool` trait. A key design choice: **`is_read_only()` defaults to `false`**. Every read-only tool must explicitly declare itself safe, so the guardian always errs on the side of caution.

### 📐 8-layer prompt assembly

The system prompt is assembled from 8 independently toggleable layers:

```
base_instructions -> permissions -> developer -> collaboration
  -> environment -> skills -> agents_md -> wgenty_md
```

### 👥 RLM — recursive task decomposition

Complex tasks flow through a **Planner → Executor → Aggregator** pipeline:

- `task` tool — simple, single-shot delegation; auto-routes complex prompts to the RLM pipeline
- `delegate` tool — decomposes a task into structured sub-tasks, runs them in parallel by dependency layer, and merges results
- Recursion is hard-limited by `agent.subagent.max_depth` (default `1`)

### 🏗️ Plan Mode

Enable `plan_mode` in config or press `Ctrl+P` in the REPL:

1. The agent explores the codebase, reads relevant files, and asks clarifying questions
2. It calls `update_plan` to render a structured plan in the UI panel
3. It waits for your approval before making any changes

The plan panel shows per-step status: `○ pending / ◐ in_progress / ✓ done`.

### 🖥️ TUI

A terminal interface built on [ratatui](https://ratatui.rs/):

- **Turn-based chat** — solid separators between turns, dashed within
- **Structured plan panel** — inline plan rendering with status markers
- **Collapsible tool output** — `Ctrl+O` to expand, keeping noise down
- **Agent mode switching** — `Normal / Plan / Accept Edits / Yolo` with color-coded labels
- **Multi-line input** — `Shift+Enter` for newline, full IME/CJK support

---

## Quick Start

### Install via npm (recommended)

Requires [Node.js](https://nodejs.org/) 14+. The npm package downloads the correct prebuilt binary for your platform automatically — no Rust toolchain needed.

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
wgenty-code                            # if installed via npm
# ./target/release/wgenty-code         # if built from source
```

> Alternatively, set `api_key` in `~/.wgenty-code/settings.json`. Environment variables take priority over the config file.

### Docker

```bash
docker build -t wgenty-code:latest .
docker run -it --rm -v ~/.wgenty-code:/root/.wgenty-code wgenty-code:latest repl
```

### Configuration

Settings live in `~/.wgenty-code/settings.json` (auto-generated on first run). Key options:

| Setting | Default | Purpose |
|:--------|:--------|:--------|
| `models.main.name` | `sonnet` | Main model alias (auto-mapped) |
| `models.small.name` | *(none)* | Smaller/cheaper model for delegated sub-tasks |
| `models.planner.name` | *(none)* | Dedicated model for plan generation |
| `models.transport.max_tokens` | `4096` | Max tokens per request |
| `agent.plan_mode` | `false` | Enable plan-before-execute mode |
| `agent.subagent.max_depth` | `1` | Max recursion depth for nested sub-agents (1 = subagents cannot spawn further subagents; raise to allow recursion) |
| `agent.subagent.max_concurrent` | `5` | Max parallel sub-agents |
| `agent.token_budget.main_k` | `0` | Cumulative token limit (0 = unlimited) |
| `integrations.guardian.enabled` | `true` | Toggle command safety review |
| `storage.transcript.max_age_days` | `30` | Days to retain subagent transcripts |

> Use `wgenty-code config set <dotted.path> <value>` to change any setting, e.g. `config set agent.subagent.max_depth 5`.

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

**Repository**: [github.com/zhentingWu-wzt/wgenty-code](https://github.com/zhentingWu-wzt/wgenty-code)
