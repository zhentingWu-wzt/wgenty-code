## MODIFIED Requirements

### Requirement: Caller analysis

The system SHALL return the list of all functions that call a given function. The `codegraph_explore` tool description SHALL include scenario-based usage guidance ("PREFER FOR ... AVOID WHEN ...") to direct Agent decision-making toward call relationship and module structure exploration tasks.

#### Scenario: Direct callers

- **WHEN** querying `codegraph_node("execute")` with `callers` option
- **THEN** the system returns every function that directly invokes `execute()`, with call site location

#### Scenario: No callers (entry point)

- **WHEN** querying callers for `main()`
- **THEN** the system returns an empty caller list with `is_entry_point: true` indication

#### Scenario: Tool description includes scenario guidance

- **WHEN** Agent reads the `codegraph_explore` tool description
- **THEN** the description includes:
  - A "PREFER FOR" clause listing scenarios: exploring module structure, browsing call graphs across multiple symbols, understanding cross-module relationships
  - An "AVOID WHEN" clause distinguishing it from `codegraph_node` (single-symbol lookup) and from grep (text patterns)
- **THEN** the description differentiates `codegraph_explore` from `codegraph_node` in scope (multiple symbols / relationships vs single symbol)
