# Design: gui-desktop-foundation

> 高层架构决策文档。深度技术设计（框架最终选型、组件树、状态模型细化）在 /comet-design 阶段的 Design Doc 中完成。

## Context

路线 B 已定且主体已落地：合并 feature/web-ui-redesign 后，agent loop 在 daemon 内运行（`run_session_turn` + `RunRegistry`），UI→daemon 为 HTTP 命令（`POST /sessions/:id/run`、`cancel`、审批应答），daemon→UI 为会话事件流（`SessionEventHub`，SSE，per-session seq + fan-out）。`web/`（React + Vite + zustand）已是该模型的参照客户端。daemon-session-orchestration 将补齐重放/续传/失步信号等可靠性缺口。GUI 是纯视图：不跑 agent loop，不持有会话真相，本地状态只是事件流的投影。

约束：
- 遵循 AGENTS.md：性能约束针对默认构建（桌面壳为独立 crate，不在主 workspace，默认 `cargo build` 不编译 Tauri）
- 跨平台：linux/macos/windows 均可编译运行（Tauri 依赖系统 webview）
- 复用 `web/` 前端：Tauri webview 装入同一 React 应用，桌面端与浏览器版共享组件层（通过 platform/ Adapter 隔离平台差异）

## Goals / Non-Goals

**Goals:**
- GUI 桌面应用可启动、可发现/连接 daemon（常驻实例优先，内嵌拉起兜底）
- 完成一轮完整对话：命令发起 turn、订阅事件流渲染流式输出、markdown、工具调用展示、权限审批
- 与 TUI 同时观看/操作同一会话（多屏同步由 daemon 编排层保证）
- 会话编排客户端抽象为 UI 无关模块（`src/client/`）

**Non-Goals:**
- 会话管理、配置/模型界面、高级面板（后续 3 个 change）
- ~~GUI 框架最终选型~~ → 已解决：Tauri 2.0（spike 验证，见 Decision 3）
- daemon 服务端实现（daemon-session-orchestration 的职责）
- 移动端、打包分发渠道（签名/商店）

## Decisions

1. **GUI 为纯视图，不跑 agent loop**
   对话通过命令端点发起，渲染完全由订阅会话事件流驱动；本地不维护消息历史真相，断线后按 seq 续传、失步则从会话存储全量恢复。
   备选（GUI 进程内跑 loop、走旧的无会话端点）被否决——违背路线 B 的多屏同步目标。

2. **会话编排客户端抽象为 UI 无关模块**
   新建 `src/client/`：命令通道（HTTP）+ 事件通道（SSE 订阅、seq 跟踪、自动重连续传）。不抽离 `src/tui/client.rs`（那是旧模型的客户端，TUI 迁移时再处理）；新模块面向新编排 API 设计。
   备选（在 GUI 模块内直接写 HTTP 调用）被否决——协议逻辑应与 UI 解耦，供后续 Web 客户端参照。

3. **Tauri 2.0 桌面壳 + 复用 web/ 前端，独立 crate 隔离**
   ~~原决策（纯 Rust GUI，排除 Tauri/Webview）已推翻。~~ 推翻原因：spike（`desktop/`）验证了 Tauri 2.0 能直接装入现有 `web/` React 前端，token 注入、CORS、流式渲染全部通过，且为零重写 + 移动端路径（Tauri 2.0 正式支持 iOS/Android）。纯 Rust GUI（egui/iced）需用另一套技术重写全部 UI（markdown 渲染、代码高亮、面板、权限审批交互），且移动端支持不成熟。Tauri 壳代码（`desktop/src-tauri/`）为独立 crate，不在主 `wgenty_code` workspace 中，默认 `cargo build` 零影响。spike 度量：.app 10MB，冷启动 ~0.43s，详见 `desktop/README.md`。

4. **连接策略：发现常驻实例优先，内嵌拉起兜底**
   启动时先按 daemon-session-orchestration 的发现机制连接已驻留 daemon（与 TUI 共享实例、共享会话）；校验失败则进程内拉起（保留现有内嵌模式体验）。

## Risks / Trade-offs

- [系统 webview 版本差异导致渲染/行为不一致] → spike 已在 macOS WKWebView 验证通过；正式实现需覆盖 Windows WebView2 + Linux WebKitGTK 验收；必要时降级为 polyfill
- [Tauri webview 的 SSE（fetch + ReadableStream）兼容性] → spike 已验证流式对话正常；webview 标准兼容性依赖系统 webview 版本，最低要求记录在 README
- [依赖的前置 change（daemon-session-orchestration）延期] → 任务编排上严格后置；客户端模块可先针对契约 mock 开发
- [GUI 依赖显著增大 release 构建时间与二进制体积] → feature flag 隔离，默认构建不含 GUI；GUI 构建单独验证性能约束

## Open Questions

- ~~egui vs iced 最终选型~~ → 已解决：spike 确定使用 Tauri 2.0（见 Decision 3）
- ~~事件→状态投影在 GUI 侧的具体范式~~ → 已解决：复用 web/ 的 zustand store，Tauri 仅是 webview 壳
- GUI 入口形态：`wgenty-code gui` 子命令 vs 独立二进制 `wgenty-code-desktop`（spike 用独立 crate/独立 bin，待正式确认）
