[![Rust](https://img.shields.io/badge/Rust-1.75%2B-orange?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/Platform-Windows%20%7C%20Linux%20%7C%20macOS-lightgrey.svg)]()

# Wgenty Code 🦀

> **High-performance coding agent CLI, rewritten in Rust** — 2.5× faster startup, 97% smaller binary, zero runtime dependencies.

Wgenty Code is an LLM-powered coding agent that reads, writes, and refactors your codebase through a terminal interface. It supports multiple AI providers (Anthropic, DeepSeek, DashScope) and ships as a single, self-contained binary with no Node.js or Python runtime required.

---

## Why Rust?

The original TypeScript implementation carried the weight of an entire Node.js runtime — 164 MB of dependencies, 100 MB of idle memory, and JIT warm-up latency on every invocation. Rewriting in Rust eliminates all of that:

| Metric | Rust | TypeScript | Improvement |
|:-------|:----:|:----------:|:-----------:|
| Cold start | **58 ms** | 152 ms | **2.6× faster** |
| Binary size | **5 MB** | 164 MB | **97% smaller** |
| Idle memory | **10 MB** | 100 MB | **90% less** |
| Config read | **6 ms** | 150 ms | **25× faster** |
| REPL keystroke | **<1 ms** | 100 ms | **instant** |

Beyond raw numbers, Rust's ownership model eliminates entire categories of bugs: no null-pointer exceptions, no data races, no garbage-collection pauses. The compiler proves memory safety and thread safety at build time — before the binary ever runs.

See [PERFORMANCE_BENCHMARKS.md](PERFORMANCE_BENCHMARKS.md) for the full benchmark report.

---

## Design Highlights

### 🔒 Security by Default

Every command the agent wants to execute passes through a **two-stage guardian review** before touching the system:

1. **Rule-based filter** — static patterns block obviously dangerous operations (e.g., `rm -rf /`, `curl | sh`).
2. **LLM review** (optional) — the model itself evaluates the risk of ambiguous commands, classifying them as `Low / Medium / High / Critical`.

Critical-risk operations are auto-denied. The entire execution surface is further isolated by **OS-level sandboxing**: Seatbelt on macOS, seccomp-bpf on Linux, Job Objects on Windows. If sandbox facilities are unavailable, the system degrades gracefully to a no-op backend rather than crashing.

### 🧩 25 Tools, One Abstraction

All agent capabilities — file operations, code search, command execution, web access — implement a single `Tool` trait with a crucial design choice: **`is_read_only()` defaults to `false`**. Every read-only tool (like `file_read`, `grep`, `glob`) must explicitly declare itself safe. This forces tool authors to think about side effects and prevents the guardian from accidentally granting write access to tools that modify state.

Tools are also **provider-aware**: the `ToolRegistry` can dynamically remove tools that an AI provider doesn't support (e.g., `apply_patch` for non-Anthropic backends), keeping the agent's capabilities honest and preventing runtime failures.

### 📐 8-Layer Prompt Assembly

The system prompt isn't a single blob — it's assembled from 8 independently toggleable instruction layers:

```
base_instructions → permissions → developer → collaboration
  → environment → skills → agents_md → wgenty_md
```

Each layer can be enabled/disabled, allowing precise control over what context the model receives. Repository-specific instructions from `AGENTS.md` and `WGENTY.md` files are automatically injected with scoping rules — deeper-nested files take precedence over parent-level ones.

### 👥 RLM Architecture — Recursive Language Model

Complex tasks automatically decompose into independent sub-tasks through a **Planner → Executor → Aggregator** pipeline:

```
model → task tool (simple tasks)
      → delegate tool (complex: auto-decompose → parallel execute → merge)
      → dispatch tool (map-reduce: grep results → per-item analyze → aggregate)
```

**RLM pipeline (delegate tool):**
- Planner calls LLM to decompose the task into structured JSON sub-tasks
- Executor runs sub-tasks in parallel, ordered by dependency levels
- Aggregator merges all results into a coherent response

**Auto-routing (`task` tool):**
The `task` tool detects complex prompts (>500 chars, multi-step indicators) and automatically routes to the RLM pipeline — the model doesn't need to decide which tool to use.

**Recursion control:**
- Depth propagation via `_subagent_depth` — each sub-agent knows its level
- Hard limit via `agent.subagent.max_depth` (default: 3) — prevents runaway recursion
- Self-referential: sub-agents can spawn further sub-agents when depth allows
- Trace ID: every sub-agent logs a unique monotonically-increasing ID

### 🏗️ Plan Mode

Toggle `plan_mode` in config or press `Ctrl+P` in the REPL. In plan mode:

1. The agent explores the codebase, reads relevant files, and asks clarifying questions
2. Calls `update_plan` to present a structured plan in the UI panel
3. Waits for user approval before executing any mutations

The plan panel renders each step with a status indicator: `○ pending / ◐ in_progress / ✓ completed`. After approval, the agent executes step-by-step, updating the plan as it progresses.

### 🖥️ TUI Features

Built with [ratatui](https://ratatui.rs/), the terminal interface includes:

- **Turn-based chat** — solid separators between turns, dotted separators within turns
- **Structured plan panel** — plan steps rendered inline with status indicators
- **Collapsed tool results** — tool outputs default-collapsed (Ctrl+O to expand), minimizing noise
- **Agent mode switching** — `Normal / Plan / Accept Edits / Yolo` modes with color-coded labels
- **Multi-line input** — Shift+Enter for newlines, full IME/CJK support
- **Session management** — save/load/delete sessions with search

### 📦 Feature-Gated Modularity

The project compiles to **three separate binaries** from a single codebase, each with its own feature gate:

| Binary | Features | Purpose |
|:-------|:---------|:--------|
| `wgenty-code` | `default` (gui-egui, i18n, daemon) | Full-featured CLI with TUI |
| `wgenty-code-gui` | `gui-egui` | Desktop GUI via egui |
| `wgenty-code-web` | `web` | Web interface via Askama + Axum |

Build without any features (`--no-default-features`) and you get a pure CLI binary under 5 MB — ideal for CI pipelines, Docker containers, or embedding into other tools. Add features on demand: WASM compilation for browser targets, i18n for 10-language Fluent localization, daemon mode for long-running server processes.

### 🌍 Multi-Provider, Transparent Routing

The API layer automatically detects which AI provider to use based on your configuration and routes requests accordingly — no code changes needed to switch between Anthropic, DeepSeek, and DashScope. Model aliases like `sonnet`, `haiku`, and `opus` are transparently mapped to full provider-specific model IDs, and request/response formats are transformed behind a unified `ApiClient` trait.

---

## Quick Start

### Prerequisites
- **Rust** 1.75+ ([rustup.rs](https://rustup.rs/))
- **Git**

### Install & Run

```bash
git clone https://github.com/zhentingWu-wzt/wgenty-code.git
cd wgenty-code
cargo build --release

# Set your API key
export ANTHROPIC_API_KEY="sk-ant-..."

# Start coding
./target/release/wgenty-code repl
```

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
| `agent.rlm.enabled` | `true` | RLM pipeline master switch |
| `prompt.collaboration_mode` | *(none)* | Agent behavior: `plan`, `execute`, or `pair_programming` |
| `integrations.guardian.enabled` | `true` | Toggle command safety review |
| `storage.transcript.max_age_days` | `30` | Days to retain subagent transcripts |

> Use `wgenty-code config set <dotted.path> <value>` to change any setting, e.g. `config set agent.subagent.max_depth 5`. The old flat key names (`model`, `max_subagent_depth`, etc.) are no longer supported.

---

## CLI at a Glance

```bash
wgenty-code repl                      # Interactive TUI session
wgenty-code query -p "Refactor this"  # One-shot query
wgenty-code config set model haiku    # Switch models
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

# Performance check
cargo build --release
time ./target/release/wgenty-code --version
ls -lh ./target/release/wgenty-code
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for branch conventions, commit format, and PR workflow.

---

## Documentation

- [QUICKSTART.md](QUICKSTART.md) — Hands-on getting started
- [INSTALL.md](INSTALL.md) — Platform-specific installation
- [PERFORMANCE_BENCHMARKS.md](PERFORMANCE_BENCHMARKS.md) — Full benchmark data
- [MIGRATION_GUIDE.md](MIGRATION_GUIDE.md) — Moving from TypeScript version
- [CHANGELOG.md](CHANGELOG.md) — Release history
- [CONTRIBUTING.md](CONTRIBUTING.md) — How to contribute

---

## License

MIT — see [LICENSE](LICENSE).

**Repository**: [github.com/zhentingWu-wzt/wgenty-code](https://github.com/zhentingWu-wzt/wgenty-code)
