# Brainstorm Summary: tui-context-usage-indicator

## Status: COMPLETED

## Requirements (from open phase)

- 在 TUI 输入框模式标签栏右侧添加上下文占比指示器
- 显示形式：进度条 + 百分比（`▓▓▓░░░░░░ 32%`）
- 窗口上限：`settings.json` 可配置 `models.context_window`，默认 200000
- 颜色编码：绿色 <50%，黄色 50-80%，红色 >80%
- 数据源：API 报告的 `prompt_tokens`
- 首次调用前：显示 0% 空条
- 窄终端（width < 40）：隐藏进度条

## Technical Decisions

| # | 决策 | 选择 | 理由 |
|---|------|------|------|
| D1 | TokenCounter 扩展 | 新增 `last_prompt_tokens` 字段 | 已在 App/AgentLoop 间共享，避免新通道 |
| D2 | 进度条字符 | Unicode `▓`/`░`，固定 8 格 | 零依赖，兼容性好，避免布局抖动 |
| D3 | 颜色阈值 | 绿<50%, 黄50-80%, 红>80% | 直观反映上下文健康状态 |
| D4 | 配置位置 | `ModelsConfig.context_window` | 顶层模型配置，serde default |
| D5 | 0-token 显示 | 显示 0% 空条 | 视觉一致 |
| D6 | 窄终端处理 | width<40 隐藏进度条 | 避免拥挤 |

## Design Doc

`docs/superpowers/specs/2026-06-22-tui-context-usage-indicator-design.md`

## Open Questions

无。所有决策已确认。
