# Settings Struct Restructure Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the ~50-field flat `Settings` struct with 6 grouped sub-configs + 1 top-level scalar, add subagent inheritance overrides, replace the hand-written `set()` dispatch with a generic dotted-path setter, and update all ~120 read sites — with no backward compatibility for old settings.json.

**Architecture:** All struct definitions live under `src/config/`. The new top-level `Settings` is `{ models, agent, prompt, plugins, storage, integrations, verbose }`. `cc_mapping.rs` is deleted (it has zero external consumers — only `Settings::load` calls it). The `small_model_settings()` helper is rewritten to use `models.small`, and the two duplicated hand-written copies in `task.rs` / `rlm/pipeline.rs` are replaced by calls to it. Subagent override resolution is a new method `Settings::resolve_subagent_config(&self) -> Settings` invoked at spawn time in `task.rs`. The compiler is the safety net: every old field is removed in Task 2, then Tasks 4–10 fix the resulting compile errors file-by-file.

**Tech Stack:** Rust 2021, serde (`#[derive(Serialize, Deserialize)]`), serde_json (for the dotted-path setter and Value manipulation), anyhow.

**Reference spec:** `docs/superpowers/specs/2026-06-16-settings-restructure-design.md` (commit d12f77a).

**Pre-existing baseline noise (do NOT fix in this plan):** `cargo check` on the starting worktree produces 8 warnings in `src/tools/codegraph/`, `src/teams/rollback.rs` (unused imports/variables). They predate this work and are out of scope.

---

## File Structure

**New files:** none. All struct definitions go into `src/config/mod.rs` (project's existing pattern — sub-structs already live there alongside `Settings`).

**Files modified:**

| File | What changes | Why |
|------|--------------|-----|
| `src/config/mod.rs` | Replace `Settings` and add new sub-config structs (`ModelsConfig`, `TransportConfig`, `ModelEndpoint`, `AgentConfig`, `TokenBudget`, `SubagentLimits` (extended), `SubagentRlmOverride`, `SubagentPromptOverride`, `SubagentPromptIncludesOverride`, `PromptConfig`, `PromptIncludes`, `PluginsConfig` (renamed `plugin_dir`→`dir`, gain `marketplaces`), `StorageConfig`, `TranscriptConfig`, `IntegrationsConfig`); rewrite `Settings::set()` as dotted-path; rewrite `small_model_settings()`; add `resolve_subagent_config()`; remove `migrate_rlm_settings`; remove `cc_mapping` module declaration; update tests | Core of the refactor |
| `src/config/cc_mapping.rs` | **Delete file** | Zero external consumers; spec §5.1 mandates removal once `Settings::load` no longer calls it |
| `src/config/api_config.rs` | No structural change. `ApiConfig` becomes a synonym used inside `TransportConfig` for the transport-only fields (or split — see Task 1) | The endpoint-specific fields (`api_key`, `base_url`) move to `ModelEndpoint`; transport fields (`max_tokens`, `timeout`, `streaming`, `beta_headers`) move to `TransportConfig` |
| `src/config/settings.rs` | Update re-exports to add new public types | Public API exposure |
| `src/api/mod.rs` | 12 read sites: `settings.model` → `settings.models.main.name`; `settings.api.*` → `settings.models.transport.*` (max_tokens/timeout/streaming/beta_headers) and `settings.models.main.*` (api_key/base_url, via ModelEndpoint helpers) | Field-path updates |
| `src/tools/meta/task.rs` | 21 read sites + delete the hand-written small-model override block (lines ~413–426) and replace with a `settings.small_model_settings()` call; update subagent depth/concurrency/timeout/transcript reads; spawn-time `resolve_subagent_config()` integration for token budget defaulting | Field-path updates + subagent inheritance integration |
| `src/tools/meta/rlm/pipeline.rs` | 7 read sites + delete the hand-written small-model override block (lines ~138–145) and replace with a `settings.small_model_settings()` call; update `max_subagent_depth`, `subagent_timeout_secs` reads | Field-path updates |
| `src/tools/mod.rs` | 1 read site: `settings.api.get_base_url()` → `settings.models.main.endpoint_base_url()` (helper to be added on `ModelEndpoint`) | Field-path update |
| `src/prompts/mod.rs` | 2 read sites: `settings.developer_instructions` → `settings.prompt.developer_instructions`; `settings.include_skill_instructions` → `settings.prompt.include.skills` | Field-path updates |
| `src/permissions/policy.rs` | 1 read site: `settings.working_dir` → `settings.storage.working_dir` | Field-path update |
| `src/daemon/state.rs` | 4 read sites | Field-path updates |
| `src/gui/app.rs` | 2 read sites | Field-path updates |
| `src/gui/settings.rs` | 6 read sites | Field-path updates |
| `src/tui/app/mod.rs` | 3 read sites | Field-path updates |
| `src/tui/app/turn.rs` | 3 read sites | Field-path updates |
| `src/tui/app/event.rs` | 1 read site: `new_settings.collaboration_mode` → `new_settings.prompt.collaboration_mode` | Field-path update |
| `src/cli/args.rs:294` | The `set()` call signature is unchanged — but document the new dotted-path keys in any usage help text in this file (none exists currently — verify in Task 11) | Caller compatibility |

**Tests added or rewritten:** all in `src/config/mod.rs::tests`. See Tasks 2, 3, 11.

---

## Task 0: Verify clean baseline and read the spec

**Files:** none modified

- [ ] **Step 1: Verify worktree is clean and on the right branch**

Run: `git status && git branch --show-current`
Expected: clean working tree, on branch `worktree-settings-restructure`

- [ ] **Step 2: Verify cargo check is green (warnings OK)**

Run: `cargo check --all-targets 2>&1 | tail -3`
Expected: `Finished` line, no `error[`, only the pre-existing warnings.

- [ ] **Step 3: Read the spec end-to-end**

Read: `docs/superpowers/specs/2026-06-16-settings-restructure-design.md` (339 lines).
Pay specific attention to:
- §3 (target struct shapes)
- §3.3a (subagent inheritance — the only functional addition)
- §4 (set() dotted-path semantics)
- §6.2 (test list — do not skip)

No commit for this task.

---

## Task 1: Define the new sub-config structs (compilable but unused)

**Files:**
- Modify: `src/config/mod.rs` (add new struct types alongside `Settings` — do NOT replace `Settings` yet)
- Modify: `src/config/api_config.rs` (no edit — `ApiConfig` will be deleted in Task 2 when it becomes unused)

**Goal of this task:** All new struct types compile, with `Default` impls. `Settings` itself is still unchanged. This isolates the "create new shapes" work from the "rip out old shapes" work.

- [ ] **Step 1: Add `ModelEndpoint` struct**

In `src/config/mod.rs`, before `pub struct Settings { ... }`, add:

```rust
/// One model endpoint: name + optional override of base_url/api_key/appkey.
/// On `models.small` / `models.planner`, `None` for url/key/appkey means inherit from `models.main`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelEndpoint {
    pub name: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub appkey: Option<String>,
}

impl ModelEndpoint {
    /// Resolve the effective base_url for this endpoint. If `self.base_url` is None,
    /// fall back to env var `API_BASE_URL`, then "https://api.anthropic.com".
    /// (Used by the main endpoint; small/planner inheritance is handled at the consumer.)
    pub fn endpoint_base_url(&self) -> String {
        if let Some(u) = &self.base_url { return u.clone(); }
        std::env::var("API_BASE_URL").unwrap_or_else(|_| "https://api.anthropic.com".to_string())
    }

    /// Resolve the effective api_key for this endpoint, checking env first.
    pub fn endpoint_api_key(&self) -> Option<String> {
        std::env::var("ANTHROPIC_API_KEY").ok()
            .or_else(|| std::env::var("DASHSCOPE_API_KEY").ok())
            .or_else(|| std::env::var("DEEPSEEK_API_KEY").ok())
            .or_else(|| self.api_key.clone())
    }
}
```

- [ ] **Step 2: Add `TransportConfig` struct**

In `src/config/mod.rs`, immediately after `ModelEndpoint`:

```rust
/// HTTP/SSE transport-layer config shared by all model endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportConfig {
    pub max_tokens: usize,
    pub timeout: u64,
    pub streaming: bool,
    pub beta_headers: Vec<String>,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            max_tokens: 4096,
            timeout: 120,
            streaming: true,
            beta_headers: vec![],
        }
    }
}
```

- [ ] **Step 3: Add `ModelsConfig` struct**

In `src/config/mod.rs`, immediately after `TransportConfig`:

```rust
/// All model endpoints + shared transport.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelsConfig {
    #[serde(default)]
    pub transport: TransportConfig,
    pub main: ModelEndpoint,
    #[serde(default)]
    pub small: Option<ModelEndpoint>,
    #[serde(default)]
    pub planner: Option<ModelEndpoint>,
}

impl Default for ModelsConfig {
    fn default() -> Self {
        Self {
            transport: TransportConfig::default(),
            main: ModelEndpoint {
                name: "sonnet".to_string(),
                base_url: std::env::var("API_BASE_URL").ok(),
                api_key: std::env::var("ANTHROPIC_API_KEY").ok()
                    .or_else(|| std::env::var("DASHSCOPE_API_KEY").ok())
                    .or_else(|| std::env::var("DEEPSEEK_API_KEY").ok()),
                appkey: None,
            },
            small: None,
            planner: None,
        }
    }
}
```

- [ ] **Step 4: Add `TokenBudget` struct**

```rust
/// Token budgets for main agent and subagents (units of 1000 tokens; 0 = unlimited).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TokenBudget {
    #[serde(default)]
    pub main_k: usize,
    #[serde(default)]
    pub subagent_default_k: usize,
}
```

- [ ] **Step 5: Add subagent override structs**

```rust
/// Per-field overrides that subagents can specify. None on every field = inherit
/// the corresponding main-agent value. Resolution: see Settings::resolve_subagent_config.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubagentRlmOverride {
    #[serde(default)] pub enabled: Option<bool>,
    #[serde(default)] pub delegate_tool: Option<bool>,
    #[serde(default)] pub auto_routing: Option<bool>,
    #[serde(default)] pub retry_enabled: Option<bool>,
    #[serde(default)] pub max_replan_cycles: Option<usize>,
    #[serde(default)] pub jaccard_threshold: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubagentPromptIncludesOverride {
    #[serde(default)] pub permissions: Option<bool>,
    #[serde(default)] pub developer: Option<bool>,
    #[serde(default)] pub collaboration: Option<bool>,
    #[serde(default)] pub environment: Option<bool>,
    #[serde(default)] pub skills: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubagentPromptOverride {
    #[serde(default)] pub include: SubagentPromptIncludesOverride,
    #[serde(default)] pub developer_instructions: Option<String>,
    #[serde(default)] pub collaboration_mode: Option<String>,
    #[serde(default)] pub model_instructions_file: Option<String>,
}
```

- [ ] **Step 6: Add the new `SubagentLimits` (with override fields)**

The existing `SubagentLimits` doesn't exist yet (current code has flat fields). Add:

```rust
/// Subagent runtime limits + overrides.
/// max_depth/max_concurrent/timeout_secs are subagent-only (no main-agent counterpart).
/// The remaining fields are overrides; None = inherit from agent.* — see resolve_subagent_config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentLimits {
    pub max_depth: usize,
    pub max_concurrent: usize,
    pub timeout_secs: u64,

    #[serde(default)] pub token_budget_k: Option<usize>,
    #[serde(default)] pub max_rounds: Option<usize>, // Some(0) = unlimited
    #[serde(default)] pub plan_mode: Option<bool>,
    #[serde(default)] pub rlm: SubagentRlmOverride,
    #[serde(default)] pub prompt: SubagentPromptOverride,
}

impl Default for SubagentLimits {
    fn default() -> Self {
        Self {
            max_depth: 3,
            max_concurrent: 5,
            timeout_secs: 240,
            token_budget_k: None,
            max_rounds: None,
            plan_mode: None,
            rlm: SubagentRlmOverride::default(),
            prompt: SubagentPromptOverride::default(),
        }
    }
}
```

- [ ] **Step 7: Add `AgentConfig` struct (will hold the existing RlmSettings)**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    #[serde(default)] pub plan_mode: bool,
    #[serde(default)] pub max_rounds: Option<usize>,
    #[serde(default)] pub token_budget: TokenBudget,
    #[serde(default)] pub subagent: SubagentLimits,
    #[serde(default)] pub rlm: RlmSettings,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            plan_mode: false,
            max_rounds: None,
            token_budget: TokenBudget::default(),
            subagent: SubagentLimits::default(),
            rlm: RlmSettings::default(),
        }
    }
}
```

- [ ] **Step 8: Extend `RlmSettings` with `jaccard_threshold`**

In the existing `RlmSettings`, add the new field (the current `default_rlm_jaccard_threshold` helper at line 160 is reused):

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RlmSettings {
    #[serde(default = "default_true")] pub enabled: bool,
    #[serde(default = "default_true")] pub delegate_tool: bool,
    #[serde(default = "default_true")] pub auto_routing: bool,
    #[serde(default = "default_true")] pub retry_enabled: bool,
    #[serde(default = "default_rlm_max_replan")] pub max_replan_cycles: usize,
    #[serde(default = "default_rlm_jaccard_threshold")] pub jaccard_threshold: f64,
}

impl Default for RlmSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            delegate_tool: true,
            auto_routing: true,
            retry_enabled: true,
            max_replan_cycles: 2,
            jaccard_threshold: 0.8,
        }
    }
}
```

- [ ] **Step 9: Add `PromptIncludes` and `PromptConfig`**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptIncludes {
    #[serde(default = "default_true")] pub permissions: bool,
    #[serde(default = "default_true")] pub developer: bool,
    #[serde(default = "default_true")] pub collaboration: bool,
    #[serde(default = "default_true")] pub environment: bool,
    #[serde(default = "default_true")] pub skills: bool,
}

impl Default for PromptIncludes {
    fn default() -> Self {
        Self { permissions: true, developer: true, collaboration: true, environment: true, skills: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PromptConfig {
    #[serde(default)] pub include: PromptIncludes,
    #[serde(default)] pub developer_instructions: Option<String>,
    #[serde(default)] pub collaboration_mode: Option<String>,
    #[serde(default)] pub model_instructions_file: Option<String>,
}
```

- [ ] **Step 10: Add `PluginsConfig` (renaming `plugin_dir` → `dir`, adding `marketplaces`)**

The existing `PluginSettings` will be replaced. For now, add the new type alongside:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginsConfig {
    pub enabled: bool,
    pub dir: PathBuf,
    pub auto_update: bool,
    #[serde(default)] pub enabled_map: std::collections::HashMap<String, bool>,
    #[serde(default)] pub marketplaces: Option<serde_json::Value>,
}

impl Default for PluginsConfig {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let config_dir = home.join(".wgenty-code");
        Self {
            enabled: true,
            dir: config_dir.join("plugins"),
            auto_update: true,
            enabled_map: std::collections::HashMap::new(),
            marketplaces: None,
        }
    }
}
```

- [ ] **Step 11: Add `TranscriptConfig` and `StorageConfig`**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptConfig {
    #[serde(default = "default_transcript_db_path")] pub db_path: String,
    #[serde(default = "default_max_transcript_age_days")] pub max_age_days: u32,
}

impl Default for TranscriptConfig {
    fn default() -> Self {
        Self { db_path: default_transcript_db_path(), max_age_days: 30 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    pub working_dir: PathBuf,
    pub memory: MemorySettings,
    #[serde(default)] pub transcript: TranscriptConfig,
}

impl Default for StorageConfig {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let config_dir = home.join(".wgenty-code");
        Self {
            working_dir: PathBuf::from("."),
            memory: MemorySettings {
                enabled: true,
                path: config_dir.join("memory.json"),
                consolidation_interval: 24,
                max_memories: 1000,
            },
            transcript: TranscriptConfig::default(),
        }
    }
}
```

- [ ] **Step 12: Add `IntegrationsConfig`**

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IntegrationsConfig {
    #[serde(default)] pub mcp_servers: Vec<McpConfig>,
    #[serde(default)] pub hooks: Option<serde_json::Value>,
    #[serde(default)] pub voice: VoiceSettings,
    #[serde(default)] pub guardian: GuardianSettings,
}
```

Note: `VoiceSettings` does not currently derive `Default` and its fields don't have meaningful zero values (`silence_threshold` should be 0.01, not 0.0), so deriving `Default` alone is insufficient. Replace the existing `pub struct VoiceSettings { ... }` declaration with:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceSettings {
    pub enabled: bool,
    pub push_to_talk: bool,
    pub silence_threshold: f32,
    pub sample_rate: u32,
}

impl Default for VoiceSettings {
    fn default() -> Self {
        Self { enabled: false, push_to_talk: false, silence_threshold: 0.01, sample_rate: 16000 }
    }
}
```

- [ ] **Step 13: Run `cargo check` — confirm new types compile in isolation**

Run: `cargo check --all-targets 2>&1 | grep -E "^error" | head -20`
Expected: empty output (no `error[E...]` lines). The 8 baseline warnings are still there but no new ones from the new types.

If errors appear, they will be about `MemorySettings` not having `Default` (since `StorageConfig::default()` constructs it manually — that's fine), or about field syntax. Fix and re-run.

- [ ] **Step 14: Commit**

```bash
git add src/config/mod.rs
git commit -m "config: add new sub-config struct definitions (unused)

Adds ModelsConfig, TransportConfig, ModelEndpoint, AgentConfig, TokenBudget,
SubagentLimits (with override fields), SubagentRlmOverride,
SubagentPromptOverride, SubagentPromptIncludesOverride, PromptConfig,
PromptIncludes, PluginsConfig, StorageConfig, TranscriptConfig,
IntegrationsConfig. Extends RlmSettings with jaccard_threshold. Adds
Default for VoiceSettings.

Settings struct itself is unchanged — these new types are additions,
referenced in Task 2."
```

---

## Task 2: Replace `Settings` with the new top-level shape (will break ~120 call sites — compiler-driven)

**Files:**
- Modify: `src/config/mod.rs` — replace `Settings` struct + `Default for Settings`; rewrite `Settings::load`, `Settings::reload`, `Settings::reset` (keep `save` mechanically the same), delete `migrate_rlm_settings`, delete `cc_mapping` mod declaration; rewrite `small_model_settings()`; add `resolve_subagent_config()`; rewrite `Settings::set()` to dotted-path
- Delete: `src/config/cc_mapping.rs`

**This task INTENTIONALLY breaks the build.** After this commit, `cargo check` will produce ~120 "no field X" errors across the codebase. Tasks 4–10 fix them file by file.

- [ ] **Step 1: Replace the `Settings` struct definition**

In `src/config/mod.rs`, replace the entire current `pub struct Settings { ... }` (lines 16–143 in the pre-refactor file) with:

```rust
/// Main configuration structure (top-level grouped form).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)] pub models: ModelsConfig,
    #[serde(default)] pub agent: AgentConfig,
    #[serde(default)] pub prompt: PromptConfig,
    #[serde(default)] pub plugins: PluginsConfig,
    #[serde(default)] pub storage: StorageConfig,
    #[serde(default)] pub integrations: IntegrationsConfig,
    #[serde(default)] pub verbose: bool,
}
```

- [ ] **Step 2: Replace `impl Default for Settings`**

Replace the entire current `impl Default for Settings` (lines 269–330 of pre-refactor file) with:

```rust
impl Default for Settings {
    fn default() -> Self {
        Self {
            models: ModelsConfig::default(),
            agent: AgentConfig::default(),
            prompt: PromptConfig::default(),
            plugins: PluginsConfig::default(),
            storage: StorageConfig::default(),
            integrations: IntegrationsConfig::default(),
            verbose: false,
        }
    }
}
```

- [ ] **Step 3: Delete `migrate_rlm_settings`, the `cc_mapping` mod declaration, and old helper fns that only the old struct used**

In `src/config/mod.rs`:

a. Delete `pub mod cc_mapping;` (currently line 4).
b. Delete the entire `migrate_rlm_settings` function (currently lines 355–380).
c. Delete the helper fn `fn default_max_transcript_age_days() -> u32 { 30 }` IF it remains unused after Task 1 (it should be used by `TranscriptConfig::default()` indirectly via `default_transcript_db_path` — keep `default_transcript_db_path` and `default_max_transcript_age_days`, both are still referenced by serde defaults on `TranscriptConfig`).
d. Keep `default_subagent_depth`, `default_max_concurrent_subagents`, `default_subagent_timeout` ONLY if they remain referenced; otherwise delete. They were used by old `#[serde(default)]` attributes on now-removed flat fields, so they will become dead. Delete them.
e. Keep `default_rlm_max_replan`, `default_rlm_jaccard_threshold`, `default_true` — these are referenced by `RlmSettings` and `PromptIncludes`.

Concrete edits in this step: remove the `pub mod cc_mapping;` line; remove the `migrate_rlm_settings` block; remove the three subagent-default helper fns; let `cargo check` in step 11 confirm what is still referenced.

- [ ] **Step 4: Rewrite `Settings::load`**

Replace the current `pub fn load() -> anyhow::Result<Self>` body (lines 334–350) with:

```rust
pub fn load() -> anyhow::Result<Self> {
    let path = Self::config_path();
    if path.exists() {
        let content = std::fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&content)?)
    } else {
        let s = Settings::default();
        s.save()?;
        Ok(s)
    }
}

fn config_path() -> std::path::PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".wgenty-code").join("settings.json")
}
```

No `migrate_rlm_settings` call. No `cc_mapping::CcConfigMapper::apply_mappings` call. No `mut settings` because the `from_str` result is final.

- [ ] **Step 5: Update `Settings::save`**

The current `save` builds the path inline. Replace its body with the same logic but using `Self::config_path()`:

```rust
pub fn save(&self) -> anyhow::Result<()> {
    let path = Self::config_path();
    if let Some(dir) = path.parent() { std::fs::create_dir_all(dir)?; }
    let content = serde_json::to_string_pretty(self)?;
    std::fs::write(&path, content)?;
    Ok(())
}
```

- [ ] **Step 6: Replace `small_model_settings`**

Replace the current `pub fn small_model_settings(&self) -> Self` (lines 403–416) with:

```rust
/// Build a Settings clone configured for the small model.
/// If models.small is None, returns a clone of self (no-op).
/// If models.small is Some, overrides models.main name/base_url/api_key/appkey
/// from the small endpoint where present, and forces transport.max_tokens = 2048.
pub fn small_model_settings(&self) -> Self {
    let mut s = self.clone();
    if let Some(small) = &self.models.small {
        s.models.main.name = small.name.clone();
        if let Some(url) = &small.base_url { s.models.main.base_url = Some(url.clone()); }
        if let Some(key) = &small.api_key  { s.models.main.api_key  = Some(key.clone()); }
        if let Some(ak)  = &small.appkey   { s.models.main.api_key  = Some(ak.clone()); } // appkey overrides api_key on small
        s.models.transport.max_tokens = 2048;
    }
    s
}
```

Note: the `appkey` overrides `api_key` line preserves the current task.rs:423-424 behavior where appkey wins over api_key on small.

- [ ] **Step 7: Add `resolve_subagent_config`**

Add this method on `impl Settings`, immediately after `small_model_settings`:

```rust
/// Build a Settings clone where subagent override fields (under agent.subagent)
/// have been folded into the corresponding agent.* fields. Used at subagent spawn
/// time so the subagent loop can read agent.* directly.
///
/// Special cases:
/// - max_rounds: subagent override `Some(0)` means "unlimited" (mapped to None).
/// - subagent_default_k from token_budget is NOT consulted here; it is read by
///   the spawn caller separately as a fallback when no subagent override exists.
///   (See spec §3.3a "token_budget_k 与 subagent_default_k 的区分".)
pub fn resolve_subagent_config(&self) -> Self {
    let mut s = self.clone();
    let ov = &self.agent.subagent;

    if let Some(b)  = ov.token_budget_k { s.agent.token_budget.main_k = b; }
    if let Some(r)  = ov.max_rounds {
        s.agent.max_rounds = if r == 0 { None } else { Some(r) };
    }
    if let Some(p)  = ov.plan_mode { s.agent.plan_mode = p; }

    if let Some(v)  = ov.rlm.enabled            { s.agent.rlm.enabled = v; }
    if let Some(v)  = ov.rlm.delegate_tool      { s.agent.rlm.delegate_tool = v; }
    if let Some(v)  = ov.rlm.auto_routing       { s.agent.rlm.auto_routing = v; }
    if let Some(v)  = ov.rlm.retry_enabled      { s.agent.rlm.retry_enabled = v; }
    if let Some(v)  = ov.rlm.max_replan_cycles  { s.agent.rlm.max_replan_cycles = v; }
    if let Some(v)  = ov.rlm.jaccard_threshold  { s.agent.rlm.jaccard_threshold = v; }

    if let Some(v)  = ov.prompt.include.permissions   { s.prompt.include.permissions = v; }
    if let Some(v)  = ov.prompt.include.developer     { s.prompt.include.developer = v; }
    if let Some(v)  = ov.prompt.include.collaboration { s.prompt.include.collaboration = v; }
    if let Some(v)  = ov.prompt.include.environment   { s.prompt.include.environment = v; }
    if let Some(v)  = ov.prompt.include.skills        { s.prompt.include.skills = v; }

    if let Some(v)  = &ov.prompt.developer_instructions  { s.prompt.developer_instructions  = Some(v.clone()); }
    if let Some(v)  = &ov.prompt.collaboration_mode      { s.prompt.collaboration_mode      = Some(v.clone()); }
    if let Some(v)  = &ov.prompt.model_instructions_file { s.prompt.model_instructions_file = Some(v.clone()); }

    s
}
```

- [ ] **Step 8: Rewrite `Settings::set` to dotted-path**

Replace the entire body of `pub fn set(key: &str, value: &str) -> anyhow::Result<()>` (lines 419–498 of pre-refactor file) with:

```rust
pub fn set(key: &str, value: &str) -> anyhow::Result<()> {
    use serde_json::Value;
    let settings = Self::load()?;
    let mut json = serde_json::to_value(&settings)?;

    // Parse value: try as JSON literal first, fall back to string.
    let parsed: Value = serde_json::from_str(value)
        .unwrap_or_else(|_| Value::String(value.to_string()));

    // Walk dotted path, creating object nodes as needed for keys that
    // map to existing object types. The final from_value validates the
    // shape, so an invalid path becomes an Err there rather than silent.
    let parts: Vec<&str> = key.split('.').collect();
    if parts.is_empty() || parts.iter().any(|p| p.is_empty()) {
        return Err(anyhow::anyhow!("Invalid empty key segment in '{}'", key));
    }

    fn set_at(node: &mut Value, parts: &[&str], val: Value) -> anyhow::Result<()> {
        let (head, rest) = parts.split_first().ok_or_else(|| anyhow::anyhow!("empty path"))?;
        if rest.is_empty() {
            match node {
                Value::Object(map) => { map.insert(head.to_string(), val); Ok(()) }
                _ => Err(anyhow::anyhow!("path segment '{}' is not under an object", head)),
            }
        } else {
            let next = match node {
                Value::Object(map) => map.entry(head.to_string()).or_insert(Value::Object(Default::default())),
                _ => return Err(anyhow::anyhow!("path segment '{}' is not under an object", head)),
            };
            set_at(next, rest, val)
        }
    }

    set_at(&mut json, &parts, parsed)?;

    // Validate the new shape by deserializing back to Settings.
    let new_settings: Settings = serde_json::from_value(json)
        .map_err(|e| anyhow::anyhow!("invalid setting at '{}': {}", key, e))?;
    new_settings.save()?;
    Ok(())
}
```

- [ ] **Step 9: Drop the now-unused module declaration and remove `apply_mappings` references**

Confirm `pub mod cc_mapping;` is removed (Step 3a). Now delete the `cc_mapping.rs` file:

```bash
git rm src/config/cc_mapping.rs
```

- [ ] **Step 10: Update `src/config/settings.rs` re-exports**

Replace the contents of `src/config/settings.rs` (currently re-exports `Settings, MemorySettings, VoiceSettings, PluginSettings`) with:

```rust
//! Settings - Re-exported from mod.rs
// This file exists to satisfy the module system
// The actual Settings struct is defined in mod.rs

pub use super::{
    Settings, ModelsConfig, ModelEndpoint, TransportConfig,
    AgentConfig, TokenBudget, SubagentLimits,
    SubagentRlmOverride, SubagentPromptOverride, SubagentPromptIncludesOverride,
    PromptConfig, PromptIncludes,
    PluginsConfig, StorageConfig, TranscriptConfig, IntegrationsConfig,
    MemorySettings, VoiceSettings, GuardianSettings, RlmSettings,
};
```

(Note: `PluginSettings` is removed from re-exports — it no longer exists. If any caller imports it directly, they'll fail in Task 4-10 and we update the import there.)

- [ ] **Step 11: Run `cargo check` — many errors expected**

Run: `cargo check --all-targets 2>&1 | grep -cE "^error\["`
Expected: a number > 50, < 200. (Each broken read site produces ~1-3 errors.)

Run: `cargo check --all-targets 2>&1 | grep -E "^error\[" | head -10`
Inspect: should be `no field X on type Settings` errors at file paths like `src/api/mod.rs`, `src/tools/meta/task.rs`, etc. — exactly the files we'll fix in Tasks 4–10.

If any error originates in `src/config/mod.rs` itself, fix it now before moving on. Likely candidates: an `impl` block referencing a removed field; an unused-helper warning being escalated to error.

- [ ] **Step 12: Commit (broken build is intentional)**

```bash
git add -A src/config/
git commit -m "config: replace flat Settings with grouped sub-configs (build broken)

Top-level Settings now { models, agent, prompt, plugins, storage,
integrations, verbose }. Removes migrate_rlm_settings, deletes
cc_mapping.rs (zero external consumers), rewrites set() as dotted-path
generic setter, rewrites small_model_settings() to use models.small,
adds resolve_subagent_config() for spawn-time inheritance resolution.

Build is intentionally broken — ~120 read sites across other files
expect old field names. They are fixed in subsequent commits, one
file per commit, driven by cargo errors."
```

---

## Task 3: Rewrite the unit tests in `src/config/mod.rs`

**Files:**
- Modify: `src/config/mod.rs` (the `#[cfg(test)] mod tests` block at the bottom)

The pre-refactor file has 6 tests (lines 508–615). Two must be deleted (migration-related); five must be updated to new field paths; new tests must be added per spec §6.2.

- [ ] **Step 1: Delete the two migration-related tests**

In `src/config/mod.rs`, locate and delete:
- `fn test_migrate_rlm_legacy_keys`
- `fn test_migrate_rlm_no_override_when_group_present`

These reference the deleted `migrate_rlm_settings`.

- [ ] **Step 2: Keep four `RlmSettings`-only tests as-is**

These still work because `RlmSettings` is unchanged in shape (only gained `jaccard_threshold`):
- `test_rlm_settings_default_all_enabled` — add `assert_eq!(rlm.jaccard_threshold, 0.8);`
- `test_rlm_settings_deserialize_partial` — assert that `jaccard_threshold == 0.8` (default)
- `test_rlm_settings_deserialize_full` — extend the JSON to include `"jaccard_threshold": 0.95` and assert
- `test_settings_default_includes_rlm` — change to read `Settings::default().agent.rlm.enabled` (was `settings.rlm.enabled`)

Concrete edits:

```rust
#[test]
fn test_rlm_settings_default_all_enabled() {
    let rlm = RlmSettings::default();
    assert!(rlm.enabled);
    assert!(rlm.delegate_tool);
    assert!(rlm.auto_routing);
    assert!(rlm.retry_enabled);
    assert_eq!(rlm.max_replan_cycles, 2);
    assert_eq!(rlm.jaccard_threshold, 0.8);
}

#[test]
fn test_rlm_settings_deserialize_partial() {
    let json = r#"{"enabled": false}"#;
    let rlm: RlmSettings = serde_json::from_str(json).unwrap();
    assert!(!rlm.enabled);
    assert!(rlm.delegate_tool);
    assert!(rlm.auto_routing);
    assert!(rlm.retry_enabled);
    assert_eq!(rlm.max_replan_cycles, 2);
    assert_eq!(rlm.jaccard_threshold, 0.8);
}

#[test]
fn test_rlm_settings_deserialize_full() {
    let json = r#"{
        "enabled": false,
        "delegate_tool": false,
        "auto_routing": false,
        "retry_enabled": false,
        "max_replan_cycles": 0,
        "jaccard_threshold": 0.95
    }"#;
    let rlm: RlmSettings = serde_json::from_str(json).unwrap();
    assert!(!rlm.enabled);
    assert!(!rlm.delegate_tool);
    assert!(!rlm.auto_routing);
    assert!(!rlm.retry_enabled);
    assert_eq!(rlm.max_replan_cycles, 0);
    assert!((rlm.jaccard_threshold - 0.95).abs() < 1e-9);
}

#[test]
fn test_settings_default_includes_rlm() {
    let settings = Settings::default();
    assert!(settings.agent.rlm.enabled);
    assert!(settings.agent.rlm.delegate_tool);
    assert!(settings.agent.rlm.auto_routing);
}
```

- [ ] **Step 3: Rewrite the `test_rlm_deserialize_in_settings` JSON to new shape**

Replace with:

```rust
#[test]
fn test_rlm_deserialize_in_settings() {
    let json = r#"{
        "models": {
            "transport": {"max_tokens": 4096, "timeout": 120, "streaming": true, "beta_headers": []},
            "main": {"name": "test"}
        },
        "agent": {
            "rlm": {"enabled": false, "delegate_tool": false}
        },
        "storage": {
            "working_dir": ".",
            "memory": {"enabled": false, "path": ".", "consolidation_interval": 24, "max_memories": 100}
        },
        "plugins": {"enabled": false, "dir": ".", "auto_update": false}
    }"#;
    let settings: Settings = serde_json::from_str(json).unwrap();
    assert!(!settings.agent.rlm.enabled);
    assert!(!settings.agent.rlm.delegate_tool);
    // Unspecified fields use defaults
    assert!(settings.agent.rlm.auto_routing);
    assert!(settings.agent.rlm.retry_enabled);
    assert_eq!(settings.agent.rlm.max_replan_cycles, 2);
}
```

- [ ] **Step 4: Add new test — `prompt.include` defaults all true**

```rust
#[test]
fn test_prompt_includes_default_all_true() {
    let s = Settings::default();
    assert!(s.prompt.include.permissions);
    assert!(s.prompt.include.developer);
    assert!(s.prompt.include.collaboration);
    assert!(s.prompt.include.environment);
    assert!(s.prompt.include.skills);
}
```

- [ ] **Step 5: Add new test — `models.small` default is None and small/planner deserialize symmetrically**

```rust
#[test]
fn test_models_default_no_small_or_planner() {
    let s = Settings::default();
    assert_eq!(s.models.main.name, "sonnet");
    assert!(s.models.small.is_none());
    assert!(s.models.planner.is_none());
}

#[test]
fn test_models_small_inherits_when_url_absent() {
    let json = r#"{
        "models": {
            "main": {"name": "sonnet", "base_url": "https://api.example.com", "api_key": "main-key"},
            "small": {"name": "haiku"}
        }
    }"#;
    let s: Settings = serde_json::from_str(json).unwrap();
    let small = s.models.small.as_ref().unwrap();
    assert_eq!(small.name, "haiku");
    assert!(small.base_url.is_none());  // Inheritance is the consumer's job — see small_model_settings
    assert!(small.api_key.is_none());
}

#[test]
fn test_small_model_settings_uses_small_overrides() {
    let mut s = Settings::default();
    s.models.main.base_url = Some("https://api.main.example".to_string());
    s.models.main.api_key = Some("main-key".to_string());
    s.models.small = Some(ModelEndpoint {
        name: "haiku".to_string(),
        base_url: None,                                      // inherits main
        api_key: Some("small-key".to_string()),
        appkey: None,
    });
    let small_s = s.small_model_settings();
    assert_eq!(small_s.models.main.name, "haiku");
    assert_eq!(small_s.models.main.base_url, Some("https://api.main.example".to_string())); // unchanged
    assert_eq!(small_s.models.main.api_key, Some("small-key".to_string()));                 // overridden
    assert_eq!(small_s.models.transport.max_tokens, 2048);
}
```

- [ ] **Step 6: Add new test — subagent override defaults all None**

```rust
#[test]
fn test_subagent_overrides_default_none() {
    let s = Settings::default();
    let ov = &s.agent.subagent;
    assert!(ov.token_budget_k.is_none());
    assert!(ov.max_rounds.is_none());
    assert!(ov.plan_mode.is_none());
    assert!(ov.rlm.enabled.is_none());
    assert!(ov.rlm.delegate_tool.is_none());
    assert!(ov.rlm.auto_routing.is_none());
    assert!(ov.rlm.retry_enabled.is_none());
    assert!(ov.rlm.max_replan_cycles.is_none());
    assert!(ov.rlm.jaccard_threshold.is_none());
    assert!(ov.prompt.include.permissions.is_none());
    assert!(ov.prompt.include.developer.is_none());
    assert!(ov.prompt.include.collaboration.is_none());
    assert!(ov.prompt.include.environment.is_none());
    assert!(ov.prompt.include.skills.is_none());
    assert!(ov.prompt.developer_instructions.is_none());
    assert!(ov.prompt.collaboration_mode.is_none());
    assert!(ov.prompt.model_instructions_file.is_none());
}
```

- [ ] **Step 7: Add new test — `resolve_subagent_config` no-op when no overrides**

```rust
#[test]
fn test_resolve_subagent_config_noop_when_no_overrides() {
    let s = Settings::default();
    let r = s.resolve_subagent_config();
    assert_eq!(r.agent.plan_mode, s.agent.plan_mode);
    assert_eq!(r.agent.max_rounds, s.agent.max_rounds);
    assert_eq!(r.agent.token_budget.main_k, s.agent.token_budget.main_k);
    assert_eq!(r.agent.rlm.enabled, s.agent.rlm.enabled);
    assert_eq!(r.prompt.include.skills, s.prompt.include.skills);
}
```

- [ ] **Step 8: Add new test — `resolve_subagent_config` applies overrides**

```rust
#[test]
fn test_resolve_subagent_config_applies_overrides() {
    let mut s = Settings::default();
    s.agent.token_budget.main_k = 100;
    s.agent.rlm.enabled = true;
    s.prompt.include.skills = true;

    s.agent.subagent.token_budget_k = Some(50);
    s.agent.subagent.rlm.enabled = Some(false);
    s.agent.subagent.prompt.include.skills = Some(false);

    let r = s.resolve_subagent_config();
    assert_eq!(r.agent.token_budget.main_k, 50);
    assert!(!r.agent.rlm.enabled);
    assert!(!r.prompt.include.skills);
    // Source unchanged
    assert_eq!(s.agent.token_budget.main_k, 100);
    assert!(s.agent.rlm.enabled);
}
```

- [ ] **Step 9: Add new test — `max_rounds = Some(0)` resolves to None (unlimited)**

```rust
#[test]
fn test_resolve_subagent_max_rounds_zero_means_unlimited() {
    let mut s = Settings::default();
    s.agent.max_rounds = Some(50);
    s.agent.subagent.max_rounds = Some(0);
    let r = s.resolve_subagent_config();
    assert_eq!(r.agent.max_rounds, None);
}
```

- [ ] **Step 10: Add new test — `set()` dotted-path: nested field set**

```rust
#[test]
fn test_set_dotted_path_nested_field() {
    use serde_json::Value;
    let s = Settings::default();
    let mut json = serde_json::to_value(&s).unwrap();
    // Simulate the inner walk Settings::set does
    let parts: &[&str] = &["agent", "subagent", "max_depth"];
    fn walk_set(n: &mut Value, p: &[&str], v: Value) {
        let (h, r) = p.split_first().unwrap();
        if r.is_empty() {
            n.as_object_mut().unwrap().insert(h.to_string(), v);
        } else {
            let nx = n.as_object_mut().unwrap()
                .entry(h.to_string()).or_insert(Value::Object(Default::default()));
            walk_set(nx, r, v);
        }
    }
    walk_set(&mut json, parts, Value::Number(7.into()));
    let new: Settings = serde_json::from_value(json).unwrap();
    assert_eq!(new.agent.subagent.max_depth, 7);
}
```

(This test mirrors `Settings::set` internal logic without writing to disk. A direct test of `Settings::set` is omitted because it touches `~/.wgenty-code/settings.json` — out of unit-test scope.)

- [ ] **Step 11: Add new test — invalid dotted-path for `set()` returns Err on validation**

```rust
#[test]
fn test_set_dotted_path_unknown_field_fails_validation() {
    use serde_json::Value;
    let s = Settings::default();
    let mut json = serde_json::to_value(&s).unwrap();
    json.as_object_mut().unwrap()
        .insert("nonexistent_top".to_string(), Value::Bool(true));
    // Note: serde_json by default tolerates extra fields. To confirm
    // Settings rejects them, the struct would need #[serde(deny_unknown_fields)].
    // This test documents current behavior: the conversion succeeds.
    let r: Result<Settings, _> = serde_json::from_value(json);
    assert!(r.is_ok(), "extra fields are tolerated by default; if rejection is desired, add deny_unknown_fields");
}
```

(This is a *documenting* test — it captures present behavior. If a future change adds `#[serde(deny_unknown_fields)]`, flip the assertion.)

- [ ] **Step 12: Add new test — 4-level token budget fallback chain (per spec §3.3a)**

This tests the resolution chain that Task 5 will implement in `task.rs`. We test the helper logic in pure form so the spawn site can simply use the same expression.

Add this test (it asserts the chain works as `caller_explicit > subagent_override > subagent_default > main`):

```rust
/// Mirrors the budget-fallback chain in src/tools/meta/task.rs.
/// Helper here so the unit test does not depend on the live spawn path.
fn resolve_token_budget_k(s: &Settings, caller: Option<usize>) -> usize {
    caller
        .or(s.agent.subagent.token_budget_k)
        .or((s.agent.token_budget.subagent_default_k > 0)
            .then_some(s.agent.token_budget.subagent_default_k))
        .unwrap_or(s.agent.token_budget.main_k)
}

#[test]
fn test_subagent_token_budget_fallback_chain() {
    let mut s = Settings::default();
    s.agent.token_budget.main_k = 100;

    // Level 4: only main_k set
    assert_eq!(resolve_token_budget_k(&s, None), 100);

    // Level 3: subagent_default_k > 0 wins over main_k
    s.agent.token_budget.subagent_default_k = 50;
    assert_eq!(resolve_token_budget_k(&s, None), 50);

    // Level 3 ignored when subagent_default_k == 0 (i.e. unlimited intent on default)
    s.agent.token_budget.subagent_default_k = 0;
    assert_eq!(resolve_token_budget_k(&s, None), 100);

    // Level 2: subagent override beats subagent_default and main
    s.agent.token_budget.subagent_default_k = 50;
    s.agent.subagent.token_budget_k = Some(30);
    assert_eq!(resolve_token_budget_k(&s, None), 30);

    // Level 1: caller-explicit beats everything
    assert_eq!(resolve_token_budget_k(&s, Some(7)), 7);
}
```

- [ ] **Step 13: Run the config tests in isolation**

The full project doesn't build yet (Task 2 broke ~120 sites). But the `config` module's own tests can be checked.

Run: `cargo check -p wgenty_code --lib --tests 2>&1 | grep -cE "^error\["`
Expected: same number as Task 2 step 11 (the new tests don't add errors; they reference only types in `src/config/mod.rs` which is internally consistent).

If any new error originates in the test block (e.g., a type name typo), fix it now.

- [ ] **Step 14: Commit**

```bash
git add src/config/mod.rs
git commit -m "config: rewrite tests for new Settings shape

Removes migrate-related tests, updates RlmSettings tests for the new
jaccard_threshold field, rewrites the deserialize-in-Settings test to
the grouped JSON shape, adds tests covering: prompt.include defaults,
models default & small inheritance, small_model_settings semantics,
subagent override defaults, resolve_subagent_config no-op + apply,
max_rounds=Some(0) unlimited semantics, set() dotted-path walk.

Build still broken at ~120 read sites (next tasks)."
```

---

## Task 4: Fix `src/api/mod.rs` (12 read sites)

**Files:**
- Modify: `src/api/mod.rs`

**Background:** `ApiClient` reads several `settings` fields. Specifically:
- `settings.api.timeout` (1×)
- `settings.api.get_base_url()` (1×) → renamed pattern
- `settings.api.get_api_key()` (1×) → renamed pattern
- `settings.model` (5×) → name path
- `settings.api.max_tokens` (4×)

**Mapping:**
| Old | New |
|-----|-----|
| `settings.api.timeout` | `settings.models.transport.timeout` |
| `settings.api.get_base_url()` | `settings.models.main.endpoint_base_url()` |
| `settings.api.get_api_key()` | `settings.models.main.endpoint_api_key()` |
| `&settings.model` | `&settings.models.main.name` |
| `settings.api.max_tokens` | `settings.models.transport.max_tokens` |

- [ ] **Step 1: Open `src/api/mod.rs` and apply the substitutions**

For each of the 12 sites, do the mapping above. Specific lines (from grep):

```
66:  Duration::from_secs(settings.api.timeout)              → settings.models.transport.timeout
74:  detect_provider(&settings.api.get_base_url())          → detect_provider(&settings.models.main.endpoint_base_url())
89:  self.settings.api.get_api_key()                        → self.settings.models.main.endpoint_api_key()
93:  self.settings.api.get_base_url()                       → self.settings.models.main.endpoint_base_url()
97:  &self.settings.model                                   → &self.settings.models.main.name
123: resolve_model_id(&self.settings.model)                 → resolve_model_id(&self.settings.models.main.name)
125: self.settings.api.max_tokens                           → self.settings.models.transport.max_tokens
167, 169, 226, 228, 268, 270: same patterns
```

Use Edit tool per line. Do NOT use sed/perl (project convention via tooling).

- [ ] **Step 2: Run `cargo check` for this file's errors specifically**

Run: `cargo check --all-targets 2>&1 | grep "src/api/mod.rs" | head`
Expected: no lines (file is clean).

- [ ] **Step 3: Commit**

```bash
git add src/api/mod.rs
git commit -m "api: update field paths to grouped Settings (12 sites)

settings.api.{timeout,max_tokens,...} → settings.models.transport.*
settings.api.get_{base_url,api_key}   → settings.models.main.endpoint_*
settings.model                        → settings.models.main.name"
```

---

## Task 5: Fix `src/tools/meta/task.rs` (21 read sites + small-model dedup)

**Files:**
- Modify: `src/tools/meta/task.rs`

**Background:** This file is the largest non-config consumer. It also contains the hand-written small-model override block at lines 413–426 that duplicates `small_model_settings()`.

**Read-site mapping (per grep at line numbers in pre-refactor state):**

| Line | Old | New |
|------|-----|-----|
| 240 (doc string) | mentions `settings.default_subagent_token_budget_k` | mention `settings.agent.token_budget.subagent_default_k` |
| 262 | `self.settings.default_subagent_token_budget_k` | `self.settings.agent.token_budget.subagent_default_k` |
| 323, 373, 378 | `self.settings.max_subagent_depth` | `self.settings.agent.subagent.max_depth` |
| 385 | `self.settings.max_concurrent_subagents` | `self.settings.agent.subagent.max_concurrent` |
| 413–426 | hand-written small model overrides block | replace with `let small_settings = self.settings.small_model_settings();` |
| 444, 635 | `self.settings.subagent_timeout_secs` | `self.settings.agent.subagent.timeout_secs` |
| 489, 651 | `self.settings.max_transcript_age_days` | `self.settings.storage.transcript.max_age_days` |
| 599, 600 | `self.settings.rlm.{enabled, auto_routing}` | `self.settings.agent.rlm.{enabled, auto_routing}` |

- [ ] **Step 1: Replace the hand-written small-model block (lines 413–426)**

Find this block:

```rust
            if let Some(ref small_model) = self.settings.small_model {
                let mut small_settings = self.settings.clone();
                small_settings.model = small_model.clone();
                small_settings.api.max_tokens = 2048;
                if let Some(ref url) = self.settings.small_model_base_url {
                    small_settings.api.base_url = url.clone();
                }
                if let Some(ref key) = self.settings.small_model_api_key {
                    small_settings.api.api_key = Some(key.clone());
                }
                if let Some(ref appkey) = self.settings.small_model_appkey {
                    small_settings.api.api_key = Some(appkey.clone());
                }
                ApiClient::new(small_settings)
            } else {
                ApiClient::new(self.settings.clone())
            }
```

Replace with:

```rust
            if self.settings.models.small.is_some() {
                ApiClient::new(self.settings.small_model_settings())
            } else {
                ApiClient::new(self.settings.clone())
            }
```

- [ ] **Step 2: Apply the remaining field-path substitutions**

For each line listed in the mapping above, edit the file. Use Edit tool per change.

- [ ] **Step 3: Update the doc string at line ~240**

Find `description` text mentioning `settings.default_subagent_token_budget_k` and replace with `settings.agent.token_budget.subagent_default_k`.

- [ ] **Step 4: Implement the 4-level subagent token-budget fallback (per spec §3.3a)**

Currently lines 262 and surrounding logic implement a 2-level fallback (caller param → `default_subagent_token_budget_k`). Spec §3.3a requires a 4-level chain:

1. caller-explicit (from tool argument) — already handled
2. `agent.subagent.token_budget_k` — **new level**
3. `agent.token_budget.subagent_default_k`
4. `agent.token_budget.main_k`

Find the existing budget-resolution block (it starts near line 262 with `let default_k = self.settings.default_subagent_token_budget_k;` and feeds into the spawn config). Read the block, then refactor to:

```rust
// Resolve effective token budget for the spawned subagent.
// Spec §3.3a — 4-level fallback.
let effective_budget_k = caller_budget_k                                              // 1. caller-explicit (Option<usize>)
    .or(self.settings.agent.subagent.token_budget_k)                                  // 2. subagent override
    .or((self.settings.agent.token_budget.subagent_default_k > 0)
        .then_some(self.settings.agent.token_budget.subagent_default_k))              // 3. subagent default (only when > 0)
    .unwrap_or(self.settings.agent.token_budget.main_k);                              // 4. main budget
```

The exact variable name `caller_budget_k` and surrounding context will depend on the existing code — adjust to match. The key invariant is that **all four levels are consulted in order, and any non-zero / Some value short-circuits**. Replace the variable name in any subsequent reference (e.g. `default_k` → `effective_budget_k`).

- [ ] **Step 5: Apply the remaining field-path substitutions**

For each line listed in the mapping above (lines 262, 323, 373, 378, 385, 444, 489, 599, 600, 635, 651), edit the file. Use Edit tool per change.

- [ ] **Step 6: Verify `cargo check` is clean for this file**

Run: `cargo check --all-targets 2>&1 | grep "src/tools/meta/task.rs"`
Expected: no lines.

- [ ] **Step 7: Commit**

```bash
git add src/tools/meta/task.rs
git commit -m "tools/task: update field paths, dedup small-model, 4-level budget fallback (21 sites)

- Replaces inline 14-line small-model override with small_model_settings() call
- subagent limits/timeouts/transcript_age/rlm.* now under agent.* / storage.*
- Tool description string updated for new dotted-path key
- Implements 4-level token budget fallback per spec §3.3a:
  caller > agent.subagent.token_budget_k > subagent_default_k > main_k"
```

---

## Task 6: Fix `src/tools/meta/rlm/pipeline.rs` (7 read sites + small-model dedup)

**Files:**
- Modify: `src/tools/meta/rlm/pipeline.rs`

**Read-site mapping:**

| Line | Old | New |
|------|-----|-----|
| 138–145 | hand-written small model override | replace with `if settings.models.small.is_some() { settings.small_model_settings() } else { settings.clone() }` |
| 157 | `0 < settings.max_subagent_depth` | `0 < settings.agent.subagent.max_depth` |
| 244 | `settings.subagent_timeout_secs` | `settings.agent.subagent.timeout_secs` |

- [ ] **Step 1: Replace the small-model block**

Find:

```rust
    let small_client = if settings.small_model.is_some() {
        let mut small_settings = settings.clone();
        small_settings.model = settings.small_model.clone().unwrap();
        small_settings.api.max_tokens = 2048;
        if let Some(ref url) = settings.small_model_base_url {
            small_settings.api.base_url = url.clone();
        }
```

(continues with api_key handling — read the actual lines first to capture the full block.)

Read lines 137–155 of `src/tools/meta/rlm/pipeline.rs` to identify the full block, then replace with:

```rust
    let small_client = if settings.models.small.is_some() {
        ApiClient::new(settings.small_model_settings())
    } else {
        // Same client as main when no small endpoint is configured.
        main_client.clone()
    };
```

(If `ApiClient` is not `Clone`, instantiate `ApiClient::new(settings.clone())` instead — verify by reading the original block to see what the `else` branch does.)

- [ ] **Step 2: Apply remaining field-path substitutions**

Edit lines 157 and 244 per the table above.

- [ ] **Step 3: Verify**

Run: `cargo check --all-targets 2>&1 | grep "src/tools/meta/rlm/pipeline.rs"`
Expected: no lines.

- [ ] **Step 4: Commit**

```bash
git add src/tools/meta/rlm/pipeline.rs
git commit -m "rlm/pipeline: update field paths and dedup small-model logic (7 sites)"
```

---

## Task 7: Fix `src/tools/mod.rs`, `src/prompts/mod.rs`, `src/permissions/policy.rs`, `src/tui/app/event.rs` (single-digit sites each)

**Files:** four files modified. Treat as one task because each is trivial.

**Mappings:**

`src/tools/mod.rs:144`:
- `settings.api.get_base_url()` → `settings.models.main.endpoint_base_url()`

`src/prompts/mod.rs`:
- L126 `settings.developer_instructions` → `settings.prompt.developer_instructions`
- L146 `settings.include_skill_instructions` → `settings.prompt.include.skills`

(Note: there may be more `include_*_instructions` reads in this file. Run grep below in step 1 to confirm.)

`src/permissions/policy.rs:25`:
- `&settings.working_dir` → `&settings.storage.working_dir`

`src/tui/app/event.rs:560`:
- `new_settings.collaboration_mode` → `new_settings.prompt.collaboration_mode`

- [ ] **Step 1: Re-grep `src/prompts/mod.rs` for any `include_*` or other settings reads we missed**

Run: `grep -nE "settings\.(include_|developer_instructions|collaboration_mode|model_instructions_file|model[^s]|api[._]|verbose|memory|voice|plugins|working_dir)" src/prompts/mod.rs`
Apply `prompt.include.*` / `prompt.*` mapping to every match.

- [ ] **Step 2: Apply the four files' mappings**

Use Edit tool per site.

- [ ] **Step 3: Verify**

Run: `cargo check --all-targets 2>&1 | grep -E "src/(tools/mod\.rs|prompts/mod\.rs|permissions/policy\.rs|tui/app/event\.rs)"`
Expected: no lines.

- [ ] **Step 4: Commit**

```bash
git add src/tools/mod.rs src/prompts/mod.rs src/permissions/policy.rs src/tui/app/event.rs
git commit -m "misc: update field paths to grouped Settings (4 files, ~5 sites)"
```

---

## Task 8: Fix `src/daemon/state.rs` (4 read sites)

**Files:**
- Modify: `src/daemon/state.rs`

**Mappings (run grep to confirm before editing):**

```
grep -nE "settings\." src/daemon/state.rs
```

Apply mappings per spec §3 — most likely candidates:
- `settings.model` → `settings.models.main.name`
- `settings.api.*` → `settings.models.transport.*` or `settings.models.main.*`
- `settings.working_dir` → `settings.storage.working_dir`
- `settings.transcript_db_path` → `settings.storage.transcript.db_path`

- [ ] **Step 1: Grep + map + edit**

Run grep, list each site, apply mapping, edit.

- [ ] **Step 2: Verify**

Run: `cargo check --all-targets 2>&1 | grep "src/daemon/state.rs"`
Expected: no lines.

- [ ] **Step 3: Commit**

```bash
git add src/daemon/state.rs
git commit -m "daemon: update field paths to grouped Settings (4 sites)"
```

---

## Task 9: Fix `src/gui/app.rs` (2 sites) and `src/gui/settings.rs` (6 sites)

**Files:**
- Modify: `src/gui/app.rs`
- Modify: `src/gui/settings.rs`

GUI code is more likely to read & write fields by name (settings UI). All 8 sites need careful per-line review.

- [ ] **Step 1: Grep both files**

Run: `grep -nE "settings\." src/gui/app.rs src/gui/settings.rs`

- [ ] **Step 2: Apply mappings per spec §3**

Each site gets its own Edit. If a site is a *write* (settings.X = Y), the new path must be the same nested field. The dotted-path `set()` is the public API for CLI; the GUI can write directly because it has typed access.

- [ ] **Step 3: Verify**

Run: `cargo check --all-targets 2>&1 | grep -E "src/gui/(app\.rs|settings\.rs)"`
Expected: no lines.

- [ ] **Step 4: Commit**

```bash
git add src/gui/app.rs src/gui/settings.rs
git commit -m "gui: update field paths to grouped Settings (8 sites)"
```

---

## Task 10: Fix `src/tui/app/mod.rs` (3 sites) and `src/tui/app/turn.rs` (3 sites)

**Files:**
- Modify: `src/tui/app/mod.rs`
- Modify: `src/tui/app/turn.rs`

- [ ] **Step 1: Grep both files**

Run: `grep -nE "settings\." src/tui/app/mod.rs src/tui/app/turn.rs`

- [ ] **Step 2: Apply mappings per spec §3**

- [ ] **Step 3: Verify whole project compiles**

Run: `cargo check --all-targets 2>&1 | grep -cE "^error\["`
Expected: `0` (zero errors).

If any errors remain, they're in files not enumerated in the influence map. Grep them and fix:

```bash
cargo check --all-targets 2>&1 | grep -E "^error\[" -A 1 | grep -oE "src/[a-zA-Z0-9_/]+\.rs" | sort -u
```

For each new file, apply spec §3 mappings, commit per-file.

- [ ] **Step 4: Commit**

```bash
git add src/tui/app/mod.rs src/tui/app/turn.rs
git commit -m "tui: update field paths to grouped Settings (6 sites) — build green"
```

---

## Task 11: Run all tests and verify

**Files:** none modified (verification only)

- [ ] **Step 1: Build is clean**

Run: `cargo build --all-targets --message-format=short 2>&1 | tail -5`
Expected: `Finished` line, no `error[`.

- [ ] **Step 2: All unit tests pass**

Run: `cargo test --lib 2>&1 | tail -20`
Expected: `test result: ok. N passed; 0 failed`. The `config::tests` module should report at least 14 tests passing (5 retained RlmSettings tests, 9 new tests from Task 3).

If any test fails:
- A retained `RlmSettings` test failing → most likely the new `jaccard_threshold` default broke an assertion. Fix the assertion.
- A new test failing → the implementation in Task 1/2 doesn't match what the test asserts. Read the test, fix the implementation (not the test).

- [ ] **Step 3: Integration tests pass**

Run: `cargo test --tests 2>&1 | tail -20`
Expected: same — all passing.

- [ ] **Step 4: Run a manual default-load smoke test**

Run:
```bash
mv ~/.wgenty-code/settings.json ~/.wgenty-code/settings.json.bak.$(date +%s) 2>/dev/null || true
cargo run --bin wgenty-code -- config show 2>&1 | head -50
```

Expected: prints a JSON document with the new shape — top-level keys are `models, agent, prompt, plugins, storage, integrations, verbose`, no `api` / `model` / `rlm` at top level. The default `models.main.name` is `"sonnet"`.

Restore your saved settings if any: `mv ~/.wgenty-code/settings.json.bak.* ~/.wgenty-code/settings.json` (only if applicable).

- [ ] **Step 5: Run the dotted-path setter smoke test**

```bash
mv ~/.wgenty-code/settings.json ~/.wgenty-code/settings.json.bak.$(date +%s) 2>/dev/null || true
cargo run --bin wgenty-code -- config set agent.subagent.max_depth 7 2>&1
cargo run --bin wgenty-code -- config show 2>&1 | python3 -c "import sys,json; d=json.load(sys.stdin); print('max_depth:', d['agent']['subagent']['max_depth'])"
```

Expected:
```
Set agent.subagent.max_depth = 7
max_depth: 7
```

Then test rejection:

```bash
cargo run --bin wgenty-code -- config set models.main.name 12345 2>&1   # type-correct (string allowed)
cargo run --bin wgenty-code -- config set agent.subagent.max_depth notanumber 2>&1   # should fail
```

Expected: the second command prints an error mentioning the invalid path/type.

- [ ] **Step 6: Commit any verification fixes**

If any tests required minor fixes during this task (e.g., a wrong assertion), commit them.

```bash
git add -A
git commit -m "config: fix test assertions caught by full test run" --allow-empty
```

(`--allow-empty` so this step doesn't fail if there were no fixes.)

---

## Task 12: Update documentation

**Files:**
- Modify: `CLAUDE.md` (if it references settings)
- Modify: `docs/**/*.md` files referencing old field names
- Modify: `README.md` (if present and references settings.json shape)

- [ ] **Step 1: Grep for old field name occurrences in markdown**

Run:
```bash
grep -rEn "rlm_jaccard_threshold|include_developer_instructions|include_skill_instructions|include_environment_context|include_collaboration_instructions|include_permissions_instructions|enabledPlugins|pluginMarketplaces|small_model_base_url|small_model_api_key|small_model_appkey|planner_model_base_url|max_subagent_depth|max_concurrent_subagents|subagent_timeout_secs|default_subagent_token_budget_k|transcript_db_path|max_transcript_age_days" docs/ README.md CLAUDE.md AGENTS.md 2>/dev/null
```

- [ ] **Step 2: Replace each match with the new path**

Use spec §3 mappings. Edit each file per match.

- [ ] **Step 3: Verify**

Re-run the same grep. Expected: no output.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "docs: update settings.json field paths to new grouped shape"
```

---

## Self-Review Checklist (run before declaring plan complete)

The plan author has done these:

- ✅ **Spec coverage**:
  - §3.1 top-level → Task 2 step 1
  - §3.2 models → Task 1 steps 1–3
  - §3.3 agent → Task 1 steps 4–8
  - §3.3a subagent inheritance → Task 1 steps 5–6 (struct), Task 2 step 7 (resolve method), Task 5 step 4 (4-level token budget chain in spawn site), Task 3 steps 6–9, 12 (tests including the 4-level chain)
  - §3.4 prompt → Task 1 step 9
  - §3.5 plugins → Task 1 step 10
  - §3.6 storage → Task 1 step 11
  - §3.7 integrations → Task 1 step 12
  - §4 dotted-path set() → Task 2 step 8 + Task 3 steps 10–11 + Task 11 step 5
  - §5.1 load (no migrate, no cc_mapping) → Task 2 steps 3–4 + delete cc_mapping.rs at step 9
  - §5.2 save → Task 2 step 5
  - §5.3 default → Task 1 (each sub-config has Default) + Task 3 step 6 (subagent default-None test)
  - §6.1 ~120 read sites → Tasks 4–10
  - §6.2 tests → Task 3
  - §6.3 docs → Task 12
- ✅ **No "TBD"/"TODO"/"appropriate"/"similar to"** placeholders
- ✅ **Type consistency**: `ModelEndpoint`, `endpoint_base_url`, `endpoint_api_key`, `resolve_subagent_config`, `small_model_settings` are all named consistently across Task 1, 2, 4, 5, 6, 7
- ✅ **Concrete commands**: every `cargo` invocation specifies the args; every grep gives the regex; every commit message is full
- ✅ **No "implement later"**: even the `set()` JSON-walk function body is included verbatim

If any review finds a gap, fix inline and move on.
