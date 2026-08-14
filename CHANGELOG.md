# Changelog

All notable changes to Wgenty Code will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Added (Daemon lifecycle)

- CLI 新增 `wgenty-code daemon status` / `wgenty-code daemon stop`：status 输出
  discovery 文件状态（端口/PID/心跳/token 一致性）并探测健康端点；stop 通过新的
  鉴权端点 `POST /api/v1/shutdown` 触发优雅退出（与 SIGINT 同路径，清理 token 与
  discovery 文件），并等待 daemon 停止应答。内嵌 daemon（TUI fallback）同样响应该端点。
- TUI 的 `DaemonClient` 在请求收到 401 时自动重读 `~/.wgenty-code/daemon.token` 并重试
  一次：daemon 重启后旧 TUI 不再因启动时缓存的 token 失效而全部 401（覆盖发消息、
  SSE 事件流、chat、viewer 创建、session 保存等路径）。
- TUI spawn 外部 daemon 后的健康探测从裸 TCP connect 升级为校验
  `GET /api/v1/health` 返回合法的 wgenty health body，避免误连占用 8371 端口的
  无关进程。

### Added (Desktop packaging)

- Desktop 打包链路打通：`tauri.conf.json` 的 `bundle.externalBin` 配置
  `binaries/wgenty-code`，daemon 以 target-triple 命名（`wgenty-code-<triple>[.exe]`）
  随安装包分发；本地一键打包脚本 `desktop/scripts/bundle.sh`（构建 daemon →
  复制为 externalBin 约定命名 → `web` 前端构建 → `cargo tauri build`）。
- 修复打包 app 内 daemon 查找：externalBin 二进制实际落在**主可执行文件同目录**
  （macOS `Contents/MacOS/`），此前 `locate_daemon_binary` 只查 resource dir
  （`Contents/Resources/`，仅有图标），打包后找不到 daemon 会回退 dev 路径而失败。
  现在按「exe 同目录 → resource dir → dev target/」顺序查找，并用精确命名约定
  （含 target-triple 连字符）排除 shell 自身 `wgenty-code-desktop` 与非二进制资源
  （`wgenty-code.icns` 等）；新增 6 个单元测试覆盖命名判定与查找顺序。
- 完整图标集：`tauri icon` 生成 `.icns`/`.ico`/多尺寸 png，替换原单一 `icon.png`。
- CI：`release.yml` 新增 `desktop` job（tag 触发），原生架构矩阵
  （macos-13 x64 / macos-14 arm64 / ubuntu-22.04 x64 / windows-2022 x64），
  构建 daemon + web 前端 + `tauri build`，产物上传 Release 与 artifact。

### Added (Multi-Project, web + daemon)

- daemon 新增项目注册表（`~/.wgenty-code/projects.json`）：`GET/POST/DELETE /api/v1/projects`，
  主项目（daemon working_dir）恒为第一项；项目 = 任意目录（不要求 git 仓库）。
- session 按项目路由：`POST /sessions` 接受 `project_path`，sessions 存于各项目
  `.wgenty-code/sessions/`；list/get/update/delete/run/events 跨项目聚合与路由。
- 权限 policy、checkpoint 快照（`<project>/.wgenty-code/checkpoints/`）、memory 池
  （`<project>/.wgenty-code/memory/`）、codegraph 探测均按 session 所属项目隔离；
  同时修复了"相对路径按主项目校验、按绑定 workdir 执行"的权限校验旁路。
- worktree 端点支持 `project` 参数（list/create/delete），非 git 项目返回 400。
- web 侧边栏升级为多项目树：添加/移除项目、按项目分组 task(worktree) 与 session，
  非 git 项目隐藏 task 功能；新建 session 对话框按项目预填工作区。

### Fixed (Web)

- 修复 web 端工具调用"不可见"：daemon 此前把每一轮 LLM 流结束（含
  `finish_reason="tool_calls"` 的工具轮次）都广播为 `turn_done`，web 在工具执行前
  就停止监听，工具执行过程和最终回答均不渲染。现在仅真正的 turn 结束才发布
  `TurnDone`（每个 run 恰好一次），web 端对 `tool_calls` 轮次边界亦做了兼容。

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

### Changed (Checkpoint)

- Per-mutating-tool `git stash` checkpoints are replaced by **per-turn file-content snapshots** under `.wgenty-code/checkpoints/<turn-id>/`.
- Before `file_edit` / `file_write` / `apply_patch`, the pre-edit content is captured once per (turn, path). `undo` rewinds only those files — unrelated untracked files are left alone (no `git reset` / `git clean` / stash pop).
- Retention: `agent.checkpoint.keep_n` (default 10). Subagent edits fold into the root turn snapshot.
- Manual `checkpoint` / `undo` tools remain; they now target turn ids instead of stash SHAs.

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
