# Design: multi-provider-models

## Context

当前 `ModelsConfig` 为 `main` + `small`/`planner` 遗留槽 + 平铺 `profiles: HashMap<String, ModelEndpoint>`，`active_profile` 是全局唯一 key（`src/config/models.rs`）。`ModelEndpoint` 自带 `base_url`/`api_key`/`provider`/`tier`，同厂商多模型时连接信息逐条重复。`ApiClient::with_clients` 从 `settings.models.main` 解析 provider 与鉴权（`src/api/mod.rs`）。权限模式按 canonical root 存于 `permission-modes.json`（`PermissionModeStore`），GET/POST 已强制 `session_id`。web 底部 StatusBar 承载模型名与模式 picker（`web/src/components/StatusBar.tsx`）。

## Goals / Non-Goals

**Goals:**
- 厂商分组成为模型配置的一级维度；旧配置无损迁移
- 两级选择与 session 级覆盖在 web 端完整落地
- 现有 tier 路由、subagent 快照、429 退避等行为在新结构下语义保持

**Non-Goals:**
- TUI 两级 picker / chip 布局（另 change）。本 change 仅做 TUI `/model` 的**数据源适配**：`GET /models` 原地升级为两级结构后，TUI picker 平铺投影 `vendor/model` 单行展示，保持可用（经 design 阶段确认为方案乙的组成部分）。
- 新 API 协议实现；模型用量计费

## Decisions

### D1: schema — `providers` 为新真源，`profiles` 迁移后移除

`models.providers: HashMap<String, ProviderGroup>`，`ProviderGroup { display_name, base_url, api_key, models: Vec<ProviderModel> }`，`ProviderModel { name, tier?, api_key?, base_url?, context_window?, temperature? … }`。全局 `active` 改为 `active_model: { provider, model }`。

- 迁移：load 时若存在 `profiles` 且无 `providers`，按归一化 base_url host 聚合生成厂商组（host 作默认组名，已知 host 映射内置名 kimi/zhipu/deepseek/openai/anthropic）；profile 的 endpoint 级覆盖落入 `ProviderModel`。幂等：已迁移则跳过；迁移结果写回磁盘时**同时删除旧 `profiles` 字段**（用户后续手工加的 profiles 在下次 load 再触发迁移路径合入，满足 spec「不丢失」）。
- 备选「保留 profiles 双读」被否：双真源永久并存，tier 归并与两级展示都要处理冲突，复杂度高于一次性迁移；`active_profile` → `active_model` 映射在迁移时一次性完成。

### D2: 协议 provider 与配置厂商分离

`ProviderGroup` 增加可选 `protocol`（默认按 base_url 探测，即现 `detect_provider` 行为）。Kimi/智谱不注册专属 Provider trait 实现——其差异（模型名映射、reasoning 回传）目前不存在或未证实，先用 openai-compat 通道；`resolve_model_id` 的厂商特殊映射按需后补，不阻塞本次。env key 映射表 `KIMI_API_KEY/ZHIPU_API_KEY/DEEPSEEK_API_KEY/ANTHROPIC_API_KEY/OPENAI_API_KEY` 内置于 ProviderGroup 解析。

### D3: session 覆盖存 Session 记录，运行时「全局 settings + overlay」合成

`Session` 增 `model_override: Option<{provider, model}>`、`permission_override: Option<PermissionModeEntry>`（`#[serde(default)]`）。daemon 增 `resolve_session_model(session_id) -> Settings`：clone 全局 settings → 若有覆盖则替换 `models.main`（base_url/key/protocol 取自 ProviderGroup）→ 返回。`run_session_turn` 第 3 步改用该入口；TaskTool 的 `self.settings()` 注入 overlay 后的 settings，subagent tier 路由自然在覆盖厂商范围内解析（`endpoint_for_tier` 改为遍历 `providers` 下所有声明 tier 的模型，跨厂商按 `(provider, model)` 序确定性取首）。权限查询入口统一为 `effective_permission(session_id)`：session 覆盖 > `PermissionModeStore`（project/worktree 层不动）> Normal。

- 备选「独立 `HashMap<SessionId, _>` store」被否：需自写 GC 与持久化，且 Session 记录已有完整生命周期管理。
- `switch_model` API：带 `session_id` 时只写 Session 覆盖（不落全局 settings.json）；不带时维持现全局语义。`list_models` 带 `session_id` 时 active 标记按合成值。

### D4: 模型目录三级合并 + TTL 缓存

daemon 侧 `ModelCatalogService`：`fetch(provider) -> Vec<model>` 走 OpenAI 兼容 `GET {base}/models`；结果与内置目录（代码内静态表，随版本更新）、手工 `ProviderModel` 条目按模型名去重合并，手工条目优先并标记 `configured: true`。缓存每 provider 一份，TTL 5 分钟，失败沿用旧值。Anthropic 协议组跳过拉取。新增 `GET /api/v1/providers`（组列表+每组合并目录）与 `POST /api/v1/model/switch` 扩展（接收 `{provider, model, session_id?}`）。

### D5: web — per-session 状态 + Composer 上方 chip 行

`sessionManager` 的 `modelName`/`permissionMode` 全局单值改为 per-entry（`entries[id].modelOverride/effectiveModel/modeOverride/effectiveMode`），`ModelChanged`/`ModeChanged` 事件按 `session_id` 定向更新。`Composer` 上方新增 chip 行组件（idle 态两 chip；busy 态收窄为模式 chip，沿 TUI status.rs 先例）。StatusBar 移除模型/模式控件。重置入口位于两级 picker 内（「跟随默认」项）。

## Risks / Trade-offs

- [迁移写回删 profiles，用户误操作无回退] → 写回前把原 `models` 段备份为 `settings.json.bak-<date>` 一次；迁移有覆盖单测（旧格式 fixture）。
- [动态目录拉取在 daemon 启动即触发，拖慢启动或限流] → 懒加载：首次 `GET /providers` 请求时触发 + TTL 缓存；启动不拉。
- [tier 跨厂商归并改变现有路由结果] → 迁移单测断言旧配置迁移后 `endpoint_for_tier` 选择不变（alphabetically-first 语义映射到 `(provider, model)` 序）。
- [session 覆盖与 mid-turn 切换竞态] → 覆盖在轮次边界读取（同现状）；事件广播滞后一轮可接受。
- [Plan 不持久化] → `permission_override` 落盘前归一化（Plan → derived），与 mode_store 现有规则一致。

## Migration Plan

1. 配置迁移独立成第一批任务，带旧格式 fixture 测试，先行合入（纯后向兼容，无 UI 依赖）。
2. daemon API/session 覆盖 → web chip → 目录拉取依序落地；每步全量测试绿后进入下一步。
3. 回滚：schema 迁移不可逆但留有 `.bak`；其余步骤按提交粒度回滚。

## Open Questions

- Kimi `resolve_model_id` 是否需要友好名映射（如 k2 → kimi-k2）：待实际调用验证，可后补，不影响结构。
- 内置目录首版收录范围（各厂商收录几个常用模型）：实现时定。
