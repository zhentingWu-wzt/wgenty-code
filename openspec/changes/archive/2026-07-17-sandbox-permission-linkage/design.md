# Design: Sandbox ↔ Permission Mode Linkage

Canonical design lives at:

`docs/superpowers/specs/2026-07-17-sandbox-permission-linkage-design.md`

Implementation plan:

`docs/superpowers/plans/2026-07-17-sandbox-permission-linkage.md`

## Summary

- **Approach:** Profile Matrix (A) — mode selects default isolation + fail policy; settings override.
- **EffectiveMode** on ToolContext only; Plan is sandbox-High while RootPermissionMode Plan maps to Normal for auto-approve.
- **FailMode:** HardFail vs DegradeWithMark; silent degrade forbidden.
- **Platform partial enforcement:** level is intent; backend success ≠ full Seatbelt parity on Windows.

## Phases

| Phase | Scope |
|-------|--------|
| P0 | Settings, resolver, ToolContext, exec HardFail/Degrade, tests, CHANGELOG |
| P1 | TUI bypass, CLI persist, full Plan sync to daemon |
| P2 | Linux NetworkPolicy honor, macOS assertions, docs fidelity |
