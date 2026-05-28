# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 构建与运行命令

### Rust 后端

```bash
cargo build                          # Debug 构建
cargo build --release                # Release 构建
cargo run -- --version               # 查看版本
cargo run -- daemon --port 8371      # 启动 daemon API 服务
cargo test                           # 运行所有测试
cargo test --test integration_test   # 仅运行集成测试
cargo fmt -- --check                 # 检查格式
cargo clippy --all-targets -- -D warnings  # 代码检查（CI 使用此命令）
```

### TypeScript 前端

```bash
npm install                          # 安装依赖（首次）
npm run -w packages/core typecheck   # core 包类型检查
npm run -w packages/cli typecheck    # cli 包类型检查
npm run -w packages/cli dev          # 启动最小化 readline CLI
npm run -w packages/cli dev:ink      # 启动 Ink 富 CLI（推荐）
```

`packages/cli/package.json` 中定义了 npm scripts：
- `dev` — 运行最小化 readline REPL（`tsx src/index.ts`）
- `dev:ink` — 运行 Ink React 终端 UI（`tsx src/ink-app.tsx`）

Cargo.toml 中定义了三个二进制目标：
- `claude-code`（默认，src/main.rs）
- `claude-code-gui`（需要 `gui-egui` feature，src/gui/main.rs）
- `claude-code-web`（需要 `web` feature，src/web/main.rs）

## Feature Flags

默认 feature：`gui-egui`、`i18n`、`daemon`。可用 feature：`wasm`、`gui-egui`、`web`、`i18n`、`daemon`、`full`。

按需构建：`cargo build --features web` 或 `cargo build --no-default-features --features wasm`。

`daemon` feature 包含：`axum`、`tower`、`tower-http`、`tokio-stream`。

## 架构概述

本项目采用 **前后端分离架构**：
- **Rust 后端**：API 调用、工具执行、MCP 管理、权限验证，通过 HTTP daemon 暴露 REST + SSE API
- **Rust 前端（ratatui）**：Agent Loop、TUI 渲染、交互处理（权限提示、ask_user_question）。`src/tui/` 模块。推荐使用此前端。
- **TypeScript 前端（遗留）**：Agent Loop、UI 渲染、交互处理。位于 `packages/`。不再作为默认构建目标。

Rust 使用 **Tokio 异步优先架构**，调用 **OpenAI/DeepSeek 兼容的 chat completions API**（非 Anthropic Messages API）。

### 入口流程

**ratatui TUI 路径**（推荐）：
`cargo run` → 后台启动 daemon（随机端口）→ ratatui Terminal + AgentLoop → 通过 HTTP/SSE 与 daemon 通信

**TypeScript CLI 路径**（遗留）：
`npm run dev:ink` → 启动 Rust daemon（`cargo run -- daemon`）→ Ink React 应用渲染终端 UI → `useAgent` hook 驱动 agent loop → 通过 HTTP/SSE 与 daemon 通信

**Rust CLI 路径**（传统命令）：
`main.rs` → clap 解析 CLI 参数 → `cli/args.rs` 分发到命令处理器。

### 核心模块依赖

- **`state`** — `AppState` 持有共享状态（配置、对话、工具注册表、内存）。按值传递或用 `Arc<RwLock<>>` 包装后传给服务层。
- **`config`** — `Settings` 从 `~/.claude-code/settings.json` 加载。包含 `ApiConfig`、`McpConfig[]`、模型选择、内存/语音/插件设置。API Key 解析优先级：环境变量 `ANTHROPIC_API_KEY` > `DASHSCOPE_API_KEY` > `DEEPSEEK_API_KEY` > 配置文件值。
- **`api`** — `ApiClient` 封装 reqwest，请求 `/v1/chat/completions`（OpenAI 兼容格式）。支持 `chat()` 和 `chat_stream()`。`ChatMessage` 支持角色：user、assistant、system、tool。工具调用使用 `ToolDefinition`/`ToolCall` 类型。
- **`cli`** — CLI 参数解析和子命令分发（`CliArgs`、`Commands` 枚举）。交互式 REPL 已迁移至 TypeScript 前端（`packages/cli/`），Rust 侧仅保留 `args`、`branding`、`commands` 模块。
- **`daemon`** — HTTP API 服务（`src/daemon/`）。提供 REST + SSE 端点供 TypeScript 前端调用。启动：`cargo run -- daemon --port 8371`。
- **`agent`** — 共享的 SSE 流解析器（`StreamProcessor`），用于解析 OpenAI 兼容的 SSE 流式响应。

### 工具系统 (`tools`)

所有工具实现 `Tool` trait（`async_trait`）：`name()`、`description()`、`input_schema()`、`execute(input: Value) -> Result<ToolOutput, ToolError>`。`ToolRegistry` 内部为 `HashMap<String, Box<dyn Tool>>`，内置 9 个工具：file_read、file_edit、file_write、execute_command、search、list_files、git_operations、task_management、note_edit。

工具通过 `tool_definition()` 转换为 OpenAI function-calling 格式。

### MCP 模块 (`mcp`)

`McpManager` 管理 MCP 服务器连接（作为子进程启动，`filesystem` 除外，它作为内置工具运行）。内部子管理器：`ToolRegistry`（MCP 层级，与 `tools::ToolRegistry` 不同）、`ResourceManager`、`PromptManager`、`SamplingManager`。使用 JSON-RPC 2.0 消息协议。

### 技能框架 (`skills`)

`Skill` trait 定义 `execute(params, context) -> Result<SkillResult, SkillError>`。内置技能：commit、review、test、document、build。按 `SkillCategory` 枚举分类。`SkillRegistry` 管理注册，`SkillExecutor` 负责分发执行。

### 服务层 (`services`)

后台服务由 `ServiceManager` 管理：AutoDream（内存整合）、Voice、MagicDocs、TeamMemorySync、PluginMarketplace、Agents。每个服务接收 `Arc<RwLock<AppState>>` 和可选配置。

### TypeScript 前端 (`packages/`)

npm workspaces 管理两个包：

**`packages/core`** — 共享库：
- `client.ts` — `ApiClient` 类，封装 HTTP 请求（chatStream、executeTool、approveTool 等）
- `sse.ts` — SSE 行解析器（`parseSseLine`）
- `agent-loop.ts` — `AgentLoop` 类，完整的 agent loop 实现（SSE → tool calls → 循环）
- `types.ts` — TypeScript 类型定义

**`packages/cli`** — CLI 应用：
- `src/index.ts` — 最小化 readline REPL 入口
- `src/ink-app.tsx` — Ink React 终端 UI 入口
- `src/hooks/use-agent.ts` — React hook，封装 AgentLoop 状态管理（消息、状态、权限/问题弹窗）
- `src/components/` — Ink React 组件：
  - `app.tsx` — 根组件，初始化 ApiClient，管理布局
  - `chat-view.tsx` — 消息列表，自然流式布局
  - `message.tsx` — 消息气泡（user "▸ You" / assistant "● Wgenty" / tool / system）
  - `status-bar.tsx` — 状态栏，动画 spinner（thinking/streaming/executing/idle）
  - `input-box.tsx` — 输入框，圆角边框，▸ prompt，基于 ink-text-input
  - `welcome-banner.tsx` — ASCII 像素 Art 欢迎 Banner
  - `permission-modal.tsx` — 工具权限确认弹窗（y/n/a）
  - `question-modal.tsx` — ask_user_question 交互弹窗（单选/多选/自定义）

### Daemon API 端点

```
GET  /api/v1/health          → { status, version }
GET  /api/v1/config          → { model, api_base, max_tokens, ... }
POST /api/v1/chat/stream     → SSE 流式响应（OpenAI 兼容格式）
GET  /api/v1/tools           → [{ name, description, input_schema }]
POST /api/v1/tools/execute   → { success, output_type, content, permission_required? }
POST /api/v1/tools/approve   → { success }
GET  /api/v1/mcp/servers     → [{ name, status }]
```

### Feature-Gated 模块

- `wasm` — 通过 wasm-bindgen 支持浏览器环境
- `gui-egui` — 基于 eframe/egui 的原生 GUI
- `daemon` — HTTP API 服务（axum + SSE），供 TypeScript 前端调用
- `web` — 基于 axum 的插件市场 Web 界面
- `i18n` — 基于 fluent 的国际化（中英文本地化文件在 `locales/`）

### 配置与环境变量

- 配置文件：`~/.claude-code/settings.json`
- 环境变量：支持 `.env` 文件；关键变量：`ANTHROPIC_API_KEY`、`DASHSCOPE_API_KEY`、`DEEPSEEK_API_KEY`、`API_BASE_URL`、`CLAUDE_MODEL`、`RUST_LOG`
- 前端环境变量：`CLAUDE_DAEMON_PORT`（默认 8371）、`CLAUDE_DAEMON_CMD`、`CLAUDE_PROJECT_ROOT`
- 日志：使用 `tracing` + `EnvFilter`，默认级别 `claude_code_rs=info`

### CI

GitHub Actions 在 push/PR 到 main/develop 时运行：`cargo check`、`cargo test --all`、`cargo fmt -- --check`、`cargo clippy --all-targets -- -D warnings`，以及跨平台 Release 构建（Linux/Windows/macOS）。

## 历史架构变更

- **交互式 REPL 已从 Rust 迁移至 TypeScript**。原 `src/cli/repl.rs`、`tui_repl.rs`、`ui.rs`、`tui_input.rs`、`tui_history.rs`、`tui_ime.rs` 已删除。
- **已移除的 Rust 依赖**：`colored`、`crossterm`、`ratatui`、`tui-textarea`、`terminal_size`、`unicode-width`、`indicatif`、`libc`。
- **Agent Loop 归属前端**。Rust 端 `src/agent/` 模块保留 `StreamProcessor` 作为 SSE 解析参考实现，实际 agent loop 逻辑在 `packages/core/src/agent-loop.ts`。
- **SSE 透传注意事项**：daemon 的 `chat_stream` handler（`src/daemon/handlers.rs`）从上游 API 接收 SSE 行后会 strip `data: ` 前缀再重新包装，避免双重 `data:` 前缀导致前端解析失败。
