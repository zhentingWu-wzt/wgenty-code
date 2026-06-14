# RLM Toggle: Fine-Grained Control Switches for RLM Architecture

**Status:** approved  
**Date:** 2026-06-14  
**Author:** wuzhenting  

## Problem

The RLM (Recursive Language Model) architecture is currently always-on with no way to disable it. There is no mechanism to:

1. Turn off the entire RLM system
2. Hide the `delegate` tool from the model while keeping `task` working
3. Prevent `task` from auto-routing complex tasks to the RLM pipeline

Existing RLM-related config keys (`rlm_retry_enabled`, `rlm_max_replan_cycles`) are scattered as flat fields rather than grouped logically.

## Design

### Configuration Structure

Introduce a nested `RlmSettings` struct consolidating all RLM config:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RlmSettings {
    /// Master kill switch: when false, RLM is completely unavailable.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Whether the `delegate` tool is registered and visible to the model.
    #[serde(default = "default_true")]
    pub delegate_tool: bool,

    /// Whether `task` tool auto-routes complex tasks to RLM pipeline.
    #[serde(default = "default_true")]
    pub auto_routing: bool,

    /// Whether RLM pipeline retries failed sub-tasks.
    #[serde(default = "default_true")]
    pub retry_enabled: bool,

    /// Max re-plan cycles when executor failure rate exceeds 50%.
    #[serde(default = "default_rlm_max_replan")]
    pub max_replan_cycles: usize,
}
```

Replace in `Settings`:
- `rlm_retry_enabled: bool` → removed, now `rlm.retry_enabled`
- `rlm_max_replan_cycles: usize` → removed, now `rlm.max_replan_cycles`
- Add `rlm: RlmSettings` (serde default)

### Three-Layer Switch Logic

```
rlm.enabled = false
  └── Entire RLM unavailable (regardless of other flags)

rlm.enabled = true
  ├── delegate_tool = false  → delegate tool not registered, model can't see it
  ├── auto_routing = false   → task doesn't check complexity, always uses simple subagent
  └── both true              → full RLM capability (default behavior)
```

### Enforcement Points

| Switch | File | Location | What changes |
|--------|------|----------|--------------|
| `enabled` + `delegate_tool` | `src/daemon/state.rs` | Tool registration (~line 113) | Conditionally register `RlmDelegateTool` |
| `enabled` + `auto_routing` | `src/tools/meta/task.rs` | Sync execution path (~line 474) | Guard `is_complex_task()` check with config flags |
| `retry_enabled` | `src/tools/meta/rlm/pipeline.rs` | Pipeline retry logic | Rename `settings.rlm_retry_enabled` → `settings.rlm.retry_enabled` |
| `max_replan_cycles` | `src/tools/meta/rlm/pipeline.rs` | Re-plan logic | Rename `settings.rlm_max_replan_cycles` → `settings.rlm.max_replan_cycles` |

### Backward Compatibility

On `Settings::load()`, detect legacy flat keys and migrate into `rlm` group:

```rust
// In CcConfigMapper or a dedicated migration step:
// - If rlm_retry_enabled found in JSON, move to rlm.retry_enabled
// - If rlm_max_replan_cycles found in JSON, move to rlm.max_replan_cycles
```

CLI `config set` retains legacy aliases:
- `rlm_retry_enabled` → maps to `rlm.retry_enabled`
- `rlm_max_replan_cycles` → maps to `rlm.max_replan_cycles`
- New keys: `rlm.enabled`, `rlm.delegate_tool`, `rlm.auto_routing`

### Files Changed

| File | Change type |
|------|-------------|
| `src/config/mod.rs` | Add `RlmSettings` struct, replace flat fields, add migration, add `set()` aliases |
| `src/daemon/state.rs` | Conditionally register `RlmDelegateTool` |
| `src/tools/meta/task.rs` | Guard auto-routing with `rlm.enabled && rlm.auto_routing` |
| `src/tools/meta/rlm/pipeline.rs` | Update field references: `rlm_retry_enabled` → `rlm.retry_enabled`, etc. |

### Usage

```bash
# Disable all RLM
wgenty-code config set rlm.enabled false

# Keep task but hide delegate tool
wgenty-code config set rlm.delegate_tool false

# Keep delegate but disable auto-routing in task
wgenty-code config set rlm.auto_routing false
```

### Defaults

All flags default to `true`, preserving current behavior. Existing users see no change unless they explicitly toggle a switch.
