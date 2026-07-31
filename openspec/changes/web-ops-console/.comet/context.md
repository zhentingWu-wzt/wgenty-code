# Comet Design Handoff

- Change: web-ops-console
- Phase: design
- Mode: compact
- Context hash: 88d3fe4f648e3bc93debbd9837958f09cfd5210d6d6eb145de9b4762ef5bdfc3

Generated-by: comet-handoff.sh

OpenSpec remains the canonical capability spec. This handoff is a deterministic, source-traceable context pack, not an agent-authored summary.

## openspec/changes/web-ops-console/proposal.md

- Source: openspec/changes/web-ops-console/proposal.md
- Lines: 1-60
- SHA256: 6a65cf6d218282747422704ec81514c596746e4392408e455d0cc56912707fb4

```md
# 提案：Web Agent 前端（原 Web 运维控制台）

> **复活说明（2026-07-31）：** 本变更取代此前被搁置的 `web-ops-console`（只读运维大盘，原 `DEFERRED.md` 已删除）。产品重新定位为：**以 agent 驱动的 web 前端为主体；运维视图（会话 / 记忆 / 配置）作为内嵌侧边面板。** `web/` 目录已存在可用的 MVP（基于现有 daemon 的 React + Vite + TS thin client）。本提案将该 MVP 扩展为完整前端。

## 为什么（Why）

当前驱动 agent 的唯一途径是 CLI/TUI。一个浏览器前端能解锁：更丰富的渲染（Markdown、diff、代码高亮）、终端无法比拟的审批交互体验，以及未来在平板/轻量设备上的访问能力。daemon 已暴露完整的 HTTP+SSE API 面（`/api/v1/chat/stream`、`/tools/execute`、权限审批、sessions、models 等），因此 web 前端是 `src/tui` 的**平行 thin client**——不是新的后端。

MVP 已验证这一架构：流式聊天、客户端 agent loop（工具调用 + 续轮）、root 工具权限审批，全部在**零 daemon/Rust 改动**下跑通。本变更将 MVP 扩展为完整的生产级前端，并按"运维作为侧边面板"的目标，有选择地补齐少量后端缺口。

## 变更内容（What Changes）

### Tier 1 —— 核心 agent 体验（无后端改动；纯前端）

1. **Markdown + 代码渲染**：将 assistant 输出渲染为 GFM Markdown，代码块带语法高亮（替换 MVP 当前的 `pre-wrap` 纯文本）。
2. **Diff 预览**：将 `file_edit` / `apply_patch` 工具结果可视化为并排或统一 diff，而非原始文本。
3. **停止 / 取消控制**：基于 `AbortController` 的停止按钮，允许用户在轮次之间中断正在运行的会话。
4. **子 agent 异步权限**：在长时运行的 `task`/`delegate` 工具执行期间轮询 `GET /api/v1/tools/pending-permissions`，并弹出 `resolve-permission` 提示（MVP 当前只处理 root 工具的同步审批）。
5. **会话管理 UI**：通过现有 `/api/v1/sessions*` 端点实现列表 / 搜索 / 打开 / 删除（MVP 当前只用单一内存会话）。
6. **Todo / Plan 面板**：将 `GET /api/v1/todos` 与任务进度渲染为实时侧边面板。
7. **模型选择器**：通过现有 `GET /api/v1/models` + `POST /api/v1/model/switch` 实现 `/model` 切换 UI。

### Tier 2 —— 运维侧边面板（选择性、最小的后端新增）

8. **记忆面板**：需要新增 daemon 端点包装 `MemoryManager`（`GET /api/v1/memory/status`、`GET /api/v1/memory`、`GET /api/v1/memory/:id`、`POST /api/v1/memory/prune`）。`MemoryManager` 已具备这些方法，仅缺 HTTP 层。
9. **概览 + 配置（只读）面板**：轻量概览（health、计数、模型）与脱敏配置视图。概览由现有端点拼装；配置读取可在 `GET /api/v1/config` 基础上做脱敏扩展。

### Tier 3 —— 硬化

10. **响应式 / 移动端布局**；**错误恢复**（流式恢复、daemon 重连）；**由 daemon 托管的生产构建**（可选嵌入静态资源，使终端用户无需 Vite dev server）。

## 非目标（Non-goals）

- 多项目切换器、多用户 RBAC、公网级安全加固。
- 配置写回 / 热重载（配置保持只读）。
- 第二条 agent 执行路径：浏览器**始终**驱动客户端 agent loop（daemon 对 `/chat/stream` 保持纯透传代理）。
- 取代 TUI——web 前端是兄弟，不是继任者。

## 能力（Capabilities）

### 新增能力

- `web-agent-frontend`：浏览器前端——聊天交互、渲染、agent loop 驱动、权限交互、session/todo/plan/model 面板。
- `ops-panel-api`（*仅 Tier 2*）：最小化的新增 HTTP 端点（记忆 CRUD+prune、脱敏 config/overview），包装现有 manager。

### 修改的能力

- 无强制修改。Tier 2 对 `GET /api/v1/config` 的扩展与新增 memory 路由均为增量；若 `openspec/specs/` 存在 `daemon` spec，则以 delta 形式落地。

## 影响（Impact）

- **代码**：`web/`（前端，主要工作量）；`src/daemon/`（仅 Tier 2——新增 memory/overview handler + 路由，约 4 个端点，包装 `MemoryManager`）。
- **API**：Tier 1 = 无改动。Tier 2 = 增量的 `memory*` + 扩展的 `config`。
- **依赖（前端）**：`react-markdown`、`remark-gfm`、语法高亮器（`shiki` 或 `prismjs`），轮询场景可能引入 `@tanstack/react-query`。后端依赖不变。
- **安全**：daemon 现有 bearer token 模型继续生效；dev 沿用 MVP 的 Vite dev server token 注入；生产静态托管复用 daemon 鉴权。密钥永不以明文离开服务端。
- **文档**：`WGENTY.md` Daemon 小节补充 web 前端 URL 与能力。

## MVP 现状

`web/` MVP（`dev` 分支上的 commit `2bf57250` + `a025634b`）是 **Tier 0 基线**——已合并、视为完成。本变更从 Tier 1 开始。

```

## openspec/changes/web-ops-console/design.md

- Source: openspec/changes/web-ops-console/design.md
- Lines: 1-87
- SHA256: 671c7ed9e36df5fe0f23c74bb59dbbf8899455efdb46872c7b37afdb28923311

[TRUNCATED]

```md
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

- 浏览器始终驱动 `runAgentLoop`（`web/src/agent/loop.ts`）。Tier 1 扩展它：`AbortController` 实现取消；在 `task`/`delegate` 执行期间派生 pending-permissions 轮询器（镜像 `src/tui/agent/adapters.rs:152-235`）。
- daemon 永不运行该 loop。这保留了干净的透传契约，并使 `src/agent/runtime/loop_.rs` 保持唯一事实源（web 端移植持续跟踪它）。

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

```

Full source: openspec/changes/web-ops-console/design.md

## openspec/changes/web-ops-console/tasks.md

- Source: openspec/changes/web-ops-console/tasks.md
- Lines: 1-58
- SHA256: 03af54a8880b4e7b77126d52f789db2e6874a1b4a5a028a67ff8b7a7343f055d

```md
# 任务：Web Agent 前端

> Tier 0（MVP）已完成（`dev` 分支上的 commit `2bf57250`、`a025634b`）。下列任务从 Tier 1 开始。复选框遵循 OpenSpec 约定。

## Tier 1 —— 核心 agent 体验（无后端改动）

### 1. 渲染

- [ ] 1.1 引入 `react-markdown` + `remark-gfm`；将 assistant 内容渲染为 GFM Markdown（替换 `ChatView.tsx` 中的 `pre-wrap`）
- [ ] 1.2 为围栏代码块加语法高亮（按设计 OQ1 在 `shiki` 与 `prismjs` 间抉择）；评估 bundle 体积影响
- [ ] 1.3 以区别样式渲染 `reasoning` 块（默认折叠）
- [ ] 1.4 新增 `<DiffView>` 组件；将 `file_edit` 工具输出解析为统一 diff 并渲染

### 2. agent loop 扩展

- [ ] 2.1 将 `AbortController` 贯穿 `runAgentLoop` + `chatStream`；UI 加停止按钮，在轮次之间中断
- [ ] 2.2 实现子 agent 异步权限：在 `task`/`delegate` 执行期间派生轮询器命中 `GET /api/v1/tools/pending-permissions`；每条作为权限弹窗呈现，经 `POST /api/v1/tools/resolve-permission` 解决
- [ ] 2.3 在 `<DiffView>` 中处理 `apply_patch` 多 hunk diff

### 3. 会话管理

- [ ] 3.1 新增会话侧边栏：列表 + 搜索（`/api/v1/sessions`、`/sessions/search`）
- [ ] 3.2 打开 / 切换到既有会话（`GET /api/v1/sessions/:id`）；将历史接入 chat store
- [ ] 3.3 在轮次边界持久化当前会话（`PUT /api/v1/sessions/:id`）；带确认的删除

### 4. 侧边面板（只读，现有 API）

- [ ] 4.1 Todo 面板：运行中轮询 `GET /api/v1/todos`；渲染条目 + 状态
- [ ] 4.2 任务进度面板：`GET /api/v1/tasks/progress` + `GET /api/v1/tasks`
- [ ] 4.3 模型选择器 UI：`GET /api/v1/models` → `POST /api/v1/model/switch`

## Tier 2 —— 运维侧边面板（最小的后端新增）

### 5. 记忆 API（后端）

- [ ] 5.1 在 `src/daemon/models.rs` 新增 DTO：`MemoryStatusResponse`、`MemoryItemResponse`（含 `origin`）、`MemoryListResponse`、`PruneRequest`/`PruneResult`
- [ ] 5.2 实现包装 `MemoryManager` 的 handler：`GET /memory/status`、`GET /memory`、`GET /memory/:id`、`POST /memory/prune`
- [ ] 5.3 注册路由（protected）；补充 handler 测试，断言 scope/importance 过滤 + prune dry_run
- [ ] 5.4 前端记忆面板消费上述端点

### 6. 概览 + 脱敏配置（后端）

- [ ] 6.1 决定 `/overview` 形状（设计 OQ2）：客户端拼装 vs 新端点
- [ ] 6.2 扩展 `GET /api/v1/config` 为增量脱敏字段；断言 `api_key` 永不明文（服务端测试）
- [ ] 6.3 前端概览面板：health、project root、计数、模型摘要
- [ ] 6.4 前端配置面板：只读分组展示，脱敏

## Tier 3 —— 硬化（草图）

- [ ] 7.1 响应式 / 移动端布局
- [ ] 7.2 错误恢复：流式恢复、daemon 重连指示
- [ ] 7.3 由 daemon 托管的生产构建（ServeDir 或 rust-embed）——按设计 OQ3 决定

## 文档与收尾

- [ ] 8.1 更新 `WGENTY.md` Daemon 小节：web 前端 URL、能力、token 路径
- [ ] 8.2 更新 `web/README.md`：新面板与 Tier 2 端点
- [ ] 8.3 `cargo fmt` + `cargo clippy --all-targets -- -D warnings` + 相关 `cargo test`（后端）；`npm run build` + `npm test`（前端）

```

## openspec/changes/web-ops-console/specs/ops-panel-api/spec.md

- Source: openspec/changes/web-ops-console/specs/ops-panel-api/spec.md
- Lines: 1-94
- SHA256: f8db09978dfdab447716fd1361d5ab2b76623eb747df291ab0984ee4fbd105e2

[TRUNCATED]

```md
## ADDED Requirements

### Requirement: Overview API

The daemon SHALL expose `GET /api/v1/overview` on the protected router, returning a JSON summary for the current project-bound daemon instance.

#### Scenario: Overview success

- **WHEN** an authenticated client calls `GET /api/v1/overview`
- **THEN** the response includes project root path, session count, memory status summary fields (including project and global counts when available), main model name, and daemon/app version string

#### Scenario: Overview requires auth

- **WHEN** an unauthenticated client calls `GET /api/v1/overview`
- **THEN** the request is rejected with the same auth failure behavior as other protected routes

### Requirement: Memory status API

The daemon SHALL expose `GET /api/v1/memory/status` that returns the current `MemoryStatus` (or equivalent JSON) from `MemoryManager`.

#### Scenario: Status reflects dual pools

- **WHEN** project and global memories exist
- **THEN** the status payload reports both project and global counts

### Requirement: Memory list API

The daemon SHALL expose `GET /api/v1/memory` to list memories with optional filters.

#### Scenario: Default list

- **WHEN** an authenticated client calls `GET /api/v1/memory` without filters
- **THEN** the response is a JSON list (or `{ items: [...] }`) of memory summaries including id, type, importance, timestamp, origin (project|global), and a content preview or full content consistent with size limits documented by implementation

#### Scenario: Scope filter

- **WHEN** the client passes `scope=project` or `scope=global`
- **THEN** only memories from that origin are returned

#### Scenario: Importance and pagination

- **WHEN** the client passes `min_importance`, `limit`, and `offset`
- **THEN** results respect those constraints

### Requirement: Memory get by id API

The daemon SHALL expose `GET /api/v1/memory/:id` returning one memory including origin.

#### Scenario: Found

- **WHEN** the id exists
- **THEN** the response includes full content, metadata/tags when present, and origin

#### Scenario: Not found

- **WHEN** the id does not exist
- **THEN** the API returns a 404-class error response

### Requirement: Memory prune API

The daemon SHALL expose `POST /api/v1/memory/prune` that invokes existing prune logic and returns a structured prune result.

#### Scenario: Prune executes

- **WHEN** an authenticated client posts to `/api/v1/memory/prune`
- **THEN** the response includes before/after/removed counts (including per-pool fields when available)

### Requirement: Expanded read-only config API

`GET /api/v1/config` SHALL return an ops-oriented read-only DTO broader than model transport alone, and MUST redact secrets.

#### Scenario: Grouped safe fields

- **WHEN** an authenticated client calls `GET /api/v1/config`
- **THEN** the response includes safe summaries for models (names/base URLs without raw api_key), transport, agent toggles/budgets summary, guardian/sandbox enablement summary, and memory storage thresholds summary

#### Scenario: Secrets redacted

- **WHEN** settings contain an API key
- **THEN** the config JSON does not include the raw api_key string

```

Full source: openspec/changes/web-ops-console/specs/ops-panel-api/spec.md

## openspec/changes/web-ops-console/specs/web-agent-frontend/spec.md

- Source: openspec/changes/web-ops-console/specs/web-agent-frontend/spec.md
- Lines: 1-104
- SHA256: 6373180c506a404f2bd37f7f5381a2340ccb01aca3d0d18511750a5b84172826

[TRUNCATED]

```md
## ADDED Requirements

### Requirement: Browser agent frontend

The frontend SHALL be a browser application (React + Vite + TS in `web/`) that drives the agent by consuming the daemon's `/api/v1/*` HTTP+SSE surface. It is a parallel thin client of `src/tui`, not a new backend.

#### Scenario: Standalone dev server

- **WHEN** a developer runs the Vite dev server against a running daemon
- **THEN** the browser at the dev port can stream chat, execute tools, and approve permissions without any daemon code change beyond what already exists

#### Scenario: Client-side agent loop

- **WHEN** the model emits tool calls during a chat stream round
- **THEN** the frontend executes each via `POST /api/v1/tools/execute`, appends results, and re-streams the next round until a round produces no tool calls (the daemon never runs the loop)

#### Scenario: Optional daemon-hosted production build

- **WHEN** a production build is served (Tier 3)
- **THEN** it MAY be hosted by the daemon as static assets, reusing the same bearer-token auth as the API

### Requirement: Token-gated API access

The frontend SHALL authenticate to protected `/api/v1/*` endpoints with the daemon bearer token and MUST NOT embed the token in committed source or served HTML.

#### Scenario: Dev server token injection

- **WHEN** the Vite dev server proxies an `/api` request
- **THEN** the bearer token (read from `~/.wgenty-code/daemon.token`) is injected server-side by the proxy, never reaching browser bundle code

#### Scenario: No token in client bundle

- **WHEN** the production frontend bundle is inspected
- **THEN** it contains no hardcoded daemon token

### Requirement: Rich content rendering

The frontend SHALL render assistant output as GFM Markdown with syntax-highlighted code blocks, and SHALL render `file_edit`/`apply_patch` tool results as diffs.

#### Scenario: Markdown rendering

- **WHEN** the assistant streams Markdown content (headings, lists, fenced code)
- **THEN** the UI renders it as formatted Markdown, not raw text

#### Scenario: Code block highlighting

- **WHEN** a fenced code block is rendered
- **THEN** its syntax is highlighted

#### Scenario: Tool diff preview

- **WHEN** a `file_edit` or `apply_patch` tool result is displayed
- **THEN** the UI shows a unified diff view rather than raw tool output text

### Requirement: Interruptible agent turns

The frontend SHALL allow the user to interrupt a running agent turn.

#### Scenario: Stop between rounds

- **WHEN** the user activates the stop control while a turn is running
- **THEN** the in-flight stream is aborted at the next round boundary and the conversation is left in a consistent state

### Requirement: Permission approval UX

The frontend SHALL surface both root-tool synchronous permission prompts and subagent asynchronous permission prompts, resolving each via the appropriate endpoint.

#### Scenario: Root-tool approval

- **WHEN** `POST /api/v1/tools/execute` returns `permission_required`
- **THEN** the UI presents a modal offering Allow once / Always allow / Deny, and on approval follows the approve → execute → (optionally unapprove) sequence

#### Scenario: Subagent async approval

- **WHEN** a long-running `task`/`delegate` tool causes pending subagent permissions
- **THEN** the frontend polls `GET /api/v1/tools/pending-permissions` and surfaces each request, resolving via `POST /api/v1/tools/resolve-permission`

### Requirement: Session management UI

The frontend SHALL provide session list, search, open, and delete-with-confirmation using existing session APIs.

```

Full source: openspec/changes/web-ops-console/specs/web-agent-frontend/spec.md
