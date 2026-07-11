# Changelog

All notable changes to Wgenty Code will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### BREAKING

- 项目说明（`AGENTS.md` / `WGENTY.md`）不再以 system message 形式注入 prompt 链。
  新增 `<system-reminder>` 通道，每轮拼到 user message 头部；同时聚合
  `~/.wgenty-code/WGENTY.md` 与 `~/.wgenty-code/rules/*.md`，以及
  `UserPromptSubmit` hook 的 `InjectContext` 动态注入。

  影响范围: 依赖旧 system message 文本（如 `# AGENTS.md`、
  `# WGENTY.md — 项目规则与约定`）的下游工具需要更新。

### Added

- 通用 MCP stdio client：支持 `initialize`、`tools/list`、`tools/call`，并将远程工具注册到主 Agent 与子 Agent 共用的 `ToolRegistry`。
- 第三方本地 CodeGraph MCP 集成，默认尝试启动 `codegraph serve --mcp`；不可用时非致命降级到 grep/lsp。
- `<system-reminder>` 注入通道（与 Claude Code 1:1 对齐：`# wgentyMd` 标题、双 preamble、`Contents of <abs-path> (<desc>):` 来源标注）。
- 4 个文件源 reader：用户级 `~/.wgenty-code/WGENTY.md` + `~/.wgenty-code/rules/*.md`，项目级 `WGENTY.md` + `AGENTS.md`。
- `HookAction::InjectContext` 端到端接通：`UserPromptSubmit` hook 的 `injected_content` + `priority` + `visibility` 现在通过 reminder 通道注入下一轮 user message。
- `PromptContext::project_root` + `with_project_root` builder。
- `ReminderOutput { to_model, to_transcript }` 双轨输出（按 `LayerVisibility` 分流）。
- Token 预算警告：按完整 reminder 块（preamble + 4 文件源）估算，超 2000 tokens 时 session 启动期一次性 `tracing::warn!`。

### Changed

- 移除项目内置 CodeGraph 索引器、`.codegraph/index.db` 格式和 `wgenty-code codegraph` CLI，统一使用第三方 CodeGraph MCP。
- `UserPromptSubmit` hook 触发时机：从 `tui/app/input.rs` 的 `tokio::spawn` fire-and-forget 改为 `AgentLoop::process_input_inner` 内 `await`（10s 超时降级为空 outcomes）。

## [0.1.0] - Unreleased

### Added

- Initial Rust rewrite of Wgenty Code CLI
- High-performance REPL with ratatui TUI
- Multi-provider API support (Anthropic, DeepSeek, DashScope)
- 25 built-in agent tools (filesystem, search, execution, meta)
- Two-stage guardian security review (rule-based + LLM)
- OS-level sandboxing (macOS Seatbelt, Linux seccomp-bpf, Windows Job Objects)
- 8-layer prompt assembly system
- RLM architecture (Planner → Executor → Aggregator) for complex task decomposition
- Plan mode with structured plan panel
- Sub-agent delegation with recursion control
- MCP protocol support
- Plugin system with hot-reload
- Session management (save/load/delete/search)
- Feature-gated modularity (CLI, GUI, Web)
- Internationalization (10 languages, Fluent format)
- Daemon mode with HTTP API
- Team memory sync
- Skills system with bundled skills

[0.1.0]: https://github.com/zhentingWu-wzt/wgenty-code/releases/tag/v0.1.0
