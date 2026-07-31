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
- **Tier 3（生产）**：**本期决定不做**（保留 Vite dev server 双进程模式）。调研结论：ServeDir 二进制 0 增量但破坏单二进制分发（npm 包装器只发二进制）；rust-embed 保单二进制但烘进 ~415KB 几乎吃满 AGENTS.md 的 500KB 上限（jieba-rs 先例已因类似体积被拒），且破坏 `cargo run` dev 工作流。**等真有跑不了 `npm run dev` 的终端用户再决**，那时按"是否必须保单二进制分发"二选一。
- **为何不从第一天起同端口**（推翻原设计 D1）：MVP 已证明独立方式可行，且避免在 Tier 1 阶段把前端工具链强塞进 Rust 构建/CI。

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

### D7 —— 错误恢复（修复三个真实问题）

调研发现三个问题（其中 #2 是回归 bug，优先级最高）：

1. **状态栏撒谎**（`App.tsx:34-53`）：`connection` 只在启动探活一次，daemon 死后永远显示"已连接"。
   - **修法**：把一次性探活改成周期性心跳（复用 `usePolling` 命中 `/health`，~10s 间隔；失败设 `disconnected`，成功恢复 `connected`）。
2. **trace SSE 静默挂死**（`usePermissionTrace.ts:50-53`，**本变更引入的回归**）：bare `catch {}` 退出且不重连，依赖数组是稳定引用，effect 不会重跑。daemon 重启后子 agent 权限推送通道**永久死亡**，agent 看似在思考实则在等永不弹出的权限框。
   - **修法**：在 catch 后用指数退避重连（初始 1s，上限 30s）；重连成功重置退避。把重连逻辑放进循环而非依赖 effect 重跑。
3. **流式中断无重试**（`App.tsx:104-112`）：错误是纯字符串，无法区分 daemon 死 / LLM 拒绝 / 网络抖；半句话保留（好）但无"重试"入口。
   - **修法**：给 `lastError` 加结构（区分 transport vs upstream），transport 类错误在错误条上加"重试"按钮（重发上一条 user message）。

### D8 —— 响应式布局

- 当前**零个 `@media` 查询**，主要硬编码是 `.sidebar { width: 280px }`（占 375px 屏 75%）。
- 侧边栏已有 `collapsed` 布尔状态（按钮触发），改成移动端 overlay 抽屉很自然——无需新状态机，只需 CSS。
- 断点策略（纯增量 `@media`，不拆现有 CSS）：
  - **≤1024px（平板）**：侧边栏默认折叠成 28px rail。
  - **≤768px（手机）**：侧边栏变 `position: fixed` 滑出抽屉 + backdrop 遮罩（复用 `collapsed` 切换）；`.chat-list { max-width }` 收起；`.app { height: 100dvh }`（修复移动浏览器工具栏导致的 `100vh` 跳动）。
  - **≤375px（小手机）**：Composer 紧凑化、状态栏截断模型名。
- 无客户端路由（纯状态切换），故无 SPA fallback 顾虑（也佐证 D1 托管方案无需 fallback 配置）。

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
