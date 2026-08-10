# Design: gui-config-and-models

## Context

daemon 已提供 config、models/switch、mcp servers 等端点（`src/daemon/routes.rs`）。Tauri 桌面端复用 `web/` 前端，配置/模型/MCP/skills/memory 面板直接复用 web/ 已有的 React 组件（web/src/features/panels/ 下的 ModelPanel、MemoryPanel、SkillsPanel 等），密钥脱敏逻辑也一并复用。foundation 提供 Tauri 壳与 daemon 连接。

## Goals / Non-Goals

**Goals:**
- 模型列表展示与切换，切换对新对话生效
- 基础配置的图形化读写，敏感值脱敏
- MCP servers / skills / memory 的管理界面

**Non-Goals:**
- 对话与会话管理界面（其他 change）
- 密钥高级管理（keychain 集成、加密存储改造）
- daemon API 变更（发现缺口走范围决策点）

## Decisions

1. **配置写入走 daemon API，不直接写配置文件**：保持单一写路径，避免 GUI 与 CLI/TUI 并发写配置产生不一致。
2. **敏感值默认脱敏**：API key 类字段只展示掩码，编辑时才输入新值；不在界面回显明文。
3. **模型切换即时生效于新对话**：切换通过 daemon models/switch 端点，进行中对话不强制中断。

## Risks / Trade-offs

- [daemon 配置写 API 覆盖不全（部分配置项可能只读）] → build 阶段先盘点可写配置项清单，缺口升级为范围决策点
- [配置项数量多导致界面臃肿] → 只暴露常用项，高级项引导用户编辑配置文件
- [memory 数据量大时列表面板性能] → 分页加载，复用会话列表的懒加载模式
