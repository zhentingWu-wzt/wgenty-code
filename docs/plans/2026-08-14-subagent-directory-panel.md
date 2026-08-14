# 实现计划: Subagent Directory Panel

- 设计文档: `docs/design/2026-08-14-subagent-directory-panel.md`
- 日期: 2026-08-14
- 语言: 代码与注释英文,遵循仓库 AGENTS.md 约定

## 目标

Web 端右侧 Subagents 面板从「纯 SSE push(默认无事件、永远空)」改为
「daemon 轻量目录端点 3s 轮询 + 会话内全量树(含已完成)+ SSE 仅补充 currentTool」,
点击子代理打开已有 `subagent:<agent_id>` 详情 tab。

## 关键上下文(实现者必读)

- 现有 pull 链路: `src/daemon/handlers.rs::get_agent_self`(1694 行)→
  `root_context(session_id)` → `build_local_view`;DTO 在 `src/daemon/models.rs`
  (`SelfAgentResponse`/`DirectChildResponse`/`LocalAgentViewResponse`)。
- 递归数据源: `src/agent/store.rs::InMemoryAgentStore`,
  `local_records_for_trusted_ui`(257 行)展示了单层读法:
  `state.children.get(&(session, Some(agent)))` → 逐个查 `state.records`。
  `direct_children`(292 行)是另一个单层读法(public)。
- coordinator 包装模式: `src/agent/coordinator.rs::trusted_ui_local_records`(1195 行)。
- 路由注册: `src/daemon/routes.rs:28`(agent_routes 内)。
- handler 测试模板: `src/daemon/handlers.rs:2613`
  `agent_routes_support_recursive_generation_bound_navigation`
  (tempdir + Settings::default + DaemonState::new + axum::serve + viewer token)。
- 前端: 轮询 hook 挂载 `web/src/App.tsx:210`(usePermissionTrace 旁);
  会话 id 取法 `web/src/App.tsx:234` 已有 `daemonId ?? id` 先例;
  面板 `web/src/features/panels/SubagentTreePanel.tsx`;
  trace store `web/src/state/subagentTraceStore.ts`(SubagentNode 形状、
  buildChildrenMap);tab `web/src/state/uiStore.ts::openSubagentTab`;
  client `web/src/api/client.ts`(getAgentSelf 在 351 行附近)。
- node_id ≡ agent_id: 现有 detail tab 已依赖此等价(subagent:<nodeId> tab
  用 agent API 导航),无需 trace_id 映射。
- AgentRecord 无 tokens/elapsed/round 字段——这些来自
  `state.subagent_progress`(按 session 分桶的 RwLock,先 clone 本 session 的 map)
  模式见 `assemble_local_view`(handlers.rs:1594)与 `live_elapsed_ms`。
  directory 沿用同一交叉填充,但 messages 字段一律不带。

## Task 1: 后端 — store 递归遍历 + coordinator 包装

文件: `src/agent/store.rs`, `src/agent/coordinator.rs`

1. store.rs 新增(pub(crate),模仿现有命名):

```rust
/// Directory entry for trusted UI tree projection: hierarchy records only.
/// Progress cross-fill (tokens/elapsed/round) happens in the daemon handler
/// layer, mirroring assemble_local_view's separation of concerns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryEntry {
    pub agent_id: AgentId,
    pub status: AgentLifecycleStatus,
    pub label: String,
    pub summary: Option<String>,
    pub depth: usize,
    pub children: Vec<DirectoryEntry>,
}

impl InMemoryAgentStore {
    /// Recursive whole-tree projection for the trusted UI directory,
    /// depth-capped and cycle-safe. Hierarchy records only (no progress).
    pub(crate) async fn directory_for_trusted_ui(
        &self,
        session: &SessionId,
        root: &AgentId,
    ) -> Result<DirectoryEntry, StoreError> {
        // 单次 read lock 递归;MAX_DIRECTORY_DEPTH = 5 常量;
        // visited: HashSet<(SessionId, AgentId)> 防环;
        // children 按 agent_id 排序(与 local_records_for_trusted_ui 一致)。
        // 递归体参考 local_records_for_trusted_ui(257 行)的单层读法:
        // state.children.get(&(session, Some(agent))) → 逐个查 state.records。
    }
}
```

2. coordinator.rs 新增(1195 行旁):

```rust
/// Recursive whole-tree directory for a trusted UI projection.
pub(crate) async fn trusted_ui_directory(
    &self,
    session: &SessionId,
    root: &AgentId,
) -> Result<crate::agent::store::DirectoryEntry, CoordinatorError> {
    self.store
        .directory_for_trusted_ui(session, root)
        .await
        .map_err(Into::into)
}
```

store 不接触 progress 类型——分层与 assemble_local_view 一致,
elapsed_ms 计算无需在 store 重复(Task 2 在 handler 层复用 live_elapsed_ms)。

验收: `cargo test --lib agent::store` 通过;新增单测覆盖
空 session / 单层 / 深度截断 / 环防护 / Done 保留。

## Task 2: 后端 — DTO + handler + 路由 + 集成测试

文件: `src/daemon/models.rs`, `src/daemon/handlers.rs`, `src/daemon/routes.rs`

1. models.rs(344 行 Scoped agent views 区块内,SelfAgentResponse 旁):

```rust
/// `GET /api/v1/agents/directory` -- lightweight recursive tree for the
/// session's whole subagent hierarchy. No messages (directory polling
/// payload must stay small); lifecycle data cross-filled from progress.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDirectoryResponse {
    pub session_id: String,
    pub root: AgentDirectoryEntry,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDirectoryEntry {
    pub agent_id: String,
    pub status: AgentLifecycleStatus,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub cumulative_tokens: u64,
    #[serde(default)]
    pub started_at: i64,
    #[serde(default)]
    pub elapsed_ms: u64,
    #[serde(default)]
    pub round: Option<usize>,
    #[serde(default)]
    pub max_rounds: Option<usize>,
    #[serde(default)]
    pub depth: usize,
    pub children: Vec<AgentDirectoryEntry>,
}
```

(`trace_id` 字段砍掉——store 无此字段,设计文档中的 trace_id 是冗余,
SSE 合并用 agent_id 已足够,见"关键上下文"node_id ≡ agent_id。)

2. handlers.rs(get_agent_self 旁新增,模式完全对标):

```rust
/// `GET /api/v1/agents/directory?session_id=<id>` -- recursive whole-tree
/// directory (no messages) for panel polling.
pub async fn get_agent_directory(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Result<Json<AgentDirectoryResponse>, StatusCode> {
    let viewer = resolve_viewer_from_headers(&state, &headers)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;
    let _ = viewer; // same auth posture as get_agent_self
    let session_id = params
        .get("session_id")
        .map(|s| s.as_str())
        .ok_or(StatusCode::BAD_REQUEST)?;
    let root = state
        .root_context(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    // Clone this session's progress map under a short read guard, then
    // release before the coordinator await (pattern: build_local_view:1554).
    let session_progress = {
        let progress_store = state.subagent_progress.read().await;
        progress_store
            .get(root.session_id.as_str())
            .cloned()
            .unwrap_or_default()
    };
    let entry = state
        .coordinator
        .trusted_ui_directory(&root.session_id, &root.agent_id)
        .await
        .map_err(map_scoped_coordinator_error)?;
    Ok(Json(AgentDirectoryResponse {
        session_id: session_id.to_string(),
        root: to_directory_entry(entry, &session_progress),
    }))
}
```

`to_directory_entry`: store::DirectoryEntry → models::AgentDirectoryEntry
的递归映射 + progress 交叉填充(与 assemble_local_view 同层职责):
`agent_id.as_str().to_string()` 等直映,tokens/started_at/round/max_rounds
取自 `session_progress.get(agent_id)`,elapsed_ms 用本文件已有的私有函数
`live_elapsed_ms`(1580 行,零重复)。progress 缺失时字段取默认值
(root 记录通常无 progress 条目)。

3. routes.rs agent_routes: `.route("/agents/directory", get(get_agent_directory))`
   (与 `/agents/self` 相邻,同前缀已隐含 /api/v1)。

4. 集成测试(handlers.rs mod tests,复制 2613 行模板):
   - 空 session(root 无 children)→ 200,children: []。
   - root + child + grandchild(max_depth=3)→ 全树嵌套,depth 字段正确。
   - child 状态 Done → 仍出现在树里(历史保留)。
   - 未知 session_id → 内部错误语义与 get_agent_self 一致(root_context
     惰性创建,不会 404——文档已按此语义,测试断言行为而非 404)。

验收: `cargo test --lib daemon` 通过;`cargo clippy --all-targets -- -D warnings` 零告警。

## Task 3: 前端 — client 方法 + 类型

文件: `web/src/api/client.ts`, `web/src/api/types.ts`

types.ts:

```ts
/** `GET /api/v1/agents/directory` -- recursive tree entry (no messages). */
export interface AgentDirectoryEntry {
  agent_id: string;
  status: string;
  label: string;
  summary: string | null;
  cumulative_tokens: number;
  started_at: number;
  elapsed_ms: number;
  round: number | null;
  max_rounds: number | null;
  depth: number;
  children: AgentDirectoryEntry[];
}

export interface AgentDirectoryResponse {
  session_id: string;
  root: AgentDirectoryEntry;
}
```

client.ts(getAgentSelf 旁,同模式 agentHeaders + jsonOrThrow):

```ts
/** `GET /api/v1/agents/directory?session_id=<id>` -- lightweight recursive
 *  tree for the subagents panel (poll this; use agents/self for detail). */
async getAgentDirectory(sessionId: string): Promise<AgentDirectoryResponse> {
  const headers = await this.agentHeaders();
  return jsonOrThrow(
    await fetch(
      `${this.base}/agents/directory?session_id=${encodeURIComponent(sessionId)}`,
      { headers },
    ),
  );
}
```

验收: `cd web && npm run typecheck` 通过。

## Task 4: 前端 — directory store + 轮询 hook

文件: `web/src/state/subagentDirectoryStore.ts`(新), `web/src/hooks/useSubagentDirectory.ts`(新), `web/src/App.tsx`

1. subagentDirectoryStore.ts:

```ts
import { create } from "zustand";
import type { AgentDirectoryEntry, AgentDirectoryResponse } from "../api/types";

interface SessionDirectory {
  tree: AgentDirectoryEntry | null;
  fetchedAt: number;
  stale: boolean;
}

interface SubagentDirectoryState {
  bySession: Record<string, SessionDirectory>;
  /** Apply a successful fetch for a session (clears stale). */
  apply: (sessionId: string, res: AgentDirectoryResponse) => void;
  /** Mark a session stale after consecutive failures (keep cached tree). */
  markStale: (sessionId: string) => void;
  forget: (sessionId: string) => void;
}

export const useSubagentDirectoryStore = create<SubagentDirectoryState>((set) => ({
  bySession: {},
  apply: (sessionId, res) =>
    set((s) => ({
      bySession: { ...s.bySession, [sessionId]: { tree: res.root, fetchedAt: Date.now(), stale: false } },
    })),
  markStale: (sessionId) =>
    set((s) => {
      const cur = s.bySession[sessionId];
      if (!cur) return s;
      return { bySession: { ...s.bySession, [sessionId]: { ...cur, stale: true } } };
    }),
  forget: (sessionId) =>
    set((s) => {
      const next = { ...s.bySession };
      delete next[sessionId];
      return { bySession: next };
    }),
}));
```

导出辅助(供面板):`flattenCount(tree)` 返回 `{ running, total }`
(status 小写后 "running"|"thinking"|"pending" 视为 running)。

2. useSubagentDirectory.ts(自包含重连循环,照抄 usePermissionTrace 的
   backoff 骨架但简化——轮询不需要 backoff,只需固定间隔 + 可见性暂停):

```ts
const POLL_INTERVAL_MS = 3000;
const MAX_CONSECUTIVE_FAILURES = 3;

export function useSubagentDirectory(client: DaemonClient | null): void {
  // useEffect 内:
  // - 读 active session(useSessionManager.getState(),activeId);
  //   无 activeId → 不轮询。
  // - sid = entry.daemonId ?? entry.id(App.tsx:234 先例;
  //   daemonId 类型与赋值时机实现时以 sessionManager 源码为准)。
  // - tick(): document.hidden 时跳过;成功 → apply + failures=0;
  //   失败 → failures++, >=3 时 markStale。
  // - setInterval(tick, POLL_INTERVAL_MS);挂载即立即 tick 一次;
  //   activeId 变化时 clearInterval 重建 + 立即 tick。
  // - cleanup: clearInterval。
}
```

3. App.tsx: 在 `usePermissionTrace(client)`(210 行)旁挂
   `useSubagentDirectory(client)`。

验收: `npm run typecheck`;手测:打开两个会话各 spawn 过 subagent,切换会话
时面板立即显示各自缓存树。

## Task 5: 前端 — SubagentTreePanel 改造

文件: `web/src/features/panels/SubagentTreePanel.tsx`

- 数据源改为 directory store(按 activeId 分桶);SSE trace store 仅做
  currentTool 合并:`traceStore.nodes.get(entry.agent_id)?.currentTool`。
- 适配函数 `toSubagentNode(entry, sessionId, traceNodes): SubagentNode`:
  nodeId=entry.agent_id, parentId 由父层传入(递归时闭包携带,
  root 为 null), label/status/round/elapsedMs/cumulativeTokens 直映,
  lastUpdated=Date.now()。
- 排序: running(thinking/pending)在前,其余按 started_at 倒序;
  stable sort。
- 头部:`● {running} running / {total} total`;stale 时追加灰色 `离线`。
- 空态:tree 为 null(还没拉到)→ "加载中…";tree 存在且无 children →
  「本会话暂无子代理,发起任务后此处显示」。
- 点击:onOpen 复用现有 openSubagentTab({nodeId: agent_id, label,
  rootSessionId: 当前会话 sid})——root 节点自身也可点(detail 已支持)。
- TreeNode 组件保持不动(字段兼容)。
- 移除面板对 upsertFromEvent 清树行为的依赖(directory 分桶天然解决)。

验收: `npm run typecheck` + `npm run test`(如有面板相关测试则更新);
手测: spawn 一个 subagent,3s 内出现在面板;完成后灰显保留;点击开 tab。

## Task 6: 全量验证

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test --all
cd web && npm run typecheck && npm run test && npm run lint
```

## 提交策略(每 Task 一次 commit)

1. `feat(agent): add trusted recursive directory projection to agent store`
2. `feat(daemon): expose GET /api/v1/agents/directory for panel polling`
3. `feat(web): add getAgentDirectory client method`
4. `feat(web): poll agent directory into per-session subagent panel`
5. `feat(web): render directory tree with running-first sort and tab click`
(4/5 可按实际实现合并为一次,以保持每 commit 可编译为准)

## 风险与回退

- store 单锁递归:树极深时持锁时间长——深度上限 5 + 每层子节点数实际
  很小(max_concurrent 5),可接受;不做分页。
- 轮询与 root_context 惰性创建副作用:directory 对从未见过的 session_id
  会 ensure_root(root_context 语义),与 get_agent_self 一致,无新增风险。
- 回退:整链路纯增量,无现有行为依赖 directory;revert commit 即可。
