---
change: daemon-hosted-web-ui
design-doc: docs/superpowers/specs/2026-08-17-daemon-hosted-web-ui-design.md
base-ref: 4b7bfd89cac6d308213cc8db95a0325c0947e1f4
---

# 实施计划：Daemon-Hosted Web UI

- **设计文档**：`docs/superpowers/specs/2026-08-17-daemon-hosted-web-ui-design.md`（本计划引用 §1–§6 均指向该文档章节）
- **任务清单**：`openspec/changes/daemon-hosted-web-ui/tasks.md`（16 任务 / 6 组，本计划末尾附映射表，逐条覆盖）
- **目标**：daemon 在编译时嵌入 `web/dist` 并直接托管，`wgenty-code daemon --port 8371` 启动后浏览器打开 `http://127.0.0.1:8371` 即得完整界面；同源 bootstrap 端点向页面发放 bearer token；dev 工作流与旧 daemon 客户端零行为变化。
- **明确不做**（设计文档"明确不做"节）：LAN/远程访问、自动开浏览器、HttpOnly cookie 会话、build.rs 调 npm、移动端适配、TUI/desktop 客户端改动。

## 代码锚点速查（已核实，实施时直接引用）

| 锚点 | 位置 | 说明 |
|---|---|---|
| `create_routers` | `src/daemon/routes.rs:63` | 返回 `(health_router, protected_router)` 二元组；public 组含 health/heartbeat/ws-push，protected 组有 auth `route_layer` |
| daemon 启动/合并点 | `src/daemon/mod.rs:43` `run()`；`:104-106` bind + `info!("daemon binding to http://{}", addr)`；`:128` create_routers；`:130-134` `health_router.merge(protected_router).layer(DefaultBodyLimit::disable()).layer(CorsLayer…)` | fallback 挂在 merge 后的 app 上（设计 §1） |
| token 状态 | `src/daemon/state.rs`：`set_api_token`（mod.rs:115 调用）、`current_api_token()` | bootstrap handler 复用，无新增状态（设计 §2） |
| rust-embed 先例 | `src/knowledge/embedded.rs`、`src/i18n/loader.rs`（rust-embed 8.5，optional dep） | 任务 1.2 沿用 |
| feature 定义 | `Cargo.toml:139` `daemon = ["axum", "tower", "tower-http", "tokio-stream"]`；`default = ["i18n", "daemon", "bundled-skills"]` | 需追加 `rust-embed` |
| 集成测试骨架 | `tests/integration/daemon_harness.rs`（真实 `create_routers` + 临时端口）、`daemon_ws_push.rs`、`src/daemon/handlers.rs:3511+` 单元测试用 create_routers 先例 | 边界测试复用（任务 3.1） |
| `resolveDaemonDirect` | `web/src/api/client.ts:120-130`（`fetch("/__daemon-info")`，成功返回 `{ base: http://127.0.0.1:<port>/api/v1, token }`） | 任务 4.1 扩展 |
| `ensureViewer` | `client.ts:152` 裸 `fetch(\`${this.base}/ui/viewers\`, { method: "POST" })` | protected 路由，hosted 模式必须带头（任务 4.2） |
| 裸 fetch 调用点 | `client.ts` 中 `await fetch(\`${this.base}…\`)` 共 **58 处**（55 单行 + 3 多行：543/558/569） | 任务 4.2 机械替换 |
| 不替换的 fetch | `client.ts:122`（`/__daemon-info`）、`:434`（fetchStream 直连分支，已带 auth）、`:451`（同源回退，定义上无 token） | — |
| `wsChannel` | `web/src/api/wsChannel.ts:216` 用 `direct.base`/`direct.token` 推导 `ws://…?token=` | 经 4.1 自动获益，无需改动 |
| vitest 位置 | 测试与源码同目录（`web/src/**/*.test.ts`），`web/src/api/client.test.ts` 尚不存在 | 任务 4.3 新建 |
| 发布脚本 | `desktop/scripts/bundle.sh`：`[1/4] cargo build --release`（:24）**先于** `[3/4] (cd web && npm run build)`（:31） | **顺序缺陷**：release 构建时嵌入的是空/旧 dist，任务 5.1 必须把 web 构建前置 |
| `.gitignore` | `web/.gitignore:5` 已忽略 `dist/` | 任务 5.1 仅确认 |

---

## Phase 0 — 基线验证（不改代码）

**Step 0.1 确认基线绿色**

```bash
cargo test --features daemon 2>&1 | tail -5
cd web && npm run typecheck && npm test 2>&1 | tail -15
```

**验收**：两者全绿。若基线已红，先停手上报，不得把红基线混入本变更。

---

## Phase 1 — Daemon 静态托管基础（tasks 1.1–1.5）

设计依据：设计文档 §1（模块结构）、§6（构建细节）。

### Task 1.1 新增 `build.rs`（task 1.1）

新建仓库根 `build.rs`（纯 std，不依赖 Node）：

```rust
fn main() {
    let dist = std::path::Path::new("web/dist");
    if !dist.exists() {
        std::fs::create_dir_all(dist).expect("create web/dist");
    }
    // 目录为空时放 .gitkeep 占位，保证 rust-embed 的 folder 属性在 cargo 元数据
    // 校验阶段不因目录缺失而失败；vite build 的 emptyOutDir 会清掉它，属正常
    // （下次 cargo 构建时本脚本仅重建占位，见设计 §6 "vite 交互"）。
    let empty = dist
        .read_dir()
        .map(|mut d| d.next().is_none())
        .unwrap_or(true);
    if empty {
        std::fs::write(dist.join(".gitkeep"), b"").expect("write .gitkeep");
    }
    println!("cargo:rerun-if-changed=web/dist");
}
```

**验收**：

```bash
rm -rf web/dist && cargo check --features daemon && test -f web/dist/.gitkeep && echo OK
touch web/dist/x && cargo check --features daemon && echo RERUN_OK   # 观察构建被触发（重编 web_ui 所在 crate）
```

### Task 1.2 `Cargo.toml` 挂 `rust-embed` 到 daemon feature（task 1.2）

`Cargo.toml:139` 改为：

```toml
daemon = ["axum", "tower", "tower-http", "tokio-stream", "rust-embed"]
```

**验收**：

```bash
cargo check --features daemon 2>&1 | grep -E "error|warning: unused" || echo OK
cargo tree -e features -p wgenty-code --features daemon 2>/dev/null | grep rust-embed | head -1
```

### Task 1.3 新增 `src/daemon/web_ui.rs`：嵌入 + MIME + `GET /`、`GET /assets/*`（task 1.3）

骨架（完整逻辑照设计 §1 组件表）：

```rust
use axum::{http::{header, StatusCode, Uri}, response::{IntoResponse, Response}, routing::get, Router};
use rust_embed::RustEmbed;
use std::sync::Arc;
use crate::daemon::state::DaemonState;

#[derive(RustEmbed)]
#[folder = "web/dist"]
struct WebAssets;

/// 扩展名→MIME（设计 §1）：html/js/mjs/css/svg/png/ico/json/wasm/woff2/map，
/// 其余 `application/octet-stream`。纯函数，供单元测试直接驱动。
fn mime_for(ext: &str) -> &'static str { /* match 表 */ }

/// `GET /`：嵌入有 index.html → 200 + `text/html; charset=utf-8` + `Cache-Control: no-cache`；
/// 无 → 内联降级页（Task 1.4）。
async fn serve_index() -> Response { /* WebAssets::get("index.html") */ }

/// `GET /assets/*path`：路径查嵌入资产（`assets/<path>`），MIME 映射，
/// `Cache-Control: public, max-age=31536000, immutable`；未命中 404。
async fn serve_asset(axum::extract::Path(path): axum::extract::Path<String>) -> Response { /* … */ }

/// 挂入 health（public）路由组（设计 §1）。
pub(crate) fn public_router() -> Router<Arc<DaemonState>> {
    Router::new()
        .route("/", get(serve_index))
        .route("/assets/*path", get(serve_asset))
        // Step 2.1 追加 .route("/auth/bootstrap", get(bootstrap_token))
}
```

注意：
- 静态路由必须进 public 组——页面加载先于任何 token 获取（设计 §1）。
- `/assets/*path` 的通配路由在 axum 0.8 为 `/*path` 语法（与仓库现有 `ws/push` 通配写法核对）。
- `mime_for` 做成纯函数，Phase 2 单测直接调用。

**验收**：

```bash
cargo check --features daemon
```

### Task 1.4 降级路径（task 1.4）

在 Step 1.3 的 `serve_index` 内实现：`WebAssets::get("index.html")` 为 `None` 时返回 Rust 内联最小 HTML（200，非 500），标题 + 一行提示：``Web UI not bundled — run `npm --prefix web run build` ``。降级页不依赖嵌入资产存在（设计 §1）。

**验收**（单测放本文件 `#[cfg(test)]`，Phase 3 前先行覆盖最小集）：

```bash
cargo test --features daemon web_ui:: 2>&1 | tail -3
```

至少含：`mime_for` 全表 + 默认值、降级页 200 且含提示文案。

### Task 1.5 路由接线：public 组 + `/api/` 前缀感知 fallback（task 1.5）

1. `src/daemon/routes.rs::create_routers`：health（public）组 `.merge(web_ui::public_router())`。
2. `src/daemon/web_ui.rs` 新增 fallback 处理器（设计 §1 组件表）：

```rust
/// 挂在 mod.rs merge 后的最终 app（.fallback），不受 protected 组 route_layer
/// 影响——静态深链公开可达，与 §2 "跨源读不到 token" 边界一致。
pub(crate) async fn spa_fallback(uri: Uri, method: axum::http::Method) -> Response {
    // 1. uri.path().starts_with("/api/") → 404 JSON（显式判前缀，未知 API 不吐 HTML）
    // 2. method != GET → 405
    // 3. 其余 GET → serve_index()（SPA 深链兜底；单视图应用，仅兜 / 与未来扩展）
}
```

3. `src/daemon/mod.rs:130` 合并点改为：

```rust
let app = health_router
    .merge(protected_router)
    .fallback(web_ui::spa_fallback)   // ← 新增；设计 §1：挂在 merge 后的最终 app
    .layer(DefaultBodyLimit::disable())
    .layer( /* 现有 CorsLayer 不动 */ );
```

4. `src/daemon/mod.rs` 顶部 `mod web_ui;` 声明（按文件现有 mod 组织核对）。

**验收**：

```bash
cargo check --features daemon
cargo test --features daemon 2>&1 | tail -5   # 既有测试（含 401 回归）不红
```

---

## Phase 2 — 同源 Bootstrap 认证端点（tasks 2.1–2.2）

设计依据：设计文档 §2（谓词三检查、放行/拒绝响应、威胁模型）。

### Task 2.1 `GET /auth/bootstrap` 处理器（task 2.1）

1. **谓词做成纯函数**（便于四方向单测，无需构造 DaemonState）：

```rust
/// 设计 §2：三检查全部通过才放行。
/// 1. Origin 头存在 → 其 host（含端口）必须 ∈ {127.0.0.1:<port>, localhost:<port>}
/// 2. Sec-Fetch-Site 头存在 → 必须 ∈ {same-origin, none}（none 覆盖用户直接打开 URL）
/// 3. Host 头必须 ∈ {127.0.0.1:<port>, localhost:<port>} —— 防 DNS rebinding
fn is_same_origin_request(origin: Option<&str>, sec_fetch_site: Option<&str>,
                          host: Option<&str>, port: u16) -> bool
```

   实现要点：头不存在时该维度放行（1、2 条）；Host 缺失或不匹配即拒。Origin 解析取 `://` 之后的 host:port 部分；`localhost:<port>` 与 `127.0.0.1:<port>` 均接受。

2. **bind port 传递**（设计 §2 "port 取自 listener 绑定地址"）：在 `src/daemon/state.rs::DaemonState` 仿照 `api_token` 的 `RwLock` 模式新增 `bind_port: RwLock<Option<u16>>` + `set_bind_port(u16)` / `current_bind_port() -> Option<u16>`；`mod.rs::run()` 在 bind 成功（`:106`）后调用 `daemon_state.set_bind_port(port)`。不采用改 `create_routers` 签名的方式——那会波及 `daemon_harness.rs` / `daemon_ws_push.rs` / `handlers.rs` 全部测试调用点。

3. **处理器**（`web_ui.rs`，注册进 `public_router()`）：

```rust
async fn bootstrap_token(
    axum::http::HeaderMap headers,
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
) -> Response {
    // 谓词用 headers 中的 Origin / Sec-Fetch-Site / Host + state.current_bind_port()
    // 通过 → 200 JSON {"token": state.current_api_token()} + `Cache-Control: no-store`
    // 拒绝 → 403 JSON {"error": "cross-origin request rejected"}，token 不出（设计 §2、错误处理表）
}
```

   token 读 `DaemonState::current_api_token()`（与 `ws_push` in-handler 认证同源，无新增状态，设计 §2）。

**验收**：

```bash
cargo check --features daemon
```

### Task 2.2 Bootstrap 单元/边界测试（task 2.2）

`src/daemon/web_ui.rs` 的 `#[cfg(test)]` 内，对 `is_same_origin_request` 覆盖设计 §5 表中四方向：

1. **同源放行**：`Origin: http://127.0.0.1:<port>` + `Sec-Fetch-Site: same-origin` + `Host: 127.0.0.1:<port>` → true；`localhost:<port>` 变体亦 true；`Sec-Fetch-Site: none`（用户直接开 URL）true；Origin/Sec-Fetch-Site 缺失放行。
2. **跨 Origin 拒**：`Origin: http://evil.example` → false。
3. **rebinding Host 拒**：`Host: attacker.example`（即使 Origin 缺失）→ false。
4. **`Sec-Fetch-Site: cross-site` 拒** → false。

再加 HTTP 层两测（复用 `create_routers` + `tower::ServiceExt::oneshot` 先例，构造带 api token 的最小 state；若 state 构造成本高，改走 `daemon_harness.rs` 集成测试）：

- 放行路径：响应体含 token 字段、`Cache-Control: no-store`。
- 拒绝路径：403 + `{"error": "cross-origin request rejected"}`、响应体无 token。

**认证回归**（task 2.2 后半）：无 token 调 protected API 仍 401——现有测试**一行不改**，全量跑通即证明（设计 §5"认证回归"行）。

**验收**：

```bash
cargo test --features daemon web_ui 2>&1 | tail -3
cargo test --features daemon 2>&1 | tail -3
```

---

## Phase 3 — Daemon 测试与启动日志（tasks 3.1–3.2）

### Task 3.1 托管边界测试（task 3.1）

设计 §5 "Rust 边界"行要求复用 boundary 测试与生产路由共表先例。落在 `tests/integration/`（复用 `daemon_harness.rs` 启动真实 `create_routers`；先读该骨架再写，命名跟随现有集成测试风格，如 `daemon_web_ui.rs`）：

| 断言 | 期望 |
|---|---|
| `GET /` | 200，`text/html`，`Cache-Control: no-cache`（构建过 dist 时含 index 内容） |
| `GET /assets/<hash>.js`（若 dist 存在） | 200，正确 MIME，`Cache-Control: public, max-age=31536000, immutable` |
| 未知 `/api/v1/nonexistent` | 404，`application/json`，**非 HTML**（fallback 前缀判定） |
| SPA 深链 `GET /some/deep/link` | 200 index HTML（fallback） |
| 非 GET 深链（如 `POST /foo`） | 405 |
| dist 为空（单测内直接构造空 embed 不可行 → 用降级页单测覆盖） | `web_ui.rs` 单测：降级页 200 + 提示文案（Task 1.4 已覆盖，此处引用） |

注意：CI/测试环境若未构建过 web，`GET /` 命中降级页——断言写成"200 + HTML"两形态皆可（`WebAssets::get("index.html")` 有无两种），避免对 dist 状态的硬依赖（与 build.rs 占位配合）。

**验收**：

```bash
cargo test --features daemon --test '*' web 2>&1 | tail -3   # 或按实际测试文件名单跑
```

### Task 3.2 启动日志（task 3.2）

`src/daemon/mod.rs::run()`，在 `info!("daemon binding to http://{}", addr)`（:105）附近追加：

```rust
if web_ui::has_index() {          // web_ui.rs 暴露 pub(crate) fn has_index() -> bool
    info!("Web UI: http://{}", addr);
} else {
    info!("Web UI not bundled (web/dist empty at build time)");   // 文案照设计错误处理表
}
```

**验收**：

```bash
cargo test --features daemon 2>&1 | tail -3
cargo run --features daemon -- daemon --port 8371 & sleep 2; kill %1   # 肉眼确认两形态日志其一出现
```

---

## Phase 4 — Web 客户端适配（tasks 4.1–4.3）

设计依据：设计文档 §3（三个触点）、§4（与 sse-to-websocket 的冲突面说明）。

> ⚠️ 冲突提示（设计 §4）：活跃变更 `sse-to-websocket` 同改 `web/src/api/client.ts`（对方动 `fetchStream`/`wsChannel`，本方动 `resolveDaemonDirect` 与 fetch 调用点）。合并时留意小范围文本冲突；本计划不触碰 `fetchStream` 内部逻辑（`:434/:451` 两处不改）。

### Task 4.1 `resolveDaemonDirect` 回退链扩展（task 4.1）

`web/src/api/client.ts:120-130`，现有 `/__daemon-info` 失败路径（`!res.ok` / 字段缺失 / catch）在**非 vite dev 环境**追加同源 bootstrap 尝试：

```ts
export async function resolveDaemonDirect(): Promise<DaemonDirectInfo | null> {
  try {
    const res = await fetch("/__daemon-info");
    if (res.ok) {
      const info = (await res.json()) as { port?: number; token?: string };
      if (typeof info.port === "number" && info.token) {
        return { base: `http://127.0.0.1:${info.port}/api/v1`, token: info.token };
      }
    }
  } catch { /* fall through */ }
  // __daemon-info 不可用（hosted 模式无 vite 中间件）。dev 下不尝试：vite dev server
  // 会对未知路径回退返回 index.html（appType: "spa"），徒增一次无效往返。
  if (import.meta.env.DEV) return null;
  try {
    const res = await fetch("/auth/bootstrap");
    if (!res.ok) return null;                       // 旧 daemon：404 → null → 现行为
    const info = (await res.json()) as { token?: string };
    if (!info.token) return null;
    // 绝对地址保证 wsChannel 的 ws:// 推导成立（设计 §3）
    return { base: location.origin + "/api/v1", token: info.token };
  } catch {
    return null;                                    // 客户端 bootstrap 失败 → 回退现行为（错误处理表）
  }
}
```

语义要求（设计 §3.1 + §3.3）：`fetchStream`（client.ts:431 直连分支）与 `wsChannel`（wsChannel.ts:21/216）**零改动**自动获益——hosted 模式下 direct.base 即同源绝对地址、直连分支已注入 auth 头；每次调用新解，daemon 重启换 token 后无需刷新页面。

**验收**：

```bash
cd web && npm run typecheck
```

### Task 4.2 `DaemonClient.authedFetch` 包装 + 全量替换（task 4.2）

`client.ts` 的 `DaemonClient` 内新增私有包装（token 按调用时解析，沿用现有"每次新解"语义，设计 §3.2/§3.3）：

```ts
/** token 可用（dev 的 __daemon-info 或 hosted 的 bootstrap）时统一注入
 *  Authorization 头。dev 下 vite 代理本身会注入同值 token，重复无害。 */
private async authedFetch(url: string, init?: RequestInit): Promise<Response> {
  const direct = await resolveDaemonDirect();
  if (!direct) return fetch(url, init);
  const headers = new Headers(init?.headers);
  headers.set("Authorization", `Bearer ${direct.token}`);
  return fetch(url, { ...init, headers });
}
```

机械替换 `DaemonClient` 内全部 `${this.base}` 裸 fetch（**58 处**：55 单行 + 3 多行 `client.ts:543/558/569`）：

```bash
cd web && grep -n 'await fetch(`${this.base}' src/api/client.ts | wc -l   # 替换后应为 0
grep -n 'fetch(' src/api/client.ts | grep -v authedFetch | grep -vE ':(122|434|451)\b'   # 仅剩白名单三处
```

**必须包含** `ensureViewer`（`client.ts:152`，`POST /ui/viewers` 是 protected 路由，hosted 模式需要头——设计 §3.2）。**不替换**：`:122`（`/__daemon-info` 自身）、`:434`（fetchStream 直连，已带 auth）、`:451`（同源回退，resolveDaemonDirect 为 null 时才走到，无 token 可注入）。`agentHeaders()` 等既有 header 组装路径与新 header 经 `new Headers(init.headers)` 合并，不冲突。

**验收**：

```bash
cd web && npm run typecheck && npm test 2>&1 | tail -5   # vitest 全量回归（设计 §3.2 "机械替换，全量回归"）
```

### Task 4.3 vitest（task 4.3）

新建 `web/src/api/client.test.ts`（跟随仓库 vitest 同目录约定），用 `vi.stubGlobal("fetch", …)` 依次打桩，覆盖设计 §5 vitest 行三组：

1. **回退链**：`/__daemon-info` 404 → `/auth/bootstrap` 200 `{token}` → 返回 `{ base: location.origin + "/api/v1", token }`；`/__daemon-info` 404 → bootstrap 404 → `null`；bootstrap 网络异常 → `null`。
2. **头注入**：bootstrap 成功态下调用任一 API 方法（如 `health()`）→ 捕获 fetch 参数断言含 `Authorization: Bearer <token>`；`ensureViewer()` 的 POST 同样带头。
3. **零行为变化**：`/__daemon-info` 200 且带 port/token（dev 形态）→ API 调用不再请求 bootstrap、头取自 daemon-info；两端点都 404（旧 daemon）→ 不注入 Authorization、不抛错（现行为）。

注意 `import.meta.env.DEV` 分支：hosted 回退链用例需以 dev=false 假设编写（如 `vi.mock` 或直接测函数在 `import.meta.env.DEV === false` 下的行为；实现时可把 dev 判定提为可注入参数/模块级常量以便测试，不改运行语义）。

**验收**：

```bash
cd web && npm test 2>&1 | tail -5
```

---

## Phase 5 — 构建流水线与文档（tasks 5.1–5.2）

### Task 5.1 发布脚本与 `.gitignore`（task 5.1）

**`desktop/scripts/bundle.sh` 修正顺序缺陷**（见锚点表）：release 构建的 rust-embed 在**编译时**嵌入 `web/dist`，而当前 `[1/4] cargo build --release`（:24）先于 `[3/4] web build`（:31），嵌入的必然是空/旧 dist。重排为：

```bash
echo ">> [1/4] Building web frontend..."
npm --prefix web run build          # 必须先于 cargo release build（设计 §6 "发布脚本"）

echo ">> [2/4] Building daemon (release, $TRIPLE)..."
cargo build --release
# 后续 staging / tauri build 顺延为 [3/4] [4/4]，逻辑不变
```

**`.gitignore` 确认**：`web/.gitignore:5` 已含 `dist/`，无需改动；确认 `web/dist/.gitkeep`（build.rs 生成）不被额外忽略即可。

**验收**：

```bash
bash -n desktop/scripts/bundle.sh && echo SYNTAX_OK
git check-ignore -v web/dist/ && echo IGNORED_OK
```

### Task 5.2 文档更新（task 5.2）

- `web/README.md`：新增 "Daemon-hosted" 运行节——`npm --prefix web run build` → `cargo run --features daemon -- daemon --port 8371` → 打开 `http://127.0.0.1:8371`；说明认证自动经同源 bootstrap、无需手动 token；保留现有 dev（`npm run dev`）节为默认开发路径。
- 根 `README.md`：仅当其提及 daemon 使用方式时补一行 hosted 入口（先 grep 确认是否涉及，不涉及则跳过并在此记录）。

**验收**：

```bash
grep -n "daemon" web/README.md | head   # 新节存在
```

---

## Phase 6 — 端到端验证（tasks 6.1–6.3，手动）

### Task 6.1 Hosted 正常流程（task 6.1）

```bash
npm --prefix web run build
cargo run --features daemon -- daemon --port 8371
# 浏览器打开 http://127.0.0.1:8371
```

**清单**：页面加载（`GET /` 200 + `/assets/*` immutable）；Network 面板见 `GET /auth/bootstrap` 200 且后续 API 调用带 `Authorization: Bearer`；完成一次流式对话；权限弹窗出现并可裁决；会话列表可切换。

### Task 6.2 降级启动（task 6.2）

```bash
rm -rf web/dist && cargo run --features daemon -- daemon --port 8372
```

**清单**：daemon 正常起（不 panic）；日志出现 `Web UI not bundled (web/dist empty at build time)`；`GET /` 返回 200 降级提示页；API（curl 带 token 调 health/heartbeat）可用。注意 debug 构建下 rust-embed 运行时读磁盘——本步验证需在 build.rs 占位重建后进行，若要严格验证"编译期空嵌入"用 release 构建。

### Task 6.3 Dev 回归（task 6.3）

```bash
cd web && npm run dev   # 打开 vite 提示的 5173 地址
```

**清单**：`/__daemon-info` 正常生效（hosted 回退不触发）；API 经 vite 代理注入 token；流式对话/WS 推送与变更前一致；控制台无新增报错。

---

## 最终全量验证

```bash
# Rust 侧（daemon feature 相关全量，含既有 401 回归 = 零破坏证明）
cargo test --features daemon 2>&1 | tail -5

# Web 侧
cd web && npm run typecheck && npm test 2>&1 | tail -5

# （可选，发布形态冒烟）release 构建验证真嵌入
npm --prefix web run build && cargo build --release
```

## 任务 → 计划步骤映射（16/16 覆盖）

| tasks.md | 计划步骤 |
|---|---|
| 1.1 build.rs | Task 1.1 |
| 1.2 Cargo.toml feature | Task 1.2 |
| 1.3 web_ui.rs 嵌入/MIME/路由 | Task 1.3 |
| 1.4 降级页 | Task 1.4 |
| 1.5 路由接线 + fallback | Task 1.5 |
| 2.1 bootstrap 端点 | Task 2.1 |
| 2.2 bootstrap 测试 + 401 回归 | Task 2.2 |
| 3.1 边界测试 | Task 3.1 |
| 3.2 启动日志 | Task 3.2 |
| 4.1 resolveDaemonDirect 扩展 | Task 4.1 |
| 4.2 authedFetch + 调用点替换 | Task 4.2 |
| 4.3 vitest | Task 4.3 |
| 5.1 发布脚本 + .gitignore | Task 5.1 |
| 5.2 README | Task 5.2 |
| 6.1 hosted E2E | Task 6.1 |
| 6.2 / 6.3 降级与 dev 回归 | Task 6.2 / 6.3 |

## 实施顺序与回滚

严格按 Phase 0 → 6 顺序执行（Phase 1 内 Step 1.1→1.5 有序；Phase 2 依赖 1.5 的路由骨架；Phase 4 依赖 Phase 2 的端点存在才有可测行为；Phase 5.1 的脚本重排在 Phase 1 验证嵌入语义后进行）。每个 Phase 完成即跑该 Phase 验收命令 + `git diff` 自查，绿了再进下一 Phase；任何一步验收失败，回滚该步改动后重做，不携带半成品前进。
