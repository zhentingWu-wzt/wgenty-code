---
comet_change: cc-ecosystem-compat
role: technical-design
canonical_spec: openspec
archived-with: 2026-06-14-cc-ecosystem-compat
status: final
---

# Technical Design: Claude Code 生态兼容

## 1. Architecture Overview

采用 **适配器层模式（方案 A）**：在外部 CC 格式入口处做转换，内部 wgenty-code 类型最小化改动。

```
┌──────────────────────────────────────────────────────────────────┐
│                        User / CLI / API                          │
└──────────────────────────────┬───────────────────────────────────┘
                               │
┌──────────────────────────────▼───────────────────────────────────┐
│                    Existing wgenty-code Core                      │
│  ┌──────────────┐ ┌──────────────┐ ┌──────────────┐              │
│  │ PluginManager │ │ HookManager  │ │   Settings   │              │
│  │ PluginLoader  │ │ HookEvent    │ │ PluginSettings│             │
│  │ PluginRegistry│ │ HookDefinition│ │              │              │
│  └──────┬───────┘ └──────┬───────┘ └──────┬───────┘              │
│         │                │                │                       │
│  ┌──────▼────────────────▼────────────────▼───────────────────┐  │
│  │                 Adapter Layer (NEW)                         │  │
│  │  ┌──────────────┐ ┌──────────────┐ ┌────────────────────┐  │  │
│  │  │ package_json │ │ cc_adapter   │ │ marketplace_resolver│  │  │
│  │  │    .rs       │ │    .rs       │ │       .rs           │  │  │
│  │  └──────────────┘ └──────────────┘ └────────────────────┘  │  │
│  │  ┌──────────────────────────────────────────────────────┐  │  │
│  │  │              cc_mapping.rs                            │  │  │
│  │  └──────────────────────────────────────────────────────┘  │  │
│  └───────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────────┘
                               │
┌──────────────────────────────▼───────────────────────────────────┐
│              External CC Format Files / Git Remotes               │
│  ┌────────────────┐ ┌──────────────────────┐ ┌─────────────────┐ │
│  │  package.json   │ │ installed_plugins.json│ │ marketplace repos│ │
│  └────────────────┘ └──────────────────────┘ └─────────────────┘ │
└──────────────────────────────────────────────────────────────────┘
```

**四个适配器组件（4 个新文件）**：

| 文件 | 职责 |
|------|------|
| `src/plugins/package_json.rs` | `package.json` → `PluginManifest` 解析与映射 |
| `src/hooks/cc_adapter.rs` | CC hooks 嵌套数组格式 → `Vec<HookDefinition>` |
| `src/services/marketplace_resolver.rs` | 3 种 marketplace source 类型解析 + Git 操作 |
| `src/config/cc_mapping.rs` | CC 配置键名 → 内部字段映射 |

## 2. Component Design

### 2.1 `src/plugins/package_json.rs` — Manifest 格式适配器

**PackageJsonManifest 结构**：

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageJsonManifest {
    pub name: String,            // 可能含 @scope/ 前缀
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub author: Option<AuthorField>,  // string | { name, email }
    #[serde(default)]
    pub main: Option<String>,
    #[serde(default)]
    pub opencode: Option<serde_json::Value>,   // CC 扩展点
    #[serde(default)]
    pub claude: Option<serde_json::Value>,     // 旧版 CC 扩展点
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,  // 捕获未知字段
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AuthorField {
    String(String),
    Object { name: Option<String>, email: Option<String> },
}
```

**映射逻辑 `into_plugin_manifest()`**：

```
package.json field          →  PluginManifest field
─────────────────────────────────────────────────────
name ("@scope/pkg")         →  name: "pkg"
                                publisher: Some("scope")
name ("pkg")                →  name: "pkg"
                                publisher: None
version                     →  version
description                 →  description
author: "name"              →  author: Some("name")
author: {name, email}       →  author: Some("name <email>")
main                        →  main
.opencode / .claude         → 保留在 extra 中，后续扩展点解析使用
```

**@scope/ 前缀处理**：
- 检测 `name` 中首个 `/` 位置
- `/` 前为 scope（去掉 `@`），`/` 后为裸名
- 无 `/` 则 scope=None，bare_name=name

### 2.2 `src/plugins/loader.rs` — 多格式加载

**修改 `load_manifest()` 逻辑**：

```rust
async fn load_manifest(&self, plugin_dir: &Path) -> Result<PluginManifest> {
    // Priority 1: package.json (CC format)
    let pkg_json_path = plugin_dir.join("package.json");
    if pkg_json_path.exists() {
        return self.try_load_package_json(&pkg_json_path).await;
    }
    // Priority 2: plugin.json (legacy format)
    let plugin_json_path = plugin_dir.join("plugin.json");
    if plugin_json_path.exists() {
        return self.try_load_plugin_json(&plugin_json_path).await;
    }
    Err(anyhow!("No manifest found in {}", plugin_dir.display()))
}
```

### 2.3 `src/plugins/mod.rs` — PluginManifest 扩展

新增可选字段（全部 `#[serde(default)]`）：

```rust
pub struct PluginManifest {
    // ... existing fields ...
    pub publisher: Option<String>,       // 从 @scope 提取
    pub install_path: Option<PathBuf>,   // 安装路径（CC 格式）
    pub git_commit_sha: Option<String>,  // git commit hash
    pub source_format: Option<String>,   // "cc" | "wgenty" — 来源标记
}
```

### 2.4 `src/plugins/registry.rs` — 注册表持久化

**InstalledPluginsRegistry 结构**：

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledPluginsRegistry {
    pub version: u32,  // 固定为 2
    pub plugins: HashMap<String, Vec<InstalledPluginEntry>>,
    // key: "plugin-name@publisher"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledPluginEntry {
    pub scope: String,           // "user" | "project"
    #[serde(rename = "installPath")]
    pub install_path: PathBuf,
    pub version: String,
    #[serde(rename = "installedAt")]
    pub installed_at: String,    // ISO 8601
    #[serde(rename = "lastUpdated")]
    pub last_updated: String,    // ISO 8601
    #[serde(rename = "gitCommitSha")]
    pub git_commit_sha: Option<String>,
}
```

**加载路径**：`~/.wgenty-code/plugins/installed_plugins.json`

**save_installed_registry()**：序列化当前注册表，atomic write（先写 tmp 再 rename）。

### 2.5 `src/plugins/mod.rs` — PluginManager::load_all() 扩展

**扫描优先级**：
1. `cache/<publisher>/<plugin>/<version>/package.json` — CC 格式
2. `cache/<publisher>/<plugin>/<version>/plugin.json` — CC 目录下的遗留格式
3. `<plugin-name>/plugin.json` — 扁平遗留格式
4. 同名插件 CC 格式覆盖扁平格式

**流程**：
```
load_all():
  plugins = {}
  // Phase 1: scan cache/ subdirectories (CC format)
  for entry in walk(cache_dir, max_depth=3):
    if entry is package.json:
      manifest = try_load_package_json(entry)
      key = manifest.name@publisher
      plugins[key] = manifest
  // Phase 2: scan flat directories (legacy)
  for entry in walk(plugin_dir, max_depth=1):
    if entry has plugin.json AND key not in plugins:
      manifest = try_load_plugin_json(entry)
      plugins[key] = manifest
  // Phase 3: merge installed_plugins.json metadata
  registry = load_installed_registry()
  for (key, entries) in registry.plugins:
    if key in plugins:
      enrich with installPath, gitCommitSha, etc.
  // Phase 4: apply enabledPlugins filter
  return filter_enabled(plugins)
```

### 2.6 `src/hooks/cc_adapter.rs` — CC Hook 格式适配

**CC 原始格式**（嵌套数组）：

```json
{
  "PostToolUse": [
    [
      {
        "type": "command",
        "command": "python3 analyze.py",
        "matcher": "TaskCreate|TaskUpdate",
        "env": { "PROJECT_DIR": "${CLAUDE_PROJECT_DIR}" }
      }
    ]
  ],
  "Stop": [
    [
      { "type": "prompt", "prompt": "Summarize the session" }
    ]
  ]
}
```

**CcHookConfig 反序列化结构**：

```rust
#[derive(Debug, Deserialize)]
struct CcHookConfig {
    #[serde(flatten)]
    events: HashMap<String, Vec<Vec<CcHookItem>>>,
}

#[derive(Debug, Deserialize)]
struct CcHookItem {
    pub r#type: String,                    // "command" | "prompt"
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub matcher: Option<String>,
    #[serde(default)]
    pub env: Option<HashMap<String, String>>,
    #[serde(default)]
    pub timeout: Option<u64>,
}
```

**adapt_cc_hooks() 展开逻辑**：
- 最外层 key → HookEvent 变体
- 第二层 `Vec<Vec<...>>` → 外层 Vec 是独立的 hook 组，内层 Vec 是串行执行的子 hook
- 每个 CcHookItem 生成一个 HookDefinition，`matcher` 直接传递
- `type: "prompt"` + `prompt` 字段 → `HookDefinition.hook_type = "prompt"`

### 2.7 `src/hooks/mod.rs` — HookEvent & HookDefinition 扩展

**HookEvent 新增变体**：

```rust
pub enum HookEvent {
    // existing
    PreToolUse,
    PostToolUse,
    SessionStart,
    SessionEnd,
    Notification,
    // NEW — CC compatible
    Stop,
    UserPromptSubmit,
    PermissionRequest,
}
```

**HookDefinition 新增字段**：

```rust
pub struct HookDefinition {
    // ... existing ...
    pub matcher: Option<String>,     // "" = match all, "A|B" = pipe-separated
    pub hook_type: Option<String>,   // "command" | "prompt"
}
```

**matcher 匹配逻辑 `matches_matcher()`**：

```
fn matches_matcher(matcher: &Option<String>, event: &HookEvent, tool_name: Option<&str>) -> bool:
    match matcher:
        None | Some("")           → true (match all)
        Some(pattern) if pattern.contains('|') →
            for part in pattern.split('|'):
                if matches_single(part.trim(), event, tool_name):
                    return true
        Some(pattern)             → matches_single(pattern, event, tool_name)

fn matches_single(pattern: &str, event: &HookEvent, tool_name: Option<&str>) -> bool:
    if event == Notification:
        pattern == notification_subtype  // e.g., "permission_prompt"
    else:
        pattern == tool_name.unwrap_or("")
```

**变量展开 `expand_variables()`**：

```rust
fn expand_variables(command: &str, ctx: &HookContext) -> String {
    command
        .replace("%tool%", shell_escape(ctx.tool_name.as_deref().unwrap_or("")))
        .replace("%input%", shell_escape(&ctx.tool_input
            .as_ref()
            .map(|v| serde_json::to_string(v).unwrap_or_default())
            .unwrap_or_default()))
}

/// Wrap value in single quotes, escape any internal single quotes
fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
```

**HookManager::from_settings() 修改**：
- 先尝试 CC 格式解析（通过 `cc_adapter::adapt_cc_hooks()`）
- 若 CC 格式解析失败或为空，回退到现有格式
- 合并结果

### 2.8 `src/services/marketplace_resolver.rs` — Marketplace 源解析

**MarketplaceEntry 结构**：

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceEntry {
    pub source: MarketplaceSource,
    #[serde(rename = "installLocation")]
    pub install_location: PathBuf,
    #[serde(rename = "lastUpdated", default)]
    pub last_updated: Option<String>,
    #[serde(rename = "autoUpdate", default)]
    pub auto_update: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceSource {
    pub source: String,  // "github"
    pub repo: String,    // "owner/repo"
}
```

**MarketplaceIndex（从 .claude-plugin/marketplace.json 解析）**：

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct MarketplaceIndex {
    pub name: String,
    pub owner: String,
    pub plugins: Vec<MarketplacePluginEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MarketplacePluginEntry {
    pub name: String,
    pub description: String,
    pub version: String,
    pub source: PluginSource,  // 3 种类型之一
    pub author: Option<AuthorField>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum PluginSource {
    /// Local path within the marketplace repo: `"./plugins/some-plugin"`
    LocalPath(String),
    /// Git subdirectory: `{"source": "git-subdir", "url": "...", "path": "plugins/x", "ref": "main"}`
    GitSubdir {
        source: String,  // "git-subdir"
        url: String,
        path: Option<String>,
        #[serde(rename = "ref")]
        ref_: Option<String>,
    },
    /// Independent git repo: `{"source": "url", "url": "https://github.com/..."}`
    RemoteUrl {
        source: String,  // "url"
        url: String,
    },
}
```

**Resolution 流程**：

```
resolve(marketplace_entry, plugin_name) → Result<ResolvedPlugin>:
  // 1. Ensure marketplace repo cloned
  let repo_path = ensure_cloned(&marketplace_entry)?  // git clone --depth 1 if needed
  // 2. Parse marketplace index
  let index = parse_marketplace_index(&repo_path)?     // .claude-plugin/marketplace.json
  // 3. Find plugin entry
  let plugin = index.plugins.iter().find(|p| p.name == plugin_name)?;
  // 4. Resolve source to local path/URL
  match &plugin.source:
    LocalPath(rel) →
      install_from_local(repo_path.join(rel), plugin)
    GitSubdir { url, path, ref_ } →
      install_from_git_subdir(url, path, ref_, plugin)
    RemoteUrl { url } →
      install_from_git(url, plugin)
```

**install_to_cache()**：
- 目标路径：`cache/<publisher>/<plugin_name>/<version>/`
- LocalPath: `cp -r` 到 cache 目录
- GitSubdir: `git clone --depth 1 --branch <ref> <url>` → `cp -r <path>` 到 cache
- url: `git clone --depth 1 <url>` 到 cache
- 所有操作后写入 `installed_plugins.json`

**Git 操作安全**：
- 30 秒 timeout（`tokio::time::timeout`）
- 失败返回明确错误，不静默跳过
- `git-subdir` 的 path 不存在 → `Err("path '{path}' not found in repo")`

### 2.9 `src/config/cc_mapping.rs` — 配置键名映射

**CcConfigMapper 结构**：

```rust
pub struct CcConfigMapper;

impl CcConfigMapper {
    pub fn apply_mappings(settings: &mut Settings) {
        // 1. enabledPlugins → plugins.enabled_map
        if let Some(ref enabled) = settings.enabled_plugins {
            for (key, val) in enabled {
                settings.plugins.enabled_map.entry(key.clone())
                    .or_insert(*val);
            }
        }
        // 2. pluginMarketplaces → known_marketplaces.json
        if let Some(ref marketplaces) = settings.plugin_marketplaces {
            let mut known = load_or_create_known_marketplaces();
            for (name, entry) in marketplaces {
                known.entry(name.clone()).or_insert_with(|| entry.clone());
            }
            save_known_marketplaces(&known);
        }
    }
}
```

**优先级规则**：
- `enabledPlugins` 存在时，其值覆盖 `plugins.enabled` 中同名键
- 独立键保留各自的值（不互斥）
- `pluginMarketplaces` 和 `plugins.marketplaces` 合并，同名 CC 键覆盖

**Settings 扩展**：

```rust
pub struct Settings {
    // ... existing ...
    pub plugins: PluginSettings,

    /// CC compatible: enabledPlugins
    #[serde(default, alias = "enabledPlugins")]
    pub enabled_plugins: Option<HashMap<String, bool>>,

    /// CC compatible: pluginMarketplaces
    #[serde(default, alias = "pluginMarketplaces")]
    pub plugin_marketplaces: Option<HashMap<String, MarketplaceSource>>,
}
```

## 3. Data Flow

### 3.1 Plugin Load Flow

```
Settings::load("settings.json")
  └→ CcConfigMapper::apply_mappings()
       ├→ enabledPlugins → plugins.enabled_map
       └→ pluginMarketplaces → known_marketplaces.json

PluginManager::load_all()
  ├→ scan cache/<publisher>/<plugin>/<version>/package.json (CC format)
  ├→ scan flat dirs plugin.json (legacy, skip if already found)
  ├→ load installed_plugins.json → enrich manifests
  └→ filter by enabledPlugins / plugins.enabled_map

PluginLoader::load_manifest(dir)
  ├→ try package.json → PackageJsonManifest → PluginManifest
  └→ try plugin.json → PluginManifest (existing path)
```

### 3.2 Hook Execution Flow

```
HookManager::from_settings(settings)
  ├→ try CC format → cc_adapter::adapt_cc_hooks(hooks_config)
  └→ fallback to existing flat format

HookManager::run_hooks(event, context)
  ├→ filter hooks by event type
  ├→ filter by matcher: hooks.iter().filter(|h| matches_matcher(&h.matcher, event, &context.tool_name))
  ├→ expand variables: expand_variables(hook.command, &context)
  └→ execute each hook
```

### 3.3 Marketplace Search & Install Flow

```
MarketplaceService::search(query)
  ├→ load known_marketplaces.json
  ├→ for each marketplace:
  │    ├→ ensure_cloned() — git clone --depth 1 if not cached
  │    └→ parse_marketplace_index(repo_path)
  │         └→ .claude-plugin/marketplace.json
  ├→ merge & filter by query
  └→ return results

MarketplaceService::install(plugin_name)
  ├→ find plugin in all marketplace indexes
  ├→ resolve source:
  │    ├→ LocalPath → cp to cache/
  │    ├→ GitSubdir → git clone + extract subdir
  │    └→ RemoteUrl → git clone
  ├→ write installed_plugins.json
  └→ return installed plugin info
```

## 4. Modified Existing Files

| 文件 | 改动 |
|------|------|
| `src/plugins/mod.rs` | PluginManifest 新增 publisher/install_path/git_commit_sha/source_format；PluginManager::load_all() 重写扫描逻辑 |
| `src/plugins/loader.rs` | load_manifest() 优先探测 package.json，新增 try_load_package_json() |
| `src/plugins/registry.rs` | 新增 InstalledPluginsRegistry/InstalledPluginEntry + load/save |
| `src/hooks/mod.rs` | HookEvent 新增 Stop/UserPromptSubmit/PermissionRequest；HookDefinition 新增 matcher/hook_type；matches_matcher() + expand_variables() |
| `src/config/mod.rs` | Settings 新增 enabled_plugins/plugin_marketplaces；load() 调用 CcConfigMapper |
| `src/services/plugin_marketplace.rs` | 重写为真实 marketplace 服务 |
| `src/services/mod.rs` | 注册 marketplace_resolver 模块 |

## 5. Backward Compatibility

所有改动向后兼容：
- `plugin.json` 格式继续工作（优先级低于 package.json）
- 扁平目录结构继续工作（CC cache 目录优先）
- 现有 hooks 配置格式继续工作（CC 格式优先解析，失败回退）
- 现有 `plugins.enabled`、`plugins.plugin_dir` 配置键继续工作
- `Settings` 所有新增字段使用 `#[serde(default)]`
- 无数据库迁移或配置格式强制升级

## 6. Testing Strategy

### 6.1 Unit Tests

| 测试目标 | 测试内容 |
|---------|---------|
| `package_json.rs` | 正常字段解析、@scope 前缀处理（`@anthropic/test` → name=test, publisher=anthropic）、author string/object 两种格式、main 字段映射、缺字段时的 default 行为、extra 未知字段捕获 |
| `cc_adapter.rs` | matcher `""` 全匹配、matcher `"ToolA\|ToolB"` 管道分隔、matcher 无匹配、Notification 子类型匹配、变量展开（`%tool%` / `%input%` 含特殊字符）、shell escape 单引号处理 |
| `cc_mapping.rs` | enabledPlugins 映射到 plugins.enabled_map、pluginMarketplaces 合并、CC 键名覆盖优先级、空配置不 panic |
| `marketplace_resolver.rs` | LocalPath 解析、GitSubdir 解析、RemoteUrl 解析、marketplace.json 反序列化、3 种 source 的 untagged enum 正确匹配 |

### 6.2 Integration Tests

| 测试目标 | 测试内容 |
|---------|---------|
| 完整加载流程 | 创建 mock `cache/<publisher>/<plugin>/<version>/package.json` 结构 → load_all() → 验证 manifest 正确加载 |
| 向后兼容 | 现有 `plugin.json` 格式插件 → load_all() → 验证仍然被加载 |
| CC 格式优先 | 同名插件两种格式都存在 → 验证 CC 格式覆盖 |
| Hook 扩展 | 包含 Stop/UserPromptSubmit 事件的 CC 格式 hooks 配置 → from_settings() → 验证正确解析和执行 |
| Marketplace E2E | 创建 local mock marketplace repo → 配置 known_marketplaces → search → install → 验证安装到 cache |

### 6.3 Manual Verification

- 配置 `claude-plugins-official` marketplace（`anthropics/claude-plugins-official`）
- `search` 一个真实插件（如 superpowers）
- `install` 并验证加载

## 7. Risks & Mitigations

| 风险 | 等级 | 缓解 |
|------|------|------|
| `package.json` 字段随 CC 版本变化 | Low | `#[serde(flatten)]` extra 捕获未知字段；`#[serde(default)]` 容错 |
| Git clone 超时/网络错误 | Medium | 30s timeout + 明确错误信息 + 缓存降级 |
| `git-subdir` 的 path 不存在 | Low | 安装失败并返回明确错误，不静默跳过 |
| Marketplace repo 格式不统一 | Medium | 先探测 `.claude-plugin/marketplace.json`，回退到目录扫描 |
| Hook 变量展开 shell 注入 | Medium | `shell_escape()` 单引号包裹 + 内部引号转义 |
| 两套目录结构增加维护复杂度 | Low | 扫描逻辑统一抽象，CC 格式优先的简单规则 |
| installed_plugins.json 并发写入损坏 | Low | Atomic write（tmp + rename） |

## 8. Open Questions

1. **CC 插件 `package.json` 中 `.opencode` 字段的完整 schema** — 需从实际插件逆推确认（如 superpowers）
2. **Marketplace repo 标准目录结构** — 需验证 `.claude-plugin/marketplace.json` vs 其他布局
3. **`enabledPlugins` 中 key 格式** — 确认是 `"plugin@publisher"` 还是 `"@publisher/plugin"` 格式

These will be resolved during implementation by inspecting actual CC plugin and marketplace repos.
