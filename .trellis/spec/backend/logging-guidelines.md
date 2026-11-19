# Logging Guidelines

> How logging is done in this project.

---

## Overview

The project uses **`tracing`** + `tracing-subscriber`, initialized once in `src/utils/logging.rs`.

Key design decision: **logs never go to the terminal** (they would corrupt the TUI). All logs append to `~/.wgenty-code/logs/wgenty-code.log`; if the file can't be opened, a silent sink is used as fallback. Filter level comes from `RUST_LOG`, defaulting to `wgenty_code=info`.

User-facing progress/status output is a separate channel: `eprintln!` in CLI/headless flows (`src/cli/headless_runtime.rs`) — that is UX, not logging.

---

## Log Levels

| Level | When | Real example |
|-------|------|--------------|
| `error!` | Operation failed and the failure propagates or breaks a feature | `src/tools/meta/task.rs:1126` |
| `warn!` | Recoverable failure / degraded path taken — **always include the original error** | `tracing::warn!(error = %e, "task: worktree creation failed; falling back to shared")` |
| `info!` | Significant lifecycle events (task created, sub-agent spawned) | `src/tools/meta/task.rs:352` |
| `debug!` | Diagnostic detail useful when `RUST_LOG=debug` | `src/tasks/management.rs:28` |

---

## Required Patterns

- **Structured fields over string interpolation**: `tracing::warn!(error = %e, file = %p, "checkpoint capture failed")` — not `format!("failed: {}", e)` inside the message.
- **Log before fallback**: any silent-recovery path (fallback, retry, prune failure) must `warn!` first so the degradation is observable (see `src/tools/checkpoint_store.rs`).
- **Verbose payload split**: concise message to the user, full payload to the log (see `format_api_error` in `src/api/error.rs` — upstream body goes to `tracing::warn!`).
- Call sites use the crate prefix path (`tracing::warn!`) — no `log` crate macros anywhere.

---

## What NOT to Log

- **API keys, tokens, auth headers** — never log credentials, even at debug level.
- **Full user prompts / file contents** — log paths, sizes, hashes; not content.
- **Terminal output** — logs go to file only; `println!` in library code is forbidden (use `eprintln!` only for CLI UX output).

---

## Viewing Logs

```bash
# tail the dev log
tail -f ~/.wgenty-code/logs/wgenty-code.log

# raise verbosity for one run
RUST_LOG=wgenty_code=debug wgenty-code
```
