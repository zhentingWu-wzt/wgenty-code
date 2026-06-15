## MODIFIED Requirements

### Requirement: Symbol definition lookup

The system SHALL return the exact file path, line number, column, signature, and visibility of a symbol given its name. The result SHALL include `confidence` and `source` fields for explainability. The query SHALL support fuzzy matching when exact match fails. The `codegraph_node` tool description SHALL include scenario-based usage guidance ("PREFER FOR ... AVOID WHEN ...").

#### Scenario: Find a function definition

- **WHEN** querying `codegraph_node("ToolRegistry")`
- **THEN** the system returns `src/tools/mod.rs:75` with the full signature, visibility, and `confidence: "high"`, `source: "treesitter-ast"`

#### Scenario: Find a struct definition

- **WHEN** querying `codegraph_node("StreamEvent")`
- **THEN** the system returns the file path, line, column, and all fields of the struct, with confidence/source fields

#### Scenario: Symbol not found → fuzzy suggestions

- **WHEN** querying a symbol name that does not exist in the index (exact match fails)
- **THEN** the system returns a `not_found` result with up to 5 similarly-named symbols (Levenshtein distance ≤ 3, length difference ≤ 50%), each with `confidence: "low"` and `source: "fuzzy-match"`

#### Scenario: Symbol not found → no fuzzy candidates

- **WHEN** neither exact nor fuzzy matching yields results
- **THEN** the system returns `not_found` with an empty suggestions array

#### Scenario: Tool description includes scenario guidance

- **WHEN** Agent reads the `codegraph_node` tool description
- **THEN** the description includes "PREFER FOR" clause and "AVOID WHEN" clause (consistent with #1 agent-adoption)

#### Scenario: Filter and sort support

- **WHEN** querying `codegraph_node` with `sort_by: "confidence"` and/or `filter: {"name_prefix": "run_"}`
- **THEN** results are sorted by confidence descending and filtered to matching names only

### Requirement: Explainability fields in output

The system SHALL include `audit_id`, `confidence`, and `source` fields in every codegraph query response.

#### Scenario: audit_id present

- **WHEN** any codegraph query returns results
- **THEN** the response includes an `audit_id` field (UUID v4) that matches an entry in `.codegraph/audit.log`

#### Scenario: confidence and source per symbol

- **WHEN** `codegraph_node` returns one or more symbols
- **THEN** each symbol in the result has `confidence` ("high"/"medium"/"low"/"unresolved") and `source` ("treesitter-ast"/"regex-match"/"inferred"/"fuzzy-match"/"none")
