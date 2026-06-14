# plugin-format-compat

插件系统兼容 Claude Code 标准插件格式。

## Requirements

- **REQ-PFC-001**: `PluginLoader::load_manifest()` 必须首先探测 `package.json`，回退到 `plugin.json`
- **REQ-PFC-002**: 必须正确解析 `package.json` 的 `name`, `version`, `description`, `author`, `main` 字段并映射到内部 `PluginManifest`
- **REQ-PFC-003**: 必须支持 `cache/<publisher>/<plugin>/<version>/` 目录结构（除现有扁平结构外）
- **REQ-PFC-004**: 必须能从 `installed_plugins.json` 加载/保存已安装插件注册表
- **REQ-PFC-005**: 必须支持 `enabledPlugins` 配置键（`{ "plugin@publisher": true }` 格式）
- **REQ-PFC-006**: 向后兼容——现有 `plugin.json` 插件继续正常工作
- **REQ-PFC-007**: 同名插件在两种目录结构都存在时，CC 格式优先
