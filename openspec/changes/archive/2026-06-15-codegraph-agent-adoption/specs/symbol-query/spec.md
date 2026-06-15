## MODIFIED Requirements

### Requirement: Symbol definition lookup

The system SHALL return the exact file path, line number, column, signature, and visibility of a symbol given its name. The `codegraph_node` tool description SHALL include scenario-based usage guidance ("PREFER FOR ... AVOID WHEN ...") to direct Agent decision-making toward symbol-related navigation tasks.

#### Scenario: Find a function definition

- **WHEN** querying `codegraph_node("ToolRegistry")`
- **THEN** the system returns `src/tools/mod.rs:75` with the full signature and visibility

#### Scenario: Find a struct definition

- **WHEN** querying `codegraph_node("StreamEvent")`
- **THEN** the system returns the file path, line, column, and all fields of the struct

#### Scenario: Symbol not found

- **WHEN** querying a symbol name that does not exist in the index
- **THEN** the system returns a `not_found` result with suggestions for similarly-named symbols (Levenshtein distance ≤ 3)

#### Scenario: Tool description includes scenario guidance

- **WHEN** Agent reads the `codegraph_node` tool description
- **THEN** the description includes:
  - A "PREFER FOR" clause listing scenarios: finding symbol definitions, listing callers/callees, finding references
  - An "AVOID WHEN" clause indicating when grep is more appropriate (text patterns, non-symbol concepts)
- **THEN** the description's first sentence describes the symbolic capability (not just "look up by name")
