# call-graph Specification

## Purpose
TBD - created by archiving change code-graph-tool. Update Purpose after archive.
## Requirements
### Requirement: Caller analysis

The system SHALL return the list of all functions that call a given function. The `codegraph_explore` tool description SHALL include scenario-based usage guidance ("PREFER FOR ... AVOID WHEN ...").

#### Scenario: Direct callers

- **WHEN** querying `codegraph_node("execute")` with `callers` option
- **THEN** the system returns every function that directly invokes `execute()`, with call site location

#### Scenario: No callers (entry point)

- **WHEN** querying callers for `main()`
- **THEN** the system returns an empty caller list with `is_entry_point: true` indication

#### Scenario: Tool description includes scenario guidance

- **WHEN** Agent reads the `codegraph_explore` tool description
- **THEN** the description includes "PREFER FOR" and "AVOID WHEN" clauses (consistent with #1 agent-adoption)

### Requirement: Callee analysis

The system SHALL return the list of all functions called by a given function.

#### Scenario: Direct callees

- **WHEN** querying `codegraph_node("run_async")` with `callees` option
- **THEN** the system returns every function directly called within `run_async()`, with call site locations

#### Scenario: Leaf function

- **WHEN** a function makes no calls to other user-defined functions
- **THEN** the system returns an empty callee list with `is_leaf: true` indication

### Requirement: Transitive call graph
The system SHALL support querying call relationships up to a configurable depth (default depth=2, max depth=5).

#### Scenario: Callers with depth=2
- **WHEN** querying `codegraph_node("checkpoint_file")` with `callers` and `depth=2`
- **THEN** the system returns direct callers AND their callers (transitive callers up to depth 2)

#### Scenario: Callees with depth=3
- **WHEN** querying `codegraph_node("process_request")` with `callees` and `depth=3`
- **THEN** the system returns the full call tree up to 3 levels deep

### Requirement: Method resolution for impl blocks
The system SHALL correctly resolve method calls on struct/enum types through their `impl` blocks.

#### Scenario: Method call on struct
- **WHEN** a function calls `registry.register(...)` and `register` is defined in `impl ToolRegistry`
- **THEN** the call graph resolves the call target to `ToolRegistry::register` in the corresponding impl block

### Requirement: Multi-hop call path tree

The system SHALL support querying call relationships as a multi-hop path tree from a given symbol, up to depth 5, with per-hop evidence.

#### Scenario: Build path tree from symbol

- **WHEN** `codegraph_explore` results include call relationships
- **THEN** the output includes a `call_paths` field containing an array of paths, each with a `hops[]` array where each hop contains `from` (symbol name), `to` (symbol name), `rel` (RelKind), `location` (file:line)

#### Scenario: Depth truncation

- **WHEN** the call path tree exceeds 5 levels of depth
- **THEN** paths are truncated at depth 5 and the response includes `truncated: true`

#### Scenario: Per-hop evidence

- **WHEN** any hop in the call path tree is rendered
- **THEN** the hop MUST include the `location` field (file path and line number) and `rel` field (which RelKind connects the two symbols)

### Requirement: Two-point shortest call path

The system SHALL provide a `call_path` tool that finds the shortest call path between two named symbols.

#### Scenario: Path found

- **WHEN** querying `call_path("main", "run_async")` and a call path exists
- **THEN** the system returns the shortest path as a hops[] array with total depth and per-hop evidence (from/to/rel/location)

#### Scenario: No path

- **WHEN** the two symbols have no connecting call path in the index
- **THEN** the system returns `{"path_found": false, "reason": "no_connecting_path"}`

