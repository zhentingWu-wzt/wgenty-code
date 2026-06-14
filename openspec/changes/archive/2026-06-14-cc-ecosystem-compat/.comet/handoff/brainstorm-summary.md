# Brainstorm Summary

- Change: cc-ecosystem-compat
- Date: 2026-06-14

## 确认的技术方案

**架构**：适配器层模式（方案 A）。在外部 CC 格式入口处做转换，内部 wgenty-code 类型最小化改动。

**四个适配器组件**（4 个新文件）：
1. `src/plugins/package_json.rs` — `package.json` → `PluginManifest` 映射
2. `src/hooks/cc_adapter.rs` — CC hooks 嵌套数组格式 → 内部 `Vec<HookDefinition>`
3. `src/services/marketplace_resolver.rs` — 3 种 marketplace source 类型解析 + Git 操作
4. `src/config/cc_mapping.rs` — CC 配置键名 → 内部字段映射

**核心实现细节**：
- `package.json` 解析：优先探测 `package.json`，回退到 `plugin.json`；name 处理 `@scope/` 前缀；额外字段通过 `#[serde(flatten)]` 捕获
- 注册表：`installed_plugins.json`（version 2, plugins 按 `name@publisher` 索引，含 installPath/version/gitCommitSha）
- 目录结构：`cache/<publisher>/<plugin>/<version>/package.json` + 扁平目录共存，CC 格式优先
- Hook 事件：新增 `Stop`, `UserPromptSubmit`, `PermissionRequest` 三个 HookEvent 变体
- Matcher：支持 `""` 全匹配 + `"ToolA|ToolB"` 管道分隔 + Notification 子类型匹配
- 变量展开：`%tool%` → 工具名，`%input%` → escape 后的 JSON
- Marketplace：`known_marketplaces.json` → git clone --depth 1 → 解析 `.claude-plugin/marketplace.json`
- 3 种 source 类型：`"./plugins/x"`（本地） / `git-subdir`（sparse） / `url`（独立 repo）
- 配置映射：`enabledPlugins` + `pluginMarketplaces` 加载后映射，CC 键名优先

## 关键取舍与风险

| 取舍 | 理由 |
|------|------|
| 适配器层而非内部重构 | 改动量最小（~200行新代码），向后兼容最安全 |
| CC 格式优先于 wgenty-code 原有格式 | 替代 CLI 定位，用户期望 CC 行为 |
| `git clone --depth 1` 而非 HTTP API | CC 生态基于 Git，无需额外服务端 |
| 同名插件 CC 格式覆盖扁平格式 | 避免混淆，明确优先级 |

| 风险 | 缓解 |
|------|------|
| `package.json` 字段可能随 CC 版本变化 | `#[serde(flatten)]` extra 捕获未知字段 |
| Git clone 超时/网络错误 | 30s timeout + 明确错误信息 + 缓存降级 |
| `git-subdir` 的 path 不存在 | 安装失败并返回明确错误，不静默跳过 |

## 测试策略

- **单元测试**：package.json 解析（正常字段/缺字段/@scope前缀）、matcher 匹配（空/单/管道分隔/通知子类型）、变量展开（含特殊字符/shell注入防护）
- **集成测试**：完整 CC 格式插件安装→加载流程（mock marketplace repo）、向后兼容验证（旧 plugin.json 继续工作）
- **手动验证**：配置 claude-plugins-official marketplace，搜索+安装一个真实插件

## Spec Patch

回写 `specs/plugin-marketplace/spec.md`，新增验收场景：
- **REQ-PM-007**: `install()` 必须处理 marketplace entry 的三种 `source` 类型——`LocalPath`（marketplace 本地子目录）、`git-subdir`（Git repo 子目录）、`url`（独立 Git repo）——每种都正确安装到 `cache/<publisher>/<plugin>/<version>/` 路径
