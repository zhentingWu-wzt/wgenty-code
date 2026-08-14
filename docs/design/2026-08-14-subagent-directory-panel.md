# Subagent Directory Panel(当前会话子代理目录面板)设计

- 日期: 2026-08-14
- 状态: 待评审
- 范围: `src/daemon/`(1 个只读端点)+ `web/src/`(目录 store、轮询 hook、面板改造)

## 1. 背景与问题

Web 端右侧「Subagents」面板(`SubagentTreePanel`)当前**只**由 daemon trace SSE 流
(`/subagents/trace/stream`)的 `progress` 事件驱动(`usePermissionTrace` →
`subagentTraceStore.upsertFromEvent`)。这导致两个问题:

1. **列表永远为空**:trace sink 默认 `file`(`agent.subagent.trace.sink`),SSE 通道
   默认没有事件;即使配置了 `daemon`/`both`,晚打开面板或刷新页面也看不到之前的
   subagent(store 是前端内存,纯 push 累积)。
2. **无历史**:`upsertFromEvent` 收到新 root 且 session 变化时清空整树;事件没来
   就没有任何记录。

用户需求:

- 面板应展示**当前会话**的子代理列表,**含已完成记录**(刷新/晚开面板仍在)。
- 运行中的子代理可点击,打开详情 tab(`subagent:<nodeId>`,tab 系统已存在)。

## 2. 方案选择

| 方案 | 说明 | 结论 |
|------|------|------|
| A. 纯前端轮询现有 `/agents/self` | 零后端改动,但响应携带每个 child 的 `messages` 快照,轮询负载冗余;嵌套需逐层请求 | 弃 |
| **B. daemon 新增轻量目录端点**(本设计) | 服务端递归返回全树(无 messages),前端轮询;单请求、小负载、天然带会话内历史 | **采纳** |
| C. 修 SSE 链路(sink 指向 daemon) | 纯 push 无法满足刷新/晚开面板的历史需求 | 仅作可选补充,不在本期 |

已确认的需求边界:**当前会话**(含已完成),不做跨会话历史(那需要读 SQLite
transcript,另一套链路,本期不做)。

## 3. 后端设计

### 端点

    GET /api/v1/agents/directory?session_id=<id>

### 响应 DTO

```rust
#[derive(Debug, Clone, Serialize)]
pub struct AgentDirectoryResponse {
    pub session_id: String,
    pub root: AgentDirectoryEntry,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentDirectoryEntry {
    pub agent_id: String,
    pub trace_id: Option<String>,
    pub label: String,
    pub status: String,          // "Running" | "Thinking" | "Done" | "Failed" | ...
    pub started_at: Option<DateTime<Utc>>,
    pub elapsed_ms: u64,
    pub round: Option<usize>,
    pub max_rounds: Option<usize>,
    pub cumulative_tokens: u64,
    pub children: Vec<AgentDirectoryEntry>,
}
```

### 递归规则

- 从 session 的 `InMemoryAgentStore` root 出发,沿 parent→children 下探。
- **深度上限 5**(`agent.subagent.max_depth` 默认 1,上限仅防病态树);visited set 防环。
- **不携带 `messages`**(这是与 `/agents/self` 的本质区别,目录轮询要轻)。
- store 保留已完成(`Done`/`Failed`)记录 → 历史天然存在,无需额外存储。

### Handler

- 复用 `get_agent_self` 的 session 解析路径(ExecutionSession → store)。
- 只读;无 capability 前置(与 `/agents/self` 一致,按 session_id 鉴权语义)。
- 错误:未知 session → 404;session 无子代理 → 200 `children: []`。

### 测试

- 单测:空 session、单层 children、深度截断、环防护、已完成节点保留。
- 复用 daemon handler 现有测试基建(handlers.rs 内嵌测试模式)。

## 4. 前端数据流

### 新 store:`web/src/state/subagentDirectoryStore.ts`

- 形态:`{ bySession: { [sessionId]: { tree, fetchedAt, stale } }, lastError }`。
- 按会话分桶:切换会话瞬间展示该会话缓存,随后刷新——**移除**现有
  「新 root 清空整树」的 wipe 行为对面板的影响。

### 新 hook:`useSubagentDirectory(client)`

- 与 `usePermissionTrace` 同级挂载 App。
- 固定 **3s** 轮询 active session(directory payload 小);`visibilitychange`
  隐藏时暂停,恢复可见立即补一次。
- 会话 id 取 `daemonId ?? id`(实现时核实 web 会话与 daemon session 的 id 对应)。
- 连续 3 次失败置 `stale = true`(不弹错误,UI 灰点标记),下个 tick 继续重试。

### 与 SSE 的分工

- **directory = 树结构唯一事实来源**(结构、成员、状态、历史)。
- SSE `progress` 降级为实时补充:渲染时按 `agent_id ↔ node_id` 合并 `currentTool`
  等字段(实现时校验两个 id 空间一致;不一致则经 `trace_id` 映射)。

## 5. UI 呈现(`SubagentTreePanel` 改造)

- 复用 `TreeNode` 组件;新增 `AgentDirectoryEntry → SubagentNode` 适配器
  (`currentTool` 从 trace store 渲染层合并)。
- 顶层固定显示会话 root(main agent),子代理嵌套其下,默认展开。
- children 排序:**运行中在前**(Running/Thinking),已完成按 `started_at` 倒序。
- `Done`/`Failed` 保留在列表(现有 `statusColor` 灰显/红显)。
- 点击:`openSubagentTab({ nodeId: agent_id, label, rootSessionId: sessionId })`
  → 打开/激活 `subagent:<agent_id>` tab,详情/transcript/能力导航全复用现有实现。
- 面板头部计数徽标:`● 2 running / 5 total`;`stale` 时旁显灰「离线」。
- 空态:「本会话暂无子代理,发起任务后此处显示」。
- 文案沿用 web 前端现有内联字符串风格。

## 6. 实现时需核实的点(非设计决策,属实现校验)

1. trace 事件 `node_id` 与 store `agent_id` 是否同一 id 空间(决定 SSE 合并的映射方式)。
2. web 会话 `daemonId` 的设置时机,`daemonId ?? id` 是否在所有会话创建路径下成立。
3. `InMemoryAgentStore` 中 root 自身的记录字段是否与 `SelfAgentResponse` 对齐。
4. daemon handler 测试的 session 构造方式(复用现有 fixture)。

## 7. 明确不做(本期)

- 跨会话子代理历史(SQLite transcript 链路)。
- trace sink 默认值变更(方案 C)。
- TUI 侧对应改造。
