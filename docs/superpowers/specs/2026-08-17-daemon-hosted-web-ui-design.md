---
comet_change: daemon-hosted-web-ui
role: technical-design
canonical_spec: openspec
date: 2026-08-17
archived-with: 2026-08-18-daemon-hosted-web-ui
status: final
---

# Daemon-Hosted Web UI — 深度设计

## 背景与目标

Web 客户端当前仅能在 Vite dev server 下使用（`web/README.md` "Run it" 流程）：页面由 5173 端口服务、`/api` 靠代理注入 bearer token、长连流经 `/__daemon-info` 直连 daemon origin。本设计让 daemon 在编译时嵌入 `web/dist` 并直接托管，实现单二进制开箱即用：`wgenty-code daemon --port 8371` 启动后浏览器打开 `http://127.0.0.1:8371` 即得完整界面。

高层决策（D1–D6）见 `openspec/changes/daemon-hosted-web-ui/design.md`：rust-embed 嵌入 / 同源 bootstrap 认证 / public 路由组 + 前缀感知 fallback / 分层缓存 / 客户端单一解析点 / 默认开启 + URL 日志。本文档将其深化到组件级。

关键代码事实：

- 路由分 public（health/heartbeat）与 protected（auth `route_layer`）两组：`src/daemon/routes.rs::create_routers` 返回二元组，`src/daemon/mod.rs` merge 后再叠 CORS + body-limit 层
- `DaemonState` 已持有 api token（`set_api_token`，`ws_push` 的 in-handler 认证同源）
- 客户端普通调用走 `this.base`（`/api/v1` 相对路径，dev 靠代理注入，自身不带头）；流走 `fetchStream`（优先 `resolveDaemonDirect` 直连带头，回退同源不带）；WS 经 `wsChannel` 复用 `resolveDaemonDirect` 推导 `ws://…?token=`
- Web 应用单视图（无路由库），SPA fallback 仅需兜 `/` 与未来扩展
- `rust-embed` 8.5 已有先例（`src/knowledge/embedded.rs`、`src/i18n/loader.rs`）；`tower-http::ServeDir` 只能服务磁盘目录，不适用

## §1 模块结构

新增 `src/daemon/web_ui.rs`（估 ~250 行）：

```rust
#[derive(RustEmbed)]
#[folder = "web/dist"]
struct WebAssets;

/// 挂入 health（public）路由组
pub(crate) fn public_router() -> Router<Arc<DaemonState>> {
    Router::new()
        .route("/", get(serve_index))
        .route("/assets/*path", get(serve_asset))
        .route("/auth/bootstrap", get(bootstrap_token))
}

/// 挂在 mod.rs merge 后的最终 app 上（.fallback(spa_fallback)）
pub(crate) async fn spa_fallback(uri, method, state) -> Response
```

组件职责：

| 单元 | 职责 | 依赖 |
|---|---|---|
| `serve_index` | 嵌入有 `index.html` → 返回（no-cache）；无 → Rust 内联降级页（200） | `WebAssets` |
| `serve_asset` | 路径查嵌入资产，MIME 表映射，`immutable` 长缓存，未命中 404 | `WebAssets`、`mime_for()` |
| `bootstrap_token` | §2 同源谓词，放行返回 `{token}`（no-store），拒绝 403 JSON | `DaemonState` |
| `spa_fallback` | `/api/` 前缀 → JSON 404；GET → `serve_index`；非 GET → 405 | `serve_index` |
| `mime_for(ext)` | 扩展名→MIME（html/js/mjs/css/svg/png/ico/json/wasm/woff2/map），默认 `application/octet-stream` | — |

静态路由必须在 public 组：页面加载先于任何 token 获取。fallback 挂在 merge 后的 app，不受 protected 组 `route_layer` 影响（axum 语义），静态深链公开可达，与 §2 的"跨源读不到 token"边界一致。

降级页为 Rust 内联最小 HTML（标题 + 一行提示 `Web UI not bundled — run \`npm --prefix web run build\``），不依赖嵌入资产存在。

## §2 Bootstrap 同源谓词

`GET /auth/bootstrap`，全部检查通过才放行（任一失败 403 JSON `{ "error": "cross-origin request rejected" }`）：

1. **Origin** 头存在 → 其 host（含端口）必须 ∈ `{127.0.0.1:<port>, localhost:<port>}`（port 取自 listener 绑定地址）
2. **Sec-Fetch-Site** 头存在 → 必须 ∈ `{same-origin, none}`（`none` 覆盖用户直接打开 URL 的场景）
3. **Host** 头必须 ∈ `{127.0.0.1:<port>, localhost:<port>}` —— 防 DNS rebinding（attacker.com 解析到 127.0.0.1 时 Host 是攻击者域名）

放行响应：`{ "token": "<bearer>" }` + `Cache-Control: no-store`。token 读 `DaemonState`（与 `ws_push` in-handler 认证同源，无新增状态）。

威胁模型：同用户本地进程本可直接读 `~/.wgenty-code/daemon.token`（0600），本端点不降低该边界；防的是跨源网页读取。全局 CORS（`allow_origin Any`）会对带 Origin 的响应附加 ACAO 头，不构成泄露——跨源时 handler 已 403，浏览器亦拒绝读取响应体。无 token 调 API 的 401 语义不变。

## §3 客户端接线（3 个触点）

1. **`resolveDaemonDirect()`**（`web/src/api/client.ts`）：现有 `/__daemon-info` 请求 404/失败时（非 vite 环境），尝试同源 `GET /auth/bootstrap`；成功返回 `{ base: location.origin + "/api/v1", token }`（绝对地址保证 `wsChannel` 的 `ws://` 推导成立），失败返回 `null`。dev 与旧 daemon 行为不变。`fetchStream`（client.ts:431）与 `wsChannel`（wsChannel.ts:216）零改动自动获得 hosted 模式。
2. **`DaemonClient.authedFetch()`**：新增私有包装，token 可用时注入 `Authorization: Bearer <token>`；替换 `DaemonClient` 内全部裸 `fetch(` 调用点（含 `ensureViewer` 的 `POST /ui/viewers`——protected 路由，hosted 模式同样需要头）。机械替换，vitest 全量回归。
3. **token 生命周期**：按调用时解析（沿用现有"每次新解"语义）——daemon 重启换 token 后无需刷新页面。

hosted 模式下 streams 为同源 fetch + Authorization 头：无 CORS preflight，直连预算与页面共享（见 §4）。

## §4 与 sse-to-websocket 的协同

活跃变更 `sse-to-websocket`（build 阶段，16/17）正把多路 SSE 迁移到单 WS 推送通道——恰好缓解本设计风险表中的"同源连接预算"限制（hosted 模式页面与流同 origin，HTTP/1.1 约 6 连接/origin）。本变更无需额外处理，迁移完成后预算压力自然消失。

冲突面：双方都改 `web/src/api/client.ts`（对方改 `fetchStream`/`wsChannel`，本方改 `resolveDaemonDirect` 与 fetch 调用点）。逻辑无耦合，预期小范围文本冲突，合并时留意。

## §5 测试策略

| 层 | 内容 |
|---|---|
| Rust 单元（web_ui.rs 内） | `mime_for` 表；缓存头（`/assets/*` immutable、index no-cache）；bootstrap 谓词四方向：同源放行 / 跨 Origin 拒 / rebinding Host 拒 / `Sec-Fetch-Site: cross-site` 拒 |
| Rust 边界 | 复用 boundary 测试与生产路由共表先例：`GET /` 200 HTML；未知 `/api/v1/*` 404 JSON 非 HTML；深链 fallback 返回 index；dist 为空时降级页 |
| 认证回归 | 无 token 调 protected API 仍 401（现有测试不动即证明） |
| vitest | `resolveDaemonDirect` 回退链（`__daemon-info` 404 → bootstrap 成功/失败）；`authedFetch` 头注入；无 bootstrap 端点时零行为变化 |
| 手动 E2E | tasks.md 6.1 正常流程 / 6.2 降级启动 / 6.3 dev 回归 |

## §6 构建细节

- **`build.rs`**（新增，纯 std）：`create_dir_all("web/dist")` + 写 `.gitkeep`（若目录为空）；`println!("cargo:rerun-if-changed=web/dist")` 使新 dist 触发重编。cargo 全链路不依赖 Node。
- **rust-embed 行为**：debug 构建运行时读磁盘（改 UI 免重编 Rust）；release 真嵌入。发布形态为 release，正确。
- **vite 交互**：`npm run build` 默认 `emptyOutDir` 清空 dist 重建，`.gitkeep` 消失属正常（下次 cargo 构建时 build.rs 重建占位仅当目录不存在）。
- **`Cargo.toml`**：`rust-embed` 已为可选依赖，确认其挂入 `daemon` feature 所需列表（随实现核对）。
- **发布脚本**：预构建步骤 `npm --prefix web run build` 置于 cargo release build 之前（tasks 5.1 落实）。

## 错误处理汇总

| 路径 | 行为 |
|---|---|
| dist 未构建 | 200 降级页 + 启动日志 `Web UI not bundled (web/dist empty at build time)` |
| 资产未命中 | 404（`/assets/*` 与 fallback 均不吞掉） |
| 未知 API 路径 | 404 JSON（fallback 显式判 `/api/` 前缀） |
| bootstrap 跨源 | 403 JSON，token 不出 |
| 客户端 bootstrap 失败 | 回退现行为（dev 代理/无 token），页面显示连接错误（现状语义） |

## 明确不做

LAN/远程访问、自动开浏览器、HttpOnly cookie 会话、build.rs 调 npm、移动端适配、对 TUI/desktop 客户端的任何改动。
