# symbol-query Specification

## Purpose
TBD - created by archiving change code-graph-tool. Update Purpose after archive.
## Requirements
### Requirement: Symbol definition lookup
The system SHALL return the exact file path, line number, column, signature, and visibility of a symbol given its name.

#### Scenario: Find a function definition
- **WHEN** querying `codegraph_node("ToolRegistry")`  
- **THEN** the system returns `src/tools/mod.rs:75` with the full signature and visibility

#### Scenario: Find a struct definition
- **WHEN** querying `codegraph_node("StreamEvent")`
- **THEN** the system returns the file path, line, column, and all fields of the struct

#### Scenario: Symbol not found
- **WHEN** querying a symbol name that does not exist in the index
- **THEN** the system returns a `not_found` result with suggestions for similarly-named symbols (Levenshtein distance ≤ 3)

#### Scenario: Ambiguous symbol name
- **WHEN** multiple symbols share the same name (e.g., `Config` in different modules)
- **THEN** the system returns all matches with their fully-qualified paths, letting the caller disambiguate

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

