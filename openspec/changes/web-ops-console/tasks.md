# 任务：Web Agent 前端

> Tier 0（MVP）已完成（`dev` 分支上的 commit `2bf57250`、`a025634b`）。下列任务从 Tier 1 开始。复选框遵循 OpenSpec 约定。

## Tier 1 —— 核心 agent 体验（无后端改动）

### 1. 渲染

- [x] 1.1 引入 `react-markdown` + `remark-gfm`；将 assistant 内容渲染为 GFM Markdown（替换 `ChatView.tsx` 中的 `pre-wrap`）
- [x] 1.2 为围栏代码块加语法高亮（按设计 OQ1 在 `shiki` 与 `prismjs` 间抉择）；评估 bundle 体积影响
- [x] 1.3 以区别样式渲染 `reasoning` 块（默认折叠）
- [x] 1.4 新增 `<DiffView>` 组件；将 `file_edit` 工具输出解析为统一 diff 并渲染

### 2. agent loop 扩展

- [x] 2.1 将 `AbortController` 贯穿 `runAgentLoop` + `chatStream`；UI 加停止按钮，在轮次之间中断
- [x] 2.2 实现子 agent 异步权限（改为推送模型，见 design D2.1）：`TraceEvent` 增 `kind`/`permission` 字段；`PermissionBridge` 注册/解决时经 `trace_hub()` 广播；前端订阅 `/subagents/trace/stream` SSE，按 `kind` 分发到权限弹窗，经 `POST /tools/resolve-permission` 解决（取代 500ms 轮询）
- [x] 2.3 在 `<DiffView>` 中处理 `apply_patch` 多 hunk diff

### 3. 会话管理

- [x] 3.1 新增会话侧边栏：列表 + 搜索（`/api/v1/sessions`、`/sessions/search`）
- [x] 3.2 打开 / 切换到既有会话（`GET /api/v1/sessions/:id`）；将历史接入 chat store
- [x] 3.3 在轮次边界持久化当前会话（`PUT /api/v1/sessions/:id`）；带确认的删除

### 4. 侧边面板（只读，现有 API）

- [x] 4.1 Todo 面板：运行中轮询 `GET /api/v1/todos`；渲染条目 + 状态
- [x] 4.2 任务进度面板：`GET /api/v1/tasks/progress` + `GET /api/v1/tasks`
- [x] 4.3 模型选择器 UI：`GET /api/v1/models` → `POST /api/v1/model/switch`

## Tier 2 —— 运维侧边面板（最小的后端新增）

### 5. 记忆 API（后端）

- [x] 5.1 在 `src/daemon/models.rs` 新增 DTO：`MemoryItemResponse`/`MemoryDetailResponse`（含 `origin`，flatten MemoryEntry）、`MemoryListResponse`、`MemoryListQuery`、`PruneRequest`（复用现有 `MemoryStatus`/`PruneResult`）
- [x] 5.2 实现包装 `MemoryManager` 的 handler：`GET /memory/status`、`GET /memory`、`GET /memory/:id`、`POST /memory/prune`
- [x] 5.3 注册路由（protected）；补充 handler 测试，断言 scope/importance 过滤 + prune dry_run
- [x] 5.4 前端记忆面板消费上述端点

### 6. 概览 + 脱敏配置（后端）

- [x] 6.1 决定 `/overview` 形状（设计 OQ2）：客户端拼装（从 /health + /sessions + /memory/status 组装，不新增端点）
- [x] 6.2 复用现有 `GET /api/v1/config`（已脱敏，不含 api_key）；P0 不扩展后端字段
- [x] 6.3 前端概览面板：health、计数（会话/记忆）
- [x] 6.4 前端配置面板：只读分组展示，脱敏

## Tier 3 —— 硬化（草图）

- [ ] 7.1 响应式 / 移动端布局
- [ ] 7.2 错误恢复：流式恢复、daemon 重连指示
- [ ] 7.3 由 daemon 托管的生产构建（ServeDir 或 rust-embed）——按设计 OQ3 决定

## 文档与收尾

- [x] 8.1 更新 `WGENTY.md` Daemon 小节：web 前端 URL、能力、token 路径
- [x] 8.2 更新 `web/README.md`：新面板与 Tier 2 端点
- [x] 8.3 `cargo fmt` + `cargo clippy --all-targets -- -D warnings` + 相关 `cargo test`（后端）；`npm run build` + `npm test`（前端）
