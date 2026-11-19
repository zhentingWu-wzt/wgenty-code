# Error Handling

> How errors are handled in this project.

---

## Overview

Two-layer strategy, matching the split in `AGENTS.md`:

- **Library / reusable code** (`api/`, `agent/runtime/`, domain stores): `thiserror`-derived error enums with precise variants.
- **Application layer** (`cli/`, `tui/`, top-level orchestration): `anyhow::Result` + `.context("human-readable description")` on every fallible call.

---

## Error Types

Custom error enums live close to their domain, named `<Domain>Error`:

```rust
// src/agent/runtime/error.rs — canonical example
#[derive(Debug, Clone, thiserror::Error)]
pub enum RuntimeError {
    #[error("{0}")]
    StreamTimeout(String),
    #[error("Exceeded {max_rounds} LLM rounds")]
    MaxRoundsExceeded { max_rounds: usize },
    #[error("Tool error: {0}")]
    Tool(String),
}
```

Rules:
- Every variant gets a `#[error("...")]` message a user can act on.
- Frontends map domain errors into their own type (e.g. TUI `AgentError`) at one boundary — callers must **never** substring-match error strings. The single allowed exception is a documented classification helper (see `RuntimeError::from_stream_failure`), kept in one place.

---

## Error Handling Patterns

**Library layer** — propagate typed errors, no context loss:

```rust
// ✅ src/agent/runtime/error.rs pattern
Result<T, RuntimeError>   // typed, matchable
```

**Application layer** — `anyhow` with mandatory context:

```rust
// ✅ Recommended
std::fs::read_to_string(path)
    .context("读取配置文件失败")?;

// ❌ Forbidden — bare ? swallows context
Ok(std::fs::read_to_string(path)?)
```

- **No bare `?`** in app code: every `?` chains `.context()` describing *what operation* failed.
- **No `unwrap()` / `expect()`** outside tests and genuinely infallible paths. If a panic-by-construction is justified (e.g. an index known in-bounds), add a comment explaining why.
- **Error + log split**: when a rich payload is too verbose for the user surface, show the concise message to the user and emit the full detail via `tracing::warn!` to the dev log (pattern: `format_api_error` in `src/api/error.rs`).

---

## API Error Responses

Network/provider errors are wrapped into user-friendly messages by dedicated helpers in `src/api/error.rs`:

- `wrap_network_error(err, api_name)` — classifies reqwest errors (timeout / DNS / TLS / refused) into actionable ⚠️ messages.
- `format_api_error(status, body)` — parses upstream `{"error":{"code","message"}}` JSON into `"{code}: {message}"`, falling back to `"API error ({status}): {body}"` so no detail is lost. Raw body also goes to the dev log.

Add new upstream failure modes there, not ad-hoc at call sites.

---

## Common Mistakes

- **Substring-matching error strings** in callers — classify once in a helper (like `from_stream_failure`) and match on typed variants instead.
- **Double-escaped JSON blobs** shown to users — parse the upstream error shape first (`format_api_error`).
- **Silent fallbacks**: when recovering from an error (e.g. worktree creation failing → shared dir), log `tracing::warn!` with the original error before falling back (see `src/tools/meta/task.rs`).
