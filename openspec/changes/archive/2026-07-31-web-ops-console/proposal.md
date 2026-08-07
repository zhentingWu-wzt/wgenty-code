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
