## Context

当前 wgenty-code 的多 Agent 系统由三层组成：
1. **主 Agent Loop**（`src/tui/agent/`）：处理用户输入，调用 LLM，分发工具执行
2. **Task/Delegate 工具**（`src/tools/meta/`）：子 agent 调度入口，支持直接 subagent 和 RLM pipeline
3. **Subagent Loop**（`src/teams/subagent_loop.rs`）：隔离的 agent 循环，独立上下文

TUI 层通过 `SubagentTree`（内存）→ `SubagentPanel`（渲染）展示执行进度。`SubagentProgress` 事件通过共享 `HashMap<session_id, HashMap<node_id, progress>>` 传递，action log 截断到 50 条，text snapshot 截断到 200 字符。

现有护栏：`max_subagent_depth=3`、`max_concurrent_subagents=5`、`subagent_timeout_secs=240`、`StuckDetector`（3 次重复 → abort）。

### 约束
- 运行在终端环境（Ratatui TUI），无浏览器 DOM 能力
- LLM API 通过 HTTP SSE 调用，不改变协议层
- Subagent 结果通过 mailbox 机制 offload 大结果到磁盘
- 配置热加载（`ConfigChanged` 事件）

## Goals / Non-Goals

**Goals:**
1. TUI 输入框中实现 skills（`@name`）和 plugin 命令（`/cmd`）的自动补全
2. Subagent 执行完整时间线记录 + SQLite 持久化
3. Subagent 错误完整因果链展示 + 一键回滚重试
4. RLM pipeline 结构化归约输出（claims / diff）+ 跨层级进展跟踪 + per-subagent 预算

**Non-Goals:**
- 不改变 LLM API 协议
- 不引入 OpenTelemetry 等外部基础设施
- 不重写主 agent loop
- 不修改 daemon HTTP API 和 WASM 端
- 不改变 subagent 的最大并发数或深度限制的默认值

## Decisions

### D1: TUI 输入框补全架构

**选择**：在 `src/tui/input_reader.rs` 增加补全模式，由 `App` 状态机管理

```
┌─────────────────────────────────────────────────┐
│  Input Box                                       │
│  ▸ @skill-name █                                 │
│    ┌────────────┐                                │
│    │ brainstorming │  ← 弹出补全面板             │
│    │ comet-open    │                              │
│    │ tdd           │                              │
│    └────────────┘                                │
└─────────────────────────────────────────────────┘
```

- `@` 触发 skill 补全：读取 `~/.claude/skills/` 目录下所有 skill 名称
- `/` 触发 plugin command 补全：从 `PluginRegistry.commands` 获取已注册命令
- 补全面板复用现有 `PermissionState` 的 inline panel 模式（不弹窗）
- Tab / Shift+Tab 循环候选项，Enter 确认，Esc 取消

**备选方案**：使用 popup overlay → 拒绝，与现有 permission/question UI 风格不一致，且 popup 需要额外 z-order 管理

### D2: Subagent Transcript 持久化

**选择**：SQLite 数据库，每个 subagent 执行记录为一行

```sql
CREATE TABLE subagent_transcripts (
    id TEXT PRIMARY KEY,           -- UUID
    session_id TEXT NOT NULL,       -- 所属会话
    parent_id TEXT,                 -- 父节点 ID (NULL = root)
    label TEXT NOT NULL,            -- 人类可读标签
    status TEXT NOT NULL,           -- pending/running/completed/failed/cancelled
    system_prompt TEXT,
    user_prompt TEXT,
    started_at INTEGER NOT NULL,    -- unix ms
    finished_at INTEGER,
    total_tokens INTEGER DEFAULT 0,
    error_message TEXT,
    summary TEXT,                   -- 最终结果摘要 (truncated)
    created_at INTEGER DEFAULT (strftime('%s','now'))
);

CREATE TABLE subagent_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    transcript_id TEXT NOT NULL REFERENCES subagent_transcripts(id),
    round INTEGER NOT NULL,
    event_type TEXT NOT NULL,       -- 'thought' | 'action' | 'tool_result' | 'error'
    data TEXT NOT NULL,             -- JSON: 事件详情
    elapsed_ms INTEGER NOT NULL,
    created_at INTEGER DEFAULT (strftime('%s','now'))
);
```

- 数据库文件路径：`~/.wgenty-code/subagent_transcripts.db`
- 写入策略：subagent 完成后批量写入（不逐事件写入，避免 I/O 风暴）
- 查询 API：`SubagentTranscriptStore` 提供 `list_by_session`、`get_by_id`、`search` 方法
- 保留策略：默认保留最近 30 天，通过 `max_transcript_age_days` 配置

**备选方案**：JSON 文件 → 拒绝，查询和索引能力弱，多个 subagent 并发写入需要文件锁

### D3: 结构化归约格式

**选择**：分层格式 — 代码变更用 unified diff，分析结论用 structured claims

**Claims 格式**（用于分析/探索型 subagent）：
```json
{
  "format": "structured-claims/1",
  "claims": [
    {
      "id": "c1",
      "claim": "认证模块使用 session-based 而非 JWT",
      "evidence": "src/auth/middleware.rs:45 检查 req.session.user",
      "confidence": 0.95,
      "conflicts_with": [],
      "actionable": true,
      "recommendation": "迁移到 JWT 需要修改 middleware.rs 和 login.rs"
    }
  ]
}
```

**Diff 格式**（用于代码修改型 subagent）：
```json
{
  "format": "unified-diff/1",
  "changes": [
    {
      "file": "src/auth/login.rs",
      "intent": "将 session 认证替换为 JWT token 验证",
      "diff": "--- a/src/auth/login.rs\n+++ b/src/auth/login.rs\n@@ -10,7 +10,7 @@\n-    req.session.set(\"user\", user.id);\n+    let token = jwt::encode(&claims, &secret);\n+    return Json({token});",
      "confidence": 0.85,
      "depends_on": []
    }
  ]
}
```

Aggregator 合并策略：
1. 按 claim 文本相似度（>0.8）去重
2. 检测 `conflicts_with` 引用 → 两个 claims 存在矛盾 → 标记为冲突待用户裁决
3. 按 confidence 降序排列
4. 代码 diff 按 file 分组，检测同一文件的多个 diff → 标记潜在写冲突

**备选方案1**：RFC 6902 JSON Patch → 拒绝，LLM 生成精确 JSON Patch 错误率高
**备选方案2**：AST Diff → 拒绝，语言相关实现复杂，且 subagent 常输出跨语言分析

### D4: Per-Subagent Token 预算

**选择**：父 agent 在调用 `task`/`delegate` 时通过新增参数 `token_budget` 声明上限

- 在 `task` 工具的 input_schema 增加可选字段 `token_budget: number`（单位：千 tokens）
- Subagent loop 每轮 API 调用后累加 `usage.total_tokens`，超限立即返回错误
- 默认值：`token_budget = 0`（不限），由 `settings.default_subagent_token_budget_k` 配置
- RLM pipeline 中，总预算 = planner + executor + aggregator 三阶段之和，子任务均分剩余预算

**备选方案**：隐式预算分配 → 拒绝，用户/父 agent 无法控制成本

### D5: 跨层级进展跟踪

**选择**：在 `SubagentProgress` 中增加 `progress_delta` 字段

- `progress_delta: f32` — 当前轮相对于上一轮的进展增量（0.0-1.0）
- 跟踪维度：任务完成百分比（对子任务总数的比例）、新信息获取量（相对于已收集信息的新增比例）
- 父 agent 在收集子任务结果时计算 delta = 当前轮新发现数 / 总发现数
- 连续 2 轮 delta < 0.05 → 触发 `StuckStatus::NoProgress`，父 agent 收到警告后可决定终止

**备选方案**：LLM 自我报告置信度 → 拒绝，LLM 自我评估不可靠

### D6: Subagent 错误恢复

**选择**：基于沙盒化快照的回滚 + 重试

- **快照机制**：每个 subagent 在修改文件前通过 git 创建临时 stash / branch
- **回滚操作**：用户选择 Failed 节点 → 按 `r` → git checkout 到父节点状态 → 重新 spawn subagent
- **重试参数**：重试时携带失败原因作为额外 context，自动注入 `previous_attempt_error` 到 system prompt
- **不可回滚场景**：subagent 仅执行了只读操作（搜索/分析）→ 直接重试，无需回滚

**备选方案**：Checkpoint 工具集成 → 保留为后续优化，本变更优先实现基础回滚

### D7: TUI Subagent Panel 增强

**选择**：扩展现有 `SubagentPanel` + `SubagentPanelState`

- 每个节点可展开查看完整 timeline（所有 action/thought 事件，不再截断）
- Failed 节点：红色高亮 + 错误摘要 + `[r] retry  [d] details` 快捷键提示
- Completed 节点：绿色 + token 消耗 + 耗时摘要
- Running 节点：实时更新 current_tool + round counter
- 新增 `SubagentDetailView`：选中节点按 Enter → 全屏查看 transcript（从 SQLite 读取）

## Risks / Trade-offs

| Risk | Mitigation |
|------|-----------|
| SQLite 写入在 subagent 完成时批量执行，崩溃可能丢失未写入 transcript | 关键事件（Failed/Completed）立即 flush；Running 状态允许丢失中间步骤 |
| Claims 相似度去重（>0.8）可能误合并不同但相似的 claims | 阈值可配置；被合并的 claims 保留在 metadata 中供审查 |
| Per-subagent budget 硬性熔断可能导致 subagent 在关键步骤被 kill | budget=0 时不限；父 agent 需合理估算，LLM 在 plan 阶段可建议预算 |
| Git stash/checkout 回滚可能与用户未提交的改动冲突 | 回滚前检查 dirty state，提示用户先提交或 stash |

## Open Questions

- Claims 文本相似度算法：使用 LLM 判断还是传统算法（余弦相似度 / Jaccard）？→ **倾向 Jaccard**，避免额外 LLM 调用
- Subagent transcript 的 TUI 全文查看是否需要分页？→ **需要**，transcript 可能超过一屏
