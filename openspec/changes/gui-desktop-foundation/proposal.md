# Proposal: gui-desktop-foundation

> Comet 批量拆分项 2/5（batch: `.comet/batches/gui-desktop.json`）。总目标：新增纯 Rust GUI 桌面端，功能全面对齐 TUI，交互与布局优于 TUI（参考 orca / paseo）。本 change 只负责应用骨架与核心对话。**依赖 daemon-session-orchestration（路线 B：胖 daemon）**。

## Why

项目现有 TUI（ratatui）与 headless 两种交互形态，daemon-session-orchestration 将把 agent loop 与事件分发上收到 daemon，使 UI 客户端成为「命令发送器 + 事件流投影」的纯视图。终端界面在信息密度、多面板布局、富文本渲染上有天然上限；GUI 桌面端作为第一个基于新编排模型的纯视图客户端落地，可同时与 TUI/Web 观看并操作同一会话。

## What Changes

- 新增 GUI 桌面应用二进制入口（纯 Rust GUI 框架，具体框架选型在 design 阶段确定，候选 egui / iced）
- 新增 GUI 应用骨架：主窗口、导航/多面板布局框架、应用状态管理（状态为 daemon 事件流的本地投影）
- 新增 daemon 会话编排客户端：命令通道（HTTP：发起 turn / 中断 / 审批应答）+ 事件通道（SSE 订阅会话事件流，带 seq 断线续传与失步回退）；抽象为 UI 无关模块，供后续 Web 前端复用同协议
- 新增核心对话界面：流式输出渲染、markdown 渲染、工具调用过程展示、权限审批交互（审批经事件流到达、命令通道应答）
- 交互与布局设计参考 orca / paseo，目标是优于 TUI 的桌面体验，而非 TUI 的简单移植

## Capabilities

### New Capabilities

- `gui-app-shell`: GUI 桌面应用骨架——窗口生命周期、导航与多面板布局、daemon 连接管理（发现常驻实例优先、内嵌拉起兜底）、UI 无关的会话编排客户端、应用级状态与错误兜底
- `gui-chat`: GUI 核心对话界面——订阅会话事件流的流式渲染、markdown 显示、工具调用/结果展示、权限审批交互、输入区

### Modified Capabilities

（无——daemon 服务端能力由 daemon-session-orchestration 提供，本 change 不改服务端）

## Impact

- **新增代码**：`src/gui/`（应用骨架与界面）、`src/client/`（UI 无关会话编排客户端）；GUI 入口
- **依赖**：daemon-session-orchestration（前置 change）；新增 GUI 框架依赖（egui 或 iced，design 阶段定），通过 Cargo feature flag（如 `gui`）控制编译
- **不触碰**：`src/agent/`、`src/tools/`、`src/context/`、daemon 服务端、TUI（TUI 迁移到新模型另立 change）
- **后续 change**（不在本 change 内）：gui-session-management、gui-config-and-models、gui-advanced-panels
- **明确独立**：与 web-ops-console（deferred）无关，不为其预留前端复用
