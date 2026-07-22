# Tasks: Sandbox ↔ Permission Mode Linkage

## 1. P0 — Settings + SecurityLevel serde

- [x] 1.1 Add `SandboxSettings` (`enabled`, `defaults_by_mode`, `fail_mode_by_mode`) + defaults/serde tests
- [x] 1.2 Wire `IntegrationsConfig.sandbox`; update `settings.json.template`
- [x] 1.3 Derive serde on `SecurityLevel` (`snake_case`)

## 2. P0 — Policy resolver matrix

- [x] 2.1 Add `src/sandbox/policy.rs`: `EffectiveMode`, `FailMode`, `ResolvedSandboxPolicy`, `SandboxPolicyResolver`
- [x] 2.2 Unit tests: Plan/Normal/AcceptEdits/Yolo defaults, settings override, enabled=false → DegradeWithMark
- [x] 2.3 `resolve_with_network` for run_test (level kept, network Full optional)

## 3. P0 — ToolContext.effective_mode

- [x] 3.1 Extend `ToolContext` with `effective_mode: EffectiveMode`
- [x] 3.2 Fix all construction sites; map `RootPermissionMode` / TUI `AgentMode`
- [x] 3.3 Default missing mode → Normal

## 4. P0 — Exec tools FailMode

- [x] 4.1 Shared `sandbox_exec` helpers (resolve, metadata, degrade decision)
- [x] 4.2 `execute_command`: drop hard-coded Minimal; HardFail vs DegradeWithMark; metadata
- [x] 4.3 `session_manager` / `exec_command`: per-spawn resolve + fail mode
- [x] 4.4 `run_test`: mode level + allow_network network override only
- [x] 4.5 Tests: hard-fail no direct spawn; yolo degrade marks bypass
- [x] 4.6 `background` tool: mode-linked spawn + HardFail / DegradeWithMark

## 5. P0 — Daemon / subagent mode plumb

- [x] 5.1 Session-level `effective_mode` (Plan distinct from permission Normal)
- [x] 5.2 Fill `ToolContext.effective_mode` on root tool path
- [x] 5.3 Subagent `SubagentPermissionContext` carries `EffectiveMode` into ToolContext

## 6. P0 — Docs

- [x] 6.1 CHANGELOG BREAKING (Normal Standard+HardFail; network Full for package managers; Plan High+no net)
- [x] 6.2 `docs/SANDBOX.md` + WGENTY.md settings rows

## 7. P1 — Product surface

- [x] 7.1 TUI bypass / disabled indicator (`sandbox_bypassed_session` + status badge)
- [x] 7.2 CLI `sandbox status|enable|disable` persists settings
- [x] 7.3 Metadata `sandbox_enforcement_fidelity` (full/partial/none)

## 8. Verify

- [x] 8.1 Targeted unit tests (policy 9 + exec/config/background/sandbox_exec 16 = PASS)
- [x] 8.2 Verification report: `docs/superpowers/reports/2026-07-17-sandbox-permission-linkage-verify.md`
- [ ] 8.3 Optional pre-merge: full `cargo check` / clippy / `cargo test --all` on CI or local host
