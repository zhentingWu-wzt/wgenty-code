# symbol-query Specification

## Purpose
TBD - created by archiving change code-graph-tool. Update Purpose after archive.
## Requirements
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

### MODIFIED Requirement: Filter and sort support

The system SHALL support codegraph_node / codegraph_explore output filtering and sorting.

#### Scenario: Filter and sort support

- **WHEN** querying `codegraph_node` with `sort_by: "confidence"` and/or `filter: {"name_prefix": "run_"}`
- **THEN** results are sorted by confidence descending and filtered to matching names only

### Requirement: Symbol reference lookup
The system SHALL return all locations where a given symbol is referenced (called, imported, type-referenced).

#### Scenario: Find all references to a function
- **WHEN** querying references for a function `execute`
- **THEN** the system returns a list of {file, line, column, context_line} for every call site and import of `execute`

#### Scenario: No references found
- **WHEN** a symbol has only a definition and no references
- **THEN** the system returns an empty reference list with a clear indication

### Requirement: Index-first query strategy
The system SHALL query the codegraph index first before falling back to regex-based LSP search.

#### Scenario: Index available
- **WHEN** `codegraph_node` or `codegraph_explore` is called and the codegraph index exists
- **THEN** the system returns indexed results without invoking the regex-based lsp tool, and marks the result source as `[codegraph]`

#### Scenario: Index unavailable fallback
- **WHEN** `codegraph_node` or `codegraph_explore` is called and the codegraph index does not exist and cannot be auto-built
- **THEN** the system falls back to regex-based LSP search and marks the result source as `[regex fallback]`

### Requirement: Symbol exploration by query
The system SHALL accept a natural query string and return relevant symbols along with their relationships.

#### Scenario: Explore trait implementors
- **WHEN** querying `codegraph_explore("Tool implementations")`
- **THEN** the system finds the `Tool` trait and returns all `impl Tool for Xxx` blocks with their locations

#### Scenario: Explore module structure
- **WHEN** querying `codegraph_explore("tools module structure")`
- **THEN** the system returns the module hierarchy under `src/tools/` with key symbols in each submodule

### Requirement: Explainability fields in output

The system SHALL include `audit_id`, `confidence`, and `source` fields in every codegraph query response.

#### Scenario: audit_id present

- **WHEN** any codegraph query returns results
- **THEN** the response includes an `audit_id` field (UUID v4) that matches an entry in `.codegraph/audit.log`

#### Scenario: confidence and source per symbol

- **WHEN** `codegraph_node` returns one or more symbols
- **THEN** each symbol in the result has `confidence` ("high"/"medium"/"low"/"unresolved") and `source` ("treesitter-ast"/"regex-match"/"inferred"/"fuzzy-match"/"none")

