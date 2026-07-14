# Gap Analysis: wgenty-code vs learn-wgenty-code

> **更新日期**: 2026-07-14 — unified `agent::runtime::run_agent_loop` 已落地；
> TUI / CLI query / subagent 共用控制流。下表中 s09–s12 仍为未完成协作能力。

## learn-wgenty-code 的设计哲学

`learn-wgenty-code` 是一个教学项目，通过 12 个渐进式 Python 脚本演示 AI 编码 Agent 框架的构建。
其核心理念是：**智能体能力来自模型训练，而非代码编排** —— 框架只是"载体"。

关键设计原则：

1. **信任模型** — 不预先指定工作流或构建决策树，让 LLM 自行推理。约束条件（如"一次一个任务"）是为了集中注意力，而非限制行为。
2. **渐进式复杂度** — 从最小可行方案开始，只有实际使用暴露需求时才增加复杂度。
3. **按需知识注入** — 通过 `tool_result` 注入领域知识（Skill），而非预塞进 system prompt。
4. **Agent Loop 神圣不可侵犯** — 每个脚本都在循环**外部**添加机制，绝不修改循环本身。
5. **文件即集成** — 团队邮箱是 JSONL 文件，任务是 JSON 文件，技能是 SKILL.md 文件。无需数据库或进程间通信。
6. **"Bash is all you need"** — 最基础的 Agent 只需要一个 bash 工具。

---

## 机制对照表：12 层逐项对比

| 层 | 机制 | learn-wgenty-code | wgenty-code 现状 | 差距 |
|----|------|-------------------|----------------------|------|
| s01 | Agent Loop (核心) | `agent_loop()` 最小循环，tool dispatch | `agent::runtime::run_agent_loop` 统一控制流；TUI/CLI/subagent 经 ports 接入 | **已实现** |
| s02 | 工具分发表 | bash/read/write/edit 4 工具 | 25 个工具（filesystem/search/execution/meta/checkpoint） | **已实现** |
| s03 | TodoWrite + 提醒注入 | 3 轮未更新则注入提醒 | `TodoWriteTool` + `rounds_since_todo` 提醒 | **已实现** |
| s04 | 子代理（Subagent） | 独立 Agent 循环，隔离上下文，过滤工具（无递归 task） | `teams/subagent_loop` 薄封装 → 共享 loop + `FilteredToolPort` + synthesis barrier | **已实现** |
| s05 | 技能加载 | 两层注入：system prompt 列名称 → `load_skill` 工具按需加载完整 SKILL.md | `LoadSkillTool` 注册（daemon），`SkillLoader` 加载 `~/.wgenty-code/skills/`，Prompt Layer 6 注入 | **已实现** |
| s06 | 上下文压缩（3 层） | 微压缩（替换旧 tool_result）+ 自动压缩（token 超阈值摘要）+ 手动压缩（compact 工具） | micro 在 runtime 共享；auto-summary 经 `Compactor`（TUI `TuiCompactor` + CLI `ApiCompactor`） | **已实现（TUI + CLI）** |
| s07 | 任务系统（依赖图） | 文件持久化 + `blockedBy` 依赖图 | `TaskManagementTool`：`blockedBy` / `set_dependencies` / `blocked` / `ready` + 环检测 + 状态门闩；图算法在 `tasks/graph.rs` | **已实现** |
| s08 | 后台任务 | `BackgroundManager` + 通知队列 + 注入 agent loop | `background` 工具 + TUI `inject_background_results`；CLI 无后台会话 | **部分实现（TUI）** |
| s09 | 代理团队 | 多线程 Agent 循环 + JSONL 邮箱通信 | `MailboxInbox`（subagent 每轮 drain）+ `team_message` 工具（send/broadcast/shutdown_request）；多 agent 异步通信已通 | **部分实现**（运行时已通；多 agent 并行调度待 s11） |
| s10 | 团队协议 | 关闭协议 + 计划审批协议（`request_id` 关联） | `ShutdownRequest` 经 mailbox 触发子代理 cooperative cancel；`ApprovalRequest/Response` + `request_approval` 工具（oneshot + 超时）；`approval_registry` 关联 `request_id` | **部分实现**（关闭+审批已通；父侧自动审批策略待 s11） |
| s11 | 自主 Agent | 轮询任务板 + 自动认领 + 空闲超时关闭 | `AutonomousWorker` daemon service：定时 `claim_ready_root_group` + mailbox 通知 root；空闲超时停止；`settings.agent.autonomous` 配置 | **部分实现**（claim+通知已通；daemon 自主跑 loop 待做） |
| s12 | 工作树隔离 | `WorktreeManager` + `EventBus` + `bind_worktree` | 仅 `teams/mod.rs` 提到 worktree，无实现 | **未实现** |

---

## 关键架构差距（修订）

### 1. Agent Loop 已统一（2026-07-14）

- **共享控制流**：`src/agent/runtime/loop_.rs`（`run_agent_loop`）
- **TUI**：`tui/agent/adapters`（Daemon LLM/Tools、权限 UI、`TuiCompactor`）
- **CLI query**：`cli/headless_runtime`（`ApiLlmPort` + `RegistryToolPort`，micro-compact）
- **Subagent**：`teams/subagent_loop` → 同一 loop + 过滤工具 + synthesis/observer
- **Daemon**：仍为 SSE/工具代理服务；循环在 client 侧（TUI）或 in-process（CLI/subagent）

### 2. 子代理系统已实现 ✅

`teams/subagent_loop.rs`（577 行）实现了独立子代理循环：
- 独立 `messages=[]` 上下文（不共享父 agent 对话历史）
- 完整多轮 tool-use 循环
- JSON parse error 纠正（最多 3 次连续错误后注入纠正提示）
- stuck-detector 防止死循环
- `max_rounds` 安全上限
- TUI agent (`core.rs:100-117`) 支持多个 `task` 并行执行

已注册为 `task` 工具，通过 `tool_result` 返回结果给主 agent。

### 3. Skill 系统已完整接入 ✅

- **Layer 1**：Prompt Layer 6 (`prompts/mod.rs:145-160`) 注入技能名称 + 描述列表
- **Layer 2**：`LoadSkillTool` 注册为工具，按需返回完整 SKILL.md 正文
- `SkillLoader::load_from_dirs()` 从 `~/.wgenty-code/skills/` 加载
- `knowledge/builtin.rs`（365 行）提供内置技能

### 4. 上下文压缩已实现 ✅ (TUI 路径)

`tui/agent/compaction.rs`（170 行）实现了完整的三层压缩：

```
每轮 LLM 调用前:
  1. micro_compact(): 遍历 conversationHistory，非最近 3 条的 tool 消息
     替换为 "[Previous: used {tool_name}]"。始终保留 file_read 结果。
  2. needs_compaction(): chars/4 > MAX_ESTIMATED_TOKENS (50000) 时触发
  3. do_auto_compact(): 保存完整 transcript 到 ~/.wgenty-code/transcripts/,
     调用 LLM 摘要对话，替换 conversationHistory
```

Token 预算检查（`token_counter`）在每次 LLM 调用前后均有触发。

**⚠️ CLI repl 和 daemon 路径尚未移植此机制。**

### 5. 事件总线 / Hook 系统缺失

当前无任何生命周期事件系统（`PreToolUse`、`PostToolUse`、`SessionStart`、`SessionEnd` 等）。
Hook 系统是权限管理、工具执行监控、第三方扩展的基础设施。

### 6. 团队协作完全缺失

多 Agent 协作（s09-s12）在当前 repo 中仅有数据结构定义，无实际运行逻辑：
- 无 JSONL 邮箱通信
- 无多线程/多任务 Agent 循环
- 无协议（关闭/审批）
- 无工作树隔离

### 7. 状态持久化方式不同

learn 项目：文件即数据库（JSON 任务文件、JSONL 邮箱、SKILL.md 技能）
当前 repo：混合模式 — conversation history 在内存（`Arc<Mutex>`），transcript 可持久化到磁盘，memory 在 MemoryManager 中管理。

---

## 实现优先级建议（2026-07-14 修订）

### 阶段 1：统一 runtime（已完成）

| 优先级 | 功能 | 状态 |
|--------|------|------|
| **P0** | 统一 `run_agent_loop`（TUI/CLI/subagent） | ✅ `agent::runtime` |
| **P0** | CLI auto-compact | ✅ `ApiCompactor` |
| **P0** | 任务依赖图 `blockedBy` + 环检测 + ready | ✅ `tasks/graph.rs` |
| **P1** | ready-task 提醒注入 | ✅ `TaskProgressPort`（TUI 待 daemon 端点） |

### 阶段 2：安全与可验证

| 优先级 | 功能 | 状态 |
|--------|------|------|
| **P0** | 共享 loop mock 回归网 | ✅ `loop_tests` |
| **P0** | Windows Job Object 沙箱 | ✅（Restricted Token 待做） |
| **P1** | Windows CI test job | ✅ |
| **P1** | Provider 质量矩阵文档 | ✅ `docs/PROVIDERS.md` |

### 阶段 3：协作与扩展（未完成）

| 优先级 | 功能 | 状态 |
|--------|------|------|
| **P1** | TUI task 提醒（daemon `/tasks/ready` 端点） | 🔲 |
| **P1** | Daemon 内嵌完整 loop（`/agent/turn`） | 🔲 设计决策 |
| **P2** | 团队邮箱通信运行时（s09） | 🔲 仅数据结构 |
| **P2** | 团队协议：关闭 / 计划审批（s10） | 🔲 |
| **P2** | worktree 隔离（s12） | 🔲 |
| **P3** | 自主 Agent（s11） | 🔲 |
| **P2** | Linux seccomp-bpf syscall 白名单 | 🔲 |
| **P2** | WASM 插件执行 | 🔲（仅 script hooks） |

---

## 已完成的实现（对照原文档建议）

### ✅ P0-1: 上下文压缩 (s06)

原文档提议在 `agent-loop.ts` 的 `processInput` 中添加微压缩 + 自动压缩。**已在 `tui/agent/` 路径实现：**

- `tui/agent/compaction.rs:42-97` — micro_compact()
- `tui/agent/compaction.rs:99-105` — needs_compaction()
- `tui/agent/compaction.rs:107-170` — do_auto_compact()
- `tui/agent/core.rs:30-35` — 每轮调用前自动执行

### ✅ P0-2: 子代理系统 (s04)

原文档提议新建 `src/teams/subagent_loop.rs`。**已实现：**

- `teams/subagent_loop.rs:1-577` — 完整子代理循环
- TUI agent `core.rs:100-117` — 并行 task 执行
- 子代理通过 `tool_result` 返回结果，`agent.subagent.max_depth` 限制递归

### ✅ P1-1: Skill 接入 (s05)

原文档提议的 4 步全部完成：

1. DaemonState 中初始化 SkillRegistry ✅
2. LoadSkillTool 注册到 ToolRegistry ✅ (`daemon/state.rs:96-99`)
3. buildSystemPrompt 注入技能名称列表 ✅ (`prompts/mod.rs:145-160`, Layer 6)
4. load_skill 工具按需返回完整 SKILL.md ✅ (`tools/meta/load_skill.rs`)
