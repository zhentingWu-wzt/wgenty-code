## ADDED Requirements

### Requirement: CodeGraph tools auto-register with lazy initialization
CodeGraph tools SHALL be registered in the default `ToolRegistry` and SHALL lazily initialize the query engine from `.codegraph/index.db` on first use.

#### Scenario: Index exists
- **WHEN** `.codegraph/index.db` exists in the current working directory
- **THEN** the engine SHALL be initialized on first tool call and SHALL remain cached for subsequent calls

#### Scenario: Index absent
- **WHEN** `.codegraph/index.db` does not exist
- **THEN** the tool SHALL return a friendly error: "No codegraph index found. Run `wgenty-code codegraph index` first."

#### Scenario: Engine initialized once
- **WHEN** `codegraph_node` or `codegraph_explore` is called multiple times
- **THEN** the engine SHALL only be opened once (subsequent calls reuse the cached instance)

### Requirement: Indexer handles unresolved tree-sitter data gracefully
The indexer SHALL skip unresolved references, negative IDs, and cross-file relationships instead of panicking.

#### Scenario: Unresolved symbol reference
- **WHEN** a reference points to a symbol ID not in the symbol map
- **THEN** the reference SHALL be skipped (not inserted with an invalid ID)

#### Scenario: Negative relationship ID
- **WHEN** a relationship has source_id < 0 or target_id < 0
- **THEN** the relationship SHALL be skipped

#### Scenario: Cross-file relationship with partially resolved symbols
- **WHEN** a relationship's source OR target is unresolved
- **THEN** the relationship SHALL be skipped (both must be resolved)
