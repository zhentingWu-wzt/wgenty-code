# config-key-compat

MUST settings.json 兼容 Claude Code 标准配置键名。

## ADDED Requirements

### Requirement: REQ-CKC-001 — enabledPlugins key

`Settings` MUST MUST 必须支持 `enabledPlugins` 键（`HashMap<String, bool>` 格式）。

#### Scenario: enabledPlugins loaded from settings.json
- GIVEN `settings.json` 包含 `"enabledPlugins": {"plugin@pub": true}`
- WHEN `Settings::load()` 加载
- THEN `settings.enabled_plugins` 包含对应条目

### Requirement: REQ-CKC-002 — pluginMarketplaces key

`Settings` MUST MUST 必须支持 `pluginMarketplaces` 键（marketplace source 配置）。

#### Scenario: pluginMarketplaces loaded
- GIVEN `settings.json` 包含 `"pluginMarketplaces": {...}`
- WHEN `Settings::load()` 加载
- THEN `settings.plugin_marketplaces` 包含 marketplace 配置

### Requirement: REQ-CKC-003 — CC key mapping

MUST CC 键名映射到内部字段（`enabledPlugins` → `plugins.enabled_map`）。

#### Scenario: CcConfigMapper runs after load
- GIVEN enabledPlugins 在 settings 中设置
- WHEN `CcConfigMapper::apply_mappings()` 运行
- THEN `plugins.enabled_map` 包含 CC 键的条目

### Requirement: REQ-CKC-004 — native key backward compatibility

MUST 现有 `plugins.enabled`、`plugins.plugin_dir` 继续正常工作。

#### Scenario: Legacy config still works
- GIVEN 只有原有 `plugins.enabled` 键（无 CC 键）
- WHEN settings 加载
- THEN 原有配置继续生效

### Requirement: REQ-CKC-005 — CC key priority

MUST CC 键名优先——当 `enabledPlugins` 和 `plugins.enabled` 同时存在时，`enabledPlugins` 优先级更高。

#### Scenario: Conflict resolution
- GIVEN `plugins.enabled_map` 已有 `"plugin@pub": false`
- AND `enabledPlugins` 包含 `"plugin@pub": true`
- WHEN mapping 执行
- THEN CC 键值 `true` 覆盖原有值
