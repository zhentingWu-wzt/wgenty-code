# Proposal: Claude Code 生态兼容

## Why

wgenty-code 已具备自己的插件系统和技能加载能力，但其插件格式、Hook 事件、配置键名和 marketplace 机制与 Claude Code 标准生态互不兼容。这导致用户无法将 Claude Code 社区已有的插件和 marketplace 迁移到 wgenty-code 使用，阻碍了 wgenty-code 作为 Claude Code 替代 CLI 的定位。本次变更使 wgenty-code 完全兼容 Claude Code 的 skills 和 plugins 生态。

## What Changes

- **插件格式兼容**：同时支持 `package.json`（Claude Code 标准）和 `plugin.json`（wgenty-code 原有）两种 manifest 格式；支持 `installed_plugins.json` 注册表格式；支持 `enabledPlugins` 配置键
- **插件目录结构兼容**：支持 `cache/<publisher>/<plugin>/<version>/` 目录层级（除现有扁平结构外）
- **Hook 事件类型对齐**：新增 `Stop`、`UserPromptSubmit`、`PermissionRequest` 事件；新增 `matcher` 字段支持按工具名模式匹配；新增 `%tool%` 等变量展开
- **配置键名兼容**：`settings.json` 支持 `enabledPlugins`、`pluginMarketplaces` 等 Claude Code 标准键名
- **Marketplace 实时获取**：支持 `known_marketplaces.json` 配置 GitHub repo 作为 marketplace 源，通过 `git clone` 获取索引并从中搜索、安装插件

## Capabilities

### New Capabilities

- `plugin-format-compat`: 识别并加载 Claude Code 标准插件格式（package.json manifest、installed_plugins.json 注册表、cache 目录结构、enabledPlugins 配置）
- `hook-event-alignment`: Claude Code 兼容的 Hook 事件系统（Stop, UserPromptSubmit, PermissionRequest）及 matcher 模式匹配和变量展开
- `plugin-marketplace`: 从 GitHub repos 实时获取 marketplace 索引，支持搜索、安装插件
- `config-key-compat`: settings.json 中 enabledPlugins、pluginMarketplaces 等键名兼容

### Modified Capabilities

<!-- 本次不修改已有 spec 的需求，仅新增能力 -->

## Impact

- **`src/plugins/`**：loader 扩展为多格式识别；registry 扩展为多注册表格式；mod.rs 扩展 PluginManifest
- **`src/hooks/`**：新增 HookEvent 变体、matcher 字段、变量展开逻辑
- **`src/services/plugin_marketplace.rs`**：重写为基于 GitHub 的实时 marketplace
- **`src/config/mod.rs`**：新增 enabledPlugins、pluginMarketplaces 配置项
- **`src/cli/`**：可能需要新增 marketplace 管理子命令
- **向后兼容**：现有 `plugin.json` 格式、自有技能路径、自有配置键名继续正常工作
