## MODIFIED Requirements

### Requirement: REQ-PFC-003 — cache directory structure

MUST MUST 必须支持 `cache/<publisher>/<plugin>/<version>/` 目录结构（除现有扁平结构外），并且外部 skill discovery SHALL be able to discover skill documents below enabled plugin/cache roots that follow this structure.

#### Scenario: CC-format plugin loaded from cache
- GIVEN `cache/anthropic/superpowers/5.1.0/package.json` 存在
- WHEN `PluginManager::load_all()` 运行
- THEN 插件以 CC 格式加载，manifest.source_format = "cc"

#### Scenario: Skill documents discovered from CC-format plugin cache
- **WHEN** `cache/anthropic/superpowers/5.1.0/skills/brainstorming/SKILL.md` exists for an enabled plugin
- **THEN** external skill discovery includes the plugin skill using the canonical name declared by that skill's metadata or directory
- **AND** the skill source metadata identifies the plugin/cache root
