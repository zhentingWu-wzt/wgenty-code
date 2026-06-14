# RLM Toggle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add fine-grained 3-layer control switches to enable/disable the RLM architecture through a nested `RlmSettings` config group.

**Architecture:** Introduce a new `RlmSettings` struct consolidating all RLM config into `Settings.rlm`. Three independent boolean switches (`enabled`, `delegate_tool`, `auto_routing`) control tool registration and task routing behavior. Legacy flat keys are migrated on load and aliased in CLI `set`.

**Tech Stack:** Rust, serde (Serialize/Deserialize), anyhow

---

## File Structure

| File | Responsibility |
|------|---------------|
| `src/config/mod.rs` | Define `RlmSettings` struct, replace flat fields in `Settings`, add migration in `load()`, extend `set()` with aliases + new keys |
| `src/daemon/state.rs` | Guard `RlmDelegateTool` registration with `settings.rlm.enabled && settings.rlm.delegate_tool` |
| `src/tools/meta/task.rs` | Guard auto-routing branch with `settings.rlm.enabled && settings.rlm.auto_routing` |

---

### Task 1: Define `RlmSettings` struct and integrate into `Settings`

**Files:**
- Modify: `src/config/mod.rs`

- [ ] **Step 1: Add `RlmSettings` struct definition**

Insert after the `GuardianSettings` impl Default block (after line 215) and before `impl Default for Settings`:

```rust
/// RLM (Recursive Language Model) pipeline settings.
/// Controls the delegate tool, auto-routing in task, and pipeline behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RlmSettings {
    /// Master kill switch: when false, RLM is completely unavailable
    /// regardless of other flags.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Whether the `delegate` tool is registered and visible to the model.
    #[serde(default = "default_true")]
    pub delegate_tool: bool,
    /// Whether `task` tool auto-routes complex tasks to the RLM pipeline.
    #[serde(default = "default_true")]
    pub auto_routing: bool,
    /// Whether RLM pipeline retries failed sub-tasks.
    #[serde(default = "default_true")]
    pub retry_enabled: bool,
    /// Max re-plan cycles when RLM executor failure rate exceeds 50%.
    /// 0 = disabled (no feedback loop). Default: 2.
    #[serde(default = "default_rlm_max_replan")]
    pub max_replan_cycles: usize,
}

impl Default for RlmSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            delegate_tool: true,
            auto_routing: true,
            retry_enabled: true,
            max_replan_cycles: 2,
        }
    }
}
```

- [ ] **Step 2: Replace flat RLM fields in `Settings` with `rlm: RlmSettings`**

In the `Settings` struct, replace lines 50-57:

```rust
// REMOVE these lines:
    /// Whether RLM pipeline retries failed sub-tasks once with a different
    /// prompt angle. Default: true.
    #[serde(default = "default_rlm_retry")]
    pub rlm_retry_enabled: bool,
    /// Maximum re-plan cycles when RLM executor failure rate exceeds 50%.
    /// 0 = disabled (no feedback loop). Default: 2.
    #[serde(default = "default_rlm_max_replan")]
    pub rlm_max_replan_cycles: usize,

// ADD in their place (same location, after subagent_timeout_secs):
    /// RLM (Recursive Language Model) pipeline settings.
    #[serde(default)]
    pub rlm: RlmSettings,
```

- [ ] **Step 3: Remove the now-unused default functions**

Remove lines 145-150 (the `default_rlm_retry` and `default_rlm_max_replan` functions). Keep `default_true` and `default_rlm_max_replan` since they're still used by `RlmSettings`'s serde defaults.

Wait — `default_rlm_max_replan` is referenced in `RlmSettings` serde default, so keep it. Only remove `default_rlm_retry`:

```rust
// REMOVE (lines 145-147):
fn default_rlm_retry() -> bool {
    true
}
```

- [ ] **Step 4: Update `Settings::default()` to use `rlm` group**

In the `Default for Settings` impl (around lines 232-234), replace:

```rust
// REMOVE:
            rlm_retry_enabled: true,
            rlm_max_replan_cycles: 2,

// ADD in same position:
            rlm: RlmSettings::default(),
```

- [ ] **Step 5: Build check**

```bash
cargo check 2>&1
```

Expected: compilation succeeds. Any references to `settings.rlm_retry_enabled` or `settings.rlm_max_replan_cycles` will fail — we'll fix those in later tasks (Task 3 covers `set()`, Task 2 confirms no runtime code references these yet).

- [ ] **Step 6: Commit**

```bash
git add src/config/mod.rs
git commit -m "feat: add RlmSettings struct and integrate into Settings

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: Add backward-compat migration in `load()`

**Files:**
- Modify: `src/config/mod.rs`

- [ ] **Step 1: Add migration in `Settings::load()`**

In `load()` (line 284-286), insert migration after `serde_json::from_str` and before `apply_mappings`:

```rust
            let content = std::fs::read_to_string(&config_path)?;
            let mut settings: Settings = serde_json::from_str(&content)?;
            // Migrate legacy flat RLM keys into the rlm group.
            Self::migrate_rlm_settings(&content, &mut settings);
            cc_mapping::CcConfigMapper::apply_mappings(&mut settings);
```

- [ ] **Step 2: Implement `migrate_rlm_settings` private method**

Add to the `impl Settings` block (after `load()`):

```rust
    /// Migrate legacy flat `rlm_retry_enabled` / `rlm_max_replan_cycles` keys
    /// from the raw JSON into `Settings.rlm`. Only touch rlm fields when the
    /// raw JSON contains the legacy key AND the rlm group was not provided.
    fn migrate_rlm_settings(raw_json: &str, settings: &mut Settings) {
        let Ok(raw) = serde_json::from_str::<serde_json::Value>(raw_json) else {
            return;
        };
        // If the new "rlm" group is present, legacy keys are ignored.
        if raw.get("rlm").is_some() {
            return;
        }
        let mut migrated = false;
        if let Some(val) = raw.get("rlm_retry_enabled").and_then(|v| v.as_bool()) {
            settings.rlm.retry_enabled = val;
            migrated = true;
        }
        if let Some(val) = raw.get("rlm_max_replan_cycles").and_then(|v| v.as_u64()) {
            settings.rlm.max_replan_cycles = val as usize;
            migrated = true;
        }
        if migrated {
            tracing::info!(
                target: "config",
                rlm_retry = settings.rlm.retry_enabled,
                rlm_replan = settings.rlm.max_replan_cycles,
                "Migrated legacy RLM config keys into rlm group"
            );
        }
    }
```

- [ ] **Step 3: Build check**

```bash
cargo check 2>&1
```

Expected: compiles cleanly.

- [ ] **Step 4: Commit**

```bash
git add src/config/mod.rs
git commit -m "feat: add legacy RLM config key migration on load

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3: Update `set()` with new keys and legacy aliases

**Files:**
- Modify: `src/config/mod.rs`

- [ ] **Step 1: Replace old RLM key handlers in `set()`**

In the `set()` method (lines 351-352), replace:

```rust
// REMOVE:
            "rlm_retry_enabled" => settings.rlm_retry_enabled = value.parse().unwrap_or(true),
            "rlm_max_replan_cycles" => settings.rlm_max_replan_cycles = value.parse().unwrap_or(2),

// ADD:
            // rlm group — new canonical keys
            "rlm.enabled" => settings.rlm.enabled = value.parse().unwrap_or(true),
            "rlm.delegate_tool" => settings.rlm.delegate_tool = value.parse().unwrap_or(true),
            "rlm.auto_routing" => settings.rlm.auto_routing = value.parse().unwrap_or(true),
            "rlm.retry_enabled" => settings.rlm.retry_enabled = value.parse().unwrap_or(true),
            "rlm.max_replan_cycles" => {
                settings.rlm.max_replan_cycles = value.parse().unwrap_or(2)
            }
            // legacy aliases (backward compatible)
            "rlm_retry_enabled" => settings.rlm.retry_enabled = value.parse().unwrap_or(true),
            "rlm_max_replan_cycles" => {
                settings.rlm.max_replan_cycles = value.parse().unwrap_or(2)
            }
```

- [ ] **Step 2: Build check**

```bash
cargo check 2>&1
```

Expected: compiles cleanly.

- [ ] **Step 3: Commit**

```bash
git add src/config/mod.rs
git commit -m "feat: add rlm.* config set keys with legacy alias support

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 4: Conditionally register delegate tool in daemon

**Files:**
- Modify: `src/daemon/state.rs`

- [ ] **Step 1: Guard delegate tool registration**

Replace lines 113-118 (unconditional registration):

```rust
// REMOVE:
            let rlm_tool = crate::tools::meta::rlm::RlmDelegateTool::new(
                app_state.settings.clone(),
                weak_reg.clone(),
                progress_store.clone(),
            );
            registry.register(Box::new(rlm_tool));

// ADD:
            if app_state.settings.rlm.enabled && app_state.settings.rlm.delegate_tool {
                let rlm_tool = crate::tools::meta::rlm::RlmDelegateTool::new(
                    app_state.settings.clone(),
                    weak_reg.clone(),
                    progress_store.clone(),
                );
                registry.register(Box::new(rlm_tool));
            }
```

- [ ] **Step 2: Build check**

```bash
cargo check 2>&1
```

Expected: compiles cleanly.

- [ ] **Step 3: Commit**

```bash
git add src/daemon/state.rs
git commit -m "feat: conditionally register delegate tool based on rlm config

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 5: Guard task auto-routing with RLM config

**Files:**
- Modify: `src/tools/meta/task.rs`

- [ ] **Step 1: Add RLM config guard to auto-routing branch**

Replace line 474:

```rust
// BEFORE:
            let (result, routing_reason) = if is_complex_task(&full_prompt, use_small) {

// AFTER:
            let (result, routing_reason) = if self.settings.rlm.enabled
                && self.settings.rlm.auto_routing
                && is_complex_task(&full_prompt, use_small)
            {
```

- [ ] **Step 2: Build check**

```bash
cargo check 2>&1
```

Expected: compiles cleanly.

- [ ] **Step 3: Commit**

```bash
git add src/tools/meta/task.rs
git commit -m "feat: guard task auto-routing with rlm.enabled and rlm.auto_routing

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 6: Write unit tests

**Files:**
- Modify: `src/config/mod.rs`

- [ ] **Step 1: Add `#[cfg(test)]` test module at end of config/mod.rs**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rlm_settings_default_all_enabled() {
        let rlm = RlmSettings::default();
        assert!(rlm.enabled);
        assert!(rlm.delegate_tool);
        assert!(rlm.auto_routing);
        assert!(rlm.retry_enabled);
        assert_eq!(rlm.max_replan_cycles, 2);
    }

    #[test]
    fn test_rlm_settings_deserialize_partial() {
        let json = r#"{"enabled": false}"#;
        let rlm: RlmSettings = serde_json::from_str(json).unwrap();
        assert!(!rlm.enabled);
        // Other fields should use defaults
        assert!(rlm.delegate_tool);
        assert!(rlm.auto_routing);
        assert!(rlm.retry_enabled);
        assert_eq!(rlm.max_replan_cycles, 2);
    }

    #[test]
    fn test_rlm_settings_deserialize_full() {
        let json = r#"{
            "enabled": false,
            "delegate_tool": false,
            "auto_routing": false,
            "retry_enabled": false,
            "max_replan_cycles": 0
        }"#;
        let rlm: RlmSettings = serde_json::from_str(json).unwrap();
        assert!(!rlm.enabled);
        assert!(!rlm.delegate_tool);
        assert!(!rlm.auto_routing);
        assert!(!rlm.retry_enabled);
        assert_eq!(rlm.max_replan_cycles, 0);
    }

    #[test]
    fn test_migrate_rlm_legacy_keys() {
        // Simulate old config format with flat keys
        let old_json = r#"{
            "model": "sonnet",
            "rlm_retry_enabled": false,
            "rlm_max_replan_cycles": 5
        }"#;
        let mut settings = Settings::default();
        Settings::migrate_rlm_settings(old_json, &mut settings);
        // Legacy values should be copied into rlm group
        assert!(!settings.rlm.retry_enabled);
        assert_eq!(settings.rlm.max_replan_cycles, 5);
        // Fields not in old JSON stay at defaults
        assert!(settings.rlm.enabled);
        assert!(settings.rlm.delegate_tool);
    }

    #[test]
    fn test_migrate_rlm_no_override_when_group_present() {
        // When the new "rlm" group is present, legacy flat keys are ignored
        let json = r#"{
            "rlm": {"enabled": false, "retry_enabled": true},
            "rlm_retry_enabled": false
        }"#;
        let mut settings = Settings::default();
        Settings::migrate_rlm_settings(json, &mut settings);
        // rlm group takes priority, legacy key is ignored
        assert!(!settings.rlm.enabled);
        assert!(settings.rlm.retry_enabled); // from rlm group, not overridden
    }

    #[test]
    fn test_settings_default_includes_rlm() {
        let settings = Settings::default();
        assert!(settings.rlm.enabled);
        assert!(settings.rlm.delegate_tool);
        assert!(settings.rlm.auto_routing);
    }

    #[test]
    fn test_rlm_deserialize_in_settings() {
        let json = r#"{
            "api": {"base_url": "http://localhost"},
            "model": "test",
            "verbose": false,
            "working_dir": ".",
            "memory": {"enabled": false, "path": ".", "consolidation_interval": 24, "max_memories": 100},
            "voice": {"enabled": false, "push_to_talk": false, "silence_threshold": 0.0, "sample_rate": 16000},
            "plugins": {"enabled": false, "plugin_dir": ".", "auto_update": false},
            "rlm": {"enabled": false, "delegate_tool": false}
        }"#;
        let settings: Settings = serde_json::from_str(json).unwrap();
        assert!(!settings.rlm.enabled);
        assert!(!settings.rlm.delegate_tool);
        // Unspecified rlm fields use defaults
        assert!(settings.rlm.auto_routing);
        assert!(settings.rlm.retry_enabled);
        assert_eq!(settings.rlm.max_replan_cycles, 2);
    }
}
```

- [ ] **Step 2: Run the tests**

```bash
cargo test --lib config::tests -- --nocapture 2>&1
```

Expected: all 7 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add src/config/mod.rs
git commit -m "test: add unit tests for RlmSettings and migration logic

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 7: Full build and existing test verification

**Files:** (none — verification only)

- [ ] **Step 1: Full cargo check**

```bash
cargo check 2>&1
```

Expected: no errors, no warnings (or only pre-existing warnings).

- [ ] **Step 2: Run full test suite**

```bash
cargo test 2>&1
```

Expected: all existing tests pass, new config tests pass.

- [ ] **Step 3: Verify CLI config set works**

```bash
cargo run -- config set rlm.enabled false 2>&1
cargo run -- config get 2>&1 | grep -A 6 '"rlm"'
```

Expected: `rlm.enabled` is `false` in printed config.

- [ ] **Step 4: Reset config and verify defaults**

```bash
cargo run -- config reset 2>&1
cargo run -- config get 2>&1 | grep -A 6 '"rlm"'
```

Expected: `rlm.enabled` is `true` (default).

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "chore: verify full build and tests pass with RLM toggle changes

Co-Authored-By: Claude <noreply@anthropic.com>"
```
