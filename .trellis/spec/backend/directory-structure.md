# Directory Structure

> How backend code is organized in this project.

---

## Overview

wgenty-code is a **single Rust crate** (not a workspace). All backend code lives under `src/` with flat, domain-oriented modules declared in `src/lib.rs`. The crate builds one CLI binary (`src/main.rs`) plus the library.

---

## Directory Layout

```
src/
├── lib.rs            # Module declarations only — no logic
├── main.rs           # CLI binary entry point
├── agent/            # Agent runtime core (coordinator, capability, events)
│   └── runtime/      # Turn/stream execution loop, RuntimeError
├── api/              # LLM provider clients (Anthropic, etc.)
│   └── anthropic/    # Per-provider submodule
├── cli/              # CLI argument parsing and commands
├── config/           # Configuration loading (no business deps)
├── context/          # Context window mgmt: history, session, storage, tokenizer
├── daemon/           # Background daemon + skills_api
├── exec_session/     # Shell command execution sessions
├── i18n/             # Fluent-based internationalization
├── knowledge/        # Skill loading: external registry, loader, root resolver
├── mcp/              # MCP (Model Context Protocol) integration
├── org_graph/        # Org/agent graph dispatch
├── permissions/      # Guardian: permission and safety review
├── prompts/          # System prompt assembly
├── runtime/          # Shared runtime infrastructure
├── sandbox/          # Sandbox backends (OS-level isolation)
├── services/         # Long-running services
├── state/            # Persistent app state
├── tasks/            # Task management (store, lifecycle)
├── teams/            # Multi-agent team coordination
├── tools/            # Built-in tools (Tool trait impls)
│   ├── execution/    # exec_command etc.
│   ├── filesystem/   # file_read, file_edit, apply_patch
│   ├── meta/         # skill, task, rlm meta-tools
│   └── search/       # grep, glob, web_fetch, web_search
├── transcript/       # Session transcript persistence (SQLite)
├── tui/              # ratatui terminal UI frontend
├── utils/            # Cross-cutting helpers
└── wasm/             # WASM plugin runtime
```

---

## Module Organization

- **One module = one domain.** New functionality goes in the matching domain module, not in a new top-level module. Cross-domain abstractions go in `utils/` or a trait in the consumer module.
- **Submodule for size.** When a module file grows large, split into a directory with `mod.rs` (e.g. `agent/runtime/`, `tools/meta/rlm/`). Both `mod.rs` style and sibling-file style exist; stay consistent within each module.
- **Feature-gated code** uses Cargo features in `Cargo.toml` (e.g. `rhai` is optional) to keep the default build lean.
- **No circular deps**: `config/` depends on nothing; `tools/` does not depend on `agent/`; `api/` does not depend on `cli/`/`tui/`. Cross-layer coupling goes through traits (e.g. `SandboxBackend`, `Tool`).

---

## Naming Conventions

- Files/modules: `snake_case.rs` (Rust standard).
- Types/traits: `CamelCase`; functions/variables: `snake_case`; constants: `SCREAMING_SNAKE_CASE`.
- Provider-specific code is namespaced in its own submodule (`api/anthropic/`), never scattered.
- Test code lives inline with `#[cfg(test)] mod tests` in the same file (e.g. `src/transcript/store.rs` has in-file tests).

---

## Examples

- Well-organized domain module: `src/agent/` — `coordinator.rs`, `capability.rs`, `runtime/` submodule.
- Submodule split for size: `src/tools/meta/rlm/` — `classifier.rs`, `planner.rs`, `pipeline.rs`.
- Trait-based abstraction: `SandboxBackend` in `src/sandbox/`, `Tool` in `src/tools/mod.rs`.
