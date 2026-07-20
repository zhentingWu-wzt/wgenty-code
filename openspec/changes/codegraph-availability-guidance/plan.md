# Implementation Plan: CodeGraph 安装/初始化差异化引导

## Overview

Implement differentiated CodeGraph availability detection (NotInstalled / NotInitialized / Dismissed / Ready) with dual-channel guidance (CLI startup notice + prompt injection) and per-project dismissal. Source spec: `openspec/changes/codegraph-availability-guidance/design.md`.

**Architecture context (verified):**
- The REPL starts an in-process daemon (`daemon/state.rs` `AppState::new`), then runs the TUI App as frontend. MCP connect is **non-blocking background** (`state.rs:233` `tokio::spawn`).
- Therefore the actionable states (NotInstalled/NotInitialized/Dismissed) are detected via a **synchronous probe** (`which` + `.codegraph/` marker + `dismissed_paths`), available immediately at startup--no need to wait for the async MCP handshake.
- `ToolRegistry::new()` (`tools/mod.rs:116`) is the single built-in tool registration site; the daemon inherits via `ToolRegistry::new().with_settings()` (`state.rs:129`).
- `PromptContext` (`prompts/mod.rs:65`) is the per-turn dynamic context; built at `tui/app/mod.rs:209` (REPL), `tui/app/event.rs:294` (reload), `cli/headless_runtime.rs:184` (query).

**Pragmatic deviation from design doc (flagged):** The design's `CodegraphAvailability::Connected`/`ConnectionError` require threading the async MCP handshake result. Since guidance only needs the sync-determinable states, v1 uses a 4-state `CodegraphInstallState` (`Ready`/`NotInstalled`/`NotInitialized`/`Dismissed`). `ConnectionError` (binary+index present but handshake fails) is logged by the existing background connect task; a distinct TUI icon for it is deferred. `Ready` is displayed as "connected".

## Architecture Decision Constraints (ADC)

- **ADC-1**: Sync probe only. No threading of async availability through `connect_configured_tools` return type. The probe is a pure function of `&Settings`.
- **ADC-2**: `CodegraphInstallState` lives in `src/mcp/codegraph.rs`. The `McpServerStatus` enum stays untouched (no pollution).
- **ADC-3**: Dismissal via a dedicated meta tool `dismiss_codegraph_guidance` (not `config set` array semantics).
- **ADC-4**: Testability--split probe into `probe_install_state()` (calls `which`) + `classify_install_state(settings, binary_present)` (pure, unit-testable without mocking `which`).

---

## Phase 1: Foundation

### Task 1 — Config schema for per-project dismissal

**Test spec** (`src/config/services.rs` `#[cfg(test)]`):
- `codegraph_settings_default_empty`: `CodegraphSettings::default().dismissed_paths` is empty `Vec`.
- `integrations_default_has_codegraph`: `IntegrationsConfig::default().codegraph.dismissed_paths` is empty.
- `serde_roundtrip_preserves_paths`: serialize `IntegrationsConfig` with `dismissed_paths = ["/tmp/a"]`, deserialize back, assert path equals.
- `serde_old_config_no_codegraph_field`: deserialize JSON `{"mcp_servers":[],"guardian":{...}}` (no `codegraph` key) succeeds via `#[serde(default)]`, yields empty `dismissed_paths`.

**Implement** (`src/config/services.rs`):
- Add struct after `IntegrationsConfig` (line ~89):
```rust
/// Per-project CodeGraph guidance state.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct CodegraphSettings {
    /// Working dirs (canonical absolute paths, deduped) where the user has
    /// dismissed install/init guidance. Suppresses both the CLI notice and the
    /// agent's on-demanpt ask.
    #[serde(default)]
    pub dismissed_paths: Vec<std::path::PathBuf>,
}
```
- Add field to `IntegrationsConfig` (after `guardian`, line ~88):
```rust
    #[serde(default)]
    pub codegraph: CodegraphSettings,
```

**Commit**: `feat(config): add CodegraphSettings with per-project dismissed_paths`

---

### Task 2 — CodegraphInstallState probe module

**Test spec** (`src/mcp/codegraph.rs` `#[cfg(test)]`), using `tempfile::TempDir` as `working_dir`:
- `classify_dismissed_wins`: `dismissed_paths` contains the working_dir canonical path, `binary_present=true` -> `Dismissed` (even though installed+indexed).
- `classify_not_installed`: empty `dismissed_paths`, `binary_present=false`, no `.codegraph/` -> `NotInstalled`.
- `classify_not_initialized`: `binary_present=true`, no `.codegraph/` -> `NotInitialized`.
- `classify_ready`: `binary_present=true`, `.codegraph/` dir created -> `Ready`.
- `classify_dismissed_by_raw_path`: `dismissed_paths` holds a non-canonical path (e.g. `tmp` with symlink/`..`) that canonicalizes to working_dir -> still `Dismissed`.
- `notice_none_for_ready_and_dismissed`: `install_state_notice(Ready)` and `(Dismissed)` both `None`.
- `notice_text_not_installed`: `install_state_notice(NotInstalled)` contains `npm i -g @colbymchenry/codegraph`.
- `notice_text_not_initialized`: `install_state_notice(NotInitialized)` contains `codegraph init`.

**Implement** (`src/mcp/codegraph.rs`, new file):
```rust
//! CodeGraph availability probe + guidance text.
//!
//! Detects whether the third-party `codegraph` CLI is installed and whether the
//! current repo has been indexed (`.codegraph/` marker), so the agent can guide
//! the user to install vs. initialize. The probe is synchronous and cheap
//! (PATH scan + one `exists()` stat), safe to run at startup.

use crate::config::Settings;
use std::path::{Path, PathBuf};

/// Sync-determinable CodeGraph availability. Drives all guidance channels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodegraphInstallState {
    /// Binary installed and `.codegraph/` index present.
    Ready,
    /// `codegraph` not found on PATH.
    NotInstalled,
    /// Binary present, but no `.codegraph/` index dir in the working dir.
    NotInitialized,
    /// User dismissed guidance for this working dir.
    Dismissed,
}

impl CodegraphInstallState {
    /// Short human-readable hint injected into the prompt environment layer.
    pub fn guidance_hint(&self) -> &'static str {
        match self {
            Self::Ready => "ready (code navigation active)",
            Self::NotInstalled => {
                "not_installed (install: npm i -g @colbymchenry/codegraph; fallback grep/lsp)"
            }
            Self::NotInitialized => {
                "not_initialized (run `codegraph init`; fallback grep/lsp)"
            }
            Self::Dismissed => "dismissed (fallback grep/lsp)",
        }
    }
}

/// Canonicalize a path for dismissed-set comparison; falls back to the raw
/// path when canonicalization fails (e.g. path no longer exists).
fn canon(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// True if `working_dir` (canonicalized) is in the dismissed set.
fn is_dismissed(settings: &Settings, working_dir: &Path) -> bool {
    let target = canon(working_dir);
    settings
        .integrations
        .codegraph
        .dismissed_paths
        .iter()
        .any(|p| canon(p) == target)
}

/// Pure classifier (no `which` call) -- unit-testable with a synthetic
/// `binary_present` flag.
fn classify_install_state(settings: &Settings, binary_present: bool) -> CodegraphInstallState {
    let working_dir = &settings.storage.working_dir;
    if is_dismissed(settings, working_dir) {
        return CodegraphInstallState::Dismissed;
    }
    if !binary_present {
        return CodegraphInstallState::NotInstalled;
    }
    if !working_dir.join(".codegraph").exists() {
        return CodegraphInstallState::NotInitialized;
    }
    CodegraphInstallState::Ready
}

/// Full probe: checks PATH for the `codegraph` binary, then classifies.
pub fn probe_install_state(settings: &Settings) -> CodegraphInstallState {
    let binary_present = which::which("codegraph").is_ok();
    classify_install_state(settings, binary_present)
}

/// One-line CLI notice text for actionable states; `None` when silent
/// (Ready / Dismissed).
pub fn install_state_notice(state: CodegraphInstallState) -> Option<String> {
    match state {
        CodegraphInstallState::NotInstalled => Some(
            "⚠ CodeGraph 未安装，代码导航已降级到 grep/lsp。安装: npm i -g @colbymchenry/codegraph"
                .to_string(),
        ),
        CodegraphInstallState::NotInitialized => Some(
            "⚠ CodeGraph 已安装但当前仓库未初始化。在项目根运行: codegraph init".to_string(),
        ),
        CodegraphInstallState::Ready | CodegraphInstallState::Dismissed => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CodegraphSettings, IntegrationsConfig, Settings, StorageConfig};

    fn settings_in(dir: &Path) -> Settings {
        let mut s = Settings::default();
        s.storage = StorageConfig {
            working_dir: dir.to_path_buf(),
            ..Settings::default().storage
        };
        s
    }

    #[test]
    fn classify_dismissed_wins() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = settings_in(tmp.path());
        s.integrations.codegraph = CodegraphSettings {
            dismissed_paths: vec![canon(tmp.path())],
        };
        assert_eq!(
            classify_install_state(&s, true),
            CodegraphInstallState::Dismissed
        );
    }

    #[test]
    fn classify_not_installed() {
        let tmp = tempfile::tempdir().unwrap();
        let s = settings_in(tmp.path());
        assert_eq!(
            classify_install_state(&s, false),
            CodegraphInstallState::NotInstalled
        );
    }

    #[test]
    fn classify_not_initialized() {
        let tmp = tempfile::tempdir().unwrap();
        let s = settings_in(tmp.path());
        assert_eq!(
            classify_install_state(&s, true),
            CodegraphInstallState::NotInitialized
        );
    }

    #[test]
    fn classify_ready() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join(".codegraph")).unwrap();
        let s = settings_in(tmp.path());
        assert_eq!(classify_install_state(&s, true), CodegraphInstallState::Ready);
    }

    #[test]
    fn notice_text_not_installed() {
        assert!(install_state_notice(CodegraphInstallState::NotInstalled)
            .unwrap()
            .contains("npm i -g @colbymchenry/codegraph"));
    }

    #[test]
    fn notice_none_for_ready_and_dismissed() {
        assert!(install_state_notice(CodegraphInstallState::Ready).is_none());
        assert!(install_state_notice(CodegraphInstallState::Dismissed).is_none());
    }
}
```
- Declare module in `src/mcp/mod.rs` (after `pub mod client;` line 9): `pub mod codegraph;`
- Re-export: add to the `pub use` block near line 29: `pub use codegraph::{install_state_notice, probe_install_state, CodegraphInstallState};`
- Export `CodegraphSettings` from config: add `pub use services::CodegraphSettings;` in `src/config/mod.rs` (near line 19 `pub use services::*;` already covers it--verify `services::*` re-exports it; if not, add explicit re-export). The test imports `crate::config::CodegraphSettings`, so ensure it's reachable.

**Verify `which` crate**: confirm `which` is a dependency in `Cargo.toml` (WGENTY.md lists `which 6.0`). If the crate is named `which` in deps, `which::which("codegraph")` works.

**Commit**: `feat(mcp): add CodegraphInstallState probe and guidance text`

---

## Phase 2: Wiring

### Task 3 — Short-circuit NotInstalled/Dismissed in connect_configured_tools

**Test spec** (`src/mcp/mod.rs` external_registration_tests or a new `#[cfg(test)]` mod):
- `short_circuit_skips_codegraph_when_not_installed`: build a `Settings` whose `working_dir` is a temp dir with no `.codegraph/` and no `codegraph` on PATH (use `classify_install_state(&settings, false)` to confirm `NotInstalled`); assert that `connect_configured_tools` does not attempt to spawn codegraph (assert the returned proxies contain no tool whose `name()` starts with `codegraph`). Use a mock/Noop McpManager if `connect_configured_tools` is on `McpManager`; otherwise test the filter logic via a small extracted helper.
- Note: if `connect_configured_tools` is hard to unit-test in isolation (it spawns real subprocesses), extract the config-filter decision into a pure helper `should_skip_codegraph(settings) -> bool` and unit-test that instead, then wire it in.

**Implement** (`src/mcp/mod.rs`, `connect_configured_tools` at line 258):
- After the existing codegraph auto-inject + cwd-fix block (after line 276), before the `auto_start_configs` filter (line 284), insert:
```rust
        // Probe CodeGraph availability. Skip the spawn entirely when the binary
        // is absent or the user dismissed guidance -- `codegraph serve` would
        // fail with "command not found" (fast but noisy) or run unwanted.
        let skip_codegraph = {
            let state = crate::mcp::codegraph::probe_install_state(settings);
            matches!(
                state,
                crate::mcp::codegraph::CodegraphInstallState::NotInstalled
                    | crate::mcp::codegraph::CodegraphInstallState::Dismissed
            )
        };
```
- Update the `auto_start_configs` filter (line 284-287) to also drop codegraph when `skip_codegraph`:
```rust
        let auto_start_configs: Vec<&McpConfig> = configs
            .iter()
            .filter(|config| config.auto_start && config.name != "filesystem")
            .filter(|config| {
                !(skip_codegraph && config.name.eq_ignore_ascii_case("codegraph"))
            })
            .collect();
```
- Do the same skip in `list_servers_for_settings` (line 133) is NOT required (listing should still show the config); leave it.

**Commit**: `feat(mcp): short-circuit codegraph spawn when not installed or dismissed`

---

### Task 4 — PromptContext carries CodegraphInstallState + environment injection

**Test spec** (`src/prompts/mod.rs` tests):
- `environment_layer_includes_codegraph_state`: `PromptContext::new().with_cwd("/tmp").with_shell("zsh").with_codegraph_state(CodegraphInstallState::NotInitialized)` -> `assemble_instructions` -> the environment system message content contains `<codegraph>not_initialized` and `codegraph init`.
- `environment_layer_omits_codegraph_when_none`: default `PromptContext` (no codegraph_state) -> environment message has no `<codegraph>` tag.
- `environment_layer_ready_state`: with `Ready` -> contains `<codegraph>ready`.

**Implement** (`src/prompts/mod.rs`):
- Add field to `PromptContext` (after `memories`, line ~89):
```rust
    /// CodeGraph availability (sync probe result) for guidance injection.
    pub codegraph_state: Option<crate::mcp::codegraph::CodegraphInstallState>,
```
- Init in `PromptContext::new()` (line ~147): `codegraph_state: None,`
- Add Debug field (line ~120): `.field("codegraph_state", &self.codegraph_state)`
- Add builder (near other `with_*`, after line 164):
```rust
    pub fn with_codegraph_state(
        mut self,
        state: crate::mcp::codegraph::CodegraphInstallState,
    ) -> Self {
        self.codegraph_state = Some(state);
        self
    }
```
- Inject in `build_environment_layer` (line 474). Replace the `format!` to append a `<codegraph>` line when present:
```rust
fn build_environment_layer(ctx: &PromptContext) -> String {
    let now = Local::now();
    let date = now.format("%Y-%m-%d").to_string();
    let timezone = now.format("%Z").to_string();
    let codegraph_line = ctx
        .codegraph_state
        .map(|s| format!("\n  <codegraph>{}</codegraph>", s.guidance_hint()))
        .unwrap_or_default();
    format!(
        "<environment_context>\n  <cwd>{cwd}</cwd>\n  <shell>{shell}</shell>\n  <current_date>{date}</current_date>\n  <timezone>{timezone}</timezone>{codegraph_line}\n</environment_context>",
        cwd = ctx.cwd,
        shell = ctx.shell,
    )
}
```
- Add `use crate::mcp::codegraph::CodegraphInstallState;` at top if needed for ergonomics (optional; fully-qualified paths also work).

**Commit**: `feat(prompts): inject Codegraph availability into environment layer`

---

### Task 5 — Set codegraph_state at PromptContext construction sites

**Test spec**: covered by integration--manual verify that a REPL turn's assembled system prompt contains the `<codegraph>` tag when codegraph is absent. Add a focused unit test only if a constructor is directly testable.

**Implement** (3 sites):
1. `src/tui/app/mod.rs` `App::new` (~line 209): where `prompt_context` is built with `.with_cwd(...)` etc., append `.with_codegraph_state(crate::mcp::codegraph::probe_install_state(&settings))`. Use the settings available at construction.
2. `src/tui/app/event.rs` (~line 294, settings-reload `prompt_ctx` rebuild): append `.with_codegraph_state(crate::mcp::codegraph::probe_install_state(&new_settings))`.
3. `src/cli/headless_runtime.rs` (~line 184, query mode): append `.with_codegraph_state(crate::mcp::codegraph::probe_install_state(&settings))`.
4. `src/tui/agent/mod.rs` (~line 368, subagent): subagents use default empty context; leave as `None` (subagents inherit fallback behavior). Document this choice in a `//` comment.

**Commit**: `feat(prompts): wire codegraph probe into REPL/query prompt contexts`

---

### Task 6 — Update base.md codegraph guidance text

**Test spec**: none (doc/prompt text).

**Implement** (`src/prompts/base.md`, replace the paragraph at line 147):
```markdown
If CodeGraph tools are absent or report an uninitialized project, fall back to `grep` / `lsp` for the current task. The live CodeGraph status is injected in `<environment_context>` as `<codegraph>...</codegraph>` (states: ready / not_installed / not_initialized / dismissed).

When the status is `not_installed` or `not_initialized` and NOT `dismissed`, and you are about to perform code navigation (calling `codegraph_node` / `codegraph_explore`), first use `ask_user_question` to offer: (1) install/initialize now -- provide the command and, on approval, run it via `exec_command` then suggest `/mcp restart`; (2) don't remind again -- call `dismiss_codegraph_guidance` to persist; (3) skip this time -- use grep/lsp, no persistence. If `dismissed` or `ready`, do not ask. Project indexing is the user's decision; the user can install `@colbymchenry/codegraph` and run `codegraph init` in the project root.
```

**Commit**: `docs(prompts): instruct agent on differentiated codegraph guidance`

---

## Phase 3: Dismissal tool

### Task 7 — dismiss_codegraph_guidance meta tool

**Test spec** (`src/tools/meta/dismiss_codegraph_guidance.rs` `#[cfg(test)]`), using a temp `~/.wgenty-code/settings.json` (override via `Settings::config_path` is private; instead test the pure helper `add_dismissed_path`):
- Extract a pure helper `pub fn add_dismissed_path(paths: &mut Vec<PathBuf>, working_dir: &Path)` that canonicalizes + dedup-pushes. Test:
  - `add_dismissed_path_dedups`: push same canonical path twice -> length 1.
  - `add_dismissed_path_adds_new`: push two distinct dirs -> length 2.
- Tool `execute` integration: spawn the tool, call `execute(json!({}))` with cwd set to a temp dir, then `Settings::load()` and assert the temp dir's canonical path is in `dismissed_paths`. (Requires the settings file to be writable--use a test that mocks `Settings` by checking the helper + a round-trip through `Settings::save`/`load` with a known home override if available; otherwise rely on the helper unit test + a smoke test that `execute` returns `success`.)

**Implement** (`src/tools/meta/dismiss_codegraph_guidance.rs`, new file):
```rust
//! Dismiss CodeGraph install/init guidance for the current project.
//!
//! Persists the current working dir into
//! `settings.integrations.codegraph.dismissed_paths` (canonicalized, deduped)
//! so the CLI startup notice and the agent's on-demand ask go silent for this
//! project.

use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;
use std::path::{Path, PathBuf};

pub struct DismissCodegraphGuidanceTool;

impl Default for DismissCodegraphGuidanceTool {
    fn default() -> Self {
        Self::new()
    }
}

impl DismissCodegraphGuidanceTool {
    pub fn new() -> Self {
        Self
    }
}

/// Canonicalize `working_dir` and push into `paths` if not already present.
/// Returns the canonical path that was ensured present.
pub fn add_dismissed_path(paths: &mut Vec<PathBuf>, working_dir: &Path) -> PathBuf {
    let canon = std::fs::canonicalize(working_dir).unwrap_or_else(|_| working_dir.to_path_buf());
    if !paths.iter().any(|p| {
        std::fs::canonicalize(p).unwrap_or_else(|_| p.clone()) == canon
    }) {
        paths.push(canon.clone());
    }
    canon
}

#[async_trait]
impl Tool for DismissCodegraphGuidanceTool {
    fn name(&self) -> &str {
        "dismiss_codegraph_guidance"
    }

    fn description(&self) -> &str {
        "Silence CodeGraph install/initialization guidance for the current project. \
         Persists the working directory to settings so the startup notice and \
         on-demand prompts no longer appear. Use when the user chooses not to \
         install CodeGraph."
    }

    fn is_read_only(&self) -> bool {
        // Writes to settings.json.
        false
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Optional working directory to dismiss (defaults to current working dir)."
                }
            },
            "required": []
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let path = input["path"]
            .as_str()
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::current_dir().ok())
            .ok_or_else(|| ToolError {
                message: "could not resolve working directory".to_string(),
                code: Some("no_cwd".to_string()),
            })?;

        let mut settings = crate::config::Settings::load().map_err(|e| ToolError {
            message: format!("failed to load settings: {e}"),
            code: Some("settings_load".to_string()),
        })?;
        let canon =
            add_dismissed_path(&mut settings.integrations.codegraph.dismissed_paths, &path);
        settings.save().map_err(|e| ToolError {
            message: format!("failed to save settings: {e}"),
            code: Some("settings_save".to_string()),
        })?;
        Ok(ToolOutput::text(format!(
            "CodeGraph guidance dismissed for {}",
            canon.display()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn add_dismissed_path_dedups() {
        let tmp = tempfile::tempdir().unwrap();
        let mut paths: Vec<PathBuf> = Vec::new();
        add_dismissed_path(&mut paths, tmp.path());
        add_dismissed_path(&mut paths, tmp.path());
        assert_eq!(paths.len(), 1);
    }

    #[test]
    fn add_dismissed_path_adds_new() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let mut paths: Vec<PathBuf> = Vec::new();
        add_dismissed_path(&mut paths, a.path());
        add_dismissed_path(&mut paths, b.path());
        assert_eq!(paths.len(), 2);
    }
}
```
- Declare in `src/tools/meta/mod.rs` (after `pub mod compact;` line 2): `pub mod dismiss_codegraph_guidance;` and `pub use dismiss_codegraph_guidance::DismissCodegraphGuidanceTool;`
- Register in `src/tools/mod.rs` after the `note_edit` registration (line 175):
```rust
        registry.register(Box::new(meta::dismiss_codegraph_guidance::DismissCodegraphGuidanceTool::new()));
```

**Commit**: `feat(tools): add dismiss_codegraph_guidance meta tool`

---

## Phase 4: CLI startup notice

### Task 8 — Print availability notice at REPL + query startup

**Test spec** (`src/cli/args.rs` or a small extracted helper):
- Extract `pub fn maybe_print_codegraph_notice(settings: &Settings)` that calls `probe_install_state` + `install_state_notice` and `eprintln!`s when `Some`. Unit-test by capturing: hard to capture `eprintln!`; instead test the pure composition `install_state_notice(probe_install_state(settings))` returns `None` for a `Ready`-like settings (binary present + `.codegraph/` exists) -- covered by Task 2. The print wrapper is a thin glue layer.

**Implement**:
1. `src/cli/args.rs` `run_repl` (~line 153, **before** `execute!(stdout, EnterAlternateScreen)?;`): print the notice so it's visible before the TUI takes over the screen:
```rust
        // CodeGraph availability notice (before entering alt screen so it's
        // visible; silent when installed+initialized or dismissed).
        if let Some(msg) = crate::mcp::codegraph::install_state_notice(
            crate::mcp::codegraph::probe_install_state(&state.settings),
        ) {
            eprintln!("{msg}");
        }
```
2. `src/cli/headless_runtime.rs` (query mode, near the start of the run function, after settings load): same `eprintln!` block. For daemon mode (`Daemon` command), use `tracing::warn!` instead of `eprintln!` (no terminal) -- add in the daemon startup path if `daemon` command prints; if daemon has no direct entry here, skip (the REPL in-process daemon already covers notice via run_repl).

**Commit**: `feat(cli): print codegraph availability notice at startup`

---

## Phase 5: TUI status bar

### Task 9 — Upgrade TUI CG indicator to CodegraphInstallState

**Test spec** (`src/tui/app/render.rs` tests):
- `codegraph_status_span_ready`: `codegraph_status_span(&CodegraphInstallState::Ready)` -> span text contains `●`.
- `codegraph_status_span_not_installed`: `NotInstalled` -> contains `⚠`.
- `codegraph_status_span_not_initialized`: `NotInitialized` -> contains `⚠`.
- `codegraph_status_span_dismissed`: `Dismissed` -> contains `○`.

**Implement** (4 files):
1. `src/tui/app/mod.rs` field (line 190): change type
   ```rust
   pub codegraph_status: crate::mcp::codegraph::CodegraphInstallState,
   ```
2. `src/tui/app/mod.rs` `detect_codegraph_status` (line 974): replace body to actually probe:
   ```rust
   pub fn detect_codegraph_status(
       settings: &crate::config::Settings,
   ) -> crate::mcp::codegraph::CodegraphInstallState {
       crate::mcp::codegraph::probe_install_state(settings)
   }
   ```
3. `src/tui/app/mod.rs` `App::new` (~line 209+): initialize `codegraph_status: Self::detect_codegraph_status(&settings)` (find the existing init site; currently likely `McpServerStatus::Unknown`). Update any other constructor (e.g. `Default` impl) accordingly.
4. `src/tui/app/render.rs` `codegraph_status_span` (line 312): change signature + mapping:
   ```rust
   fn codegraph_status_span(status: &CodegraphInstallState) -> Span<'static> {
       use crate::mcp::codegraph::CodegraphInstallState;
       let (icon, color, label) = match status {
           CodegraphInstallState::Ready => ("●", theme::SUCCESS, "CG"),
           CodegraphInstallState::NotInstalled => ("⚠", theme::WARNING, "CG"),
           CodegraphInstallState::NotInitialized => ("⚠", theme::WARNING, "CG"),
           CodegraphInstallState::Dismissed => ("○", theme::DIM, "CG"),
       };
       Span::styled(
           format!("{} {}", icon, label),
           ratatui::style::Style::default().fg(color),
       )
   }
   ```
   - Update the call site `codegraph_status_span(&self.codegraph_status)` (~render.rs:211) -- unchanged signature-wise (still `&self.codegraph_status`), just the type flows.
   - Add the `use crate::mcp::codegraph::CodegraphInstallState;` import in render.rs (or use fully-qualified in the match).
5. `src/tui/app/event.rs` (line 364): unchanged call `self.codegraph_status = super::detect_codegraph_status(&new_settings);` -- types now align (both `CodegraphInstallState`).

**Commit**: `feat(tui): show codegraph install/init state in status bar`

---

## Phase 6: Verification

### Task 10 — Lint, format, tests, changelog

**Implement**:
1. `cargo fmt`
2. `cargo clippy --all-targets -- -D warnings` -- fix any warnings (likely: unused import if `CodegraphInstallState` imported but aliased; ensure `which` is actually used).
3. `cargo test --all` -- all green.
4. Manual smoke: `cargo run -- query --prompt "hi"` in a dir without codegraph -> stderr shows the `⚠ CodeGraph 未安装` notice. In a dir with `codegraph` installed but no `.codegraph/` -> `⚠ 未初始化` notice. After calling the dismiss tool (or manually adding the path to settings), restart -> silent.
5. Update `CHANGELOG.md` under Unreleased:
   ```
   ### Added
   - CodeGraph availability probe differentiates not-installed vs not-initialized;
     startup notice + prompt injection guide users to install (`npm i -g
     @colbymchenry/codegraph`) or initialize (`codegraph init`). Per-project
     dismissal via the `dismiss_codegraph_guidance` tool.
   ```

**Commit**: `test: verify codegraph guidance end-to-end + changelog`

---

## Risks & Notes

- **`which` crate path**: confirm `which::which` is the correct API (dep `which 6.0`). If the crate exposes a different entry, adjust one line in `probe_install_state`.
- **i18n**: notice strings are hardcoded, matching existing MCP user-facing messages (`mod.rs:235` `println!("🛑 MCP server stopped")`). Fluent migration is a follow-up, not in scope.
- **App::new init site**: the exact line where `codegraph_status` is first assigned in `App::new` must be found during Task 9 (currently `McpServerStatus::Unknown` or via `detect_codegraph_status`). Grep `codegraph_status:` in `tui/app/mod.rs`.
- **Performance**: probe is one PATH scan + one `exists()` stat, run once at REPL startup + on settings reload. Well within the ≤5% startup budget.
- **Daemon mode**: the standalone `Daemon` CLI command has no terminal; if it needs the notice it should use `tracing::warn!`. Verify whether the `Daemon` command path is exercised separately from `run_repl`; if not, Task 8.1 suffices.
