# Quality Guidelines

> Code quality standards for backend development.

---

## Overview

Quality is enforced by CI (`.github/workflows/ci.yml`) with three mandatory gates: `cargo fmt -- --check`, `cargo clippy --all-targets -- -D warnings` (zero warnings), and `cargo test --all`. Code must compile and pass on linux / macos / windows.

---

## Forbidden Patterns

- **`unwrap()` / `expect()`** outside tests (see error-handling.md).
- **Bare `?`** without `.context()` in application-layer code.
- **`is_read_only()` left at default `false`** for read-only tools — read-only tools (`file_read`, `grep`, `glob`) must explicitly return `true`, or guardian treats them as needing write permission.
- **String-matching on error messages** — use typed error variants.
- **Hardcoded user-facing strings** — must go through `i18n/` (Fluent format).
- **Platform-specific assumptions** (path separators, line endings) — code must compile and run on all three OSes.
- **Direct `agent/` dependency from `tools/`**, and `cli/`/`tui/` dependency from `api/` (layering rules in directory-structure.md).

## Required Patterns

- **`cargo fmt`** before every commit; rustfmt is the only formatter.
- **`///` doc comments** on public APIs; `//` comments for non-obvious internal logic.
- **`Tool` trait impls** return structured `ToolResult` with `success`, `output`, optional `metadata`.
- **Structured logging** via `tracing` (see logging-guidelines.md) — never `println!` in library code.
- **Feature flags** for optional heavy deps (e.g. `rhai`) with `required-features` correctly declared.
- **Lock discipline**: `Arc<RwLock<T>>` for shared mutable state; hold locks for the minimum scope; async background work via `tokio::spawn` with `JoinHandle` handled.

---

## Testing Requirements

- **Tests live inline** in the same file under `#[cfg(test)] mod tests` (see `src/transcript/store.rs`).
- **New behavior needs tests**: bug fixes include a regression test; new tools get executor-level tests.
- **DB code**: test idempotent migrations (re-opening an already-migrated DB must not error) and pre-migration schema compatibility — see `src/transcript/store.rs` tests.
- **Frontend (web/)**: vitest + React Testing Library; colocated `*.test.tsx` files (e.g. `Composer.test.tsx`).
- Run before PR: `cargo clippy --all-targets -- -D warnings && cargo fmt && cargo test --all`.

---

## Code Review Standards

Focus areas (from `AGENTS.md`):

- Error handling sufficiency — is context actionable?
- Unhandled `unwrap()` / bare `?`.
- Lock hold time in async code.
- `is_read_only()` correctness on tools (affects permission review).
- Feature flag `required-features` wiring.
- Whether WGENTY.md / docs need updating for the change.
- PR title + commits follow Conventional Commits (`feat|fix|docs|style|refactor|perf|test|chore(scope): ...`), in English, with `CHANGELOG.md` updated for notable changes.
