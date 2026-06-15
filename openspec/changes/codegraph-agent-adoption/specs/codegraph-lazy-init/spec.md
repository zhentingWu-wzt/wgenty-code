## MODIFIED Requirements

### Requirement: CodeGraph tools auto-register with lazy initialization

CodeGraph tools SHALL be registered in the default `ToolRegistry` and SHALL lazily initialize the query engine from `.codegraph/index.db` on first use. When the index is absent, the error message SHALL provide actionable, specific guidance to enable Agent recovery without abandoning the codegraph workflow entirely.

#### Scenario: Index exists

- **WHEN** `.codegraph/index.db` exists in the current working directory
- **THEN** the engine SHALL be initialized on first tool call and SHALL remain cached for subsequent calls

#### Scenario: Index absent

- **WHEN** `.codegraph/index.db` does not exist
- **THEN** the tool SHALL return a `ToolError` whose message includes:
  - The expected index path (`.codegraph/index.db`)
  - The exact command to fix the issue (`wgenty-code codegraph index`)
  - An estimate of the fix cost to reduce hesitation (e.g., "typically takes <5s on a Rust project")
  - A fallback hint limiting grep / file_read to a temporary alternative for this single task only (to prevent permanent fallback)
- **THEN** the message SHALL use actionable, parseable instructions and SHALL avoid unbounded fallback language such as "acceptable" or "fall back to grep" without a time or scope qualifier

#### Scenario: Engine initialized once

- **WHEN** `codegraph_node` or `codegraph_explore` is called multiple times
- **THEN** the engine SHALL only be opened once (subsequent calls reuse the cached instance)
