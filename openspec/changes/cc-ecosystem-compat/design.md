# Design: Claude Code 生态兼容

## Context

wgenty-code 目前有自成一体的插件系统和技能加载机制：

- **技能**：`SkillLoader::load_from_dirs()` 扫描 `skills/` 目录，解析 `SKILL.md` 的 YAML frontmatter，通过 `load_skill` 工具按需注入
- **插件**：`PluginLoader::load_manifest()` 读取 `plugin.json`，支持 Native/Wasm/Script 三种模块类型
- **注册表**：内存 `HashMap<String, PluginManifest>`，无持久化格式
- **Marketplace**：`PluginMarketplaceService` 使用硬编码示例数据
- **Hooks**：`HookEvent::{PreToolUse, PostToolUse, SessionStart, SessionEnd, Notification}`，从 `settings.json` `hooks` 字段解析，无 matcher/变量展开
- **配置**：`Settings` 有 `plugins: PluginSettings { enabled, plugin_dir, auto_update }` 和 `include_skill_instructions`

Claude Code 生态标准格式：

- **技能**：`~/.claude/skills/` + `.claude/skills/` + 插件 `skills/` 子目录
- **插件格式**：`package.json`（npm 格式），目录为 `cache/<publisher>/<plugin>/<version>/`
- **注册表**：`~/.claude/plugins/installed_plugins.json`（含 version, installPath, gitCommitSha 等）
- **Marketplace**：`~/.claude/plugins/known_marketplaces.json`，每个条目指向 GitHub repo
- **Hooks**：每个事件 hook 数组含 `matcher` 字段（支持 `""` 全部匹配或具体工具名模式），命令中支持 `%tool%` 变量
- **配置**：`settings.json` 使用 `enabledPlugins: { "plugin@publisher": true }` 格式

## Goals / Non-Goals

**Goals:**
1. 插件系统能识别并加载 Claude Code 格式的插件（package.json manifest + installed_plugins.json 注册表 + cache 目录结构）
2. Hook 系统完全兼容 Claude Code 的事件类型和配置语法（matcher、变量展开）
3. Marketplace 从 GitHub repos 实时获取真实数据
4. settings.json 兼容 Claude Code 标准键名
5. 技能发现路径保持 wgenty-code 自有路径（不在范围——用户明确要求）

**Non-Goals:**
- 不修改技能发现路径（使用 `~/.wgenty-code/skills/` + `.wgenty/skills/`）
- 不改变 WASM 插件执行机制
- 不影响 GUI / TUI / Web 前端
- 不修改 `SKILL.md` 解析逻辑（已兼容）

## Decisions

### Decision 1: 多 Manifest 格式识别策略

**选择**：在 `PluginLoader::load_manifest()` 中按优先级探测 manifest 文件：
1. 先查找 `package.json`（Claude Code 标准）
2. 回退到 `plugin.json`（wgenty-code 原有）
3. 内部统一为 `PluginManifest` 结构

```rust
// loader.rs 扩展逻辑
async fn load_manifest(&self, plugin_dir: &Path) -> Result<PluginManifest> {
    // Priority 1: package.json (Claude Code format)
    if let Ok(m) = self.try_load_package_json(plugin_dir).await {
        return Ok(m);
    }
    // Priority 2: plugin.json (wgenty-code legacy)
    if let Ok(m) = self.try_load_plugin_json(plugin_dir).await {
        return Ok(m);
    }
    Err(anyhow!("No manifest found"))
}
```

**package.json → PluginManifest 映射**：
- `name` → `PluginManifest.name`（如果含 `@scope/` 前缀，去 scope）
- `version` → `PluginManifest.version`
- `description` → `PluginManifest.description`
- `author` (string/object) → `PluginManifest.author`
- `main` → `PluginManifest.main`
- `.opencode` / `.claude` 字段 → hooks/commands 扩展点

**备选方案**：创建独立的 `CcPluginManifest` 类型 → 弃用，增加维护负担且两套结构本质同构。

### Decision 2: 注册表持久化格式

**选择**：扩展 `PluginRegistry` 支持从 `installed_plugins.json` 加载/保存，同时保留内存 HashMap。

```
installed_plugins.json 结构:
{
  "version": 2,
  "plugins": {
    "plugin-name@publisher": [{
      "scope": "user",
      "installPath": "/path/to/plugin",
      "version": "x.y.z",
      "installedAt": "ISO8601",
      "lastUpdated": "ISO8601",
      "gitCommitSha": "abc123"
    }]
  }
}
```

**加载流程**：
1. `PluginRegistry::load_installed_registry()` 读取 `~/.wgenty-code/plugins/installed_plugins.json`
2. 遍历 `plugins` 字段，从 `installPath` 加载 manifest
3. 若 JSON 中已启用但 manifest 加载失败 → 标记为 `PluginStatus::Error`
4. 同时支持旧的扁平目录扫描（向后兼容）

### Decision 3: 插件目录结构

**选择**：同时支持两种目录结构：
- **Claude Code 标准**：`cache/<publisher>/<plugin>/<version>/`
- **wgenty-code 原有**：`<plugin-name>/`（扁平结构）

`PluginManager::load_all()` 先扫描 `cache/` 子目录（CC 格式），再扫描扁平目录（遗留格式）。同名插件以 CC 格式优先。

### Decision 4: Hook 系统扩展

**新增事件类型**：
```rust
pub enum HookEvent {
    // existing
    PreToolUse,
    PostToolUse,
    SessionStart,
    SessionEnd,
    Notification,
    // NEW — Claude Code compatible
    Stop,
    UserPromptSubmit,
    PermissionRequest,
}
```

**Matcher 支持**：`HookDefinition` 新增 `matcher: Option<String>` 字段：
- `None` / `""` → 匹配所有（当前行为）
- `"TaskCreate|TaskUpdate"` → 仅匹配指定工具名（管道分隔）
- `"permission_prompt"` → Notification 事件的子类型匹配

**变量展开**：Hook 命令执行前展开：
- `%tool%` → 当前工具名
- `%input%` → JSON 序列化的工具输入（需 escape）

```rust
fn expand_variables(command: &str, ctx: &HookContext) -> String {
    command
        .replace("%tool%", ctx.tool_name.as_deref().unwrap_or(""))
        .replace("%input%", &ctx.tool_input.as_ref()
            .map(|v| v.to_string()).unwrap_or_default())
}
```

### Decision 5: Marketplace 真实获取

**选择**：基于 `known_marketplaces.json` 的 GitHub repo 模式：

```json
{
  "claude-plugins-official": {
    "source": { "source": "github", "repo": "anthropics/claude-plugins-official" },
    "installLocation": "~/.wgenty-code/plugins/marketplaces/claude-plugins-official",
    "lastUpdated": "ISO8601",
    "autoUpdate": true
  }
}
```

**工作流**：
1. 首次 `search`：`git clone --depth 1 <repo>` 到 `installLocation`
2. 从 repo 中的 `plugins/` 或 `.opencode/plugins/` 目录解析索引文件（`index.json` / `manifest.json`）
3. 缓存 marketplace 数据到内存
4. `install`：从 marketplace 条目获取源 URL → git clone 到 `cache/<publisher>/<plugin>/<version>/`
5. `update`：`git pull` → 检查新版本

**备选方案**：HTTP API marketplace → 不选，Claude Code 生态基于 Git，且 API 需要额外服务端。

### Decision 6: 配置键名兼容

**选择**：`Settings` 增加 `enabledPlugins` 和 `pluginMarketplaces` 字段，与现有 `plugins.enabled` 和 marketplace 配置共存，自动映射：

```rust
// config/mod.rs
pub struct Settings {
    // ...existing fields...
    pub plugins: PluginSettings,  // 保持不变

    /// Claude Code compatible: enabledPlugins
    #[serde(default)]
    pub enabled_plugins: Option<HashMap<String, bool>>,

    /// Claude Code compatible: pluginMarketplaces
    #[serde(default)]
    pub plugin_marketplaces: Option<HashMap<String, MarketplaceSource>>,
}
```

**映射逻辑**（在 `Settings::load()` 后处理）：
- 若 `enabled_plugins` 有值 → 同步到 `plugins.enabled_map`
- 若 `plugin_marketplaces` 有值 → 合并到 marketplace `known_marketplaces.json`
- 两个字段独立读写，不互斥

## Risks / Trade-offs

| 风险 | 缓解措施 |
|------|---------|
| `package.json` 字段不稳定（Claude Code 可能新增字段） | 使用 `#[serde(default)]` + `serde_json::Value` 捕获未知字段，不丢失数据 |
| Git marketplace 获取慢（首次 clone 耗时） | `--depth 1` 浅克隆；异步后台更新；结果缓存 |
| 两套目录结构增加复杂度 | 扫描时统一抽象为 `PluginDiscoverySource`，内部一致处理 |
| Hook 变量展开的 shell 注入风险 | 展开前对值做 shell escape |
| 配置键名映射产生冲突 | 明确优先级：`enabledPlugins` > `plugins.enabled`（CC 键名优先） |

## Migration Plan

1. 所有新增字段使用 `#[serde(default)]`，不破坏现有配置
2. 现有 `plugin.json` 格式插件无需任何修改
3. Marketplace 首次使用时自动创建 `known_marketplaces.json`
4. 无需数据库迁移或数据转换

## Open Questions

- Claude Code 插件的 `package.json` 中 `.opencode` 字段的完整 schema 需从实际插件（如 superpowers）逆推确认
- marketplace repo 的标准目录结构需验证（`plugins/` vs `.opencode/plugins/` vs 其他）
