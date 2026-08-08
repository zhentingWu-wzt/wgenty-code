# Design Doc: GUI Desktop Foundation (Tauri 2.0)

> 技术设计文档 for `gui-desktop-foundation` change。
> 高层架构决策见 `openspec/changes/gui-desktop-foundation/design.md`。

## 1. 概述

wgenty-code 桌面端基于 **Tauri 2.0**，复用现有 `web/` React 前端，作为继 TUI 和 Web 之后的第三个纯视图客户端。桌面端不跑 agent loop、不持有会话真相——本地状态仅为 daemon 事件流的投影，与 TUI/Web 共享同一 daemon 会话，支持多屏同步。

### 选型决策（推翻原先"纯 Rust GUI"）

| 维度 | 纯 Rust (egui/iced) | Tauri 2.0（选定） |
|---|---|---|
| 复用 web/ 代码 | ❌ 从零重写 | ✅ 直接装入 webview |
| 移动端 | ⚠️ 实验性 | ✅ Tauri 2.0 正式支持 |
| code rendering | 😐 手搓 | ✅ react-markdown + shiki |
| 二进制大小 | ~15-30MB | 10MB (.app) |
| 冷启动 | 未知 | 0.43s（实测） |

Spike（`desktop/README.md`）验证了 Tauri 方案的全部技术风险点：token 注入、CORS、流式 SSE、daemon 自动拉起。

## 2. 架构

```
┌─────────────────────────────────────────┐
│  web/ (React + TS)         ← 完全复用    │
│  ─────────────────────────────────────  │
│  platform/ Adapter 层（浏览器/Tauri 隔离）│
│  ─────────────────────────────────────  │
│  Tauri webview (WKWebView/WebView2)     │
│  + token-injection.js (fetch patch)     │
│  + platform-tauri.js (能力注入)          │
│  ─────────────────────────────────────  │
│  Tauri Rust 后端 (desktop/src-tauri/)   │
│    · daemon_manager.rs (发现+拉起)       │
│    · read_daemon_token command          │
│    · ensure_daemon command              │
│  ─────────────────────────────────────  │
│  daemon (HTTP + SSE, 127.0.0.1:8371)    │
└─────────────────────────────────────────┘
```

### 2.1 隔离设计

桌面壳 `desktop/src-tauri/` 是**独立 crate**（不在主 `wgenty_code` workspace 中）。默认 `cargo build` / `cargo test` / `cargo clippy` 完全不触碰 Tauri。

主 crate `wgenty_code` 的 daemon 二进制作为**独立子进程**被 spawn（不作为库依赖引入），保持壳的编译图轻量。

### 2.2 Token 注入

web/ 的 `DaemonClient` 发 `fetch("/api/v1/...")` 相对路径请求。Token 注入有两条路径（不冲突）：

- **浏览器**：Vite dev proxy 在服务端注入 `Authorization` header（浏览器永不接触 token）
- **Tauri**：`token-injection.js` 作为 initialization script，在 React 启动前 monkey-patch `window.fetch`，对 `/api/*` 请求注入 header。Token 由 Rust 宿主从 `~/.wgenty-code/daemon.token` 读取并 embed

安全等价性：两者都是"宿主进程持 token，只作用于 loopback `/api/*`"。

### 2.3 CORS

daemon 白名单写死了 `localhost:3000/5173`。生产 Tauri webview 的默认 origin（`tauri://localhost` / `https://tauri.localhost`）不在白名单。

解决：`tauri-plugin-localhost` 让生产环境也通过 `http://localhost:5173` 提供前端资源，origin 落入白名单。

### 2.4 Platform Adapter 层

`web/src/platform/` 提供薄抽象，隔离浏览器与 Tauri 的平台差异：

```typescript
interface PlatformCapability {
  name: 'browser' | 'desktop';
  ensureDaemon?(): Promise<void>;      // 浏览器 no-op，Tauri spawn daemon
  onBeforeClose?(handler): () => void; // beforeunload vs Tauri 窗口事件
  pickDirectory?(): Promise<string|null>; // input[webkitdirectory] vs Tauri dialog
}
```

`getPlatform()` 检测 `window.__wgentyPlatform`（Tauri init script 注入），无则回退浏览器实现。**App 代码零 `if (isTauri)`**。

### 2.5 Daemon 自动拉起

`daemon_manager.rs::ensure_daemon()` 决策链：

1. **discovery**：读 `~/.wgenty-code/daemon.json` + 校验 token 匹配 + 心跳新鲜（120s 内）→ 找到则直接 attach
2. **spawn**：discovery 失败则 spawn `wgenty-code daemon --port 8371` 作为独立进程
3. **health poll**：TCP 连接检查直到端口就绪
4. **token read**：重读 `daemon.token`（spawn 的 daemon 会写入）

daemon 是 **detached** 的——Tauri 退出时不杀它，留给其他 UI 复用。

**StrictMode 防护**：React StrictMode 在 dev 模式双调用 effects。`ensure_daemon` command 用 `tokio::sync::OnceCell` 保证只执行一次。

## 3. 关键修复

### 3.1 Daemon token 写入竞态

**根因**：`daemon::run()` 原先在 `TcpListener::bind` **之前**写 token/discovery 文件。端口被占用时 bind 失败退出，但新 token 已覆盖在用 daemon 的 token → 全端 401/500。

**修复**：把 `bind` 移到 `write_daemon_token` 之前（与 TUI `start_daemon` 顺序对齐）。bind 失败时直接返回，不碰任何文件。

此修复同时解决了 release build 不写 `daemon.json` 的问题（discovery writer 的首次写入不再被进程退出打断）。

## 4. 度量数据

| 指标 | 数值 |
|---|---|
| .app bundle | 10 MB |
| .dmg | 3.4 MB |
| 冷启动 | ~0.43s |
| 运行内存 (RSS) | ~95 MB |
| 默认构建影响 | 零（独立 crate） |

## 5. 后续 change

- **gui-session-management**：会话列表、搜索、checkpoint/undo-turn
- **gui-config-and-models**：模型切换、配置界面、MCP/skills/memory
- **gui-advanced-panels**：subagent 进度树、todos 面板、透视面板

三个 change 均复用 web/ 已有的 React 组件，通过 platform/ Adapter 桌面端直接渲染。
