# Tauri 桌面端 Spike

> 验证 Tauri 2.0 能否复用现有 `web/` React 前端构建 wgenty-code 桌面端。
> 本目录是 **spike 产物**（概念验证），非正式实现。正式实现需先调整
> `openspec/changes/gui-desktop-foundation` 中的技术选型文档。

## 结论：方案可行 ✅

Tauri 2.0 能装入 `web/` 前端、token 注入正常、UI 正常渲染。所有技术风险点已验证通过。

### 验证清单

| 验证项 | 状态 | 说明 |
|--------|------|------|
| Rust 编译 | ✅ | dev 22.8s / release 50.9s |
| web/ 前端装入 webview | ✅ | React UI 正常渲染 |
| token 注入（dev） | ✅ | fetch monkey-patch 注入 `Authorization` header，daemon 鉴权通过 |
| token 注入（prod） | ✅ | 同一机制，`include_str!` embed 脚本 |
| CORS（dev） | ✅ | webview origin = `http://localhost:5173`，在 daemon 白名单内 |
| CORS（prod） | ✅ | `tauri-plugin-localhost` 使 origin 保持 `http://localhost:5173` |
| 连接状态 | ✅ | StatusBar 显示 connected |

### 度量数据

| 指标 | 数值 | 说明 |
|------|------|------|
| .app bundle | 10 MB | macOS .app（含二进制 + 资源） |
| .dmg | 3.4 MB | 压缩分发包 |
| raw binary | 10 MB | 剥离后 Rust 二进制 |
| 冷启动（3 次平均） | ~0.43s | 进程启动到窗口可见 |
| 运行内存 (RSS) | ~95 MB | 含 webview 运行时 |
| web/ dist 大小 | ~1.3 MB | 前端构建产物（gzip 前） |

对比：纯 Rust GUI（egui/iced）预估二进制 15-40MB，但需要重写全部 UI。Tauri 复用 web/ 的代价是 10MB + 依赖系统 webview，换来的是 **零重写 + 移动端路径**。

---

## 架构设计

### 整体拓扑

```
┌─────────────────────────────────────┐
│  web/ (React + TS)     ← 完全复用    │
│  ─────────────────────────────────  │
│  Tauri webview (WKWebView/WebView2) │
│  + initialization_script (注入层)    │
│  ─────────────────────────────────  │
│  Tauri Rust 后端 (本目录)            │
│    · read_daemon_token command      │
│    · tauri-plugin-localhost         │
│  ─────────────────────────────────  │
│  daemon (HTTP + SSE, 127.0.0.1)     │
└─────────────────────────────────────┘
```

### Token 注入：Adapter 层模式

核心问题：`web/` 的 `DaemonClient` 发 `fetch("/api/v1/...")` 相对路径请求，依赖 Vite dev proxy 注入 `Authorization` header。Tauri 没有 proxy。

解决方案：**webview initialization script 在 React 启动前 monkey-patch `window.fetch`**，对所有 `/api/*` 请求注入 `Authorization: Bearer <token>`。

- **web/ 源码零改动**：`DaemonClient` 的 `fetch("/api/v1/...")` 照常工作
- **token 不暴露给 JS 源码**：由 Rust 宿主进程读 `~/.wgenty-code/daemon.token`，编译时 embed 为字面量
- **401 自动刷新**：daemon 重启换 token 后，fetch wrapper 通过 IPC command `read_daemon_token` 刷新并重试一次
- 安全等价性：与 Vite proxy 模型一致（宿主进程持 token，只作用于 loopback `/api/*`）

代码组织：
- `desktop/src/token-injection.js` — 注入逻辑（独立文件，有语法高亮/可维护）
- `desktop/src-tauri/src/lib.rs` — Rust 侧：`include_str!` embed + 占位符替换 + IPC command

### CORS 解决方案

daemon 的 CORS 白名单写死了 `localhost:3000/5173`（`src/daemon/mod.rs:110-122`）。

- **dev 模式**：Tauri 加载 Vite dev server（`http://localhost:5173`），origin 已在白名单
- **生产模式**：默认 origin 是 `tauri://localhost`（macOS）/ `https://tauri.localhost`（Windows），不在白名单
- **解决**：`tauri-plugin-localhost` 让生产环境也通过 `http://localhost:5173` 提供前端资源，origin 落入白名单

---

## 目录结构

```
desktop/                          ← 与 web/ 平行，完全隔离
├── README.md                     ← 本文件（spike report）
├── src/
│   └── token-injection.js        ← fetch 注入逻辑（被 Rust include_str!）
└── src-tauri/
    ├── Cargo.toml                ← 独立 crate（不在主 workspace）
    ├── build.rs                  ← tauri-build codegen
    ├── tauri.conf.json           ← Tauri 配置（devUrl → Vite，frontendDist → web/dist）
    ├── capabilities/
    │   └── default.json          ← Tauri 2 权限声明
    ├── icons/
    │   └── icon.png              ← 占位图标（spike，正式版需设计）
    └── src/
        ├── main.rs               ← 二进制入口
        └── lib.rs                ← Tauri app + token command + 注入脚本组装
```

### 与主项目的关系

- **独立 crate**：`desktop/src-tauri/Cargo.toml` 不在主 `wgenty_code` 的 workspace 中（主项目是单 crate，无 workspace 段）。`cargo build` / `cargo test` / `cargo clippy` 在仓库根执行时**完全不触碰** Tauri。
- **web/ 不改动**：spike 验证阶段零改动 web/ 源码。正式实现时，Adapter 层的 `platform/` 抽象会作为公共代码加入 web/（浏览器和 Tauri 共享接口，非分叉）。

---

## 如何运行

### 前置条件

- Rust toolchain（stable）
- Node.js + npm（web/ 前端构建）
- `cargo install tauri-cli --version "^2.0"`
- macOS: Xcode Command Line Tools（提供 WKWebView）

### Dev 模式（需要三个进程）

```bash
# 1. 启动 daemon
cargo run -- daemon --port 8371

# 2. 启动 web/ Vite dev server（另一个终端）
cd web && npm run dev

# 3. 启动 Tauri（第三个终端）
cd desktop/src-tauri && cargo tauri dev
```

Dev 模式下 Tauri webview 加载 Vite dev server（HMR 热更新可用）。

### 生产模式（单进程）

```bash
# 1. 构建 web/ dist
cd web && npm run build

# 2. 构建 Tauri app
cd desktop/src-tauri && cargo tauri build

# 3. 产物
ls target/release/bundle/macos/wgenty-code.app
```

生产模式不需要 Vite，`tauri-plugin-localhost` 在 `http://localhost:5173` 提供前端资源。

---

## 已知限制（spike 边界）

- ❌ 未验证移动端（iOS/Android）—— Tauri 2.0 支持但需 Xcode/Android Studio
- ❌ 未实现自动拉起 daemon（当前需手动启动 daemon）
- ❌ 未实现原生菜单 / 系统托盘 / 原生文件对话框
- ❌ 图标是占位 PNG（正式版需设计）
- ❌ 未签名 / 未公证（macOS Gatekeeper 会拦截分发）

---

## 下一步建议

1. **正式调整 openspec 文档**：推翻 `gui-desktop-foundation/design.md` 中"排除 Tauri"的决策；更新 5 份 GUI change 的 Impact 路径（`src/gui/` → `desktop/`）；删除 `gui-config-and-models/design.md` 中"不复用 web 代码"的语句
2. **建立 Adapter 层**：在 web/ 加薄 `platform/` 抽象（`types.ts` + `browser.ts`），desktop/ 提供 `tauri.ts` 实现。防止长期 `if (isTauri)` 分叉
3. **验证移动端**：用 Tauri 2.0 的 iOS/Android target 做一轮最小验证
4. **正式实现 gui-desktop-foundation**：基于本 spike 的架构，按调整后的 tasks.md 推进
