---
comet_change: multi-provider-models
role: technical-design
canonical_spec: openspec
archived-with: 2026-11-19-multi-provider-models
status: final
---

# Technical Design: multi-provider-models

上游需求事实源：`openspec/changes/multi-provider-models/`（proposal / specs / design.md）。本文档是实现级技术设计，与其冲突时以 OpenSpec 为准。

## 1. 总体架构

```
settings.json                          Session 记录（memory_session.rs）
models.providers = {                   model_override: Option<{provider, model}>
  <vendor>: {                          permission_override: Option<PermissionModeEntry>
    display_name?, base_url, api_key?,          │
    protocol?, models: [                        ▼
      {name, tier?, api_key?, base_url?,   DaemonState::resolve_session_model(sid)
       context_window?, temperature?}      全局 Settings clone + overlay 合成
    ]                                    }     （endpoint 字段取自 ProviderGroup）
}                                              │
      │                                        ▼
      ▼                              ┌─ run_session_turn（轮次边界读一次）
ModelCatalogService                 └─ TaskTool.execute_with_context
  懒加载 · TTL 5min · 单飞               resolver(session_id) → child_settings
  GET {base}/models（openai 兼容）        tier 路由在覆盖厂商范围内解析
  内置目录 + 手工条目合并去重
```

权限模式解析链：`effective_permission(session_id)` = session 覆盖 > `PermissionModeStore`（project/worktree 层原样保留）> Normal。

## 2. 组件设计

### 2.1 配置层（src/config/models.rs）

```rust
pub struct ProviderGroup {
    pub display_name: Option<String>,
    pub base_url: String,
    pub api_key: Option<String>,
    pub protocol: Option<String>,           // 缺省 = detect_provider(base_url)
    pub models: Vec<ProviderModel>,
}
pub struct ProviderModel {
    pub name: String,                       // API model id
    pub tier: Option<ModelTier>,
    pub api_key: Option<String>,            // 覆盖厂商级
    pub base_url: Option<String>,           // 覆盖厂商级
    pub context_window: Option<usize>,
    pub temperature: Option<f32>,
}
// ModelsConfig 增：providers: HashMap<String, ProviderGroup>
//              增：active_model: Option<ActiveModelRef{provider, model}>
//              旧 profiles/small/planner 迁移期保留读取，迁移后删除
```

- **迁移**（load 时，幂等）：存在 `profiles` 且无 `providers` → 按归一化 base_url host 聚合；内置 host→厂商名映射（api.kimi.com→kimi、api.z.ai/bigmodel.cn→zhipu、api.deepseek.com→deepseek、api.openai.com→openai、api.anthropic.com→anthropic，未知 host 用 host 作组名）；profile 的 endpoint 覆盖落入 `ProviderModel`；`active_profile` → `active_model`。写回前生成一次 `settings.json.bak-<date>`，随后删除 `profiles`/`small`/`planner`。手工再添 `profiles`（下次 load 重新触发迁移合入，同名模型不重复）。
- **key 解析**（每模型）：`ProviderModel.api_key > ProviderGroup.api_key > env_map[vendor] > 现有全局链`。env_map：kimi→KIMI_API_KEY、zhipu→ZHIPU_API_KEY、deepseek→DEEPSEEK_API_KEY、anthropic→ANTHROPIC_API_KEY、openai→OPENAI_API_KEY。
- **endpoint 合成**：`resolve_endpoint(provider, model) -> ModelEndpoint`（运行时缓存 `models.main` 由它填充，`active_model` 为持久真源——对齐 model-profiles-single-source 的方向）。
- **tier 归并**：`endpoint_for_tier` 改为遍历全部 `providers` 的模型，`(provider_key, model_name)` 字典序确定性取首；保留 legacy `small` → Light 的迁移兼容（迁移期一次性转成 tier 声明）。

### 2.2 Daemon 层

- `DaemonState::resolve_session_model(&self, session_id) -> Settings`：读全局 handle clone → `resolve_session` 查 `model_override` → 有则 `models.main = resolve_endpoint(provider, model)`（含 key/base_url/protocol 解析与 context_window/temperature）。返回的 Settings 只用于本次轮次/spawn，不写回。
- `DaemonState::effective_permission(&self, session_id) -> PermissionModeEntry`：session 覆盖 > `permission_modes.get(&effective_session_root)` > default。GET/POST `/permission-mode` handler 与 `run_session_turn`、TaskTool `build_permission_context` 全部改走此入口（`build_permission_context` 签名加 `session_id: &str`）。
- `run_session_turn` 第 3 步：`let settings = state.resolve_session_model(session_id).await`。
- TaskTool：新增字段 `session_settings_resolver: Option<Arc<dyn Fn(&str) -> Settings + Send + Sync>>`；daemon 构造时闭包捕获 `Arc<DaemonState>`（`state.clone()` + `resolve_session_model`）。`execute_with_context` 内 `self.settings()` 调用点改为「resolver 结果 ?? 全局 fallback」。测试注入假 resolver。
- `POST /model/switch`：body `{provider, model, session_id?}`。有 `session_id` → 写 Session.`model_override` 并广播（不落全局 settings.json）；无 → 迁移/设置全局 `active_model` + `models.main` 同步 + 落盘（现行为等价）。
- `GET /models`（原地升级，响应 v2）：

```json
{ "providers": [ { "key": "zhipu", "display_name": "智谱", "protocol": "openai",
    "models": [ {"name": "glm-5.3", "tier": null, "configured": true, "active": true} ] } ] }
```

  `?session_id=` 时 active 按合成值标（覆盖所在 provider/model 组内唯二 active）。无 `session_id` 时按全局 `active_model` 标。目录数据由 ModelCatalogService 合并注入（未拉取/失败时为手工+内置）。
- 重置：`DELETE /api/v1/session-model-override`（或 POST 语义化路径，实现时取一致风格）清 `model_override`；权限覆盖复用 `POST /permission-mode` 传 `reset: true` 或独立参数清空。

### 2.3 目录服务（新模块 src/api/catalog.rs 或 src/daemon/catalog.rs）

`ModelCatalogService { inner: DashMap<String, ProviderCatalog> }`；`ProviderCatalog { fetched: Option<Vec<String>>, fetched_at: Instant> }`。`merged(provider) -> Vec<MergedModel>`：

1. 手工 `ProviderModel`（`configured: true`，属性取条目）
2. 拉取结果（openai 兼容 `GET {base}/models`，Bearer key；非 openai 协议或 key 未解析 → 跳过；5min TTL；in-flight `tokio::sync::Mutex` 单飞）
3. 内置静态目录（`BUILTIN_CATALOGS: &[(&str, &[&str])]`，首版每厂商 3-6 个常用模型）

按 name 去重，手工 > 拉取 > 内置。失败回退上一级可用来源；全部不可用时仅手工。

### 2.4 Web 层

- `sessionManager`：`entries[id]` 增 `model: {provider, model, isOverride}` / `mode: {mode, isOverride}`（effective + override 标记）；顶层 `modelName`/`permissionMode` 单值字段删除，消费点改从 active entry 读。`ModelChanged`/`ModeChanged` 事件按 `session_id` 定向写 entry。
- `api/client.ts`：`listModels(sessionId?)` 适配 v2 结构；`switchModel({provider, model, sessionId?})`；`resetSessionModel(sessionId)`。
- Composer 上方 `ModelModeChips` 组件：模型 chip（点开两级 popover：厂商列表 → 模型列表 + 「跟随默认」项）+ 模式 chip（现下拉逻辑迁移，作用于 session）。busy 态收窄为模式 chip。`StatusBar.tsx` 删除两块控件及其状态依赖。
- `usePermissionModeSync`：拉取/订阅改为按活跃 session 的 effective 值。

## 3. 关键取舍

| 取舍 | 理由 |
|---|---|
| `GET /models` 原地升级（弃兼容投影） | 单端点无双轨；TUI 平铺投影适配一次即可 |
| resolver 闭包注入 TaskTool 而非传 DaemonState | TaskTool 不依赖 daemon 类型；测试可注入假实现 |
| 目录懒加载单飞，不启动预热 | 不拖慢启动、不无谓限流；首屏延迟一个请求可接受 |
| tier 挂模型条目、跨厂商归并 | tier 本质是模型属性；沿用 alphabetically-first 确定性语义（映射为 (provider,model) 序） |
| resolver 不加 session→Settings 缓存 | spawn 频率下 `resolve_session` 一次查找可接受；YAGNI，量大再加 DashMap |

## 4. 风险与缓解

- [迁移删 profiles 不可逆] → `.bak-<date>` 一次性备份 + fixture 测试覆盖 kimi/zhipu 真实形态 + 幂等重跑断言。
- [tier 路由漂移] → 迁移测试断言旧配置迁移前后 `endpoint_for_tier` 选择不变。
- [GET /models v2 破坏旧 web/TUI 同时部署窗口] → daemon 与 web 同仓同版发布，无独立部署窗口；TUI 适配列入本 change 任务。
- [目录拉取暴露 key 到日志] → 拉取失败日志只记 status/错误类别，不记 header。
- [session 覆盖与轮次中切换竞态] → 覆盖仅在轮次边界/spawn 边界读取；广播事件滞后一轮可接受（spec 已约束）。

## 5. 测试策略

- **config**：旧格式 fixture 迁移（多 profile 共享 host / active_profile 映射 / endpoint 覆盖下放 / 幂等 / small→light 兼容）；key 解析链 5 级顺序；`endpoint_for_tier` 跨厂商确定性 + 迁移不变断言。
- **daemon**：`resolve_session_model` fallback/覆盖/不落盘；`effective_permission` 三级链；`/model/switch` 带/不带 session_id 两路径 + 事件 payload；重置端点；Plan 不持久化（session 文件断言）。
- **catalog**：mock server 四场景（成功合并、失败回退、TTL 内缓存命中、anthropic/无 key 跳过）。
- **TaskTool**：假 resolver 断言 child_settings 取覆盖值；无 resolver 时回退全局（向后兼容测试）。
- **web**：vitest——chip 两级选择只写当前 session、重置后回默认、会话切换 chip 同步、StatusBar 断言无模型/模式控件、事件按 session 定向更新。

## 6. 开放问题（可延后）

- Kimi/GLM `resolve_model_id` 友好名映射：待真实调用验证后按需补（不改结构）。
- 内置目录首版收录清单：实现时定（每厂商 3-6 个）。
- 重置端点的 URL 风格（DELETE vs POST）：实现时对齐现有路由风格。
