# Proposal: multi-provider-models

## Why

模型配置目前是平铺的 `models.profiles`：每条 profile 重复填写同一厂商的 `base_url` 和 `api_key`（例如 Kimi 与智谱各需为旗下的多个模型各建一条 profile），切换时在一个不分级的长列表里盲选。同时模型与权限模式均为全局/project 粒度，多会话并行时无法让不同会话使用不同模型与权限档位。

## What Changes

- **厂商分组配置 schema**：`settings.json` 新增 `models.providers`（厂商 → `base_url` + `api_key` + 模型列表），厂商级单 key；支持 `KIMI_API_KEY` / `ZHIPU_API_KEY` / `DEEPSEEK_API_KEY` 等环境变量覆盖。旧 `models.profiles` 启动时自动迁移为厂商分组，迁移幂等且不丢手工配置。
- **两级模型选择（web）**：输入框上方新增厂商 → 模型两级选择 chip；权限模式选择器一并从底部 StatusBar 迁移到输入框上方。底部 StatusBar 不再展示模型/权限模式。
- **session 级模型与权限模式**：模型切换与权限模式支持 session 覆盖（Session 记录新增 `Option` 字段），解析链 session 覆盖 > 全局默认（模型）/ project 级（权限模式）> Normal；提供一键重置回默认层。`Plan` 模式不持久化到 session 覆盖。
- **动态模型目录**：按厂商拉取可用模型列表（list-models 端点），失败回退内置目录；手工配置的模型始终可见。
- **事件与 API 适配**：`ModelChanged` / `ModeChanged` 广播携带 `session_id`；`/api/v1/model/switch` 与 list-models 支持按 session 路由。

### Non-Goals

- TUI 的两级 picker 与 chip 布局改造（另开 change）；TUI `/model` 仅适配新 schema 数据源保持可用。
- 不引入新的 API 协议实现（Kimi/智谱走现有 OpenAI 兼容通道；是否注册专属 Provider 实现由 design 阶段决定）。
- 不做模型用量计费/配额管理。

## Capabilities

### New Capabilities

- `provider-model-config`: 厂商分组模型配置——`models.providers` schema、环境变量 key 解析、旧 profiles 自动迁移与兼容。
- `model-catalog-fetch`: 动态模型目录——按厂商拉取模型列表、内置目录回退、缓存与合并规则。
- `session-scoped-model-mode`: 模型与权限模式的 session 级覆盖——Session 字段、fallback 解析链、重置语义、事件广播 `session_id`。
- `model-mode-input-chips`: web 输入框上方厂商→模型两级 chip 与权限模式 chip 的展示与交互。

### Modified Capabilities

（无——现有 specs 中无模型配置/切换或权限模式切换粒度的 capability；`gui-app-shell`/`gui-chat` 的 StatusBar 行为变化随新 capability `model-mode-input-chips` 的 spec 一并约束。）

## Impact

- **配置**：`src/config/models.rs`（新 `ProviderGroup` schema、迁移、`endpoint_for_tier` 跨厂商 tier 语义）、`src/config/tests.rs`。
- **API 层**：`src/api/provider.rs`（厂商注册）、新增模型目录拉取模块。
- **Daemon**：`src/daemon/handlers/system.rs`（switch/list-models session 路由）、`permissions.rs`（session 层查询）、`src/daemon/run_loop.rs`（turn 边界解析 session 模型）、`src/daemon/state.rs`（解析入口）、`src/tools/meta/task.rs`（subagent spawn 快照按 session 查询）、`src/permissions/mode_store.rs`（session 层）。
- **Web**：`sessionManager`（per-session 模型/模式状态）、Composer 上方 chip 组件、两级 picker、`StatusBar.tsx` 瘦身、`usePermissionModeSync` 适配。
- **迁移风险**：旧 `settings.json` 自动迁移（写回前保留原字段直至迁移验证通过）；`Session` 新字段 `#[serde(default)]` 向后兼容旧会话文件。
