# Proposal: gui-advanced-panels

> Comet 批量拆分项 4/4（batch: `.comet/batches/gui-desktop.json`）。依赖 gui-desktop-foundation 提供的应用骨架与面板挂载点。

## Why

TUI 已具备（或正在通过 tui-inspector change 建设）subagent 进度展示、todos 面板、请求上下文透视等高级可观察性能力。这些能力信息密度高，恰恰是 GUI 多面板布局最能发挥优势的场景；GUI 要对齐 TUI 全功能，必须覆盖这些高级面板。

## What Changes

- 新增 GUI subagent 进度树：可视化展示 subagent 层级、状态与工具执行进度
- 新增 GUI todos/tasks 面板：实时同步展示任务清单及勾选状态
- 新增 GUI 请求上下文透视面板：对齐 tui-inspector 的能力——system prompt 来源分层、召回记忆、完整 messages、hook 注入、token 统计

## Capabilities

### New Capabilities

- `gui-advanced-panels`: GUI 高级可观察性面板——subagent 进度树、todos 实时面板、请求上下文透视面板

### Modified Capabilities

（无——复用 daemon 现有 subagent 进度、tasks/todos 等 API；透视面板数据若依赖 tui-inspector 的 TurnContext 采集，复用其成果而非重复建设）

## Impact

- **新增代码**：`src/gui/` 下的 subagent tree、todos、inspector 面板
- **依赖**：gui-desktop-foundation；tui-inspector change 的上下文采集成果（若已落地）；daemon 现有 API
- **不触碰**：core、daemon 服务端、TUI 组件本身
