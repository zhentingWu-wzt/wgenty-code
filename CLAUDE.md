# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 构建与运行命令

```bash
cargo build                          # Debug 构建
cargo build --release                # Release 构建（产物在 target/release/claude-code）
cargo run -- --version               # 运行 CLI
cargo run -- repl                    # 启动 REPL
cargo test                           # 运行所有测试
cargo test --test integration_test   # 仅运行集成测试
cargo test test_repl_creation        # 按名称运行单个测试
cargo fmt -- --check                 # 检查格式
cargo clippy --all-targets -- -D warnings  # 代码检查（CI 使用此命令）
```

Cargo.toml 中定义了三个二进制目标：
- `claude-code`（默认，src/main.rs）
- `claude-code-gui`（需要 `gui-egui` feature，src/gui/main.rs）
- `claude-code-web`（需要 `web` feature，src/web/main.rs）

## Feature Flags

默认 feature：`gui-egui`、`i18n`。可用 feature：`wasm`、`gui-egui`、`web`、`i18n`、`full`。

按需构建：`cargo build --features web` 或 `cargo build --no-default-features --features wasm`。

## 架构概述

本项目是 Claude Code CLI 的 Rust 重写版本，采用 **Tokio 异步优先架构**，使用 **OpenAI/DeepSeek 兼容的 chat completions API**（非 Anthropic Messages API）。

### 入口流程

`main.rs` → 通过 clap 解析 CLI 参数（`Cli`/`CliArgs`）→ 分发到 `cli/args.rs` 中的命令处理器 → 各处理器创建所需的服务/管理器并执行。

### 核心模块依赖

- **`state`** — `AppState` 持有共享状态（配置、对话、工具注册表、内存）。按值传递或用 `Arc<RwLock<>>` 包装后传给服务层。
- **`config`** — `Settings` 从 `~/.claude-code/settings.json` 加载。包含 `ApiConfig`、`McpConfig[]`、模型选择、内存/语音/插件设置。API Key 解析优先级：环境变量 `ANTHROPIC_API_KEY` > `DASHSCOPE_API_KEY` > `DEEPSEEK_API_KEY` > 配置文件值。
- **`api`** — `ApiClient` 封装 reqwest，请求 `/v1/chat/completions`（OpenAI 兼容格式）。支持 `chat()` 和 `chat_stream()`。`ChatMessage` 支持角色：user、assistant、system、tool。工具调用使用 `ToolDefinition`/`ToolCall` 类型。
- **`cli`** — `CliArgs`（clap Parser），子命令定义为 `Commands` 枚举。`Repl` 实现交互循环，支持工具调用（循环直到响应中不再有 tool_calls）。`ui` 模块用 colored/ratatui 处理终端样式。

### 工具系统 (`tools`)

所有工具实现 `Tool` trait（`async_trait`）：`name()`、`description()`、`input_schema()`、`execute(input: Value) -> Result<ToolOutput, ToolError>`。`ToolRegistry` 内部为 `HashMap<String, Box<dyn Tool>>`，内置 9 个工具：file_read、file_edit、file_write、execute_command、search、list_files、git_operations、task_management、note_edit。

工具通过 `tool_definition()` 转换为 OpenAI function-calling 格式。

### MCP 模块 (`mcp`)

`McpManager` 管理 MCP 服务器连接（作为子进程启动，`filesystem` 除外，它作为内置工具运行）。内部子管理器：`ToolRegistry`（MCP 层级，与 `tools::ToolRegistry` 不同）、`ResourceManager`、`PromptManager`、`SamplingManager`。使用 JSON-RPC 2.0 消息协议。

### 技能框架 (`skills`)

`Skill` trait 定义 `execute(params, context) -> Result<SkillResult, SkillError>`。内置技能：commit、review、test、document、build。按 `SkillCategory` 枚举分类。`SkillRegistry` 管理注册，`SkillExecutor` 负责分发执行。

### 服务层 (`services`)

后台服务由 `ServiceManager` 管理：AutoDream（内存整合）、Voice、MagicDocs、TeamMemorySync、PluginMarketplace、Agents。每个服务接收 `Arc<RwLock<AppState>>` 和可选配置。

### Feature-Gated 模块

- `wasm` — 通过 wasm-bindgen 支持浏览器环境
- `gui-egui` — 基于 eframe/egui 的原生 GUI
- `web` — 基于 axum 的插件市场 Web 界面
- `i18n` — 基于 fluent 的国际化（中英文本地化文件在 `locales/`）

### 配置与环境变量

- 配置文件：`~/.claude-code/settings.json`
- 环境变量：支持 `.env` 文件；关键变量：`ANTHROPIC_API_KEY`、`DASHSCOPE_API_KEY`、`DEEPSEEK_API_KEY`、`API_BASE_URL`、`CLAUDE_MODEL`、`RUST_LOG`
- 日志：使用 `tracing` + `EnvFilter`，默认级别 `claude_code_rs=info`

### CI

GitHub Actions 在 push/PR 到 main/develop 时运行：`cargo check`、`cargo test --all`、`cargo fmt -- --check`、`cargo clippy --all-targets -- -D warnings`，以及跨平台 Release 构建（Linux/Windows/macOS）。
