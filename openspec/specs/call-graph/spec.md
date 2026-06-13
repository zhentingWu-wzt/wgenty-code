# call-graph Specification

## Purpose
TBD - created by archiving change code-graph-tool. Update Purpose after archive.
## Requirements
### Requirement: Caller analysis
The system SHALL return the list of all functions that call a given function.

#### Scenario: Direct callers
- **WHEN** querying `codegraph_node("execute")` with `callers` option
- **THEN** the system returns every function that directly invokes `execute()`, with call site location

#### Scenario: No callers (entry point)
- **WHEN** querying callers for `main()`
- **THEN** the system returns an empty caller list with `is_entry_point: true` indication

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

