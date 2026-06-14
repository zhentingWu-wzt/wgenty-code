# Comet Design Handoff

- Change: cc-ecosystem-compat
- Phase: design
- Mode: compact
- Context hash: 51abacfd9ce5ed162204fc929afe3ec66823e90376ef0affd65745cc1582c6d0

Generated-by: comet-handoff.sh

OpenSpec remains the canonical capability spec. This handoff is a deterministic, source-traceable context pack, not an agent-authored summary.

## openspec/changes/cc-ecosystem-compat/proposal.md

- Source: openspec/changes/cc-ecosystem-compat/proposal.md
- Lines: 1-35
- SHA256: a120f4bfad7027c0b0ccda73c657179c37b1d664de9c01a97db3293dbeddfd63

```md
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
```

## openspec/changes/cc-ecosystem-compat/design.md

- Source: openspec/changes/cc-ecosystem-compat/design.md
- Lines: 1-212
- SHA256: ebf5d808cc452094b7c85f6d328660d62d9c9b2d334832ce6b5dea9ac1714151

[TRUNCATED]

```md
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
```

Full source: openspec/changes/cc-ecosystem-compat/design.md

## openspec/changes/cc-ecosystem-compat/tasks.md

- Source: openspec/changes/cc-ecosystem-compat/tasks.md
- Lines: 1-49
- SHA256: 267dc09743e046e0a8bb2dde7b83ba0cc9b43f13b51cd59e973100d486bae71a

```md
# Tasks: Claude Code 生态兼容

## 1. 插件格式兼容 — Manifest 识别

- [ ] 1.1 扩展 `PluginManifest` 结构，新增 CC 特有字段（publisher, scope, install_path, git_commit_sha），使用 `#[serde(default)]`
- [ ] 1.2 实现 `try_load_package_json()` — 解析 `package.json` 并映射到 `PluginManifest`
- [ ] 1.3 修改 `load_manifest()` — 优先探测 `package.json`，回退到 `plugin.json`
- [ ] 1.4 实现 `package.json` → `PluginManifest` 的字段映射（name 处理 @scope/ 前缀, main, description, author）

## 2. 插件格式兼容 — 注册表与目录结构

- [ ] 2.1 定义 `InstalledPluginEntry` 结构（scope, installPath, version, installedAt, lastUpdated, gitCommitSha）
- [ ] 2.2 定义 `InstalledPluginsRegistry` 结构（version, plugins: HashMap<String, Vec<InstalledPluginEntry>>）
- [ ] 2.3 实现 `load_installed_registry()` — 从 `~/.wgenty-code/plugins/installed_plugins.json` 加载
- [ ] 2.4 实现 `save_installed_registry()` — 将当前注册表持久化到 JSON
- [ ] 2.5 修改 `PluginManager::load_all()` — 先扫描 `cache/` 子目录（CC 格式），再扫描扁平目录，同名 CC 格式优先

## 3. 配置键名兼容

- [ ] 3.1 扩展 `Settings` 结构 — 新增 `enabled_plugins: Option<HashMap<String, bool>>` 和 `plugin_marketplaces: Option<HashMap<String, MarketplaceSource>>`
- [ ] 3.2 实现加载后映射逻辑 — `enabledPlugins` 同步到 `plugins.enabled_map`；`pluginMarketplaces` 合并到 marketplace 注册
- [ ] 3.3 实现优先级规则 — CC 键名优先于 wgenty-code 原有键名
- [ ] 3.4 添加 `set()` 方法对新键名的支持（`enabledPlugins.<name>`, `pluginMarketplaces.<name>`）

## 4. Hook 事件类型对齐

- [ ] 4.1 扩展 `HookEvent` 枚举 — 新增 `Stop`, `UserPromptSubmit`, `PermissionRequest`
- [ ] 4.2 扩展 `HookDefinition` 结构 — 新增 `matcher: Option<String>` 字段和 `hook_type: Option<String>`（command/prompt）
- [ ] 4.3 实现 `matcher` 匹配逻辑 — 支持空/全部匹配、管道分隔多模式、Notification 子类型匹配
- [ ] 4.4 实现变量展开 `expand_variables()` — `%tool%` → 工具名，`%input%` → 转义后的工具输入 JSON
- [ ] 4.5 修改 `HookManager::from_settings()` — 兼容 Claude Code hooks 数组嵌套格式（含 matcher + type 字段）

## 5. Marketplace 实时获取

- [ ] 5.1 定义 `MarketplaceSource` 结构（source: GitHub, repo, installLocation, lastUpdated, autoUpdate）
- [ ] 5.2 实现 `load_known_marketplaces()` — 从 `~/.wgenty-code/plugins/known_marketplaces.json` 加载
- [ ] 5.3 实现 `clone_marketplace()` — `git clone --depth 1 <repo>` 到本地缓存目录
- [ ] 5.4 实现 `parse_marketplace_index()` — 从克隆的 repo 中解析 `index.json` 或扫描 `plugins/` 目录
- [ ] 5.5 重写 `search()` — 使用真实 marketplace 数据替代硬编码示例
- [ ] 5.6 重写 `install()` — 从 marketplace 条目获取源 URL，git clone 到 `cache/<publisher>/<plugin>/<version>/`
- [ ] 5.7 实现 marketplace 自动更新 — `git pull` 检查新数据

## 6. 集成与测试

- [ ] 6.1 实现端到端流程：添加 marketplace → 搜索 → 安装 → 加载插件
- [ ] 6.2 编写单元测试 — `package.json` 解析、matcher 匹配、变量展开
- [ ] 6.3 编写集成测试 — CC 格式插件完整加载流程
- [ ] 6.4 向后兼容验证 — 现有 `plugin.json` 插件和 hooks 配置继续工作
- [ ] 6.5 运行 `cargo clippy -- -D warnings` + `cargo fmt -- --check` + `cargo test --all` 全部通过
```

## openspec/changes/cc-ecosystem-compat/specs/config-key-compat/spec.md

- Source: openspec/changes/cc-ecosystem-compat/specs/config-key-compat/spec.md
- Lines: 1-11
- SHA256: f69379e03afc2b4e40351f511dbbebdb6a351c33e964bd80f37495b96a582d7c

```md
# config-key-compat

settings.json 兼容 Claude Code 标准配置键名。

## Requirements

- **REQ-CKC-001**: `Settings` 必须支持 `enabledPlugins` 键（`HashMap<String, bool>` 格式）
- **REQ-CKC-002**: `Settings` 必须支持 `pluginMarketplaces` 键（marketplace source 配置）
- **REQ-CKC-003**: CC 键名映射到内部字段（`enabledPlugins` → `plugins.enabled_map`）
- **REQ-CKC-004**: 现有 `plugins.enabled`、`plugins.plugin_dir` 继续正常工作
- **REQ-CKC-005**: CC 键名优先——当 `enabledPlugins` 和 `plugins.enabled` 同时存在时，`enabledPlugins` 优先级更高
```

## openspec/changes/cc-ecosystem-compat/specs/hook-event-alignment/spec.md

- Source: openspec/changes/cc-ecosystem-compat/specs/hook-event-alignment/spec.md
- Lines: 1-11
- SHA256: bf858a1845b12d0358d7f34798180e62ccb2a77384cf01f150430e44eb519530

```md
# hook-event-alignment

Hook 系统兼容 Claude Code 标准事件类型和配置格式。

## Requirements

- **REQ-HEA-001**: `HookEvent` 必须新增 `Stop`、`UserPromptSubmit`、`PermissionRequest` 三种事件类型
- **REQ-HEA-002**: `HookDefinition` 必须支持 `matcher` 字段（`None`/`""` = 全部匹配，`"ToolA|ToolB"` = 管道分隔模式匹配）
- **REQ-HEA-003**: Hook 命令执行前必须展开 `%tool%` 和 `%input%` 变量
- **REQ-HEA-004**: `HookManager::from_settings()` 必须兼容 Claude Code 的 hooks 配置数组格式（含 `matcher` 和 `type` 字段的嵌套结构）
- **REQ-HEA-005**: 现有事件类型（PreToolUse, PostToolUse, SessionStart, SessionEnd, Notification）继续正常工作
```

## openspec/changes/cc-ecosystem-compat/specs/plugin-format-compat/spec.md

- Source: openspec/changes/cc-ecosystem-compat/specs/plugin-format-compat/spec.md
- Lines: 1-13
- SHA256: 007e3009fcd0fd2b18d2065e428cfc120ed805c773be86f2b2032b2282d355ea

```md
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
```

## openspec/changes/cc-ecosystem-compat/specs/plugin-marketplace/spec.md

- Source: openspec/changes/cc-ecosystem-compat/specs/plugin-marketplace/spec.md
- Lines: 1-13
- SHA256: 10901d6f77f1ee2e988f563d7128a8dbe9c194fd3d63277490b3b2f98ff5811a

```md
# plugin-marketplace

Marketplace 服务从 GitHub repos 实时获取插件索引。

## Requirements

- **REQ-PM-001**: 必须支持 `known_marketplaces.json` 配置格式（source.repo → installLocation）
- **REQ-PM-002**: 首次使用时自动 `git clone --depth 1` marketplace repo
- **REQ-PM-003**: 必须从 marketplace repo 中解析插件索引（`index.json` 或目录扫描）
- **REQ-PM-004**: `search(query)` 必须返回真实 marketplace 数据（替换硬编码示例）
- **REQ-PM-005**: `install(name)` 必须从 marketplace 找到插件源 URL 并 git clone 到标准 `cache/<publisher>/<plugin>/<version>/` 路径
- **REQ-PM-006**: 支持 marketplace 自动更新（`git pull` 检查新版本）
- **REQ-PM-007**: `install()` 必须处理 marketplace entry 的三种 `source` 类型——`LocalPath`（marketplace 本地子目录，如 `"./plugins/x"`）、`GitSubdir`（`{"source":"git-subdir","url":"...","path":"plugins/x","ref":"main"}` 格式）、`RemoteUrl`（`{"source":"url","url":"https://github.com/..."}` 格式）——每种都正确安装到 `cache/<publisher>/<plugin>/<version>/` 路径，解析使用 `#[serde(untagged)]` enum 自动匹配
```

