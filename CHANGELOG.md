# Changelog

All notable changes to this project will be documented in this file.

## Unreleased

### BREAKING

- 项目说明（`AGENTS.md` / `WGENTY.md`）不再以 system message 形式注入 prompt 链。
  新增 `<system-reminder>` 通道，每轮拼到 user message 头部；同时聚合
  `~/.wgenty-code/WGENTY.md` 与 `~/.wgenty-code/rules/*.md`，以及
  `UserPromptSubmit` hook 的 `InjectContext` 动态注入。

  影响范围: 依赖旧 system message 文本（如 `# AGENTS.md`、
  `# WGENTY.md — 项目规则与约定`）的下游工具需要更新。

### Added

- `<system-reminder>` 注入通道（与 Claude Code 1:1 对齐：`# wgentyMd` 标题、双 preamble、`Contents of <abs-path> (<desc>):` 来源标注）。
- 4 个文件源 reader：用户级 `~/.wgenty-code/WGENTY.md` + `~/.wgenty-code/rules/*.md`，项目级 `WGENTY.md` + `AGENTS.md`。
- `HookAction::InjectContext` 端到端接通：`UserPromptSubmit` hook 的 `injected_content` + `priority` + `visibility` 现在通过 reminder 通道注入下一轮 user message。
- `PromptContext::project_root` + `with_project_root` builder。
- `ReminderOutput { to_model, to_transcript }` 双轨输出（按 `LayerVisibility` 分流）。
- Token 预算警告：按完整 reminder 块（preamble + 4 文件源）估算，超 2000 tokens 时 session 启动期一次性 `tracing::warn!`。

### Changed

- `UserPromptSubmit` hook 触发时机：从 `tui/app/input.rs` 的 `tokio::spawn` fire-and-forget 改为 `AgentLoop::process_input_inner` 内 `await`（10s 超时降级为空 outcomes）。
