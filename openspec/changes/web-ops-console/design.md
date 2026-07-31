---
comet_change: web-ops-console
role: technical-design
canonical_spec: openspec
---

# 设计：Web Agent 前端

> 取代原 "Web 运维控制台（P0）" 设计。`web/` 中的 MVP 是新基线；本文档覆盖 Tier 1（核心 agent 体验）与 Tier 2（运维侧边面板）。Tier 3（硬化）仅作草图，未在此完整设计。

## 背景（Context）

- `web/` 已存在可用的 MVP（React + Vite + TS）：流式聊天、从 `src/agent/runtime/loop_.rs` 移植的客户端 agent loop、root 工具权限审批。它通过 `/api/v1/*` 与 daemon 通信，**零后端改动**（Vite dev server 注入 daemon bearer token）。
- daemon 的 `/api/v1/chat/stream` 是**纯透传代理**——它不执行工具。客户端必须自行驱动 stream → execute → re-stream（这是核心架构事实，见 `web/src/agent/loop.ts`）。
- 现有 daemon API 已覆盖：chat stream、tool execute、permission approve/unapprove、pending-permissions、sessions CRUD/search、models、config、todos、tasks/progress、checkpoints、mcp servers、subagent trace stream。
- 运维目标**缺失的部分**：memory HTTP 端点与脱敏 overview；`MemoryManager` 有方法，仅缺 HTTP 层。

## 目标 / 非目标

**目标**

- Tier 1：将 MVP 提升到与 TUI 核心 agent 体验功能对等（渲染、diff、取消、子 agent 权限、会话、todo、模型切换）。
- Tier 2：以最小的增量后端，加入 memory + overview/config 作为侧边面板。
- 保持 daemon 为纯透传代理——不引入第二条 agent 执行路径。

**非目标**

- 取代 TUI。多项目、多用户、公网级硬化。
- 配置写回。原始 prompt 全文溯源（P2）。

## 决策（Decisions）

### D1 —— 前端托管：现阶段独立 Vite app，未来 daemon 嵌入

- **Tier 1 与 2**：前端保持 MVP 的独立 Vite app（`web/`）。dev 用 Vite 代理注入 token；CORS 已允许 `:5173`。
- **Tier 3（生产）**：可选地通过 `tower-http::services::ServeDir`（或 `rust-embed`）从 daemon 托管生产构建。此项延后——对 dev/单用户场景，独立 app 已完全可用。
- **为何不从第一天起同端口**（推翻原设计 D1）：MVP 已证明独立方式可行，且避免在 Tier 1 阶段把前端工具链强塞进 Rust 构建/CI。嵌入是打包层面的事，等真有跑不了 `npm run dev` 的终端用户时再决。

### D2 —— 客户端 agent loop 留在浏览器

- 浏览器始终驱动 `runAgentLoop`（`web/src/agent/loop.ts`）。Tier 1 扩展它：`AbortController` 实现取消。
- daemon 永不运行该 loop。这保留了干净的透传契约，并使 `src/agent/runtime/loop_.rs` 保持唯一事实源（web 端移植持续跟踪它）。

### D2.1 —— 子 agent 权限通知走 trace SSE（非轮询）

- 原 tasks 草拟"前端 500ms 轮询 `/tools/pending-permissions`"（镜像 TUI 的 `adapters.rs:168-211`）。**改为推送模型**：复用 daemon 已有的进程级 `trace_hub()` broadcast + `/api/v1/subagents/trace/stream` SSE。
- `TraceEvent` 增加可选的 `kind`（默认 `"progress"`）与 `permission` 字段。`PermissionBridge` 在请求注册 / 解决时往 `trace_hub()` 发一条 `permission_pending` / `permission_resolved` 事件。
- 前端订阅已有 trace SSE，按 `kind` 分发：权限事件复用现有 `requestPermission` 弹窗，trace 事件走原 UI。
- 收益：实时（毫秒级）取代 500ms 轮询；零新端点、零新总线；TUI 也可同步去轮询。权衡：trace 流里混入权限事件，但权限请求低频，消费端按 `kind` switch 即可。
- 新字段对 cold-start replay 的旧 JSON 必须 `#[serde(default)]` 向后兼容。

### D3 —— 渲染管线

- assistant 内容通过 `react-markdown` + `remark-gfm` 渲染为 GFM Markdown。代码块语法高亮（`shiki` 更准、`prismjs` 更轻，二选一）。
- `file_edit` / `apply_patch` 的工具结果解析为统一 diff，用 diff 组件渲染（如 `react-diff-viewer` 或基于 `similar` 的自定义渲染——但 diff 在客户端对工具输出文本进行，无后端改动）。
- 流式 token 渲染必须与 Markdown 共存（每个 delta 重新解析对 MVP 规模输出可接受；如需再优化）。

### D4 —— Tier 2 后端新增（最小、增量）

| Method | Path | 包装 | 说明 |
|--------|------|------|------|
| GET | `/api/v1/memory/status` | `MemoryManager::get_status` | project/global 计数 |
| GET | `/api/v1/memory` | `list_memories` | query: scope、min_importance、limit、offset、q |
| GET | `/api/v1/memory/:id` | `get_memory` | 含 origin（project/global） |
| POST | `/api/v1/memory/prune` | `prune` | 可选 dry_run；UI 二次确认 |
| GET | `/api/v1/overview` | 拼装 | project_root、计数、model、version |
| GET | `/api/v1/config` | 扩展 | 增量脱敏字段；**api_key 永不明文** |

- 全部沿用现有 bearer 鉴权。handler 风格参照 `src/daemon/handlers.rs`。DTO 放 `src/daemon/models.rs`。路由注册到 `routes.rs::agent_routes` 或新的 `ops_routes()` 分组。

### D5 —— 状态管理

- 扩展现有 Zustand store（`web/src/state/chatStore.ts`），不引入 Redux。新增切片：sessions、todos、permissions-queue、memory。
- 轮询端点（pending-permissions、todos、task-progress）用 `@tanstack/react-query`，免费获得缓存 + 去重 + 合理轮询间隔。

### D6 —— 安全

- bearer token 模型不变。dev 用 Vite 代理注入；生产静态托管复用 daemon 鉴权。
- `/config` 与 `/overview` 的密钥脱敏在服务端强制（绝不信任客户端）。`api_key` 字段掩码为 `set: true` 或 `****last4`。
- prune / delete 需显式 POST/DELETE + UI 确认。

## 风险 / 权衡

| 风险 | 缓解 |
|------|------|
| 每个 token 重新解析 Markdown 在超大输出时变慢 | 节流解析（如 rAF）；仅最终渲染为权威 |
| 客户端 loop 与 Rust `loop_.rs` 漂移 | `web/src/agent/loop.ts` 头部注释指向镜像的行号；移植测试 |
| Tier 2 后端范围蔓延 | 严格限定：仅 memory + 脱敏 overview；无配置写回 |
| diff 渲染保真度 | 先做 `file_edit`（单 hunk）；`apply_patch` 多 hunk 后续 |

## 待决问题（Open Questions）

1. 语法高亮器：`shiki`（较重、准确）vs `prismjs`（较轻）——在 Tier 1 任务 #1 时按 bundle 体积影响决定。
2. `/overview` 是否值得独立端点，还是前端从现有 `/health` + `/sessions` + `/memory/status` 拼装——倾向客户端拼装以避免新增后端面，除非延迟有要求。
3. 生产托管（Tier 3）：ServeDir vs rust-embed——延后到 Tier 3 决定。
