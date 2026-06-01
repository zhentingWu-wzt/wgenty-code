# Gap Analysis: wgenty-code-rust vs learn-wgenty-code

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

| 层 | 机制 | learn-wgenty-code | wgenty-code-rust 现状 | 差距 |
|----|------|-------------------|----------------------|------|
| s01 | Agent Loop (核心) | `agent_loop()` 最小循环，tool dispatch | `agent-loop.ts` 完整实现 | **已实现** |
| s02 | 工具分发表 | bash/read/write/edit 4 工具 | 17 个工具（filesystem/search/execution/meta） | **已实现** |
| s03 | TodoWrite + 提醒注入 | 3 轮未更新则注入提醒 | `TodoWriteTool` + `roundsSinceTodo` 提醒 | **已实现** |
| s04 | 子代理（Subagent） | 独立 Agent 循环，隔离上下文，过滤工具（无递归 task） | `AgentsService` 存在但仅做简单 API 调用，无独立循环、无工具过滤 | **未实现** — 缺少隔离的 Agent 循环 |
| s05 | 技能加载 | 两层注入：system prompt 列名称 → `load_skill` 工具按需加载完整 SKILL.md | `knowledge/` 模块有 Skill trait + registry，但 skills 是 mock 桩代码，未注册为工具 | **未实现** — Skill 未接入 agent loop |
| s06 | 上下文压缩（3 层） | 微压缩（替换旧 tool_result）+ 自动压缩（token 超阈值摘要）+ 手动压缩（compact 工具） | `context/` 模块有 ConsolidationEngine 但只针对 memory entries，**不处理 conversation context** | **未实现** — 缺少对话级别的压缩 |
| s07 | 任务系统（依赖图） | 文件持久化 + `blockedBy` 依赖图 | `TaskManagementTool` 有 CRUD 但**无 blockedBy 依赖** | **部分实现** |
| s08 | 后台任务 | `BackgroundManager` + 通知队列 + 注入 agent loop | `exec_command` + session manager 但**无通知注入机制** | **未实现** |
| s09 | 代理团队 | 多线程 Agent 循环 + JSONL 邮箱通信 | `teams/subagent.rs` 仅定义数据结构，无邮箱、无多 agent 通信 | **未实现** |
| s10 | 团队协议 | 关闭协议 + 计划审批协议（`request_id` 关联） | 无 | **未实现** |
| s11 | 自主 Agent | 轮询任务板 + 自动认领 + 空闲超时关闭 | 无 | **未实现** |
| s12 | 工作树隔离 | `WorktreeManager` + `EventBus` + `bind_worktree` | 仅 `teams/mod.rs` 提到 worktree，无实现 | **未实现** |

---

## 关键架构差距

### 1. Agent Loop 缺乏扩展点

`agent-loop.ts` 的循环是硬编码的：user input → SSE stream → tool execution → push results → loop。没有提供 hook/拦截点来注入：
- 上下文压缩（s06）
- 后台通知（s08）
- 提醒注入（s03 是硬编码的，不是通用机制）
- 工具门控（计划审批模式）

**learn 的做法**：循环是纯函数，所有机制在循环外部通过消息列表操作实现。

### 2. 子代理系统未真正实现

`AgentsService.execute_agent()` 只做一次 `api_client.chat()` 调用，无循环、无工具执行、无独立上下文。子代理应该：
- 拥有独立的 `messages=[]` 上下文
- 运行完整的 agent loop（多轮 tool use）
- 工具集被过滤（无递归 `task` 工具）
- 通过 tool_result 返回结果给主 agent

### 3. Skill 系统未接入 Agent Loop

`knowledge/` 模块定义了 `Skill` trait + `SkillRegistry` + `SkillExecutor`，但：
- `load_skill` 没有作为工具注册到 `ToolRegistry`
- Skill 列表未注入 system prompt（两层注入的第一层）
- 内置 skills（builtin.rs）是空的桩代码

### 4. 上下文压缩缺失（关键差距）

这是最大的性能差距。当前 repo 的 conversation history 会无限增长直至 token 超限。learn 项目的三层压缩策略：
- **微压缩**：静默替换旧 tool_result 为 `"[Previous: used {tool_name}]"`
- **自动压缩**：token 预估超 50000 时，保存完整记录并让 LLM 摘要
- **手动压缩**：`compact` 工具供 Agent 按需触发

### 5. 事件总线 / Hook 系统缺失

learn 项目的 s12 有最小化的 `EventBus`（JSONL 追加）。当前 repo 无任何生命周期事件系统。Wgenty Code 生产环境有完整的 Hook 系统（`PreToolUse`、`PostToolUse`、`SessionStart`、`SessionEnd` 等）。

### 6. 团队协作完全缺失

多 Agent 协作（s09-s12）在当前 repo 中仅有数据结构定义，无实际运行逻辑：
- 无 JSONL 邮箱通信
- 无多线程/多任务 Agent 循环
- 无协议（关闭/审批）
- 无工作树隔离

### 7. 状态持久化方式不同

learn 项目：文件即数据库（JSON 任务文件、JSONL 邮箱、SKILL.md 技能）
当前 repo：内存中（`Arc<RwLock<HashMap>>`）——重启即丢失

---

## 实现优先级建议

### 阶段 1：补齐核心循环机制（影响最大）

| 优先级 | 功能 | 原因 |
|--------|------|------|
| **P0** | 上下文压缩（s06 微压缩 + 自动压缩） | 当前无任何上下文管理，长对话必定崩溃 |
| **P0** | 子代理系统（s04） | Agent tool 是 Wgenty Code 的关键差异化能力 |
| **P1** | Skill 接入 agent loop | skills/ 目录已在用，但机制未接入 |

### 阶段 2：完善协作机制

| 优先级 | 功能 | 原因 |
|--------|------|------|
| **P1** | Hook/事件系统 | 权限管理、工具执行监控的基础设施 |
| **P1** | 任务依赖图（blockedBy，s07） | 当前任务系统有 CRUD 无依赖 |
| **P2** | 后台任务通知队列（s08） | 长时间命令需要异步通知 |

### 阶段 3：高级协作（按需）

| 优先级 | 功能 | 原因 |
|--------|------|------|
| **P2** | 团队邮箱通信（s09） | 多 Agent 协作的基础 |
| **P3** | 团队协议（s10） | 计划审批在实际使用中很有价值 |
| **P3** | 工作树隔离（s12） | Git 工作树管理 |
| **P3** | 自主 Agent（s11） | 后台自主工作 |

---

## 具体实现方案概要

### P0-1: 上下文压缩

在 `agent-loop.ts` 的 `processInput` 方法中，每次循环开始前：

```
1. 微压缩：遍历 conversationHistory，将非最近 3 条的 tool 消息替换为 "{tool_name} result (omitted)"
2. Token 预估：粗略计算 conversationHistory 的 token 数（字符数 / 4）
3. 超阈值 → 自动压缩：保存完整记录到磁盘，调用 LLM 摘要对话，替换 conversationHistory
```

新增 `compact` 工具供 Agent 手动触发。

### P0-2: 子代理系统

在 Rust 侧：

```rust
// src/teams/subagent_loop.rs — 新建
pub async fn run_subagent_loop(
    api_client: &ApiClient,
    system_prompt: &str,
    user_prompt: &str,
    allowed_tools: &[String],  // 过滤后的工具集
    max_rounds: usize,
) -> Result<String>
```

在 agent-loop.ts 的 tool dispatch 中添加 `task` 工具（启动子代理），子代理通过 tool_result 返回结果。子代理的工具集排除 `task` 自身（防止递归爆炸）。

### P1-1: Skill 接入

1. 在 `DaemonState::new()` 中初始化 `SkillRegistry`，加载 `skills/` 目录下的 SKILL.md
2. 注册 `load_skill` 工具到 `ToolRegistry`
3. 在 `buildSystemPrompt()` 中注入技能名称列表（第一层注入）
4. `load_skill` 工具执行时，返回完整的 SKILL.md 主体（第二层注入）

### P1-2: Hook 系统

```rust
// src/hooks/mod.rs — 新建
pub enum HookEvent {
    PreToolUse { tool_name: String, input: Value },
    PostToolUse { tool_name: String, result: ToolOutput },
    SessionStart,
    SessionEnd,
    Notification { message: String },
}
```

Hook 注册在 settings.json 中，由 `ToolExecutor` 在执行前后触发。
