# Design: gui-desktop-foundation

> 高层架构决策文档。深度技术设计（框架最终选型、组件树、状态模型细化）在 /comet-design 阶段的 Design Doc 中完成。

## Context

路线 B 已定：daemon-session-orchestration 把 agent loop、turn 编排与事件分发上收到 daemon——UI→daemon 为 HTTP 命令（发起 turn / 中断 / 审批应答），daemon→UI 为会话事件流（SSE，序号 + 重放 + 多订阅者）。GUI 是纯视图：不跑 agent loop，不持有会话真相，本地状态只是事件流的投影。

约束：
- 遵循 AGENTS.md：新功能用 feature flag 控制编译；性能约束针对默认构建（GUI 经 feature 隔离）
- 跨平台：linux/macos/windows 均可编译运行
- 与 web-ops-console 完全独立，不预留浏览器前端复用

## Goals / Non-Goals

**Goals:**
- GUI 桌面应用可启动、可发现/连接 daemon（常驻实例优先，内嵌拉起兜底）
- 完成一轮完整对话：命令发起 turn、订阅事件流渲染流式输出、markdown、工具调用展示、权限审批
- 与 TUI 同时观看/操作同一会话（多屏同步由 daemon 编排层保证）
- 会话编排客户端抽象为 UI 无关模块（`src/client/`）

**Non-Goals:**
- 会话管理、配置/模型界面、高级面板（后续 3 个 change）
- GUI 框架最终选型（design 阶段 brainstorming 决定：egui vs iced）
- daemon 服务端实现（daemon-session-orchestration 的职责）
- 移动端、打包分发渠道（签名/商店）

## Decisions

1. **GUI 为纯视图，不跑 agent loop**
   对话通过命令端点发起，渲染完全由订阅会话事件流驱动；本地不维护消息历史真相，断线后按 seq 续传、失步则从会话存储全量恢复。
   备选（GUI 进程内跑 loop、走旧的无会话端点）被否决——违背路线 B 的多屏同步目标。

2. **会话编排客户端抽象为 UI 无关模块**
   新建 `src/client/`：命令通道（HTTP）+ 事件通道（SSE 订阅、seq 跟踪、自动重连续传）。不抽离 `src/tui/client.rs`（那是旧模型的客户端，TUI 迁移时再处理）；新模块面向新编排 API 设计。
   备选（在 GUI 模块内直接写 HTTP 调用）被否决——协议逻辑应与 UI 解耦，供后续 Web 客户端参照。

3. **纯 Rust GUI，框架候选 egui / iced，feature flag `gui` 隔离**
   用户已明确排除 Tauri/Webview 方案。egui（即时模式、迭代快）与 iced（Elm 架构、状态清晰）的权衡涉及流式渲染性能与复杂状态管理，留待 design 阶段基于 spike 结论选型。GUI 代码全部置于 `gui` feature 之后，默认构建零影响。

4. **连接策略：发现常驻实例优先，内嵌拉起兜底**
   启动时先按 daemon-session-orchestration 的发现机制连接已驻留 daemon（与 TUI 共享实例、共享会话）；校验失败则进程内拉起（保留现有内嵌模式体验）。

## Risks / Trade-offs

- [GUI 框架流式渲染性能不足（高频事件刷新）] → design 阶段用 spike 验证增量渲染与虚拟滚动；必要时降帧批量刷新
- [事件流投影的状态模型与所选框架的响应式范式不匹配] → design 阶段在选型 spike 中一并验证「事件→状态投影→渲染」链路
- [依赖的前置 change（daemon-session-orchestration）延期] → 任务编排上严格后置；客户端模块可先针对契约 mock 开发
- [GUI 依赖显著增大 release 构建时间与二进制体积] → feature flag 隔离，默认构建不含 GUI；GUI 构建单独验证性能约束

## Open Questions

- egui vs iced 最终选型（附 spike 结论）
- 事件→状态投影在 GUI 侧的具体范式（取决于框架选型）
- GUI 子命令/独立 bin 的入口形态（`wgenty-code gui` vs 独立二进制）
