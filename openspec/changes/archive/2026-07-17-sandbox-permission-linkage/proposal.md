# Proposal: Sandbox ↔ Permission Mode Linkage

## Why

Permission mode and OS sandbox are independent today:

- Shell tools hard-code `SecurityLevel::Minimal` (full network) regardless of Normal / Plan / AcceptEdits / Yolo.
- Sandbox infrastructure failures silently fall back to direct spawn (warn log only).
- CLI `sandbox enable|disable` is cosmetic; no `settings.json` control.
- Switching mode does not change isolation or fail policy, so the status bar oversells safety.

Goal: **safer defaults**, **settings overrides**, **mode-differentiated fail policy**, and **provable behavior** via tests—without blocking intentional loose workflows under Yolo.

## What Changes

1. **Profile matrix (P0)**  
   Map `EffectiveMode` (Plan | Normal | AcceptEdits | Yolo) to default `SecurityLevel` + `FailMode`:
   - Plan → High + HardFail  
   - Normal / AcceptEdits (shell) → Standard + HardFail  
   - Yolo → Minimal + DegradeWithMark  

2. **Settings (P0)**  
   `integrations.sandbox`: `enabled`, `defaults_by_mode`, `fail_mode_by_mode`.  
   `enabled: false` → global DegradeWithMark + visible marks (not silent bare exec).

3. **ToolContext-only mode (P0)**  
   Plumb `effective_mode` on `ToolContext` (no process-global sandbox lock). Subagents inherit parent effective mode.

4. **Exec tools fail policy (P0)**  
   `execute_command` / `exec_command` / `run_test`: resolve profile via `SandboxPolicyResolver`; HardFail never direct-spawns; DegradeWithMark sets metadata `sandbox_bypassed`.

5. **Observability (P0/P1)**  
   Tool metadata: permission_mode, sandbox_level, backend, enforced, bypassed, fail_mode.  
   TUI surfaces bypass/disabled (P1). CLI persists enable/disable (P1).

6. **Docs / BREAKING**  
   Normal shell loses default full network; CHANGELOG + SANDBOX.md + WGENTY.md.

## Impact

- **Specs:** new `sandbox-permission-linkage` (mode matrix, fail modes, settings, ToolContext, metadata).
- **Code:** `src/sandbox/policy.rs`, `src/config/sandbox_settings.rs`, exec tools, `ToolContext`, daemon/TUI mode sync, CLI sandbox commands.
- **Non-goals:** Linux seccomp-bpf, Windows Restricted Tokens, OS sandbox for pure FS tools, independent sandbox UI knob.

## Success Criteria

1. Matrix unit tests pass (Plan High HardFail, Normal Standard HardFail, Yolo Minimal Degrade).
2. HardFail path never calls direct spawn on sandbox infra error.
3. Yolo/disabled degrade sets `sandbox_bypassed` metadata.
4. Missing mode defaults to Normal.
5. Settings overrides change level/fail_mode.
6. `cargo fmt` / clippy / related tests pass.
