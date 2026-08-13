# Proposal: gui-config-and-models

> Comet 批量拆分项 3/4（batch: `.comet/batches/gui-desktop.json`）。依赖 gui-desktop-foundation 提供的应用骨架与面板挂载点。

## Why

模型切换、provider 配置、MCP servers、skills、memory 等管理能力目前只能通过 TUI 命令或手动编辑配置文件完成，对桌面用户门槛高。daemon 已暴露 config、models/switch、mcp servers 等端点，GUI 应提供图形化管理界面，与 TUI 能力对齐。

## What Changes

- 新增 GUI 模型切换界面：展示可用模型列表，切换当前模型并对新对话生效
- 新增 GUI 基础配置界面：查看与修改常用配置项（provider、基础行为开关等），密钥类敏感值脱敏展示
- 新增 GUI MCP servers 管理界面：查看、启用/禁用、添加/移除 MCP server
- 新增 GUI skills 与 memory 管理界面：查看已安装 skills、浏览/管理记忆条目

## Capabilities

### New Capabilities

- `gui-config`: GUI 配置与模型管理——模型切换、基础配置读写（敏感值脱敏）、MCP servers 管理、skills 查看、memory 管理

### Modified Capabilities

（无——复用 daemon 现有 config/models/mcp API；若 API 缺口在 build 阶段升级为范围决策点）

## Impact

- **新增代码**：复用/扩展 `web/src/features/panels/` 下的配置/模型/MCP/skills/memory 面板（ModelPanel、MemoryPanel、SkillsPanel 等）
- **依赖**：gui-desktop-foundation；daemon 现有 API
- **不触碰**：core、daemon 服务端、TUI；密钥的高级管理（系统 keychain 集成等）不在范围内
