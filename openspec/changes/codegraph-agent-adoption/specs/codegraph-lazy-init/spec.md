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
  - The directory in which to run the command ("in this directory" or equivalent)
  - A fallback hint indicating that grep / file_read are acceptable temporary alternatives
- **THEN** the message SHALL avoid generic phrasing such as "first" without explanation, in favor of actionable, parseable instructions

#### Scenario: Engine initialized once

- **WHEN** `codegraph_node` or `codegraph_explore` is called multiple times
- **THEN** the engine SHALL only be opened once (subsequent calls reuse the cached instance)
