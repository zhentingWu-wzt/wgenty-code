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
