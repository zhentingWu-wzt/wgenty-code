# Design: Web 项目运维控制台（P0）

## Context

- Daemon（feature `daemon`）已有 loopback Axum 服务、API token 文件、CORS（localhost dev ports）、sessions CRUD/search、精简 `GET /api/v1/config`、health。
- `MemoryManager` 已具备 `get_status` / `list_memories` / `get_memory` / `prune` 与 project/global 分层，但 **无 HTTP 面**。
- 控制台场景已锁定为 **项目运维台**（单项目、本机/受信网络），P0 = 概览 + 会话 + 记忆 + 配置只读。

## Goals / Non-Goals

**Goals**

- 浏览器打开 daemon 即可使用运维台（同端口静态资源 + `/api/v1/*`）。
- P0 四个表面：Overview / Sessions / Memory / Config（只读）。
- 危险写操作（删会话、prune）需 UI 确认；API 保持显式 POST/DELETE。
- 密钥与 token 永不以明文出现在 config API 响应中。
- 复用现有 `SessionManager` / `MemoryManager`，避免第二套存储。

**Non-Goals（P0）**

- 控制台驱动完整 agent chat。
- 配置写回 / 热重载。
- 每轮 raw prompt 溯源（P2）。
- 多项目切换、多租户 RBAC、非 loopback 绑定默认开启。

## Decisions

### D1 — 挂载方式：同端口静态 + API（推荐）

- daemon 在受保护路由旁（或合并 router）提供 `GET /` 与静态资产（`/assets/*`）。
- API 仍为 `/api/v1/*` + `Authorization: Bearer <token>`。
- 前端为 **零构建或极简静态**（单页 HTML + 原生 JS 或预构建放入 `src/daemon/web/static/`，`rust-embed` 可选；P0 优先 `tower-http::services::ServeDir` + 源码树静态文件，避免强制前端 toolchain 进 CI——若选 embed，构建期拷贝即可）。
- **Token 引导**：首屏提供 token 输入，写入 `sessionStorage`；不把 token 嵌进 HTML。

**Alternatives considered**

- 独立 Vite dev server only：开发体验好，但运维台「长期挂着」需要同进程托管。
- 服务端模板全 SSR：交互表格成本高，P0 不划算。

### D2 — API 面

| Method | Path | 说明 |
|--------|------|------|
| GET | `/api/v1/health` | 已有，公开 |
| GET | `/api/v1/overview` | **新增**：project_root、session_count、memory status 摘要、model name、version |
| GET | `/api/v1/sessions` 等 | **已有**，UI 直接消费 |
| GET | `/api/v1/memory/status` | **新增** → `MemoryStatus` |
| GET | `/api/v1/memory` | **新增** list；query: `scope=project\|global\|all`, `min_importance`, `limit`, `offset`, `q`（可选子串） |
| GET | `/api/v1/memory/:id` | **新增** 单条；响应含 `origin`（project/global） |
| POST | `/api/v1/memory/prune` | **新增**；body 可选 dry_run；返回 `PruneResult` |
| GET | `/api/v1/config` | **扩展只读**：在现有字段上增加 agent/integrations/storage 等安全子集；所有 `api_key` / token 类字段输出掩码（如 `****` + last4 或仅 `set: true`） |

Sessions 详情已有 `messages`；P0 UI 只读展示，不做控制台内续聊。

### D3 — 配置只读形状

- 响应分组：`models`（main/small/planner 的 name、base_url、**无 api_key 明文**）、`transport`、`agent`（plan_mode、token_budget、subagent 安全相关摘要）、`integrations.guardian/sandbox` 开关、`storage.memory` 阈值摘要、`prompt` 非敏感开关。
- 未知/未来字段不盲目 `Serialize` 整个 `Settings`（防止泄漏）；使用显式 `OpsConfigResponse` DTO。
- P0 **无** PUT/PATCH。

### D4 — 前端信息架构

```
/                Overview
/sessions        列表 + 搜索
/sessions/:id    详情（消息只读）+ 删除
/memory          列表（scope/importance 过滤）+ prune
/memory/:id      详情
/config          只读分组展示
```

- 客户端路由：hash router（`#/sessions`）以免 daemon 静态回退复杂化；或 fallback `index.html`（若 ServeDir 配置 nest）。
- 视觉：简洁运维风，无品牌动画要求；错误态展示 API message。

### D5 — 安全

- 默认 bind `127.0.0.1`（保持现状）。
- 除 health 外 API 均需 token；静态页面可公开（无密钥），API 调用带 Bearer。
- prune / delete session：UI 二次确认；不做额外 CSRF token（Bearer + loopback 足够 P0）。
- 日志不打印 token、api_key、完整 memory content（debug 可截断）。

### D6 — 测试策略

- 后端：handlers 级集成测试（与现有 daemon route tests 风格一致）— overview/memory/config 脱敏断言。
- 前端：P0 不做 E2E 强制；可手工脚本 + 关键 DOM 烟测可选。
- `cargo test --features daemon`（或 default 已含 daemon）覆盖新路由。

## Risks / Trade-offs

| Risk | Mitigation |
|------|------------|
| 静态资源膨胀进 binary | P0 优先 ServeDir 开发路径；release 再用 embed 或外置 `web/` |
| list_memories 全量进内存 | 沿用 MemoryManager 已有内存模型；API 做 limit/offset |
| Config DTO 落后 Settings | 文档标明 P0 子集；后续按需加字段 |
| CORS 与同端口 | 同端口无 CORS 问题；保留 dev server origins |

## Migration Plan

- 无数据迁移。
- 文档：`WGENTY.md` 增加「打开 http://127.0.0.1:<port>/ ，token 见 daemon.token」。
- 回滚：feature 内路由可整段禁用；不影响 CLI。

## Open Questions

1. 静态资源：P0 实现时在 **ServeDir（源码旁路）** vs **rust-embed** 二选一——默认 ServeDir + 可选 embed feature 后续再加。
2. Memory list 是否需要按 `memory_type` 过滤——P0 可用客户端过滤；若列表很大再补 query。
