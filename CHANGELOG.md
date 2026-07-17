# Changelog

All notable changes to Wgenty Code will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Changed (Memory Quality)

- Compact 抽取改为“少而精”：收紧 system prompt，写入前按
  `write_importance_threshold` / `max_extract_per_compaction` 过滤，并丢弃
  task 类型与常见会话噪声（todo/进度/this session 等）。
- 默认阈值更严格：`max_memories=200`、`importance_threshold=0.6`、
  `age_threshold_hours=48`、`recall_top_n=3`；写入门槛默认 0.6、单次最多 3 条。
- 低价值 `Knowledge`/`Preference` 不再永久保留，改为 4× 基础 TTL 衰减；
  high-importance 仍不受年龄限制。
- 新增 CLI：`memory prune`（project+global）、`memory list [--min-importance] [--limit]`。
- `memory` 子命令读取 `settings.json` 的 `storage.memory.*` 阈值。

### BREAKING (Sandbox)

- Shell tools no longer default to Minimal + silent bare exec on sandbox failure.
  - **Normal / AcceptEdits:** Standard + **Full network** (package managers) + **HardFail**.
  - **Plan:** High + **no network** + **HardFail**.
  - **Yolo:** Minimal + Full network + DegradeWithMark (marked bypass if direct spawn).
- Use **Yolo**, or `integrations.sandbox.defaults_by_mode` / `fail_mode_by_mode`, or
  `integrations.sandbox.enabled: false` (forces DegradeWithMark + UI/metadata marks).
- `run_test.allow_network` only forces `NetworkPolicy::Full` within the mode's security
  level; it no longer drops the level to Minimal.
- CLI `sandbox enable|disable` now persists `integrations.sandbox.enabled`.

### Added (Sandbox ↔ Permission Mode)

- Profile matrix via `SandboxPolicyResolver` (`src/sandbox/policy.rs`) and
  `ToolContext.effective_mode` (includes Plan; not a process-global lock).
- Settings block `integrations.sandbox` (`enabled`, `defaults_by_mode`,
  `fail_mode_by_mode`).
- Shared exec helper `sandbox_exec` with fail-closed / degrade-with-mark metadata
  (`permission_mode`, `sandbox_level`, `sandbox_bypassed`,
  `sandbox_enforcement_fidelity`, …).
- TUI sticky session badge `⚠ SANDBOX BYPASS` when shell runs outside OS isolation.
- `sandbox status` shows enforcement fidelity and resolved mode → level / fail_mode.

### Changed (Subagent Lifecycle)

- `task` 工具统一为单一异步路径：每次调用立即生成一个 coordinator 拥有的子代理并返回结构化确认（`child_id` / `task_group_id` / `status:"running"`），移除 `background` 同步/后台模式开关。模型传入的 `background` 参数在兼容期内被忽略并在确认元数据中以 `ignored_arguments` 标注。
- 父代理（非根）在返回最终结果前必须执行一轮子结果合成（`collect_children_for_synthesis` + `begin_finalizing`），已完成的直接子代理结果作为 `<child-results>` 系统消息注入下一轮。
- 持久主代理永不为终态；已就绪的根直接子代理组通过 `POST /api/v1/agents/task-groups/claim` 原子领取（exactly-once），并由 TUI 以隐藏的合成续轮注入模型，不产生可见用户消息。
- `/clear` 与应用关机通过 coordinator 取消过时的子代理子树并推进 generation（`POST /api/v1/agents/generation/reset`、`POST /api/v1/agents/session/cancel`），过时 generation 的结果不再可领取。
- TUI 子代理导航改为基于短期 capability 的逐层下钻（Enter 下钻、Backspace 回退），不暴露后代/兄弟/全树。

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
- CodeGraph 可用性差异化探测：区分未安装 / 未初始化 / 已就绪 / 已免打扰四态，启动时 stderr 一行通知 + prompt 环境层注入实时状态，引导用户 `npm i -g @colbymchenry/codegraph` 或 `codegraph init`。未安装/已免打扰时短路跳过 MCP spawn。按项目持久化免打扰通过 `dismiss_codegraph_guidance` 元工具。
- `<system-reminder>` 注入通道（与 Claude Code 1:1 对齐：`# wgentyMd` 标题、双 preamble、`Contents of <abs-path> (<desc>):` 来源标注）。
- 4 个文件源 reader：用户级 `~/.wgenty-code/WGENTY.md` + `~/.wgenty-code/rules/*.md`，项目级 `WGENTY.md` + `AGENTS.md`。
- `HookAction::InjectContext` 端到端接通：`UserPromptSubmit` hook 的 `injected_content` + `priority` + `visibility` 现在通过 reminder 通道注入下一轮 user message。
- `PromptContext::project_root` + `with_project_root` builder。
- `ReminderOutput { to_model, to_transcript }` 双轨输出（按 `LayerVisibility` 分流）。
- Token 预算警告：按完整 reminder 块（preamble + 4 文件源）估算，超 2000 tokens 时 session 启动期一次性 `tracing::warn!`。

### Changed

- 移除项目内置 CodeGraph 索引器、`.codegraph/index.db` 格式和 `wgenty-code codegraph` CLI，统一使用第三方 CodeGraph MCP。
- `UserPromptSubmit` hook 触发时机：从 `tui/app/input.rs` 的 `tokio::spawn` fire-and-forget 改为 `AgentLoop::process_input_inner` 内 `await`（10s 超时降级为空 outcomes）。

### Fixed

- 修复 scoped UI viewer 凭据缺失或 daemon 重启后失效时，主窗口 subagent selector 消失的问题。
- 修复 scoped agent view 丢弃 subagent task label，导致 selector 名称显示为空的问题。
- 修复 `bundled-skills` 默认 feature 在 CI/全新 checkout 时因 `.wgenty-code/skills/` 缺失导致 `rust-embed` 编译失败（`#[derive(RustEmbed)] folder does not exist`）的问题：恢复该目录为已跟踪的打包源，并在 `.gitignore` 中以 `!.wgenty-code/skills/` 例外保护，防止再次被"开源清洗"误删。
- 修复 `BundledSkills::install_to` 只识别扁平 `<name>/SKILL.md`、对命名空间技能（`superpowers/<name>/`）和支撑文件（`comet/scripts/*`、`comet/reference/*`）处理错误的问题：改为镜像整棵嵌入树，按 `SKILL.md` 派生规范名（`<namespace>:<name>`），并对 `scripts/` 下文件设置可执行位。`count`/`list_bundled` 同步改为按 `SKILL.md` 计数与命名。

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
