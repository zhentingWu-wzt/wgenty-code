# plugin-marketplace Specification

## Purpose
TBD - created by archiving change cc-ecosystem-compat. Update Purpose after archive.
## Requirements
### Requirement: REQ-PM-001 — known_marketplaces.json

MUST MUST 必须支持 `known_marketplaces.json` 配置格式（source.repo → installLocation）。

#### Scenario: Load known marketplaces
- GIVEN `known_marketplaces.json` 包含一个 marketplace 条目
- WHEN `load_known_marketplaces()` 被调用
- THEN 返回包含 marketplace 名称、repo URL、installLocation 的条目

### Requirement: REQ-PM-002 — git clone marketplace

MUST 首次使用时自动 `git clone --depth 1` marketplace repo。

#### Scenario: First-time clone
- GIVEN marketplace 尚未克隆到本地
- WHEN 首次 search/install
- THEN `git clone --depth 1` 被执行到 installLocation

### Requirement: REQ-PM-003 — parse marketplace index

MUST MUST 必须从 marketplace repo 中解析插件索引（`.claude-plugin/marketplace.json`）。

#### Scenario: Parse index JSON
- GIVEN cloned repo 包含 `.claude-plugin/marketplace.json`
- WHEN `parse_marketplace_index()` 被调用
- THEN 返回 `MarketplaceIndex` 包含插件列表

### Requirement: REQ-PM-004 — real marketplace search

`search(query)` MUST MUST 必须返回真实 marketplace 数据（替换硬编码示例）。

#### Scenario: Search real marketplace
- GIVEN known_marketplaces.json 配置了 marketplace
- WHEN `search("superpowers")` 被调用
- THEN 从克隆的 marketplace index 返回匹配结果

### Requirement: REQ-PM-005 — install to cache

`install(name)` MUST MUST 必须从 marketplace 找到插件源 URL 并 git clone 到标准 `cache/<publisher>/<plugin>/<version>/` 路径。

#### Scenario: Install plugin from marketplace
- GIVEN marketplace index 包含 "my-plugin" 条目
- WHEN `install("my-plugin")` 被调用
- THEN 插件被克隆/复制到 `cache/<publisher>/my-plugin/<version>/`

### Requirement: REQ-PM-006 — auto-update support

MUST 支持 marketplace 自动更新（`git pull` 检查新版本）。

#### Scenario: Refresh marketplace
- GIVEN marketplace 已克隆
- WHEN 更新触发
- THEN `git pull` 拉取最新 marketplace 数据

### Requirement: REQ-PM-007 — three source types

`install()` MUST MUST 必须处理 marketplace entry 的三种 `source` 类型——`LocalPath`（marketplace 本地子目录）、`GitSource`（`{"source":"git-subdir","url":"...","path":"..."}` 格式）、`RemoteUrl`（`{"source":"url","url":"..."}` 格式）——每种都正确安装到 `cache/<publisher>/<plugin>/<version>/` 路径，解析使用 `#[serde(untagged)]` enum 自动匹配。

#### Scenario: Local path source
- GIVEN 插件 source 为 `"./plugins/my-plugin"` (LocalPath)
- WHEN install 执行
- THEN 从 marketplace repo 子目录复制到 cache

#### Scenario: Git-subdir source
- GIVEN 插件 source 为 `{"source": "git-subdir", "url": "...", "path": "plugins/x", "ref": "main"}`
- WHEN install 执行
- THEN `git clone --branch main <url>` 到 cache 目录

#### Scenario: URL source
- GIVEN 插件 source 为 `{"source": "url", "url": "https://github.com/user/repo"}`
- WHEN install 执行
- THEN `git clone --depth 1 <url>` 到 cache 目录

