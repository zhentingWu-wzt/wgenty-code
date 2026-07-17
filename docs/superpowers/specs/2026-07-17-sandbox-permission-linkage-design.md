# Sandbox ↔ Permission Mode Linkage Design

**Date:** 2026-07-17  
**Status:** Implemented  
**Approach:** Profile Matrix (A)  
**Related:** `src/sandbox/`, `src/permissions/`, `src/config/agent.rs` (`RootPermissionMode`), `docs/SANDBOX.md`

## Problem

Today permission mode and OS sandbox are independent:

| Layer | Role | User control | Default |
|-------|------|--------------|---------|
| Permission mode | Whether a tool may run (approve / auto-allow) | TUI: Normal / AcceptEdits / Yolo / PlanMode | Normal |
| Sandbox `SecurityLevel` | How isolated shell runs | **None** | Hard-coded **Minimal** for exec tools |

Switching to Yolo does not change isolation. Normal does not tighten FS/network. CLI `sandbox enable|disable` is a no-op. Sandbox infrastructure failures silently fall back to direct spawn (warn log only).

Goal: **safer defaults**, **settings overrides**, **mode-differentiated fail policy**, and **provable behavior** via tests—without blocking day-to-day cargo/npm workflows under intentional loose modes.

## Goals

1. Map each effective permission mode to a default sandbox profile (level + network + fail mode).
2. Allow `settings.json` per-mode overrides.
3. Plan/Normal: **fail closed** on sandbox infrastructure failure (no silent bare exec).
4. Yolo: allow degrade-with-mark; **TUI must surface bypass** (not only tool metadata).
5. Pass mode **only via `ToolContext`** (no process-global mode lock).
6. Tests prove matrix, overrides, hard-fail, and degrade-with-mark.

## Non-goals (this change)

- Real Linux seccomp-bpf / syscall allowlists.
- Windows Restricted Tokens / FS isolation beyond Job Objects.
- OS-level sandboxing of pure filesystem tools (`file_write` / `file_edit` / `apply_patch`).
- Independent sandbox-level UI knob decoupled from permission mode (rejected dual-knob design).

## Concepts

### EffectiveMode

Runtime mode used by the resolver. Extends root permission modes with TUI Plan:

```text
EffectiveMode = Plan | Normal | AcceptEdits | Yolo
```

- `RootPermissionMode` remains `{ Normal, AcceptEdits, Yolo }` for policy auto-approve.
- TUI PlanMode is folded into `EffectiveMode::Plan` when building `ToolContext`.
- Subagents inherit root effective mode through existing permission context → into their `ToolContext` (no separate sandbox mode).

### SecurityLevel (existing)

`Minimal | Standard | High | Paranoid` — profile presets in `SandboxConfig` (not user config files).

### FailMode

```text
HardFail          — sandbox spawn/infra error → ToolError; never direct spawn
DegradeWithMark   — direct spawn allowed; metadata + TUI show sandbox_bypassed
```

Silent degrade is **forbidden** for product paths after this change.

**`enabled: false` overrides mode fail_mode:** resolver forces `DegradeWithMark` + `source=Disabled` for every mode (including Plan). Work continues; TUI/metadata must show sandbox disabled. This is intentional escape hatch, not silent bare exec.

### Naming: settings vs profile config

| Type | Role |
|------|------|
| `SandboxSettings` | User-facing `settings.json` (`integrations.sandbox`) |
| `SandboxConfig` / `SecurityLevel` | Existing profile builders (code presets, not user files) |

Do not merge these types.

### ResolvedSandboxPolicy

Output of `SandboxPolicyResolver::resolve(...)`:

- `level: SecurityLevel`
- `network: NetworkPolicy` (from level defaults unless overridden later)
- `fail_mode: FailMode`
- `profile: SandboxProfile` (built for workspace / workdir)
- `enabled: bool` (from settings)
- `source: Default | SettingsOverride | Disabled`

## Default matrix (shell / exec tools)

Applies to `execute_command`, `exec_command` (session manager), and analogous shell spawns.  
`run_test` keeps network toggle but **base level must not be looser than mode default** (see below).

| EffectiveMode | Default SecurityLevel | Network (from level) | FS read/write | Shell FailMode |
|---------------|----------------------|----------------------|---------------|----------------|
| **Plan** | High | None | Full-disk **read** + workspace write | HardFail |
| **Normal** | Standard | **Full** (package managers) | Full-disk **read** + workspace write (Codex workspace-write) | HardFail |
| **AcceptEdits** | Standard (for **shell**) | **Full** | same as Normal | HardFail |
| **Yolo** | Minimal (metadata) | Full | **OS sandbox off** (not Minimal seatbelt) | DegradeWithMark |

> **Implementation note (post-review):** Standard network was raised from `None` →
> `Full` so Normal/AcceptEdits can run cargo/npm/git remotes without forcing Yolo.
> Isolation vs Yolo is **write roots + env allowlist + HardFail + OS sandbox on**,
> not “workspace-only read”. Reads match Codex: unrestricted `(allow file-read*)`.

### AcceptEdits nuance

- Filesystem mutating tools: still permission auto-approve only; they do not use OS command sandbox.
- Shell tools under AcceptEdits stay **Standard + HardFail** so “accept edits” ≠ “any command bare-metal”.

### run_test (locked)

- Resolve **level + FailMode** from mode matrix / settings (same as shell).
- If `allow_network=true`: keep mode level and FailMode, but set `NetworkPolicy::Full` on the profile (test convenience exception). Document in CHANGELOG.
- Base level must **not** be looser than the mode default (e.g. Normal must not drop to Minimal solely because tests run).

### Paranoid

Not a mode default. Available only via `defaults_by_mode` override (or future explicit API).

### Missing ToolContext mode

Safe default: **`EffectiveMode::Normal`** → Standard + HardFail (not legacy Minimal).

### Platform partial enforcement

`SecurityLevel` is **profile intent**. Backends may only partially implement it (e.g. Windows Job Objects for all levels; Linux without real seccomp).

| Situation | Behavior |
|-----------|----------|
| Backend spawn succeeds | `sandbox_enforced=true`; isolation = what backend actually applied |
| Backend spawn / infra fails | Apply mode `FailMode` (HardFail vs DegradeWithMark) |
| Backend unavailable and `enabled=true` | Same as spawn failure (HardFail or marked degrade) |
| Profile intent > backend capability | **Not** treated as HardFail solely for “weaker than macOS Seatbelt”; document platform limits in `docs/SANDBOX.md` |

HardFail means **no silent direct spawn**, not “bit-identical isolation across OSes”.

## Settings shape

```json
{
  "integrations": {
    "sandbox": {
      "enabled": true,
      "defaults_by_mode": {
        "plan": "high",
        "normal": "standard",
        "accept_edits": "standard",
        "yolo": "minimal"
      },
      "fail_mode_by_mode": {
        "plan": "hard_fail",
        "normal": "hard_fail",
        "accept_edits": "hard_fail",
        "yolo": "degrade_with_mark"
      }
    }
  }
}
```

| Field | Semantics |
|-------|-----------|
| `enabled: false` | Global **DegradeWithMark** for all modes (work continues) + forced metadata/TUI “sandbox disabled”. Does **not** use silent bare exec without marks. |
| `defaults_by_mode` | Optional per-mode `SecurityLevel` override |
| `fail_mode_by_mode` | Optional per-mode `FailMode` override |

CLI `sandbox enable|disable` must **persist** `integrations.sandbox.enabled` (P1). `sandbox status` shows backend, effective mode, resolved level, fail_mode, enabled.

## Runtime data flow

```text
TUI / daemon session
  → EffectiveMode (Plan | Normal | AcceptEdits | Yolo)
  → ToolContext { workdir, effective_mode, ... }

Tool call
  → PermissionPolicy / guardian (unchanged order)
  → if allowed:
       policy = SandboxPolicyResolver::resolve(mode, settings, tool_kind, workspace)
       SandboxManager.execute/spawn(policy.profile)
         Ok → ToolOutput + sandbox metadata
         Err(infra) →
           HardFail → ToolError { code: sandbox_unavailable | sandbox_spawn_failed }
           DegradeWithMark → shell_command_captured + metadata sandbox_bypassed=true
                            + TUI visible bypass indicator
```

### ToolContext requirement (locked)

- `EffectiveMode` is **only** supplied through `ToolContext` (or equivalent per-call context on the tool port).
- No `Arc<RwLock<RootPermissionMode>>` process global for sandbox resolution.
- Call sites that today use `execute()` without context must be migrated to `execute_with_context` for exec tools, or construct a context with explicit mode.

### TUI bypass visibility (locked)

When `sandbox_bypassed` or `enabled=false` degrade occurs:

- Tool metadata always set.
- TUI shows a **user-visible** signal (status bar badge and/or toast / session system line). Metadata-only is insufficient.

## Module / API sketch

| Item | Location |
|------|----------|
| `EffectiveMode`, `FailMode`, `ResolvedSandboxPolicy`, `SandboxPolicyResolver` | `src/sandbox/policy.rs` (new) |
| `SandboxSettings` | `src/config/` + `IntegrationsConfig.sandbox` |
| Wire mode into context | `ToolContext` in agent/tools |
| Exec tools | `execute_command.rs`, `session_manager.rs`, `run_test.rs` |
| CLI | `cli/args.rs` `run_sandbox` |
| Docs | `docs/SANDBOX.md`, `WGENTY.md`, CHANGELOG |

Resolver is pure where possible: `(EffectiveMode, &SandboxSettings, workspace) -> ResolvedSandboxPolicy` for unit tests without OS backends.

## Observability metadata

On shell tool results (success or structured error where applicable):

```text
permission_mode                 — effective mode string
sandbox_level                   — minimal|standard|high|paranoid
sandbox_backend                 — seatbelt|seccomp+ns|job-object|none|...
sandbox_enforced                — bool (hardware/backend isolation active for this run)
sandbox_bypassed                — bool (direct spawn or disabled path)
sandbox_fail_mode               — hard_fail|degrade_with_mark
sandbox_enforcement_fidelity    — full|partial|none (backend capability honesty)
```

## Phased delivery

### P0 — Core linkage (security-critical)

1. `SandboxSettings` + serde defaults.
2. `SandboxPolicyResolver` + matrix unit tests.
3. Extend `ToolContext` with `effective_mode`.
4. Exec tools: build profile from resolver; remove hard-coded Minimal for default shell path.
5. FailMode: remove silent fallback; HardFail vs DegradeWithMark.
6. Metadata on tool output.
7. Tests: matrix, settings override, hard-fail no direct spawn, Yolo degrade marks bypass.
8. CHANGELOG BREAKING note: Normal shell is Standard+HardFail (network Full retained for package managers; Plan is High+no net).

### P1 — Product surface

1. TUI bypass / disabled indicator.
2. `sandbox status` shows mode ↔ level ↔ fail_mode.
3. `sandbox enable|disable` persists settings.
4. Subagent context carries effective mode into ToolContext.

### P2 — Platform fidelity (bounded)

1. macOS: tests that High/Plan profiles deny network (where Seatbelt available); Standard may allow Full.
2. Linux: honor `NetworkPolicy` for `unshare --net` (do not always unshare net blindly); **no** seccomp work.
3. Align `docs/SANDBOX.md` with Windows degrade semantics already in code.

## Testing plan (provable safety)

| Test | Assert |
|------|--------|
| `resolve_plan_is_high_hard_fail` | Plan → High + HardFail |
| `resolve_normal_is_standard_hard_fail` | Normal → Standard + HardFail |
| `resolve_yolo_is_minimal_degrade` | Yolo → Minimal + DegradeWithMark |
| `settings_override_level` | defaults_by_mode changes level |
| `hard_fail_no_direct_spawn` | mock backend Err → ToolError; direct spawn not called |
| `yolo_degrade_sets_bypassed` | mock Err → Ok output + sandbox_bypassed |
| `missing_mode_defaults_normal` | no mode → Normal defaults |
| `enabled_false_marks_bypass` | disabled → degrade path + flags |

Integration (macOS CI optional / local): High/Plan profile blocks `curl` to external host when Seatbelt present. Standard/Normal intentionally allows Full network.

## Migration / BREAKING

| Before | After |
|--------|-------|
| All exec ≈ Minimal + Full network + silent bare fallback | Normal/AcceptEdits ≈ **Standard + Full network + HardFail**; Plan ≈ **High + no network + HardFail** |
| Sandbox fail → silent direct | Normal/Plan/AcceptEdits → **error**; Yolo → marked degrade |
| CLI enable/disable cosmetic | Real `integrations.sandbox.enabled` persist |

Isolation under Normal is workspace FS + env allowlist + HardFail (not “no network”). Plan still has no network by default.

## Error handling

- Use typed codes: `sandbox_unavailable`, `sandbox_spawn_failed`, `sandbox_killed`, `sandbox_bypassed` (metadata, not always error).
- Prefer `thiserror` / existing `ToolError` codes; human messages explain mode + fail_mode.
- Never claim sandboxed execution in UI when `sandbox_bypassed` or `sandbox_enforced=false`.

## Security notes

- Permission and sandbox remain ordered: **authorize then isolate**.
- Linkage must not weaken guardian: Yolo still runs guardian after auto-approve (existing behavior).
- Fail-closed on Normal/Plan is the primary safety win vs today’s silent bare exec.

## Open follow-ups (out of scope)

- Per-mode network override separate from level.
- Independent sandbox level UI.
- Linux seccomp; Windows Restricted Token.
- Sandboxing non-shell tools.

## Approval record

| Section | Decision |
|---------|----------|
| Core problem | Safer defaults + mode linkage |
| Approach | A — Profile Matrix |
| Mode → level matrix | Plan High, Normal Standard, AcceptEdits shell Standard, Yolo Minimal |
| Fail policy | Plan/Normal/AcceptEdits HardFail; Yolo DegradeWithMark |
| Settings | mode defaults + fail_mode overrides; enabled flag |
| Mode plumbing | **ToolContext only** |
| Bypass UX | **TUI visible** + metadata |
| Missing mode | Default Normal (Standard + HardFail) |
| Disabled sandbox | Global DegradeWithMark + marks |
| Spec status | Approved to write implementation plan |
