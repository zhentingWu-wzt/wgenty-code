# Settings 结构重构设计

**Date:** 2026-06-16
**Status:** Design (awaiting review)
**Author:** brainstorming session

## 1. 背景与问题

`src/config/mod.rs` 的 `Settings` 当前是一个 ~50 字段的扁平结构体，已经长到难以阅读和维护。具体的"散"体现在：

1. **散户字段**：`rlm_jaccard_threshold` 应在 `rlm` 子组里却留在顶层；`default_subagent_token_budget_k` 既属 subagent 又属 token 预算定位模糊。
2. **概念上对称、写法上不对称**：main / small / planner 三个模型概念平行（"换一个模型 + 可选地换它的 url/key"），但分别写成 11 个不同字段（main 用顶层 `model` + `api`，small 用 `small_model_*` 四件套含 `appkey`，planner 用 `planner_model_*` 三件套）。
3. **同质字段平铺顶层**：5 个 `include_*_instructions` 布尔与 3 个相关内容字段一起摊在顶层。
4. **plugin 配置散在三处**：`plugins` 子结构 + 顶层 `enabled_plugins` + 顶层 `plugin_marketplaces`（后两个是 CC 兼容别名）。
5. **历史兼容代码混入主结构**：`migrate_rlm_settings`、`#[serde(alias = "enabledPlugins")]` 等让 `Settings` 兼具"运行时配置"和"兼容层"两种职责。
6. **`set()` 是字符串 dispatch**：~80 行手写 match，每加一个字段都要在 struct + Default + set + 测试四处一起改。

## 2. 范围与非目标

**In scope**:
- 重新组织 `Settings` 顶层结构：按功能领域分成 6 个子组 + 1 个顶层裸字段 (`verbose`)。
- 同步重写 `Settings::set()` 为 dotted-path 通用 setter。
- 同步更新所有读访问点（约 120 处字段读取）和 1 处 `Settings::set` 调用（`src/cli/args.rs:294`）。
- 保留并搬迁全部既有子结构（`MemorySettings` / `VoiceSettings` / `PluginSettings` / `GuardianSettings` / `RlmSettings` / `ApiConfig` / `McpConfig`）的内部字段定义（仅整体位置和外层包装会变）。

**Out of scope** (明确不做):
- **向后兼容旧 `~/.wgenty-code/settings.json`**。本次明确选择不兼容方案——旧 JSON 加载会因字段未知而失败，用户需手动重写或删除文件让默认配置重新生成。
- **保留 CC 兼容别名 (`enabledPlugins` / `pluginMarketplaces`)**。这些 alias 整体删除；如果将来仍需要 CC 兼容层，应作为独立的输入适配器实现，不再混入主 `Settings`。
- **`migrate_rlm_settings` 迁移函数**。整体删除（含相关单元测试 `test_migrate_rlm_legacy_keys`、`test_migrate_rlm_no_override_when_group_present`）。
- **新增运行时功能（除 §3.3a 所述的 subagent 继承机制以外）**。本次除了重组现有字段，还引入一项功能性扩展：subagent 可独立覆盖部分主 agent 配置字段，未覆盖项继承主 agent。详见 §3.3a。

## 3. 目标新结构

### 3.1 顶层

```rust
pub struct Settings {
    pub models:       ModelsConfig,
    pub agent:        AgentConfig,
    pub prompt:       PromptConfig,
    pub plugins:      PluginsConfig,
    pub storage:      StorageConfig,
    pub integrations: IntegrationsConfig,
    pub verbose:      bool,
}
```

`verbose` 留在顶层是有意为之：它跨子系统影响日志，硬塞进任意子组都更糟，单个顶层裸布尔不构成"散"。

### 3.2 `models`

将 `ApiConfig` 拆成两层：传输参数与端点凭据，使三套模型对称。

```rust
pub struct ModelsConfig {
    pub transport: TransportConfig,
    pub main:      ModelEndpoint,           // 必填
    pub small:     Option<ModelEndpoint>,   // None = 不启用
    pub planner:   Option<ModelEndpoint>,   // None = 不启用
}

pub struct TransportConfig {
    pub max_tokens:   usize,
    pub timeout:      u64,
    pub streaming:    bool,
    pub beta_headers: Vec<String>,
}

pub struct ModelEndpoint {
    pub name:     String,
    pub base_url: Option<String>,   // None on small/planner = 继承 main
    pub api_key:  Option<String>,   // None on small/planner = 继承 main
    pub appkey:   Option<String>,   // 仅在某些 provider 下使用，所有 endpoint 都可选
}
```

字段映射（旧 → 新）：
- `model` → `models.main.name`
- `api.base_url` / `api.api_key` → `models.main.base_url` / `models.main.api_key`
- `api.max_tokens` / `api.timeout` / `api.streaming` / `api.beta_headers` → `models.transport.*`
- `small_model` / `small_model_base_url` / `small_model_api_key` / `small_model_appkey` → `models.small.{name, base_url, api_key, appkey}`
- `planner_model*` → `models.planner.*`（无 `appkey` 字段在 small 上独有的特性被打消，所有 endpoint 一律可选 `appkey`）

`small_model_settings()` 方法逻辑不变（仍然产出一个临时 `Settings` 给 small model 使用），但实现改为：先 clone，再用 `models.small`（如果存在）覆盖 `models.main` 上对应字段；若 small 字段缺省，保持 main 值（与现状语义等价）。

### 3.3 `agent`

```rust
pub struct AgentConfig {
    pub plan_mode:    bool,
    pub max_rounds:   Option<usize>,
    pub token_budget: TokenBudget,
    pub subagent:     SubagentLimits,
    pub rlm:          RlmSettings,
}

pub struct TokenBudget {
    pub main_k:             usize,   // 0 = unlimited
    pub subagent_default_k: usize,   // 0 = unlimited
}

pub struct SubagentLimits {
    // —— 硬限额（subagent 独有，主 agent 没有对应物，不参与继承）——
    pub max_depth:      usize,
    pub max_concurrent: usize,
    pub timeout_secs:   u64,

    // —— 覆盖位（None = 继承主 agent 同名/同义字段；Some = subagent 独立设置）——
    // 详细继承规则见 §3.3a
    pub token_budget_k: Option<usize>,                  // 继承自 agent.token_budget.main_k
    pub max_rounds:     Option<usize>,                  // 继承自 agent.max_rounds，0 = 不限
    pub plan_mode:      Option<bool>,                   // 继承自 agent.plan_mode
    pub rlm:            SubagentRlmOverride,
    pub prompt:         SubagentPromptOverride,
}

#[derive(Default)]
pub struct SubagentRlmOverride {
    pub enabled:           Option<bool>,
    pub delegate_tool:     Option<bool>,
    pub auto_routing:      Option<bool>,
    pub retry_enabled:     Option<bool>,
    pub max_replan_cycles: Option<usize>,
    pub jaccard_threshold: Option<f64>,
}

#[derive(Default)]
pub struct SubagentPromptOverride {
    pub include: SubagentPromptIncludesOverride,
    pub developer_instructions:  Option<String>,
    pub collaboration_mode:      Option<String>,
    pub model_instructions_file: Option<String>,
}

#[derive(Default)]
pub struct SubagentPromptIncludesOverride {
    pub permissions:   Option<bool>,
    pub developer:     Option<bool>,
    pub collaboration: Option<bool>,
    pub environment:   Option<bool>,
    pub skills:        Option<bool>,
}

pub struct RlmSettings {
    pub enabled:           bool,
    pub delegate_tool:     bool,
    pub auto_routing:      bool,
    pub retry_enabled:     bool,
    pub max_replan_cycles: usize,
    pub jaccard_threshold: f64,   // 原 rlm_jaccard_threshold 散户归位
}
```

字段映射：
- `plan_mode` → `agent.plan_mode`
- `max_rounds` → `agent.max_rounds`
- `token_budget_k` → `agent.token_budget.main_k`
- `default_subagent_token_budget_k` → `agent.token_budget.subagent_default_k`
- `max_subagent_depth` → `agent.subagent.max_depth`
- `max_concurrent_subagents` → `agent.subagent.max_concurrent`
- `subagent_timeout_secs` → `agent.subagent.timeout_secs`
- `rlm.*` 整体保留，新增 `rlm.jaccard_threshold`（原顶层 `rlm_jaccard_threshold`）
- 新增覆盖字段：`agent.subagent.{token_budget_k, max_rounds, plan_mode, rlm.*, prompt.**}`——这些字段在旧 Settings 里**不存在**，是本次新增的功能（继承机制见 §3.3a）。

### 3.3a Subagent 继承机制

**目标**：subagent 跑起来时，未显式覆盖的字段自动继承主 agent 的同名/同义字段。用户什么都不写时，subagent 行为完全等同于主 agent；用户只在想区别对待 subagent 时才写覆盖项。

**继承映射表**（覆盖字段 → 主 agent 源字段）：

| `agent.subagent.X` | 当 `None` 时回退到 | 当 `Some(v)` 时使用 |
|---|---|---|
| `token_budget_k` | `agent.token_budget.main_k` | `v` |
| `max_rounds` | `agent.max_rounds`（注：主端类型 `Option<usize>`，约定 `0` ≡ `None`，下方说明） | `Some(v)` |
| `plan_mode` | `agent.plan_mode` | `v` |
| `rlm.enabled` | `agent.rlm.enabled` | `v` |
| `rlm.delegate_tool` | `agent.rlm.delegate_tool` | `v` |
| `rlm.auto_routing` | `agent.rlm.auto_routing` | `v` |
| `rlm.retry_enabled` | `agent.rlm.retry_enabled` | `v` |
| `rlm.max_replan_cycles` | `agent.rlm.max_replan_cycles` | `v` |
| `rlm.jaccard_threshold` | `agent.rlm.jaccard_threshold` | `v` |
| `prompt.include.permissions` | `prompt.include.permissions` | `v` |
| `prompt.include.developer` | `prompt.include.developer` | `v` |
| `prompt.include.collaboration` | `prompt.include.collaboration` | `v` |
| `prompt.include.environment` | `prompt.include.environment` | `v` |
| `prompt.include.skills` | `prompt.include.skills` | `v` |
| `prompt.developer_instructions` | `prompt.developer_instructions` | `Some(v)` |
| `prompt.collaboration_mode` | `prompt.collaboration_mode` | `Some(v)` |
| `prompt.model_instructions_file` | `prompt.model_instructions_file` | `Some(v)` |

**关于 `max_rounds` 的类型选择**：主 agent 的 `agent.max_rounds: Option<usize>` 中 `None` 表示"使用内部默认 100"。subagent 覆盖位本可写成 `Option<Option<usize>>`（外层 `None` = 继承，内层 `None` = 不限）但可读性差。本设计选择简化：覆盖位类型为 `Option<usize>`，约定 `Some(0)` ≡ "不限" / `None` ≡ "继承"。这与 `token_budget_k` 现状语义一致（0 = unlimited）。在 spawn subagent 解析配置时，遇到 subagent 端 `Some(0)` 时按"不限"处理（等价于主端的 `None`），不再设上限。

**解析时机**：subagent 配置解析发生在 spawn subagent 时（`task` tool 等触发点），而不是 `Settings::load` 时。`Settings` 结构始终保存原始覆盖位（`Option<...>`），主 agent 启动时使用 `agent.*` 直接路径——不参与合并；subagent 启动时按上表临时构造一份"effective config"传给子 agent loop。这避免了"merge 后回填到 Settings"的副作用，也使得 `set("agent.subagent.rlm.enabled", "false")` 这类 dotted-path 操作语义清晰：它就是设置覆盖位，不会污染主 agent 字段。

**不参与继承的字段**（subagent 不能覆盖）：
- `models.*`（subagent 用不同模型由 `models.small` 接住，不在 subagent override 里重复）
- `models.transport.*`（YAGNI）
- `plugins.*` / `storage.*` / `integrations.*` / `verbose`（全局生效）
- `agent.subagent.*` 自身（防止递归二级嵌套）
- `agent.token_budget.subagent_default_k`（这是"主 agent 启动子 agent 时给的默认预算"，与 subagent 自己的 `token_budget_k` 是不同概念，详见下）

**`token_budget_k` 与 `subagent_default_k` 的区分**：
- `agent.token_budget.subagent_default_k` 由**主 agent** 在 spawn subagent 时读取，作为"如果 subagent 自己没指定预算，给它分这么多"的默认。
- `agent.subagent.token_budget_k` 是**subagent 自己读取**的覆盖位——如果用户在 settings 里显式给了，subagent 启动时使用此值；否则继承 `main_k`。

实际 effective budget 解析顺序（由 spawn 逻辑实现，不是 `Settings` 结构关心的）：
1. 调用方在 `task` 工具调用里显式指定（最优先）
2. `agent.subagent.token_budget_k` 若 `Some` 则用之
3. `agent.token_budget.subagent_default_k` 若 `> 0` 则用之
4. 否则继承 `agent.token_budget.main_k`

### 3.4 `prompt`

```rust
pub struct PromptConfig {
    pub include:                 PromptIncludes,
    pub developer_instructions:  Option<String>,
    pub collaboration_mode:      Option<String>,
    pub model_instructions_file: Option<String>,
}

pub struct PromptIncludes {
    pub permissions:   bool,
    pub developer:     bool,
    pub collaboration: bool,
    pub environment:   bool,
    pub skills:        bool,
}
```

字段映射：
- `include_permissions_instructions` → `prompt.include.permissions`
- `include_developer_instructions` → `prompt.include.developer`
- `include_collaboration_instructions` → `prompt.include.collaboration`
- `include_environment_context` → `prompt.include.environment`
- `include_skill_instructions` → `prompt.include.skills`
- `developer_instructions` / `collaboration_mode` / `model_instructions_file` → 同名落到 `prompt.*`

### 3.5 `plugins`

```rust
pub struct PluginsConfig {
    pub enabled:      bool,
    pub dir:          PathBuf,
    pub auto_update:  bool,
    pub enabled_map:  HashMap<String, bool>,
    pub marketplaces: Option<serde_json::Value>,
}
```

字段映射：
- `plugins.{enabled, plugin_dir, auto_update, enabled_map}` → `plugins.{enabled, dir, auto_update, enabled_map}`（去 `plugin_` 前缀）
- 顶层 `enabled_plugins` (CC alias `enabledPlugins`) → **删除**。原本"CC alias 优先于 `enabled_map`"的合并规则一并移除；用户需直接写 `plugins.enabled_map`。
- 顶层 `plugin_marketplaces` (CC alias `pluginMarketplaces`) → `plugins.marketplaces`

### 3.6 `storage`

```rust
pub struct StorageConfig {
    pub working_dir: PathBuf,
    pub memory:      MemorySettings,
    pub transcript:  TranscriptConfig,
}

pub struct TranscriptConfig {
    pub db_path:      String,
    pub max_age_days: u32,
}

// MemorySettings 字段不变：
pub struct MemorySettings {
    pub enabled:                bool,
    pub path:                   PathBuf,
    pub consolidation_interval: u64,
    pub max_memories:           usize,
}
```

字段映射：
- `working_dir` → `storage.working_dir`
- `memory.*` → `storage.memory.*`（内部字段不变）
- `transcript_db_path` → `storage.transcript.db_path`
- `max_transcript_age_days` → `storage.transcript.max_age_days`

### 3.7 `integrations`

```rust
pub struct IntegrationsConfig {
    pub mcp_servers: Vec<McpConfig>,
    pub hooks:       Option<serde_json::Value>,
    pub voice:       VoiceSettings,
    pub guardian:    GuardianSettings,
}
```

`McpConfig` / `VoiceSettings` / `GuardianSettings` 内部字段不变。

字段映射：`mcp_servers` / `hooks` / `voice` / `guardian` 各自顶层字段 → `integrations.*`。

## 4. `Settings::set()` 改成 dotted-path 通用 setter

### 4.1 现状

`set(key, value)` 是 ~80 行的手写 match：每个字段一行，加新字段要改 4 处（struct、Default、set、测试），还混着 legacy alias (`rlm_retry_enabled`)、CC 路径前缀匹配 (`enabledPlugins.*`、`pluginMarketplaces.*`)。

### 4.2 新实现

```rust
pub fn set(key: &str, value: &str) -> anyhow::Result<()> {
    // 1. load → serialize 当前 Settings 为 serde_json::Value
    // 2. 解析 dotted-path key（如 "models.main.name"）为 JSON pointer
    // 3. 解析 value 为 JSON 字面量；解析失败时回退为字符串
    // 4. 用 pointer 在 Value 上定位并写入
    // 5. 反序列化 Value 回 Settings（serde 验证字段存在性、类型正确性）
    // 6. 调用 settings.save()
}
```

要点：

- **Key 路径用 `.` 分隔，对应 struct 字段路径**：`models.main.name`、`agent.token_budget.main_k`、`prompt.include.developer`。
- **类型校验由 serde 完成**：写入未知字段或类型不符时，最终 `serde_json::from_value::<Settings>` 会报错，set 返回 `Err`，已写入的 `~/.wgenty-code/settings.json` 不变（save 在反序列化成功之后）。
- **HashMap 的 key 也通过 dotted path**：`plugins.enabled_map.foo@bar = true` → 在 `plugins.enabled_map` 下设 key `foo@bar`。注意 `@` 不是分隔符，仅 `.` 是。如果 plugin 名本身含 `.`，本期不支持（将用户已有约定 `name@publisher` 视为合理假设；遇到含 `.` 的极端情况退化为不支持，set 返回 `Err`）。
- **删除所有 legacy alias 分支** (`rlm_retry_enabled` / `rlm_max_replan_cycles` / `enabledPlugins.*` / `pluginMarketplaces.*`)。
- **Value 解析策略**：先尝试 `serde_json::from_str`（让 `"true"` / `"42"` / `"3.14"` / `"[1,2]"` 走 JSON 字面量），失败则当作字符串。CLI 现状是把命令行传入的 value 当字符串透传，因此布尔/数字字段需要保证 JSON 解析能识别 `"true"` 这类输入——这是合理的、能被现有 1 个调用点 (`src/cli/args.rs:294`) 接受的行为。

### 4.3 调用方

`src/cli/args.rs:294` 是 `Settings::set` 的唯一调用点，调用形式为 `Settings::set(key, value)`。它接收用户在 CLI 上输入的 `key=value`，调用 set 时不知道字段类型——所以"字符串 → JSON 字面量探测"的解析策略对它是兼容的。

## 5. 加载/保存与默认值

### 5.1 加载

```rust
impl Settings {
    pub fn load() -> anyhow::Result<Self> {
        let path = Self::config_path();
        if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            Ok(serde_json::from_str(&content)?)   // 旧格式直接报错
        } else {
            let s = Settings::default();
            s.save()?;
            Ok(s)
        }
    }
}
```

不再调用 `migrate_rlm_settings` 或 `cc_mapping::CcConfigMapper::apply_mappings`。

**关于 `cc_mapping::CcConfigMapper`**：必须从 `Settings::load` 链路上移除（这是硬性要求，不在实现期争议）。该模块本身的去留按下列规则定：

- 如果模块的全部职责就是"把 CC 别名键映射到主 Settings 字段"（即配合现已删除的 `enabledPlugins` / `pluginMarketplaces` 等 alias 工作），则**整体删除该文件**及 `mod.rs` 中的 `pub mod cc_mapping;` 声明。
- 如果模块还承担了独立的、非 alias 的功能（例如读取 `.claude/settings.local.json` 这类独立文件、或被其它模块引用），则**保留为独立模块**，但 `Settings::load` 不得调用它，调用入口需上移到具体的功能调用点（由调用方自己负责调用 CC 适配）。

实现期需先 `grep -rn "cc_mapping\|CcConfigMapper"` 看清调用面后选择上述两条之一执行，禁止保持现状（即不允许"暂时保留并仍然在 `Settings::load` 中调用"）。

### 5.2 保存

`save()` 行为不变（写 pretty JSON 到 `~/.wgenty-code/settings.json`），但产出的 JSON 形状是新的嵌套结构。

### 5.3 `Settings::default`

每个子组提供 `impl Default`，`Settings::default()` 由各子组 default 聚合而成。新 default 数值与现状一致（`models.main.name = "sonnet"`、`agent.subagent.max_depth = 3`、`agent.subagent.max_concurrent = 5`、`agent.subagent.timeout_secs = 240`、`agent.token_budget.{main_k, subagent_default_k} = 0`、`agent.rlm.jaccard_threshold = 0.8`、`agent.rlm.max_replan_cycles = 2`、`prompt.include.*` 全 true、`integrations.guardian.{enabled, auto_deny_critical} = true, llm_review = false`、`storage.transcript.{db_path = ~/.wgenty-code/subagent_transcripts.db, max_age_days = 30}` 等）。

**Subagent 覆盖位的默认值**：所有 `Option<T>` 字段默认 `None`，所有 `SubagentXxxOverride` 子结构通过 `#[derive(Default)]` 默认全部为 `None`。语义即"什么都不写时，subagent 行为完全等同于主 agent"。这本身就是合理且无意外的默认——本次重构对"合理默认值"的回答是：**让继承位的"无设置"等价于"完全继承主 agent"**，而不是给每个覆盖位编造一个独立的硬编码默认。

## 6. 影响面

### 6.1 项目内字段读访问点

约 **120 处** Rust 代码读取 `settings.X` 字段（`grep` 统计）。每处都需按 3.2-3.7 的字段映射改路径。这是机械工作但量大，实现期需要分文件提交。

### 6.2 单元测试

`src/config/mod.rs` 末尾的现有 6 个测试中：
- 删除：`test_migrate_rlm_legacy_keys`、`test_migrate_rlm_no_override_when_group_present`（迁移函数被删）。
- 保留并改路径：`test_rlm_settings_default_all_enabled`、`test_rlm_settings_deserialize_partial`、`test_rlm_settings_deserialize_full`、`test_settings_default_includes_rlm`、`test_rlm_deserialize_in_settings`（最后一个的内嵌 JSON 需重写为新结构）。

新增测试：
- `set()` dotted-path：嵌套字段、HashMap 字段、未知 key、类型不符。
- `models` 三套对称：small/planner 缺省字段时继承 main 的解析结构。
- `prompt.include` 全 5 个布尔在 default 下为 true。
- **subagent 覆盖位默认全 None**：`Settings::default().agent.subagent.{token_budget_k, max_rounds, plan_mode}` 都是 `None`；`SubagentRlmOverride::default()` 与 `SubagentPromptOverride::default()` 所有字段 `None`。
- **subagent effective config 解析**：覆盖位为 `None` 时使用主 agent 字段；为 `Some` 时使用覆盖值。覆盖 `agent.subagent.rlm.enabled = false` 不影响主 agent 的 `agent.rlm.enabled`。
- **`max_rounds = Some(0)` 解析为"不限"**（subagent 端语义）。
- **`token_budget_k` 解析顺序**：调用方显式 > `agent.subagent.token_budget_k` > `agent.token_budget.subagent_default_k` > `agent.token_budget.main_k`（spawn 逻辑测试，不是 `Settings` 结构测试，但放在 spec 里以确保实现期不漏）。

### 6.3 文档与示例

- `~/.wgenty-code/settings.json` 在用户机器上的样例（README / docs）需更新为新结构。
- 任何文档里出现的旧字段名（如 `rlm_jaccard_threshold`、`include_developer_instructions`、`enabledPlugins`）需替换。

### 6.4 用户影响

- 老用户启动会因 `serde_json::from_str::<Settings>` 报"未知字段"失败。错误提示由 `load()` 抛出的 `anyhow::Error` 携带——本设计**不要求**额外的"友好升级提示"；如果运维上希望，下游可包一层错误处理建议用户重写或删除文件。
- 用户的 CLI 习惯 `wgenty-code config set X Y` 的 X 全部要改路径（如 `model` → `models.main.name`）。

## 7. 风险与缓解

| 风险 | 缓解 |
|------|------|
| 120 处读访问点遗漏 | 实现期先全量替换字段路径，再 `cargo build` 让编译器找出所有遗漏（`Settings` 的旧字段编译期不再存在，遗漏点必然编译失败）。这是"D 不兼容"路线的最大优点：编译器替我们守住了正确性。 |
| `set()` dotted-path 的 HashMap key 冲突 (`plugins.enabled_map.<plugin>`，plugin 名含 `.`) | 现状 plugin 命名约定是 `name@publisher`，不含 `.`。本期文档化此假设；若未来需要支持，再扩展 set syntax (`plugins.enabled_map['name.with.dot']`)。 |
| `cc_mapping::CcConfigMapper` 残留逻辑被忽略 | 实现计划期必须读这个模块全文，明确其用途后决定整体删除 / 保留为独立模块。本设计禁止它继续在 `Settings::load` 链路上被调用。 |
| `small_model_settings()` 行为微调 | 该方法当前仅基于 4 个 small_* 字段做覆盖，新结构下改为基于 `models.small`（如果 `Some`）覆盖。语义等价，但要补一个测试覆盖"small 缺省 base_url 时仍走 main"。 |

## 8. 实现顺序建议（非强制）

1. 先把 6 个子组的 struct 定义全部建立起来（含 `Default`），让 `Settings::default()` 能编过。
2. 删 `migrate_rlm_settings`、`CcConfigMapper::apply_mappings` 调用、所有 `#[serde(alias)]`。
3. 重写 `set()` 为 dotted-path。
4. `cargo build` 让编译器列出所有读访问点编译错误，按文件分批修复 + commit。
5. 更新单元测试。
6. 验证：手动删除 `~/.wgenty-code/settings.json`，重启生成新格式，逐项确认 default 行为。

## 9. 决策记录

本设计在 brainstorming 中明确做出了以下选择：

- **不向后兼容**（用户：D）：旧 settings.json 加载会报错，用户需重写或删除。
- **顶层按功能领域分组**（用户：A → 方案 1）：6 子组 + 1 顶层裸字段。
- **`models` 用 transport + endpoint 两层**（用户：Y）：三套模型对称。
- **`token_budget` 单独成子组**（用户：A → 挑法 b）：`{ main_k, subagent_default_k }` 配对。
- **`agent.subagent` 不再细分 `limits`**（用户：A）：3 平字段直接放 subagent 下。
- **`prompt` 用"开关组 + 内容平铺"**（用户：A → 方式 1）：5 个 include 布尔聚组，3 个内容字段平铺。
- **`set()` 改 dotted-path**（用户：B）：通用 setter，删除手写 dispatch。
- **新增 subagent 字段继承机制**（用户：B → 推荐方案 A）：subagent 可独立覆盖部分主 agent 配置字段，覆盖位类型为 `Option<T>`，默认 `None` 即"完全继承主 agent"。覆盖范围限定于 `token_budget_k` / `max_rounds` / `plan_mode` / `rlm.*` / `prompt.include.*` / `prompt.{developer_instructions, collaboration_mode, model_instructions_file}`。`models.*` / `transport.*` / `plugins.*` / `storage.*` / `integrations.*` / `verbose` / `agent.subagent.{max_depth, max_concurrent, timeout_secs, default}` 不参与继承。
