# multi-provider-models Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

---
change: multi-provider-models
design-doc: docs/superpowers/specs/2026-11-11-multi-provider-models-design.md
base-ref: e3c59ae51e32c5898715d387cb559113647e44d7
---

<!-- comet-task-authority: openspec/changes/multi-provider-models/tasks.md -->

**Goal:** 厂商分组模型配置（`models.providers`）+ 平铺 profiles 自动迁移 + session 级模型/权限覆盖 + 动态模型目录 + `GET /models` 原地升级两级结构 + web 输入框上方两级 chip。

**Architecture:** `ProviderGroup`/`ProviderModel` 为新配置真源，`active_model{provider,model}` 为持久激活态；`DaemonState::resolve_session_model` 合成全局+覆盖 settings，TaskTool 注入 resolver 闭包；`ModelCatalogService` 懒加载单飞合并三级目录；web `sessionManager` per-session 状态 + Composer chip 行。

**Tech Stack:** Rust（axum/tokio/serde）、web（React 18 + vitest + TS）。测试：cargo test（tokio::test + tempfile + serial_test）、vitest。

## Global Constraints

- 只 commit 不 push（用户明确要求）。
- commit 风格 `feat(scope): ...` / `test(scope): ...`，与 git log 现有风格一致。
- `Session` 新字段必须 `#[serde(default)]`（旧会话文件兼容）。
- Plan 权限模式不持久化：`permission_override` 写盘前 Plan→derived 归一化。
- `GET /models` 原地升级（无新 `/providers` 端点）；TUI 平铺投影 `vendor/model` 单行。
- 目录拉取失败不记 Authorization header 到日志。
- 全程 `cargo fmt` + `cargo clippy` + `npx tsc --noEmit` 干净。

---

### Task 1: ProviderGroup/ProviderModel schema + 反序列化

- [x] Task 1: ProviderGroup/ProviderModel schema + 反序列化 <!-- comet-task:c428d088-0c84-46cb-839c-bb129d22ead8 -->

**Files:**
- Modify: `src/config/models.rs`
- Test: `src/config/tests.rs`

**Interfaces:**
- Produces: `pub struct ProviderGroup { pub display_name: Option<String>, pub base_url: String, pub api_key: Option<String>, pub protocol: Option<String>, pub models: Vec<ProviderModel> }`
- Produces: `pub struct ProviderModel { pub name: String, pub tier: Option<ModelTier>, pub api_key: Option<String>, pub base_url: Option<String>, pub context_window: Option<usize>, pub temperature: Option<f32> }`
- Produces: `ModelsConfig` 新字段 `#[serde(default)] pub providers: HashMap<String, ProviderGroup>`、`#[serde(default)] pub active_model: Option<ActiveModelRef>`，`pub struct ActiveModelRef { pub provider: String, pub model: String }`（均 Clone + Serialize + Deserialize + Debug）

- **Step 1: 写失败测试**

```rust
#[test]
fn providers_deserialize_from_grouped_json() {
    let json = r#"{
        "providers": {
            "zhipu": {
                "display_name": "智谱",
                "base_url": "https://api.z.ai/api/coding/paas/v4",
                "api_key": "sk-z",
                "models": [
                    {"name": "glm-5.3"},
                    {"name": "glm-5.3-flash", "tier": "light"}
                ]
            }
        },
        "active_model": {"provider": "zhipu", "model": "glm-5.3"}
    }"#;
    let cfg: ModelsConfig = serde_json::from_str(json).expect("parse");
    let zg = cfg.providers.get("zhipu").expect("group");
    assert_eq!(zg.base_url, "https://api.z.ai/api/coding/paas/v4");
    assert_eq!(zg.models.len(), 2);
    assert_eq!(zg.models[1].tier, Some(ModelTier::Light));
    assert_eq!(cfg.active_model.as_ref().unwrap().model, "glm-5.3");
}

#[test]
fn providers_absent_keeps_old_config_parsing() {
    // 旧配置（无 providers 字段）反序列化不报错，providers 为空。
    let json = r#"{"main": {"name": "glm-5.3"}}"#;
    let cfg: ModelsConfig = serde_json::from_str(json).expect("legacy ok");
    assert!(cfg.providers.is_empty());
    assert!(cfg.active_model.is_none());
}
```

- **Step 2: 运行验证失败**

Run: `cargo test -p wgenty-code providers_ --lib 2>&1 | tail -5`
Expected: FAIL（`providers` 字段不存在/编译错误）

- **Step 3: 最小实现** — 在 `ModelsConfig` 上方新增两个 struct（字段全 `#[serde(default)]`，`base_url` 用 `#[serde(default)]` String；`ProviderModel.tier` 复用现有 `ModelTier` serde）；`ModelsConfig` 增两字段（位置：`profiles` 之后）。

- **Step 4: 运行验证通过**

Run: `cargo test -p wgenty-code providers_ --lib 2>&1 | tail -5`
Expected: 2 passed

- **Step 5: Commit** — `git add src/config/models.rs src/config/tests.rs && git commit -m "feat(config): ProviderGroup/ProviderModel schema for vendor-grouped models"`

---

### Task 2: profiles→providers 迁移（幂等 + .bak 备份 + active 映射）

- [x] 1.2 profiles→providers 迁移：按归一化 base_url host 聚合、内置 host→厂商名映射、endpoint 覆盖下放到模型条目、`active_profile`→`active_model` 映射；幂等 <!-- comet-task:5b1750e0-b863-4745-90d9-303549b44fef -->
- [x] 1.3 迁移写回：生成 `settings.json.bak-<date>` 一次性备份后删除旧 `profiles` 字段 <!-- comet-task:d1688e7f-8322-4476-8285-5fd82fa4b972 -->

**Files:**
- Modify: `src/config/models.rs`（`migrate_legacy` 或新 `migrate_providers`，load 路径调用）
- Modify: `src/config/mod.rs`（`Settings::load_from_disk` 写回前生成 `.bak`）
- Test: `src/config/tests.rs`

**Interfaces:**
- Consumes: Task 1 的 struct
- Produces: `impl ModelsConfig { pub fn migrate_providers(&mut self) }`；`pub fn vendor_for_host(host: &str) -> Option<&'static str>`（内置映射：api.kimi.com/moonshot→kimi、api.z.ai/bigmodel→zhipu、deepseek→deepseek、openai.com→openai、anthropic.com→anthropic）

- **Step 1: 写失败测试**（fixture 直接内联 JSON，形态取自真实 settings：kimi 与 glm 同 host 两 profile）

```rust
fn legacy_profiles_json() -> String {
    r#"{
        "main": {"name": "glm-5.3", "base_url": "https://api.z.ai/api/coding/paas/v4", "api_key": "sk-z"},
        "profiles": {
            "kimi": {"name": "k3", "base_url": "https://api.kimi.com/coding/v1", "api_key": "sk-k"},
            "glm": {"name": "glm-5.3", "base_url": "https://api.z.ai/api/coding/paas/v4", "api_key": "sk-z"},
            "glm-flash": {"name": "glm-5.3-flash", "base_url": "https://api.z.ai/api/coding/paas/v4", "api_key": "sk-z", "tier": "light"}
        },
        "active_profile": "glm"
    }"#.to_string()
}

#[test]
fn migrate_profiles_groups_by_host() {
    let mut cfg: ModelsConfig = serde_json::from_str(&legacy_profiles_json()).unwrap();
    cfg.migrate_providers().unwrap();
    assert_eq!(cfg.providers.len(), 2, "kimi + zhipu");
    let zg = cfg.providers.get("zhipu").expect("zhipu group");
    assert_eq!(zg.models.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["glm-5.3", "glm-5.3-flash"]);
    assert_eq!(zg.api_key.as_deref(), Some("sk-z"));
    assert_eq!(cfg.active_model, Some(ActiveModelRef{provider: "zhipu".into(), model: "glm-5.3".into()}));
    // 迁移后 profiles 置空（写回时不序列化旧结构）。
    assert!(cfg.profiles.is_empty());
}

#[test]
fn migrate_is_idempotent() {
    let mut cfg: ModelsConfig = serde_json::from_str(&legacy_profiles_json()).unwrap();
    cfg.migrate_providers().unwrap();
    let once = serde_json::to_string(&cfg.providers).unwrap();
    cfg.migrate_providers().unwrap();
    let twice = serde_json::to_string(&cfg.providers).unwrap();
    assert_eq!(once, twice);
}

#[test]
fn migrate_unknown_host_uses_host_as_group_name() {
    let mut cfg: ModelsConfig = serde_json::from_str(
        r#"{"profiles": {"x": {"name": "m", "base_url": "https://llm.internal.io/v1"}}}"#,
    ).unwrap();
    cfg.migrate_providers().unwrap();
    assert!(cfg.providers.contains_key("llm.internal.io"));
}
```

- **Step 2: 验证失败** — Run: `cargo test migrate_ --lib 2>&1 | tail -3`，Expected: FAIL

- **Step 3: 实现 `migrate_providers`** — 逻辑：`profiles` 空→no-op；非空且 `providers` 已存在→把 profile 按同规则合入现有组（同名 (group,model) 语义相同则跳过，端点/身份冲突则返回错误且保留旧文件）；否则新建组。分组：url parse host（失败用 "custom"），host 经 `vendor_for_host` 映射，未知用 host。每 profile → `ProviderModel{name, tier, api_key, base_url, context_window, temperature}`（仅当与组 base_url/key 不同的才下沉为 override：同值置 None）。组 `api_key` 取组内第一个 profile 的 key。`active_profile` 有值→查其在哪个组哪个模型→写 `active_model`，并同步 `models.main`（用现有字段拼 `ModelEndpoint`）。完成后 `profiles.clear()`、`small`/`planner` 已由现有 `migrate_legacy` 处理（保持其顺序：先 legacy 后 providers）。`Settings::load_from_disk`：检测到迁移发生（`profiles` 非空→空变化）且目标文件存在时，`std::fs::copy(settings_path, settings_path.with_extension(format!("json.bak-{}", date)))`（已存在同日 bak 不覆盖），再 `save()`。

- **Step 4: 验证通过** — Run: `cargo test migrate_ --lib 2>&1 | tail -3`，Expected: 3 passed

- **Step 5: Commit** — `test+feat(config): profiles→providers migration with idempotence and .bak backup`

---

### Task 3: key 解析链 + endpoint 合成 + 跨厂商 tier 归并

- [x] 1.4 `endpoint_for_tier` 改为跨厂商遍历 `providers` 模型 tier，`(provider, model)` 序确定性取首；`small` 遗留槽兼容并入 <!-- comet-task:3edc8cc8-ba64-41db-b944-8e4ad54f2576 -->
- [x] 1.5 API key 解析链：模型覆盖 > 厂商 key > 厂商 env（KIMI/ZHIPU/DEEPSEEK/ANTHROPIC/OPENAI）> 现有全局链 <!-- comet-task:33de6b00-3d3f-4f25-80f3-9fbec5a2b15a -->
- [x] 1.6 旧格式 fixture 迁移测试 + tier 选择不变断言 + key 解析链测试 <!-- comet-task:e25f4a2a-f162-40d2-95ee-6ac926a446dc -->

**Files:**
- Modify: `src/config/models.rs`
- Test: `src/config/tests.rs`

**Interfaces:**
- Produces: `impl ModelsConfig { pub fn resolve_endpoint(&self, provider: &str, model: &str) -> Option<ModelEndpoint> }`（组 base_url/key/protocol + 模型 override + env 链）
- Produces: `pub fn provider_env_key(vendor: &str) -> Option<String>`（kimi→KIMI_API_KEY、zhipu→ZHIPU_API_KEY、deepseek→DEEPSEEK_API_KEY、anthropic→ANTHROPIC_API_KEY、openai→OPENAI_API_KEY）
- Produces: `endpoint_for_tier` 重写：遍历 providers 全部模型，`(provider_key, model_name)` 字典序取首

- **Step 1: 写失败测试**

```rust
#[test]
fn resolve_endpoint_prefers_model_override_then_group_then_env() {
    // env 隔离：serial + 临时变量（仿 handlers/tests.rs 的 WGENTY_HOME 手法，用 std::env::set_var/remove_var + serial_test）
    // 1) 模型 override key 最高
    // 2) 组 key 次之
    // 3) 组 key None + ZHIPU_API_KEY set → env
    // 断言 ModelEndpoint.api_key / base_url / provider(protocol)
}

#[test]
fn endpoint_for_tier_cross_provider_deterministic() {
    // providers: b 组 m1(light), a 组 m2(light) → 选 a/m2（字典序 a<b）
    // 重复调用与 HashMap shuffle（insert 顺序颠倒重建）结果一致
}

#[test]
fn endpoint_for_tier_migration_preserves_choice() {
    // Task 2 fixture 迁移后 endpoint_for_tier(Light) 与迁移前
    // （profiles+small 时代：alphabetically-first "glm-flash"）一致
}
```

- **Step 2: 验证失败** — Run: `cargo test resolve_endpoint --lib; cargo test endpoint_for_tier --lib`，Expected: FAIL

- **Step 3: 实现三个函数**（`endpoint_api_key`/`endpoint_base_url` 现有方法保留为 `main` 专用；`resolve_endpoint` 是 providers 结构的解析入口）。`endpoint_for_tier`：`(providers 与 legacy small 迁移产物) 全模型收集 → filter tier → sort by (provider_key, model) → first → resolve_endpoint`；无匹配 → `main.clone()`。

- **Step 4: 验证通过** — Run: `cargo test -p wgenty-code --lib config:: 2>&1 | tail -3`，Expected: all pass

- **Step 5: Commit** — `feat(config): resolve_endpoint + env key chain + cross-provider tier routing`

---

### Task 4: Session 覆盖字段 + resolve_session_model

- [x] 2.1 `Session` 增 `model_override`/`permission_override` 字段（serde default；Plan 落盘归一化） <!-- comet-task:6ed41ead-46df-4615-8266-f194e3ab6269 -->
- [x] 2.2 `resolve_session_model(session_id) -> Settings`：全局 clone + overlay 合成（base_url/key/protocol 取自 ProviderGroup）；`run_session_turn` 接入 <!-- comet-task:1f25139c-db6a-4bed-90f1-693ae0d588c4 -->

**Files:**
- Modify: `src/context/memory_session.rs`（Session struct）
- Modify: `src/daemon/state.rs`（DaemonState 方法）
- Test: `src/daemon/state.rs`（内联 tests）或 `src/daemon/handlers/tests.rs`

**Interfaces:**
- Produces: `Session` 增 `#[serde(default, skip_serializing_if = "Option::is_none")] pub model_override: Option<crate::config::models::ActiveModelRef>`、`#[serde(default, skip_serializing_if = "Option::is_none")] pub permission_override: Option<crate::permissions::PermissionModeEntry>`
- Produces: `impl DaemonState { pub async fn resolve_session_model(&self, session_id: &str) -> Settings }`（无覆盖=全局 clone；有覆盖且 `resolve_endpoint` 成功→替换 `models.main`+`active_model`；解析失败→全局 clone + warn log）

- **Step 1: 写失败测试**

```rust
#[tokio::test]
async fn resolve_session_model_applies_override_without_global_write() {
    // tempdir settings + WGENTY_HOME 隔离（serial）
    // 1) 建会话，写 model_override=zhipu/glm-5.3-flash
    // 2) resolve_session_model(sid).models.main.name == "glm-5.3-flash"
    // 3) 全局 settings_handle 内 active_model 仍 None / main 不变
    // 4) 无覆盖会话 → 返回全局 clone
}

#[tokio::test]
async fn session_round_trips_override_fields() {
    // save→load 断言两字段往返；旧 JSON（无字段）load 得 None
}
```

- **Step 2: 验证失败** — Run: `cargo test resolve_session_model --lib`，Expected: FAIL

- **Step 3: 实现** — Session 字段 + `Session::with_model_override/with_permission_override` builder；DaemonState 方法（复用 `resolve_session` 查 session）。

- **Step 4: 验证通过** — Run: `cargo test resolve_session_model session_round_trips --lib`，Expected: 2 passed

- **Step 5: Commit** — `feat(daemon): session model_override + resolve_session_model overlay`

---

### Task 5: effective_permission + handler/TaskTool 接入

- [x] Task 5: effective_permission + handler/TaskTool 接入 <!-- comet-task:48b0ae28-cbe9-4955-a7fe-39cf687ec189 -->

**Files:**
- Modify: `src/daemon/state.rs`、`src/daemon/handlers/permissions.rs`、`src/daemon/run_loop.rs`（settings 来源换 `resolve_session_model`）
- Modify: `src/tools/meta/task.rs`（`build_permission_context` 加 `session_id: &str` 参数；新增 `session_settings_resolver: Option<Arc<dyn Fn(&str) -> Settings + Send + Sync>>` 字段 + `with_session_settings_resolver` builder；`execute_with_context` 内 `self.settings()` 调用点改为 resolver 优先）
- Modify: daemon 构造 TaskTool 处（`src/daemon/mod.rs` 或 state.rs 装配点）
- Test: `src/tools/meta/task/tests.rs`、`src/daemon/handlers/tests.rs`

**Interfaces:**
- Produces: `DaemonState::effective_permission(&self, session_id: &str) -> PermissionModeEntry`（session 覆盖 > permission_modes.get(root) > default）
- Produces: TaskTool `pub fn with_session_settings_resolver(mut self, r: Arc<dyn Fn(&str) -> Settings + Send + Sync>) -> Self`

- **Step 1: 写失败测试**

```rust
#[test]
fn tasktool_resolver_overrides_global_settings() {
    // 假 resolver 返回 main.name=="glm-5.3-flash" 的 Settings
    // 构造 TaskTool::new(...).with_session_settings_resolver(fake)
    // 断言 child_settings（通过 execute_with_context 的 settings() 等价路径或暴露测试钩子）
}

#[test]
fn tasktool_without_resolver_falls_back_to_global() { /* ... */ }

#[tokio::test]
async fn effective_permission_session_overrides_project() {
    // 同 root 两 session：A 覆盖 yolo、B 无覆盖且 project=accept_edits
    // effective_permission(A)==yolo, (B)==accept_edits
}
```

- **Step 2: 验证失败** — Run: `cargo test tasktool_resolver effective_permission --lib`，Expected: FAIL

- **Step 3: 实现**（permissions handler GET/POST 改走 `effective_permission`/写 Session 覆盖——POST 带 `session_id` 时优先写 session 层；`run_session_turn` 第 3 步 `let settings = state.resolve_session_model(session_id).await;`）。Plan 归一化：写 Session 覆盖前若 `effective_mode==Plan` → 存 derived。

- **Step 4: 验证通过 + 存量测试** — Run: `cargo test --workspace --lib 2>&1 | tail -3`，Expected: 全绿（含既有 permission/switch 测试迁移到新入口后仍过）

- **Step 5: Commit** — `feat(daemon,tools): session-scoped permission resolution + TaskTool settings resolver`

---

### Task 6: GET /models v2 + /model/switch 扩展 + 事件 session_id

- [x] 2.4 `POST /model/switch` 扩展：带 `session_id` 只写覆盖不落盘；`GET /models`（list）带 `session_id` 时 active 按合成值；`ModelChanged`/`ModeChanged` 广播携带 `session_id` <!-- comet-task:eb17ce24-aea4-4c82-9d79-ecbcaa707147 -->
- [x] 2.5 覆盖重置端点/参数（清除 session 的模型或模式覆盖） <!-- comet-task:bad50e0a-7fdc-4484-a640-4971984a4244 -->

**Files:**
- Modify: `src/daemon/handlers/system.rs`（list_models/switch_model）
- Modify: `src/daemon/models.rs`（请求/响应类型：`SwitchModelRequest{provider, model, session_id?}`；`ListModelsResponse` v2）
- Modify: `src/tui/client.rs` + `src/tui/components/model_picker.rs` + `src/tui/app/event.rs`（平铺投影 `vendor/model`）
- Test: `src/daemon/handlers/tests.rs`

**Interfaces:**
- Produces: v2 响应 `{"providers": [{"key","display_name","protocol","models":[{"name","tier","configured","active"}]}]}`；`?session_id=` 时 active 按合成值（覆盖所在 provider+model 双 active）
- Produces: `POST /model/switch` 带 session_id → 只写 Session.model_override + 广播（payload 含 session_id），不落盘全局；不带 → 设 active_model + models.main + 落盘（现行为）
- Produces: 重置 `POST /api/v1/model/reset` body `{session_id}` → 清 model_override

- **Step 1: 写失败测试**

```rust
#[tokio::test]
async fn list_models_v2_groups_and_marks_active() { /* 无 session_id：全局 active_model 标记 */ }

#[tokio::test]
async fn switch_model_with_session_writes_override_only() {
    // 带session_id switch → session.model_override 更新
    // 全局 settings.json 磁盘内容不变（读文件断言 active_model 仍 None/旧值）
    // 广播事件 payload 含 session_id
}

#[tokio::test]
async fn switch_model_without_session_preserves_global_path() { /* 现行为回归 */ }

#[tokio::test]
async fn model_reset_clears_override() { /* ... */ }
```

- **Step 2: 验证失败** — Run: `cargo test list_models_v2 switch_model_with_session model_reset --lib`，Expected: FAIL

- **Step 3: 实现**（list_models 数据源：providers（+目录合并，目录服务下一 task 注入，本 task 先手工+内置占位）→ 排序组按 key、模型按 name；TUI `ModelOption` 平铺：`format!("{}/{}", provider_key, model_name)`，label 用 display_name）。

- **Step 4: 验证通过 + TUI 编译** — Run: `cargo test --workspace --lib 2>&1 | tail -3 && cargo build 2>&1 | tail -2`，Expected: 全绿

- **Step 5: Commit** — `feat(daemon,tui): GET /models v2 vendor-grouped + session-scoped switch/reset + broadcast session_id`

---

### Task 7: ModelCatalogService（懒加载单飞 + 三级合并）

- [x] 3.1 `ModelCatalogService`：OpenAI 兼容 `GET /models` 拉取、内置目录静态表、手工条目合并去重（configured 标记）、TTL 5min 缓存、失败回退 <!-- comet-task:8e80d6ce-2b7c-4722-9e28-7b98a7d4981a -->
- [x] 3.2 `GET /api/v1/models` 原地合并（组列表 + 每组合并目录，不新增 `/providers`）；Anthropic 协议组跳过拉取 <!-- comet-task:21f95283-8be3-4448-b3d4-b79d6c8cb263 -->
- [x] 3.3 目录服务测试（mock 端点：成功合并/失败回退/缓存命中/跳过协议） <!-- comet-task:7f007b60-a899-4bfa-979f-664e1b6c9d54 -->

**Files:**
- Create: `src/daemon/catalog.rs`
- Modify: `src/daemon/mod.rs`（`pub mod catalog;`）、`src/daemon/state.rs`（DaemonState 持有 `catalog: ModelCatalogService`）、`src/daemon/handlers/system.rs`（list_models 注入合并结果）
- Test: `src/daemon/catalog.rs`（内联 tests，mock 用 `axum::test` 或 wiremock——仓库已有 reqwest；用 tokio::spawn 本地 axum 小服务做 mock 最贴合现状）

**Interfaces:**
- Produces: `pub struct ModelCatalogService { ... }`；`impl { pub fn new(http: reqwest::Client) -> Self; pub async fn merged(&self, settings: &Settings, provider_key: &str) -> Vec<MergedModel> }`
- Produces: `pub struct MergedModel { pub name: String, pub tier: Option<ModelTier>, pub configured: bool }`
- Produces: `pub const BUILTIN_CATALOGS: &[(&str, &[&str])]`（kimi/zhipu/deepseek/openai/anthropic 各 3-6 个）

- **Step 1: 写失败测试**（四场景：成功合并/失败回退/TTL 缓存命中/anthropic+无 key 跳过）

```rust
#[tokio::test]
async fn catalog_merges_fetched_builtin_and_manual() {
    // 本地 axum mock /models 返回 ["glm-5.3","glm-5.3-flash","new-model"]
    // 手工配 glm-5.3；内置目录含 glm-5.3-air
    // merged → 3 条（new-model configured=false, glm-5.3 configured=true, glm-5.3-air 来源 builtin）
}

#[tokio::test]
async fn catalog_failure_falls_back_and_caches() {
    // mock 500 → merged 仅手工+内置
    // 换 mock 200 后 TTL 内仍用缓存（断言不再请求，计数器 mock）
}
```

- **Step 2: 验证失败** — Run: `cargo test catalog_ --lib`，Expected: FAIL

- **Step 3: 实现**（`std::sync::RwLock<HashMap<String, CatalogEntry>>` + `tokio::sync::Mutex` 单飞 per provider；缓存绑定 base_url/有效协议/凭证指纹；`GET {base}/models` Bearer key；非 OpenAI 兼容协议或 key 未解析 → 跳过；TTL 300s；错误只 warn status）。

- **Step 4: 验证通过** — Run: `cargo test catalog_ --lib`，Expected: 4+ passed（最终 12 passed）

- **Step 5: Commit** — `feat(daemon): add merged provider model catalog` + `fix(daemon): align catalog protocol and cache identity`

---

### Task 8: web sessionManager per-session 状态 + API client

- [x] 4.1 `sessionManager`：`modelName`/`permissionMode` 全局单值改为 per-entry（effective + override 标记）；`ModelChanged`/`ModeChanged` 按 `session_id` 定向更新 <!-- comet-task:5082553f-2a36-497b-8a08-565d08a53c7f -->
- [x] 4.2 `api/client.ts`：`listModels(sessionId?)`、`switchModel({provider, model, sessionId?})`、`resetSessionModel`、`resetSessionMode` <!-- comet-task:264fd1ad-ea4e-466f-a635-41fce92820b0 -->

**Files:**
- Modify: `web/src/state/sessionManager.ts`、`web/src/api/client.ts`、`web/src/api/types.ts`
- Test: `web/src/state/sessionManager.test.ts`（新增/扩展）

**Interfaces:**
- Produces: `SessionEntry` 增 `model: {provider, model, isOverride} | null`、`mode: {mode: PermissionMode, isOverride} | null`
- Produces: client `listModels(sessionId?)`（v2 类型 `ProvidersResponse`）、`switchModel({provider, model, sessionId?})`、`resetSessionModel(sessionId)`、`resetSessionMode(sessionId)`
- 事件：`ModelChanged`/`ModeChanged` payload 按 `session_id` 写对应 entry

- **Step 1: 写失败测试**（vitest：事件定向更新 entry、顶层单值字段删除后无消费者崩溃）
- **Step 2: 验证失败** — Run: `cd web && npx vitest run src/state/sessionManager.test.ts 2>&1 | tail -5`，Expected: FAIL
- **Step 3: 实现** — types + client + sessionManager（`modelName`/`permissionMode` 顶层字段与 setter 删除，消费点本 task 先改为 `activeEntry?.model`，UI 组件下一 task 处理）；补 permission reset、override metadata、事件 seq 与请求代次防护
- **Step 4: 验证通过** — Run: `cd web && npx vitest run 2>&1 | tail -4`，Expected: 全绿（最终 275 passed）
- **Step 5: Commit** — `feat(web): per-session model and mode state` + 三轮经授权/scoped review 的 freshness/reconnect 修复

---

### Task 9: Composer chip 行 + 两级 picker + StatusBar 瘦身

- [x] 4.3 Composer 上方 chip 行：模型 chip（两级 picker：厂商→模型，「跟随默认」重置项）+ 模式 chip；busy 态收窄策略 <!-- comet-task:7bb70575-7926-4d98-aa93-490c2de6e626 -->
- [x] 4.4 StatusBar 瘦身：移除模型/模式控件，保留连接/审批/workspace/context <!-- comet-task:d0a62ddc-9938-4b50-99b5-02da3fcab309 -->
- [x] 4.5 web 测试：两级选择只影响当前 session、重置回退、chip 随会话切换同步、StatusBar 无模型/模式 <!-- comet-task:770fdfbc-b37c-4144-9449-cbfc9b8b8e9f -->

**Files:**
- Create: `web/src/features/chat/ModelModeChips.tsx`
- Modify: `web/src/App.tsx`（挂载 chip 行于 Composer 上方）、`web/src/components/StatusBar.tsx`（删两块控件）、`web/src/hooks/usePermissionModeSync.ts`
- Test: `web/src/features/chat/ModelModeChips.test.tsx`、`web/src/App.welcome.test.tsx`

**Interfaces:**
- Consumes: Task 8 的 entry 状态与 client 方法
- Produces: `<ModelModeChips client={client} />`（内部读 activeEntry；模型 chip 两级 popover + 「跟随默认」项；模式 chip 作用于 session；busy 态仅模式 chip——从 sessionStore 的 running 状态判定）

- **Step 1: 写失败测试**

```tsx
// 两级选择只写当前 session：mock client.switchModel 断言 sessionId 参数
// 重置项调用 resetSessionModel
// 切换会话 chip 显示各自 effective 值
// StatusBar 查询无 role=button name=/model|permission/i
```

- **Step 2: 验证失败** — Run: `cd web && npx vitest run src/features/chat/ModelModeChips.test.tsx 2>&1 | tail -5`，Expected: FAIL
- **Step 3: 实现**（chip 行样式对齐现有 Composer 上方 TurnStatus 条；popover 复用 StatusBar 现有下拉的样式模式）
- **Step 4: 验证通过 + tsc/eslint** — Web 289/289、tsc 与 scoped ESLint 通过；全量 ESLint 的 4 个既有基线错误交由 Task 10 处理
- **Step 5: Commit** — `4c31ee95` + 审查修复 `a5a69a13`，仅本地 commit

---

### Task 10: 集成验收 + fmt/clippy

- [x] Task 10: 集成验收 + fmt/clippy <!-- comet-task:f191ac35-abeb-44c7-9b9c-3d1d9ed63161 -->

- **Step 1: 全量测试** — 最终候选 Rust 2284 passed / 0 failed / 4 ignored；Web 293/293 passed
- **Step 2: fmt/clippy/tsc/eslint** — fmt、严格 clippy、tsc、全量 ESLint 与 diff check 均通过
- **Step 3: 手动冒烟脚本** — 可选项未执行（not-run）；自动化 handler、迁移和 session 行为验收通过，未访问真实厂商
- **Step 4: Commit** — `16a6e307` + 审查修复 `56620264`，仅本地 commit

## Self-Review 结论

- Spec 覆盖：provider-model-config（T1-3）、model-catalog-fetch（T7）、session-scoped-model-mode（T4-6）、model-mode-input-chips（T8-9）——4 spec 全部场景有对应任务；验收场景 5（目录失败回退）在 T7。
- 占位符：无 TBD；测试代码块具体。
- 类型一致性：`ActiveModelRef`/`ProviderGroup`/`MergedModel`/`with_session_settings_resolver` 各任务间签名一致。
