# Design: daemon-hosted-web-ui

## Context

Web 客户端当前仅能在 Vite dev server 下工作：页面由 5173 端口服务，`/api` 由代理转发并注入 bearer token，长连流通过 `/__daemon-info` 拿到 token 后直连 daemon origin。daemon 本身（Axum，loopback-only，`src/daemon/mod.rs`）没有任何静态资源路由。要实现"单二进制开箱即用"，需要解决三件事：静态资源从哪来、浏览器如何拿到 token、缓存与降级策略。

已确认的代码事实：

- `rust-embed` 8.5 已是可选依赖，`src/knowledge/embedded.rs` 与 `src/i18n/loader.rs` 有成熟使用先例；`daemon` feature 已存在
- 路由分为 public（health/heartbeat）与 protected（auth 中间件）两组（`src/daemon/routes.rs::create_routers`）
- 全局 CORS `allow_origin(Any)`（`src/daemon/mod.rs`）——对任何带 Origin 的响应都会附加 `Access-Control-Allow-Origin`
- 客户端普通 API 调用走 `this.base`（`/api/v1`，dev 下靠代理注入 token，自身不带头）；流走 `fetchStream`（优先 `resolveDaemonDirect` 直连带头，否则回退同源**不带**头）
- Web 应用为单视图（无路由库、无 history API 使用），SPA fallback 仅需兜住 `/` 与未来扩展
- `tower-http` 已启用 `fs` feature（ServeDir 可用，但只服务磁盘目录，不适用嵌入资产）

## Goals / Non-Goals

见 proposal.md。补充技术性边界：不改认证中间件本身（bootstrap 是独立端点而非中间件改动）；不动 desktop/TUI 客户端。

## Decisions

### D1. 静态资源：rust-embed 编译时嵌入（daemon feature 下）

- 新模块 `src/daemon/web_ui.rs`：`#[derive(RustEmbed)] #[folder = "web/dist"]`，随 `daemon` feature 编译
- dist 缺失时 rust-embed 会编译失败 → 新增最小 `build.rs`：编译前 `create_dir_all("web/dist")` 并放入 `.gitkeep`（纯 `std::fs`，不依赖 Node）；vite build 会清空 dist 重建，互不冲突
- debug 构建下 rust-embed 默认运行时读磁盘（便于改 UI 不重编）；release 构建真嵌入——二进制发布形态为 release，行为正确
- MIME：手写扩展名映射（html/js/mjs/css/svg/png/ico/json/wasm/woff2），不引入新依赖
- **备选否决**：`tower-http::ServeDir` 只能服务磁盘目录，运行时部署形态（用户须带 dist）违背单二进制目标

### D2. 认证：同源 bootstrap 端点

- `GET /auth/bootstrap`（public 路由侧）返回 `{ "token": "<bearer>" }`
- 同源判定（任一命中即拒绝，403）：
  1. `Origin` 头存在且 host 不属于 `{127.0.0.1, localhost}` + 当前端口
  2. `Sec-Fetch-Site` 存在且不为 `same-origin` / `none`
  3. `Host` 头不是 `127.0.0.1:<port>` / `localhost:<port>`（杀 DNS rebinding）
- 响应 `Cache-Control: no-store`
- 威胁模型说明：同用户本地进程本可直接读 `~/.wgenty-code/daemon.token`（0600），本端点不降低该边界；它防的是**跨源网页**读取。全局 CORS 层对带 Origin 的响应附加 ACAO 头不构成泄露——handler 在跨源时已 403，浏览器本就拒绝读
- **备选否决**：HttpOnly cookie 会话需新增会话状态与中间件双轨认证，收益（防 XSS 读 token）与成本不成比例——XSS 在同源下可直接调 API

### D3. 路由分层与 SPA fallback

- `web_ui.rs` 提供的路由合并进 **public（health）路由组**：静态资产在拿到 token 前必须可访问
- `GET /` 与 `GET /assets/*` → 嵌入资产；`GET /auth/bootstrap` → D2
- 在 `mod.rs` 最终 app 上设置 **fallback handler**：路径以 `/api/` 开头 → JSON 404（API 语义不被 HTML 污染）；否则返回 `index.html`（SPA 深链）
- fallback 不经过 auth 中间件（axum 语义：fallback 挂在 merge 后的 app 上，protected 的 `route_layer` 不覆盖它）——静态资源公开，符合 D2 前提
- 降级页：嵌入资产中无 `index.html` 时，返回 Rust 内联的最小 HTML 提示页（"Web UI not bundled — run `npm --prefix web run build`"），不 500

### D4. 缓存策略

- `/assets/*`（vite 内容 hash 文件名）→ `Cache-Control: public, max-age=31536000, immutable`
- `index.html` 与其余根级文件（favicon 等）→ `Cache-Control: no-cache`
- daemon 重启换版本后：刷新页面取到新 index.html → 引用新 hash 资产，旧资产缓存自然失效

### D5. 客户端 token 适配（web/）

- 新增同源 bootstrap 探测：`resolveDaemonDirect()` 返回 null（非 dev 环境）时，尝试 `GET /auth/bootstrap` 拿 token，内存缓存 + 失败重试
- token 可用时，`DaemonClient` 的受保护请求（含 `fetchStream` 同源路径与 WS 的 `?token=` query）统一附加凭证；dev 模式行为不变（bootstrap 不存在则继续走代理注入）
- 单视图应用无需路由适配

### D6. 启动与日志

- 托管默认开启，无新 CLI flag
- 绑定端口成功后日志输出：正常 `Web UI: http://127.0.0.1:<port>`；降级 `Web UI not bundled (web/dist empty at build time)`
- 发布脚本/文档增加 `npm --prefix web run build` 预构建步骤（tasks 落实）

## Risks / Trade-offs

- **同源连接预算**：hosted 模式下页面与流同 origin，HTTP/1.1 约 6 连接/origin 上限——多标签页 + 多流可能触顶（dev 模式靠直连分离 origin 缓解）。接受为已知限制：单标签页（heartbeat + trace + global + session 流）在预算内；超限的表现是排队而非失败，且现有 watchdog/重试已兜底
- **rust-embed debug 运行时读盘**：debug 二进制脱离仓库运行时 UI 降级——发布形态为 release，接受
- **bootstrap 被 SSRF 类工具直连**：loopback + 同用户权限边界内，无新增暴露（见 D2 威胁模型）

## Migration Plan

- 纯增量：新端点 + 新 fallback，现有 API/中间件/dev 流零改动；`web/dist` 缺失时代码路径与今日等价（404 → 降级页）
- 客户端改动向后兼容：bootstrap 404 时（旧 daemon）回退现行为

## Open Questions

（已在任务拆分前解决）

- ~~`Sec-Fetch-Site: none` 是否进一步收紧~~ → 维持允许 `{same-origin, none}`：`none` 仅出现在用户直接访问地址栏等无发起源场景，不构成跨源泄露向量；收紧会把"用户直接打开 bootstrap URL"变成 403，无安全收益
- ~~wsChannel 的 token 传递在 hosted 模式下的具体接线~~ → 扩展 `resolveDaemonDirect` 为单一解析点：`/__daemon-info` 不可用时尝试同源 `GET /auth/bootstrap`，成功则返回 `{ base: window.location.origin + "/api/v1", token }`（绝对地址保证 `ws://` 推导成立）。`fetchStream`（client.ts:431）与 wsChannel（wsChannel.ts:216）**零改动**自动获得 hosted 模式支持；仅剩 `this.base` 相对路径的普通调用需在 DaemonClient 内统一附加 Authorization
