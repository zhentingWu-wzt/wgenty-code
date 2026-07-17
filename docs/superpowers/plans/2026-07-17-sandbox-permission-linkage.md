# Sandbox ↔ Permission Mode Linkage Implementation Plan

> **Status:** Completed (2026-07-17). Verification: `docs/superpowers/reports/2026-07-17-sandbox-permission-linkage-verify.md`.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Link each effective permission mode to a sandbox SecurityLevel + FailMode (Profile Matrix), with settings overrides, fail-closed shell exec under Plan/Normal/AcceptEdits, Yolo degrade-with-mark, and ToolContext-only mode plumbing.

**Architecture:** Add pure `SandboxPolicyResolver` in `src/sandbox/policy.rs` that maps `EffectiveMode` + `SandboxSettings` → level/fail_mode/profile. Plumb `EffectiveMode` on `ToolContext` (no process-global sandbox lock). Exec tools (`execute_command`, `exec_command` via session manager, `run_test`) build profiles from the resolver and apply FailMode instead of silent direct spawn. TUI `AgentMode::PlanMode` becomes `EffectiveMode::Plan` for sandbox while permission auto-approve still maps Plan → Normal.

**Tech Stack:** Rust 2021, Tokio, Serde, existing `SandboxManager` / `SandboxConfig` / `ToolContext`, Cargo unit tests.

**Design Doc:** `docs/superpowers/specs/2026-07-17-sandbox-permission-linkage-design.md`

---

## File Map

### New files

| Path | Responsibility |
|------|----------------|
| `src/sandbox/policy.rs` | `EffectiveMode`, `FailMode`, `PolicySource`, `ResolvedSandboxPolicy`, `SandboxPolicyResolver`, unit tests |
| `src/config/sandbox_settings.rs` | `SandboxSettings` (enabled, defaults_by_mode, fail_mode_by_mode) + defaults + serde tests |
| `src/tools/execution/sandbox_exec.rs` | Shared helpers: resolve profile, apply fail mode, attach metadata (used by execute_command / session_manager / run_test) |

### Existing files (focused changes)

| Path | Change |
|------|--------|
| `src/sandbox/mod.rs` | `mod policy; pub use policy::*;` |
| `src/sandbox/config.rs` | Derive `Serialize, Deserialize` on `SecurityLevel` (`snake_case`) |
| `src/config/mod.rs` | `pub mod sandbox_settings; pub use …` |
| `src/config/services.rs` | `IntegrationsConfig.sandbox: SandboxSettings` |
| `src/agent/identity.rs` | `ToolContext.effective_mode: EffectiveMode` |
| `src/tools/execution/execute_command.rs` | Resolver + HardFail/Degrade + metadata; drop hard-coded Minimal |
| `src/tools/execution/session_manager.rs` | Per-spawn resolve + fail mode (no silent bare spawn) |
| `src/tools/execution/exec_command.rs` | Pass `ToolContext` mode into spawn; `execute_with_context` |
| `src/tools/execution/run_test.rs` | Mode-based level; `allow_network` only upgrades network |
| `src/tools/mod.rs` | Fix `ToolContext { … }` literals in tests; optional settings inject later |
| `src/tools/executor.rs` | Test literals + ensure context mode preserved |
| `src/daemon/state.rs` | Store `effective_mode` (or map from extended mode); expose to tool path |
| `src/daemon/handlers.rs` | Fill `ToolContext.effective_mode` from session state |
| `src/daemon/models.rs` + TUI client/mode push | Carry Plan as sandbox-effective mode (see Task 6) |
| `src/tui/app/types.rs` | `AgentMode → EffectiveMode` helper |
| `src/cli/args.rs` | Real enable/disable/status (P1) |
| `settings.json.template` | `integrations.sandbox` block |
| `docs/SANDBOX.md`, `CHANGELOG.md`, `WGENTY.md` | Document matrix + BREAKING network default |

### Out of scope (do not implement in this plan)

- Linux seccomp-bpf, Windows Restricted Tokens
- OS sandbox for `file_write` / `file_edit` / `apply_patch`
- Independent sandbox-level UI knob
- Dual global mode lock for sandbox resolution

---

## Task 1: Settings types + SecurityLevel serde

**Files:**
- Create: `src/config/sandbox_settings.rs`
- Modify: `src/config/mod.rs`
- Modify: `src/config/services.rs`
- Modify: `src/sandbox/config.rs` (`SecurityLevel` serde)
- Modify: `settings.json.template`
- Test: unit tests in `sandbox_settings.rs`

- [ ] **Step 1: Write failing tests**

```rust
// src/config/sandbox_settings.rs
#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::SecurityLevel;

    #[test]
    fn sandbox_settings_defaults() {
        let s = SandboxSettings::default();
        assert!(s.enabled);
        assert!(s.defaults_by_mode.is_empty());
        assert!(s.fail_mode_by_mode.is_empty());
    }

    #[test]
    fn sandbox_settings_serde_partial() {
        let json = r#"{
            "enabled": false,
            "defaults_by_mode": { "normal": "minimal" },
            "fail_mode_by_mode": { "yolo": "hard_fail" }
        }"#;
        let s: SandboxSettings = serde_json::from_str(json).unwrap();
        assert!(!s.enabled);
        assert_eq!(
            s.defaults_by_mode.get(&EffectiveModeKey::Normal),
            Some(&SecurityLevel::Minimal)
        );
    }

    #[test]
    fn security_level_serde_snake() {
        let v = serde_json::to_string(&SecurityLevel::AcceptPlaceholder).ok();
        // Use real variants:
        assert_eq!(serde_json::to_string(&SecurityLevel::Minimal).unwrap(), "\"minimal\"");
        assert_eq!(serde_json::to_string(&SecurityLevel::Standard).unwrap(), "\"standard\"");
        assert_eq!(serde_json::to_string(&SecurityLevel::High).unwrap(), "\"high\"");
        assert_eq!(serde_json::to_string(&SecurityLevel::Paranoid).unwrap(), "\"paranoid\"");
    }
}
```

Note: `EffectiveModeKey` is a serde-friendly copy of mode names for HashMap keys (`plan`, `normal`, `accept_edits`, `yolo`). Prefer:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectiveModeKey {
    Plan,
    Normal,
    AcceptEdits,
    Yolo,
}
```

Put `EffectiveModeKey` in `sandbox_settings.rs` **or** re-export from `policy.rs` once Task 2 lands. For Task 1 only, define the key enum in settings (Task 2 can `pub use` / merge into `EffectiveMode` with same serde names).

**Preferred shape (implement this):**

```rust
// src/config/sandbox_settings.rs
use crate::sandbox::SecurityLevel;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModeKey {
    Plan,
    Normal,
    AcceptEdits,
    Yolo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FailModeSetting {
    #[default]
    HardFail,
    DegradeWithMark,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxSettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub defaults_by_mode: HashMap<ModeKey, SecurityLevel>,
    #[serde(default)]
    pub fail_mode_by_mode: HashMap<ModeKey, FailModeSetting>,
}

fn default_true() -> bool { true }

impl Default for SandboxSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            defaults_by_mode: HashMap::new(),
            fail_mode_by_mode: HashMap::new(),
        }
    }
}
```

- [ ] **Step 2: Run tests — expect FAIL**

```bash
cargo test sandbox_settings_defaults -- --nocapture
```

- [ ] **Step 3: Implement settings + wire IntegrationsConfig**

```rust
// services.rs — IntegrationsConfig
#[serde(default)]
pub sandbox: SandboxSettings,
```

```rust
// sandbox/config.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityLevel { ... }
```

Add `use serde::{Serialize, Deserialize};` to `config.rs`.

Template:

```json
"sandbox": {
  "enabled": true,
  "defaults_by_mode": {},
  "fail_mode_by_mode": {}
}
```

under `integrations`.

- [ ] **Step 4: Run tests — expect PASS**

```bash
cargo test sandbox_settings_ -- --nocapture
cargo check
```

- [ ] **Step 5: Commit**

```bash
git add src/config/sandbox_settings.rs src/config/mod.rs src/config/services.rs \
  src/sandbox/config.rs settings.json.template
git commit -m "feat(config): add integrations.sandbox settings for mode linkage"
```

---

## Task 2: Policy resolver (pure) + matrix tests

**Files:**
- Create: `src/sandbox/policy.rs`
- Modify: `src/sandbox/mod.rs`
- Optionally re-export `FailMode` mapping from settings `FailModeSetting`

- [ ] **Step 1: Write failing matrix tests first**

```rust
// src/sandbox/policy.rs #[cfg(test)]
use super::*;
use crate::config::SandboxSettings;
use std::path::PathBuf;

#[test]
fn resolve_plan_is_high_hard_fail() {
    let p = SandboxPolicyResolver::resolve(
        EffectiveMode::Plan,
        &SandboxSettings::default(),
        PathBuf::from("/tmp/ws"),
    );
    assert_eq!(p.level, SecurityLevel::High);
    assert_eq!(p.fail_mode, FailMode::HardFail);
    assert!(p.enabled);
    assert_eq!(p.source, PolicySource::Default);
}

#[test]
fn resolve_normal_is_standard_hard_fail() {
    let p = SandboxPolicyResolver::resolve(
        EffectiveMode::Normal,
        &SandboxSettings::default(),
        PathBuf::from("/tmp/ws"),
    );
    assert_eq!(p.level, SecurityLevel::Standard);
    assert_eq!(p.fail_mode, FailMode::HardFail);
}

#[test]
fn resolve_accept_edits_shell_standard_hard_fail() {
    let p = SandboxPolicyResolver::resolve(
        EffectiveMode::AcceptEdits,
        &SandboxSettings::default(),
        PathBuf::from("/tmp/ws"),
    );
    assert_eq!(p.level, SecurityLevel::Standard);
    assert_eq!(p.fail_mode, FailMode::HardFail);
}

#[test]
fn resolve_yolo_is_minimal_degrade() {
    let p = SandboxPolicyResolver::resolve(
        EffectiveMode::Yolo,
        &SandboxSettings::default(),
        PathBuf::from("/tmp/ws"),
    );
    assert_eq!(p.level, SecurityLevel::Minimal);
    assert_eq!(p.fail_mode, FailMode::DegradeWithMark);
}

#[test]
fn settings_override_level() {
    let mut s = SandboxSettings::default();
    s.defaults_by_mode.insert(
        crate::config::ModeKey::Normal,
        SecurityLevel::Minimal,
    );
    let p = SandboxPolicyResolver::resolve(
        EffectiveMode::Normal,
        &s,
        PathBuf::from("/tmp/ws"),
    );
    assert_eq!(p.level, SecurityLevel::Minimal);
    assert_eq!(p.source, PolicySource::SettingsOverride);
}

#[test]
fn enabled_false_forces_degrade() {
    let mut s = SandboxSettings::default();
    s.enabled = false;
    let p = SandboxPolicyResolver::resolve(
        EffectiveMode::Plan,
        &s,
        PathBuf::from("/tmp/ws"),
    );
    assert!(!p.enabled);
    assert_eq!(p.fail_mode, FailMode::DegradeWithMark);
    assert_eq!(p.source, PolicySource::Disabled);
}

#[test]
fn missing_mode_defaults_normal() {
    // Document: callers use EffectiveMode::default() == Normal
    assert_eq!(EffectiveMode::default(), EffectiveMode::Normal);
}
```

- [ ] **Step 2: Run — expect FAIL**

```bash
cargo test resolve_plan_is_high_hard_fail -- --nocapture
```

- [ ] **Step 3: Implement `policy.rs`**

```rust
//! Mode → sandbox profile resolution (pure).

use crate::config::{FailModeSetting, ModeKey, SandboxSettings};
use crate::sandbox::{NetworkPolicy, SandboxConfig, SandboxProfile, SecurityLevel};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectiveMode {
    Plan,
    #[default]
    Normal,
    AcceptEdits,
    Yolo,
}

impl EffectiveMode {
    pub fn as_mode_key(self) -> ModeKey { /* match */ }
    pub fn as_str(self) -> &'static str { /* "plan" | "normal" | ... */ }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailMode {
    HardFail,
    DegradeWithMark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicySource {
    Default,
    SettingsOverride,
    Disabled,
}

#[derive(Debug, Clone)]
pub struct ResolvedSandboxPolicy {
    pub level: SecurityLevel,
    pub fail_mode: FailMode,
    pub profile: SandboxProfile,
    pub enabled: bool,
    pub source: PolicySource,
}

pub struct SandboxPolicyResolver;

impl SandboxPolicyResolver {
    pub fn resolve(
        mode: EffectiveMode,
        settings: &SandboxSettings,
        workspace: impl Into<PathBuf>,
    ) -> ResolvedSandboxPolicy {
        let workspace = workspace.into();
        if !settings.enabled {
            let level = Self::default_level(mode);
            let profile = Self::build_profile(level, &workspace, None);
            return ResolvedSandboxPolicy {
                level,
                fail_mode: FailMode::DegradeWithMark,
                profile,
                enabled: false,
                source: PolicySource::Disabled,
            };
        }

        let key = mode.as_mode_key();
        let (level, level_overridden) = match settings.defaults_by_mode.get(&key) {
            Some(l) => (*l, true),
            None => (Self::default_level(mode), false),
        };
        let (fail_mode, fail_overridden) = match settings.fail_mode_by_mode.get(&key) {
            Some(FailModeSetting::HardFail) => (FailMode::HardFail, true),
            Some(FailModeSetting::DegradeWithMark) => (FailMode::DegradeWithMark, true),
            None => (Self::default_fail_mode(mode), false),
        };
        let source = if level_overridden || fail_overridden {
            PolicySource::SettingsOverride
        } else {
            PolicySource::Default
        };
        let profile = Self::build_profile(level, &workspace, None);
        ResolvedSandboxPolicy {
            level,
            fail_mode,
            profile,
            enabled: true,
            source,
        }
    }

    /// Like resolve, then force NetworkPolicy::Full (run_test allow_network).
    pub fn resolve_with_network(
        mode: EffectiveMode,
        settings: &SandboxSettings,
        workspace: impl Into<PathBuf>,
        network: Option<NetworkPolicy>,
    ) -> ResolvedSandboxPolicy {
        let mut p = Self::resolve(mode, settings, workspace);
        if let Some(n) = network {
            p.profile.network = n;
        }
        p
    }

    fn default_level(mode: EffectiveMode) -> SecurityLevel {
        match mode {
            EffectiveMode::Plan => SecurityLevel::High,
            EffectiveMode::Normal | EffectiveMode::AcceptEdits => SecurityLevel::Standard,
            EffectiveMode::Yolo => SecurityLevel::Minimal,
        }
    }

    fn default_fail_mode(mode: EffectiveMode) -> FailMode {
        match mode {
            EffectiveMode::Yolo => FailMode::DegradeWithMark,
            _ => FailMode::HardFail,
        }
    }

    fn build_profile(
        level: SecurityLevel,
        workspace: &std::path::Path,
        network: Option<NetworkPolicy>,
    ) -> SandboxProfile {
        let mut b = SandboxConfig::builder(workspace.to_path_buf()).security_level(level);
        if let Some(n) = network {
            b = b.network(n);
        }
        // Match execute_command today: allow HOME read for toolchains
        if let Ok(home) = std::env::var("HOME") {
            b = b.readable_path(home);
        }
        let mut profile = b.build();
        profile.workdir = Some(workspace.to_path_buf());
        profile
    }
}
```

Export from `mod.rs`. Map `ModeKey` ↔ `EffectiveMode` consistently.

- [ ] **Step 4: Run tests — expect PASS**

```bash
cargo test --lib sandbox::policy -- --nocapture
```

- [ ] **Step 5: Commit**

```bash
git add src/sandbox/policy.rs src/sandbox/mod.rs
git commit -m "feat(sandbox): add SandboxPolicyResolver mode matrix"
```

---

## Task 3: ToolContext.effective_mode

**Files:**
- Modify: `src/agent/identity.rs`
- Modify: every `ToolContext { ... }` construction (compile-driven)
- Modify: `src/tui/app/types.rs` — `to_effective_mode()`

- [ ] **Step 1: Extend struct**

```rust
// identity.rs
use crate::sandbox::EffectiveMode; // or crate::sandbox::policy::EffectiveMode

pub struct ToolContext<'a> {
    pub agent: &'a AgentExecutionContext,
    pub invocation_id: ToolInvocationId,
    pub origin_turn_id: Option<&'a str>,
    pub workdir: Option<&'a std::path::Path>,
    /// Sandbox/permission effective mode for this call. Default Normal.
    pub effective_mode: EffectiveMode,
}
```

- [ ] **Step 2: Fix compile errors**

```bash
cargo check 2>&1 | head -80
```

Add `effective_mode: EffectiveMode::default()` (or specific mode) at each site:

- `src/tools/mod.rs` tests
- `src/tools/executor.rs` tests
- `src/daemon/handlers.rs` (temporary Normal until Task 6)
- `src/teams/guarding_tool_port.rs` (map `root_mode` → EffectiveMode; Plan not in RootPermissionMode → Normal/AcceptEdits/Yolo only for now)
- Any other sites from compiler

Helper on RootPermissionMode:

```rust
impl RootPermissionMode {
    pub fn to_effective_mode(self) -> EffectiveMode {
        match self {
            Self::Normal => EffectiveMode::Normal,
            Self::AcceptEdits => EffectiveMode::AcceptEdits,
            Self::Yolo => EffectiveMode::Yolo,
        }
    }
}
```

TUI:

```rust
impl AgentMode {
    pub fn to_effective_mode(&self) -> crate::sandbox::EffectiveMode {
        match self {
            AgentMode::PlanMode => EffectiveMode::Plan,
            AgentMode::Normal => EffectiveMode::Normal,
            AgentMode::AcceptEdits => EffectiveMode::AcceptEdits,
            AgentMode::Yolo => EffectiveMode::Yolo,
        }
    }
}
```

- [ ] **Step 3: cargo test identity / tools compile tests**

```bash
cargo test execute_with_context_uses_trusted -- --nocapture
cargo check
```

- [ ] **Step 4: Commit**

```bash
git add src/agent/identity.rs src/config/agent.rs src/tui/app/types.rs \
  src/tools src/daemon src/teams
git commit -m "feat(agent): plumb EffectiveMode on ToolContext"
```

---

## Task 4: Shared sandbox exec helper + execute_command FailMode

**Files:**
- Create: `src/tools/execution/sandbox_exec.rs`
- Modify: `src/tools/execution/mod.rs` (pub mod)
- Modify: `src/tools/execution/execute_command.rs`
- Test: unit tests for metadata + fail branching (mock-free pure helpers)

- [ ] **Step 1: Write pure helper tests**

```rust
#[test]
fn hard_fail_maps_to_tool_error_code() {
    let err = sandbox_infra_to_tool_error(
        FailMode::HardFail,
        &format_sandbox_err_stub(),
        "seatbelt",
    );
    assert!(err.is_err());
    let e = err.unwrap_err();
    assert_eq!(e.code.as_deref(), Some("sandbox_spawn_failed"));
}

#[test]
fn degrade_allows_direct_flag() {
    assert!(should_degrade_to_direct(FailMode::DegradeWithMark));
    assert!(!should_degrade_to_direct(FailMode::HardFail));
}

#[test]
fn metadata_includes_bypass() {
    let m = sandbox_metadata(
        EffectiveMode::Yolo,
        SecurityLevel::Minimal,
        "none",
        true,  // bypassed
        false, // enforced
        FailMode::DegradeWithMark,
    );
    assert_eq!(m.get("sandbox_bypassed").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(m.get("permission_mode").and_then(|v| v.as_str()), Some("yolo"));
}
```

- [ ] **Step 2: Implement helpers**

```rust
// sandbox_exec.rs — sketch
pub fn load_sandbox_settings() -> SandboxSettings {
    Settings::load()
        .map(|s| s.integrations.sandbox)
        .unwrap_or_default()
}

pub fn resolve_for_context(
    mode: EffectiveMode,
    workdir: Option<&Path>,
    network_override: Option<NetworkPolicy>,
) -> ResolvedSandboxPolicy {
    let settings = load_sandbox_settings();
    let cwd = workdir
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    if let Some(n) = network_override {
        SandboxPolicyResolver::resolve_with_network(mode, &settings, cwd, Some(n))
    } else {
        SandboxPolicyResolver::resolve(mode, &settings, cwd)
    }
}

pub fn sandbox_metadata(...) -> HashMap<String, serde_json::Value> { ... }

pub fn should_degrade_to_direct(fail_mode: FailMode) -> bool {
    matches!(fail_mode, FailMode::DegradeWithMark)
}
```

- [ ] **Step 3: Rewrite `ExecuteCommandTool::run`**

Signature becomes:

```rust
async fn run(
    &self,
    command: &str,
    user_timeout: u64,
    workdir: Option<&Path>,
    mode: EffectiveMode,
) -> Result<ToolOutput, ToolError>
```

- `execute` → `mode: EffectiveMode::default()` (Normal)
- `execute_with_context` → `mode: context.effective_mode`

Logic:

```text
let policy = resolve_for_context(mode, workdir, None);
policy.profile.resources.max_wall_seconds = user_timeout;

if let Some(sb) = &self.sandbox {
  if policy.enabled {
    match sb.execute(command, &policy.profile).await {
      Ok(output) => {
        // killed / non-zero as today
        // Ok path: metadata sandbox_enforced=true, bypassed=false
      }
      Err(e) => {
        if should_degrade_to_direct(policy.fail_mode) {
          // fall through to direct + metadata bypassed=true
        } else {
          return Err(ToolError {
            message: format!("sandbox unavailable ({}): {e}", sb.status().backend_name),
            code: Some("sandbox_spawn_failed".into()),
          });
        }
      }
    }
  } else {
    // enabled=false → direct + marks (DegradeWithMark)
  }
} else {
  // no manager: treat as infra missing → same FailMode branch
}

// direct shell_command_captured path with bypass metadata when used as degrade
```

**Critical:** Never fall through to direct on HardFail.

- [ ] **Step 4: Integration-style unit test without OS**

Prefer testing via a small `pub(crate)` function:

```rust
pub fn decide_after_sandbox_err(fail_mode: FailMode) -> Outcome { HardFail | Degrade }
```

If full tool test is hard, pure tests + one `execute_command` test with `ExecuteCommandTool::new()` (no sandbox) under Normal settings enabled → HardFail error (no manager ≈ unavailable).

```rust
#[tokio::test]
async fn normal_without_sandbox_manager_hard_fails() {
    // tool without sandbox Arc
    let tool = ExecuteCommandTool::new();
    let err = tool.execute(json!({"command": "echo hi"})).await.unwrap_err();
    assert_eq!(err.code.as_deref(), Some("sandbox_spawn_failed"));
}
```

**Note:** Today `new()` has `sandbox: None` and always direct-runs. After change, `sandbox: None` + enabled + HardFail → error. Yolo + none → degrade direct with metadata.

Also test:

```rust
#[tokio::test]
async fn yolo_without_sandbox_manager_degrades() {
    // need execute_with_context with EffectiveMode::Yolo
    ...
    assert!(out.metadata.get("sandbox_bypassed").unwrap().as_bool().unwrap());
}
```

- [ ] **Step 5: Run tests**

```bash
cargo test execute_command -- --nocapture
cargo test sandbox_exec -- --nocapture
```

- [ ] **Step 6: Commit**

```bash
git add src/tools/execution/sandbox_exec.rs src/tools/execution/execute_command.rs \
  src/tools/execution/mod.rs
git commit -m "feat(tools): fail-closed execute_command from mode sandbox policy"
```

---

## Task 5: session_manager + exec_command + run_test

**Files:**
- Modify: `src/tools/execution/session_manager.rs`
- Modify: `src/tools/execution/exec_command.rs`
- Modify: `src/tools/execution/run_test.rs`

### 5a session_manager

- [ ] **Step 1: Change spawn API**

```rust
pub async fn spawn(
    &self,
    command: &str,
    workdir: Option<PathBuf>,
    mode: EffectiveMode,
) -> Result<u64, ToolError>
```

On sandbox spawn Err:
- HardFail → `ToolError { code: sandbox_spawn_failed }`
- DegradeWithMark → `spawn_direct` + store session flag if needed

Optional: store per-session `sandbox_bypassed: bool` for later metadata in `exec_command` read path; minimum: return error vs direct correctly.

Remove hard-coded `SecurityLevel::Minimal` in `default_profile`; build via resolver each spawn (or cache profile only when mode/settings/cwd unchanged — keep simple: resolve every spawn).

### 5b exec_command

- [ ] **Step 2: Implement `execute_with_context`**

```rust
async fn execute_with_context(&self, context: &ToolContext<'_>, input: Value) -> Result<...> {
    // parse input; workdir: input workdir OR context.workdir
    let session_id = self.sessions.spawn(command, workdir, context.effective_mode).await?;
    ...
    // merge sandbox metadata keys if available
}
```

`execute` without context uses `EffectiveMode::Normal`.

### 5c run_test

- [ ] **Step 3: Mode-based security**

```rust
// Replace:
// let security = if allow_network { Minimal } else { Standard };
let mode = /* from context if execute_with_context, else Normal */;
let network = if allow_network { Some(NetworkPolicy::Full) } else { None };
let policy = resolve_for_context(mode, Some(cwd.as_path()), network);
// use policy.profile; on Err apply policy.fail_mode (today Ok(error json) — keep structured test_result but set metadata / error code)
```

Implement `execute_with_context` for RunTestTool.

Update schema description: remove “Uses Minimal sandbox level”; say “enables network within mode level”.

- [ ] **Step 4: Tests**

```bash
cargo test session_manager -- --nocapture
cargo test run_test -- --nocapture
cargo check
```

Add unit test: resolver + allow_network keeps Standard level for Normal with Full network (assert on ResolvedSandboxPolicy in policy tests):

```rust
#[test]
fn run_test_network_keeps_mode_level() {
    let p = SandboxPolicyResolver::resolve_with_network(
        EffectiveMode::Normal,
        &SandboxSettings::default(),
        PathBuf::from("/tmp/ws"),
        Some(NetworkPolicy::Full),
    );
    assert_eq!(p.level, SecurityLevel::Standard);
    assert_eq!(p.profile.network, NetworkPolicy::Full);
    assert_eq!(p.fail_mode, FailMode::HardFail);
}
```

- [ ] **Step 5: Commit**

```bash
git add src/tools/execution/session_manager.rs src/tools/execution/exec_command.rs \
  src/tools/execution/run_test.rs src/sandbox/policy.rs
git commit -m "feat(tools): mode-aware sandbox for exec sessions and run_test"
```

---

## Task 6: Daemon + TUI wire Plan/Yolo into ToolContext

**Problem:** TUI `AgentMode::PlanMode` currently maps to `RootPermissionMode::Normal`, so daemon cannot distinguish Plan for sandbox High.

**Approach (minimal, design-aligned):**

1. Add session-level `effective_mode: Arc<RwLock<EffectiveMode>>` next to `root_mode` in `DaemonState` / session struct.
2. Extend permission-mode API: either
   - **(A)** New field on set-mode request: `effective_mode` / reuse string with `plan`, or
   - **(B)** Separate endpoint — prefer **(A)** extend existing set root mode payload.

**Recommended concrete API:**

```rust
// daemon models — extend SetRootPermissionModeRequest
pub struct SetPermissionModeRequest {
    pub mode: RootPermissionMode,       // keep for auto-approve
    #[serde(default)]
    pub effective_mode: Option<EffectiveMode>, // if None, derive from mode
}
```

TUI on Shift+Tab:

```rust
client.set_permission_mode(
    session_id,
    agent_mode.to_root_permission_mode(),
    Some(agent_mode.to_effective_mode()),
).await?;
```

Handlers building `ToolContext`:

```rust
let effective_mode = *state.effective_mode(session_id).await; // or read lock
let tool_context = ToolContext {
    ...
    effective_mode,
};
```

Subagent `SubagentPermissionContext.root_mode` stays for Ask short-circuit; when building ToolContext inside GuardingToolPort:

```rust
effective_mode: self.perm.root_mode.to_effective_mode(),
// P1 improvement: store EffectiveMode on SubagentPermissionContext too
```

For P0 subagents: inherit mapped mode (Plan lost if only RootPermissionMode) — **Task 6b** pass `EffectiveMode` into `SubagentPermissionContext`.

- [ ] **Step 1: Add `effective_mode` to daemon session state; default Normal**

- [ ] **Step 2: Update set-mode handler + TUI client + key cycle**

- [ ] **Step 3: All root ToolContext constructions read session effective_mode**

- [ ] **Step 4: GuardingToolPort ToolContext uses stored EffectiveMode**

```rust
// SubagentPermissionContext
pub effective_mode: EffectiveMode, // set at spawn from parent session
```

- [ ] **Step 5: Tests**

```rust
#[test]
fn agent_mode_plan_to_effective_plan() {
    assert_eq!(AgentMode::PlanMode.to_effective_mode(), EffectiveMode::Plan);
    assert_eq!(
        AgentMode::PlanMode.to_root_permission_mode(),
        RootPermissionMode::Normal
    );
}
```

Daemon unit test if pattern exists for set mode.

```bash
cargo test to_effective_mode -- --nocapture
cargo test root_permission -- --nocapture
cargo check
```

- [ ] **Step 6: Commit**

```bash
git add src/daemon src/tui src/teams src/tools/meta/task.rs
git commit -m "feat(daemon): sync EffectiveMode including Plan into ToolContext"
```

---

## Task 7: Docs + CHANGELOG (P0 closeout)

**Files:**
- Modify: `docs/SANDBOX.md`
- Modify: `CHANGELOG.md`
- Modify: `WGENTY.md` (settings table row for `integrations.sandbox`)

- [ ] **Step 1: Document matrix, FailMode, settings JSON, BREAKING Normal network**

CHANGELOG entry example:

```markdown
### Breaking
- Shell tools under Normal/Plan/AcceptEdits now default to Standard/High sandbox
  profiles (**no network**) and **hard-fail** if sandbox spawn fails (no silent bare exec).
- Use Yolo or `integrations.sandbox.defaults_by_mode` to loosen.

### Added
- Mode ↔ sandbox profile matrix via `SandboxPolicyResolver`
- `integrations.sandbox` settings
```

- [ ] **Step 2: Commit**

```bash
git add docs/SANDBOX.md CHANGELOG.md WGENTY.md
git commit -m "docs: sandbox permission mode linkage and breaking defaults"
```

---

## Task 8 (P1): TUI bypass visibility

**Files:**
- TUI status bar / toast / system line when tool metadata has `sandbox_bypassed=true` or settings disabled
- Poll path that already surfaces tool results

- [ ] Detect `sandbox_bypassed` in tool result handling
- [ ] Show persistent badge e.g. `SANDBOX OFF` / `BYPASS` when session has seen bypass or settings.enabled=false
- [ ] Manual verify in TUI (Yolo + forced fail) if possible

```bash
cargo check -p wgenty-code  # or workspace default
```

Commit: `feat(tui): surface sandbox bypass / disabled state`

---

## Task 9 (P1): CLI sandbox enable/disable/status

**Files:**
- `src/cli/args.rs` `run_sandbox`

- [ ] `status`: print backend + `Settings.integrations.sandbox` + default matrix summary
- [ ] `disable`: set `enabled=false`, `Settings::save()`
- [ ] `enable`: set `enabled=true`, save

```bash
cargo test # if any cli tests
cargo check
```

Commit: `feat(cli): persist sandbox enable/disable in settings`

---

## Task 10 (P2, optional follow-up): Platform fidelity

- macOS: optional `#[cfg(target_os = "macos")]` test Standard denies external curl (ignore if sandbox-exec missing)
- Linux: only spawn `unshare --net` when `NetworkPolicy::None` (do not always unshare net if Full) — **read current backend before changing**
- Refresh `docs/SANDBOX.md` Windows partial enforcement note (already in design)

Do **not** block P0/P1 on P2.

---

## Verification gate (before claiming done)

```bash
cargo test --lib sandbox::policy -- --nocapture
cargo test sandbox_settings_ -- --nocapture
cargo test execute_command -- --nocapture
cargo check
```

Manual checklist:

| Mode | Expect shell |
|------|----------------|
| Normal | Standard profile, HardFail on infra error |
| Plan | High, HardFail |
| AcceptEdits | Standard shell HardFail; FS tools unsandboxed |
| Yolo | Minimal, degrade+metadata on fail |
| settings enabled=false | Degrade + marks all modes |

---

## Execution handoff

Plan complete and saved to `docs/superpowers/plans/2026-07-17-sandbox-permission-linkage.md`.

**Two execution options:**

1. **Subagent-driven (recommended)** — dispatch a fresh subagent per task with review between tasks (`superpowers:subagent-driven-development`).
2. **Inline** — execute tasks in this session with checkpoints (`superpowers:executing-plans`).

Which approach do you want?
