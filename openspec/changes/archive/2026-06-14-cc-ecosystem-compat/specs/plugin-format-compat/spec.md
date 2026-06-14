# plugin-format-compat

MUST 插件系统兼容 Claude Code 标准插件格式。

## ADDED Requirements

### Requirement: REQ-PFC-001 — manifest loading priority

`PluginLoader::load_manifest()` MUST MUST 必须首先探测 `package.json`，回退到 `plugin.json`。

#### Scenario: Package.json exists
- GIVEN 插件目录包含 `package.json`
- WHEN `load_manifest()` 被调用
- THEN 解析 `package.json` 并返回 `PluginManifest`

#### Scenario: Only plugin.json exists (fallback)
- GIVEN 插件目录只包含 `plugin.json`（无 `package.json`）
- WHEN `load_manifest()` 被调用
- THEN 解析 `plugin.json` 并返回 `PluginManifest`

### Requirement: REQ-PFC-002 — package.json field mapping

MUST MUST 必须正确解析 `package.json` 的 `name`, `version`, `description`, `author`, `main` 字段并映射到内部 `PluginManifest`。

#### Scenario: All fields present
- GIVEN 完整的 `package.json` 包含 name, version, description, author, main
- WHEN JSON 被反序列化为 `PackageJsonManifest`
- THEN 所有字段映射到 `PluginManifest` 对应字段
- AND `@scope/pkg` 格式的 name 拆分为 publisher="scope" + name="pkg"

#### Scenario: Minimal fields
- GIVEN `package.json` 仅包含 name 和 version
- WHEN JSON 被反序列化
- THEN description, author, main 为 None

### Requirement: REQ-PFC-003 — cache directory structure

MUST MUST 必须支持 `cache/<publisher>/<plugin>/<version>/` 目录结构（除现有扁平结构外）。

#### Scenario: CC-format plugin loaded from cache
- GIVEN `cache/anthropic/superpowers/5.1.0/package.json` 存在
- WHEN `PluginManager::load_all()` 运行
- THEN 插件以 CC 格式加载，manifest.source_format = "cc"

### Requirement: REQ-PFC-004 — installed_plugins.json registry

MUST MUST 必须能从 `installed_plugins.json` 加载/保存已安装插件注册表。

#### Scenario: Load existing registry
- GIVEN `installed_plugins.json` 文件存在且格式正确
- WHEN `load_installed_registry()` 被调用
- THEN 返回包含所有已安装插件的 `InstalledPluginsRegistry`

#### Scenario: Save registry to disk
- GIVEN 一个已填充的 `InstalledPluginsRegistry`
- WHEN `save_installed_registry()` 被调用
- THEN JSON 写入磁盘，installPath/gitCommitSha 等字段正确保留

### Requirement: REQ-PFC-005 — enabledPlugins config key

MUST MUST 必须支持 `enabledPlugins` 配置键（`{ "plugin@publisher": true }` 格式）。

#### Scenario: enabledPlugins loaded from settings.json
- GIVEN `settings.json` 包含 `"enabledPlugins": {"superpowers@anthropic": true}`
- WHEN `Settings::load()` 运行
- THEN `plugins.enabled_map` 包含对应条目

### Requirement: REQ-PFC-006 — backward compatibility

MUST 向后兼容——现有 `plugin.json` 插件继续正常工作。

#### Scenario: Legacy plugin.json still loads
- GIVEN 插件目录仅有 `plugin.json`（无 `package.json`）
- WHEN 加载插件
- THEN `plugin.json` 被正确解析和加载

### Requirement: REQ-PFC-007 — CC format priority

MUST 同名插件在两种目录结构都存在时，CC 格式优先。

#### Scenario: Both formats present for same plugin name
- GIVEN `cache/pub/name/1.0.0/package.json` 和 `plugins/name/plugin.json` 均存在
- WHEN `load_all()` 运行
- THEN CC 版本（package.json）被使用
