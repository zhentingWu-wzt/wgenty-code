# Changelog

All notable changes to Wgenty Code will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Fixed (Daemon lifecycle)

- 双实例竞态：旧 daemon 退出时无条件删除 `daemon.token` / `daemon.json`，会把仍在
  运行的新实例的鉴权文件删掉（token 只在启动时写一次，不会重写）——web/TUI 全部
  401 表现为"无法连接 daemon"，且因无 client 连接触发空闲关闭把新实例也杀掉。
  三处清理点（daemon 退出、TUI 内嵌 daemon 退出、`kill_predecessor` 强杀后）均改为
  所有权检查：仅当文件内容仍属于本实例（token 匹配 / pid 匹配）才删除。
- vite dev proxy 读取 token 增加兜底：`daemon.token` 缺失时回退读 `daemon.json`
  里的 token（discovery 文件由存活 daemon 的心跳每 30s 重写，更可靠）。
- 优雅关闭永久悬挂：hyper 的 graceful shutdown 会等在途连接结束，而 SSE 长连接
  （session events、trace 流、client 心跳）按设计永不主动结束——收到关闭请求后
  进程变成僵尸（日志有 "initiating graceful shutdown" 但进程不退，旧实例堆积）。
  现对排空阶段加 5s 兜底，超时强制退出。
- `kill_predecessor` 误判旧实例已停止：此前以"健康探测失败"判定 down，但优雅关闭
  中的 daemon 会立即停止 accept（探测失败）而进程仍在排空——于是提前宣告成功，
  从不走到 SIGTERM/SIGKILL 升级，僵尸得以堆积。现改为同时确认 PID 已退出
  （`kill -0` / `tasklist`），否则照常升级强杀。

### Fixed (Subagent 结果回传)

- **subagent 结果不实时通知主 agent（web 场景）**：`task` 工具是异步派发，
  子代理终态结果汇入 coordinator 的 task-group 后，此前只有 TUI 客户端
  轮询 `POST /agents/task-groups/claim` 来领取并发起综合轮；web 没有任何
  claim 调用，daemon 自身的 run loop 也不感知 task-group——结果永远滞留，
  主 agent 直到用户再发消息也拿不到。现改为 daemon 侧闭环：
  - coordinator 在 group 就绪（全部直接子代理结果齐）时通过 broadcast
    发出就绪 ping；
  - daemon 的 continuation 调度器收到 ping 后自行 claim 就绪组并发起
    synthesis 轮（`<child-results>` 消息格式与 TUI 一致）；会话正忙时跳过，
    由 RunFinished 事件兜底重查；尊重 failed-save 屏障；
  - claim 后照常广播 `task_group_result` 全局事件；
  - web 新增 `useContinuationObserver`：订阅全局事件流
    `GET /api/v1/events`，收到 `task_group_result` 后通过
    `observeDaemonRun` 实时附着该会话的事件流渲染综合轮（此前 web 只渲染
    自己发起的 run，daemon 发起的轮次刷新前不可见）。
  TUI 的轮询路径保留，与 daemon claim 之间由 exactly-once 语义保证不重复。

### Fixed (Web 通讯韧性)

- **页面彻底卡死的传输层根因**：旧代码每轮 turn 泄漏一条 SSE 长连接（reader 从不
  cancel），加上 HMR websocket、心跳、两条 trace 流，常驻连接累积到 Chrome 单域名
  6 条 HTTP/1.1 上限后，所有新请求（发送、停止、健康检查）被浏览器永久排队——
  表现为"发消息没响应、点停止也没反应"。修复组合：
  - 长连接 SSE（session events、trace 流）改为**直连 daemon origin**
    （`127.0.0.1:8371`，经 vite 新中间件 `/__daemon-info` 下发端口和 token），
    与页面 origin 的连接配额分离；短请求仍走 vite 代理；
  - 取消 session-scoped trace 流，冷启动恢复改用一次性 REST
    `GET /subagents/trace/replay`（daemon 新增端点），常驻流每页再减一条；
  - turn 的 reader 在结束时正确 cancel，不再泄漏连接；
  - `runSession` / 订阅 fetch 增加 15s 超时兜底，被饿死的请求报可感知错误而非永久悬挂。
  - 直连 daemon 被浏览器拦截（CORS / Private Network Access）时回退同源 vite 代理，
    仅网络层 TypeError 触发回退，用户停止与看门狗 abort 仍照常生效。
  - **`usePermissionTrace` 的 trace 流在每次会话切换时泄漏一条连接**（effect 依赖
    `activeId`，cleanup 只翻转标记位，而 keepalive-only 流的 `reader.read()` 永不
    返回，旧连接永久存活）——切换 4-5 次会话后直连 origin 的 6 连接预算被泄漏流
    占满，发消息时 `sessionEvents` 连接被浏览器排队，15s 看门狗报
    "stream connect timed out"。修复：流 effect 只依赖 `client`，cleanup 用
    AbortController 真正中止在途 fetch（`traceStream` 新增 signal 参数）；会话切换
    的一次性 replay 拆成独立 effect，不再重启流。
- **"Failed to fetch" 的 daemon 侧根因**：CORS origin 白名单只写了
  localhost/127.0.0.1:3000/5173，而 vite 常以 `--host` 运行——端口被占时用 5174、
  或经局域网 IP / 主机名访问时，浏览器直接拦死跨域请求（网络层 TypeError，
  daemon 日志里看不到任何请求）。daemon 仅监听回环且所有接口都要 bearer token，
  origin 白名单无安全收益，已改为允许任意 origin 并应答
  `Access-Control-Allow-Private-Network: true`（Chrome PNA 预检要求）。
- 停止按钮失效：`stopRunning` 此前只 abort 一个未接入任何 fetch 的控制器且不清
  `isRunning`；现在 abort 信号接入 subscribe/run 请求，且立即停止运行状态。
- 每个 turn 结束后 UI 永远卡在 running：重读循环改为内层 `for(;;)` 后漏了在
  `turn_done`/`turn_error` 时跳出——测试假流会关闭所以单测全过，但真实 daemon 的
  SSE 在 turn 结束后保持打开（15s keepalive），读循环永远阻塞在下一次 `read()`。
  现已在收到终态事件处理完缓冲后立即跳出，并新增"流不关闭也能完成 turn"的回归测试。

- 发消息偶发"没有响应"：`runSessionTurn` 改为**先订阅事件流再 POST /run**（此前顺序
  相反，快速 turn 会在订阅前的间隙内完成，live-only 流漏掉 `turn_done`，UI 永远卡在
  running）；事件流中途断开后按 `after=<lastSeq>` 重连续传（daemon 重放缺失事件），
  连续空断连退避后抛出可感知的 transport 错误；按 `run_id` 过滤旧 run 的残留事件、
  按 `seq` 去除重放接缝重复；`sync_lost` 控制帧对齐游标。
- 审批弹窗刷新/重连后消失、daemon 无限期等待：trace 流每次（重）连成功后拉取一次
  `GET /tools/pending-permissions` 重新填充弹窗（`usePermissionTrace`）。
- "Always allow" 点了仍反复弹：web 此前不传 `session_rule`，daemon 无法持久化规则；
  `resolveSubagentPermission` 现在携带 `session_rule`。

### Changed (Daemon lifecycle)

- daemon 空闲自动关闭改为活动时间驱动：无 thin client 心跳连接**且** 300s 内无任何
  已认证 API 请求时优雅退出（此前为最后客户端断开后 10s，且从未有客户端连过的
  daemon 永不退出）。活动时间窗口从 daemon 启动起算，孤儿 daemon 也会自动回收；
  公共 `/health` 探测不计为活动。
- `wgenty-code daemon` 启动时若发现仍在运行的旧 daemon（discovery 文件 + 健康探测
  确认），先经 `POST /api/v1/shutdown` 优雅停止，无响应则按 PID 升级 SIGTERM →
  SIGKILL，保证每次启动都运行当前二进制，不再复用陈旧的常驻进程。
- TUI 现在也算 thin client：启动后经 `DaemonClient::spawn_heartbeat_keeper` 持有
  `/api/v1/client/heartbeat` 连接（断线指数退避重连），挂着的 TUI 即使不发请求
  也会阻止 daemon 空闲关闭；TUI 退出后连接断开，空闲计时才开始。

### Fixed (Subagent trace)

- `task` 工具保存 transcript 时持久化终态 `summary`（此前硬编码 `None`），冷启动
  trace 回放（`trace_event_from_header`）现在能把子代理结果文本送达重连/刷新后的
  SSE 客户端；端到端验证：live 终态事件与 session-scoped 回放均携带 `result`。

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
