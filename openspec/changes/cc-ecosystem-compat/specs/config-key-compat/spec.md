# config-key-compat

settings.json 兼容 Claude Code 标准配置键名。

## Requirements

- **REQ-CKC-001**: `Settings` 必须支持 `enabledPlugins` 键（`HashMap<String, bool>` 格式）
- **REQ-CKC-002**: `Settings` 必须支持 `pluginMarketplaces` 键（marketplace source 配置）
- **REQ-CKC-003**: CC 键名映射到内部字段（`enabledPlugins` → `plugins.enabled_map`）
- **REQ-CKC-004**: 现有 `plugins.enabled`、`plugins.plugin_dir` 继续正常工作
- **REQ-CKC-005**: CC 键名优先——当 `enabledPlugins` 和 `plugins.enabled` 同时存在时，`enabledPlugins` 优先级更高
