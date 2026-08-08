# Proposal: gui-desktop-foundation

> Comet 批量拆分项 2/5（batch: `.comet/batches/gui-desktop.json`）。总目标：新增 Tauri 2.0 桌面端（复用 `web/` React 前端），功能全面对齐 TUI，交互与布局优于 TUI（参考 orca / paseo）。本 change 只负责应用骨架与核心对话。**依赖 daemon-session-orchestration（路线 B：胖 daemon）**。

## Why

项目现有 TUI（ratatui）、Web（`web/`，React + Vite）与 headless 三种交互形态；合并 feature/web-ui-redesign 后 agent loop 已在 daemon 内运行（server-side loop + SessionEventHub），UI 客户端已成为「命令发送器 + 事件流投影」的纯视图。终端界面在信息密度、多面板布局、富文本渲染上有天然上限；GUI 桌面端作为第三个纯视图客户端落地，与 TUI/Web 共享同一会话真相。daemon-session-orchestration 负责补齐事件流可靠性缺口（重放/续传/失步信号），本 change 在其上构建。

## What Changes

- 新增 GUI 桌面应用二进制入口（Tauri 2.0 桌面壳，复用 `web/` React 前端；spike 已验证技术可行性，见 `desktop/README.md`）
- 新增 GUI 应用骨架：Tauri 主窗口、复用 web/ 的导航/多面板布局、应用状态管理（状态为 daemon 事件流的本地投影，复用 web/ 的 zustand store）
- 新增 daemon 会话编排客户端：命令通道（HTTP：发起 turn / 中断 / 审批应答）+ 事件通道（SSE 订阅会话事件流，带 seq 断线续传与失步回退）；复用 web/ 的 `DaemonClient`（webview 内直接可用，token 由 Tauri 宿主注入）
- 新增核心对话界面：流式输出渲染、markdown 渲染、工具调用过程展示、权限审批交互（审批经事件流到达、命令通道应答）
- 交互与布局设计参考 orca / paseo，目标是优于 TUI 的桌面体验，而非 TUI 的简单移植

## Capabilities

### New Capabilities

- `gui-app-shell`: GUI 桌面应用骨架——窗口生命周期、导航与多面板布局、daemon 连接管理（发现常驻实例优先、内嵌拉起兜底）、UI 无关的会话编排客户端、应用级状态与错误兜底
- `gui-chat`: GUI 核心对话界面——订阅会话事件流的流式渲染、markdown 显示、工具调用/结果展示、权限审批交互、输入区

### Modified Capabilities

（无——daemon 服务端能力由 daemon-session-orchestration 提供，本 change 不改服务端）

## Impact

- **新增代码**：`desktop/`（Tauri 壳：`src-tauri/` Rust 后端 + `src/` 注入脚本）；复用 `web/` 前端（webview 装入同一 React 应用，零重写）
- **依赖**：daemon-session-orchestration（前置 change）；新增 Tauri 2.0 + tauri-plugin-localhost 依赖（独立 crate，不在主 workspace）
- **不触碰**：`src/agent/`、`src/tools/`、`src/context/`、daemon 服务端、TUI（TUI 迁移到新模型另立 change）；`web/` 源码最小改动（仅加 platform/ Adapter 抽象层，不改现有组件）
- **后续 change**（不在本 change 内）：gui-session-management、gui-config-and-models、gui-advanced-panels
- **前端复用**：Tauri webview 装入 `web/` React 应用，与浏览器版共享全部组件层；通过 platform/ Adapter 抽象隔离浏览器与桌面特有能力
