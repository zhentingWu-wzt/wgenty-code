# Comet Design Handoff

- Change: codegraph-tool-auto-registration
- Phase: design
- Mode: compact
- Context hash: 1d11efc6efe32ad0d0dad7dd0a769d23feae56c2591b93be529b46ef2215f275

Generated-by: comet-handoff.sh

OpenSpec remains the canonical capability spec. This handoff is a deterministic, source-traceable context pack, not an agent-authored summary.

## openspec/changes/codegraph-tool-auto-registration/proposal.md

- Source: openspec/changes/codegraph-tool-auto-registration/proposal.md
- Lines: 1-33
- SHA256: b248c92978cb5e149d46def6c9c30e1c3e7d18ff43cac10177ed91dd635eee2a

```md
## Why

CodeGraph tools (`codegraph_node`, `codegraph_explore`) require a pre-built `Arc<QueryEngine>` at construction time, which forces them to be registered conditionally. If registered without a valid index, the engine fails to open and the tool panics. This means codegraph tools are unavailable in most sessions — users must manually configure or recompile to enable them. Additionally, the indexer crashes on edge cases (unresolved tree-sitter references, negative IDs, cross-file relationships).

## What Changes

### Lazy Engine Initialization
- Remove `Arc<QueryEngine>` dependency from `CodegraphNodeTool` and `CodegraphExploreTool` structs
- Use `OnceLock<Arc<QueryEngine>>` to lazily open the index from `.codegraph/index.db` on first use
- No index → friendly error: "Run `wgenty-code codegraph index` first"

### Default Tool Registration
- Register `CodegraphNodeTool` and `CodegraphExploreTool` in `ToolRegistry::new()` unconditionally
- Tools are always available, gracefully degrade when index is absent

### Indexer Robustness
- Skip unresolved references instead of inserting with invalid IDs
- Guard against negative source/target IDs in relationships
- Skip cross-file references gracefully

## Capabilities

### New Capabilities
- `codegraph-lazy-init`: CodeGraph tools auto-register with lazy engine initialization, graceful fallback when index is absent

### Modified Capabilities
<!-- None -->

## Impact

- `src/tools/codegraph/tools.rs`: `OnceLock`-based lazy init, remove struct-level `Arc<QueryEngine>`
- `src/tools/codegraph/indexer.rs`: Guard unresolved refs, negative IDs, cross-file relationships
- `src/tools/mod.rs`: Register codegraph tools unconditionally in default registry
```

## openspec/changes/codegraph-tool-auto-registration/design.md

- Source: openspec/changes/codegraph-tool-auto-registration/design.md
- Lines: 1-48
- SHA256: 7b85825c0d47b179e90de28085a9a6a3f5ae9d2a6c03a4928bf624a18db3d961

```md
## Context

CodeGraph tools need an open `IndexStore` → `QueryEngine`. Currently constructed with `Arc<QueryEngine>` at struct level, forcing conditional registration. The index lives at `cwd/.codegraph/index.db` — always at the same path relative to the working directory.

## Goals / Non-Goals

**Goals:**
- Tools constructable without pre-built engine
- Lazy engine init from `.codegraph/index.db` on first tool call
- Graceful error when index absent
- Indexer handles unresolved tree-sitter data gracefully

**Non-Goals:**
- Changing `QueryEngine` or `IndexStore` APIs
- Supporting multiple index paths or dynamic index selection
- New codegraph query capabilities

## Decisions

### Decision 1: `OnceLock<Arc<QueryEngine>>` for Lazy Init

```rust
static ENGINE: OnceLock<Arc<QueryEngine>> = OnceLock::new();

fn get_engine() -> Result<Arc<QueryEngine>, ToolError> {
    if let Some(e) = ENGINE.get() { return Ok(e.clone()); }
    let cwd = std::env::current_dir()?;
    let store = IndexStore::open(&cwd.join(".codegraph"))?;
    let engine = Arc::new(QueryEngine::new(Arc::new(store)));
    let _ = ENGINE.set(engine.clone());
    Ok(engine)
}
```

**Rationale**: `OnceLock` is stdlib (Rust 1.70+), no extra dependencies. Static lifetime matches the daemon's process lifetime. Thread-safe.

### Decision 2: Skip Unresolved References (Not Panic)

**Rationale**: Tree-sitter may produce references to symbols we didn't index (external crates, stdlib). Currently `unwrap()`-ed which panics. Skip gracefully instead.

### Decision 3: Guard Against Negative IDs

**Rationale**: Tree-sitter sometimes assigns `-1` as placeholder IDs for unresolved relationships. These should be skipped, not inserted.

## Risks / Trade-offs

- **Static OnceLock limits to one index per process** → Sufficient for current architecture (single cwd, daemon-per-session)
- **Indexer changes may hide data quality issues** → The skip is intentional: unresolved refs are expected for external symbols
```

## openspec/changes/codegraph-tool-auto-registration/tasks.md

- Source: openspec/changes/codegraph-tool-auto-registration/tasks.md
- Lines: 1-23
- SHA256: b28a0d57d84cd39b4a52a4583cc902b0d96f106665c3aa4ec7540dcbba8b930b

```md
## 1. CodeGraph Tools — Lazy Init

- [x] 1.1 Replace `Arc<QueryEngine>` field with `OnceLock<Arc<QueryEngine>>` static in `src/tools/codegraph/tools.rs`
- [x] 1.2 Implement `get_engine()` helper: open `IndexStore` from `.codegraph/index.db`, cache in OnceLock
- [x] 1.3 Update `CodegraphNodeTool` and `CodegraphExploreTool` to use `get_engine()` in `execute()`
- [x] 1.4 Return friendly error when index absent: "Run `wgenty-code codegraph index` first"

## 2. CodeGraph Indexer — Robustness

- [x] 2.1 Skip unresolved references (don't insert with invalid IDs) in `src/tools/codegraph/indexer.rs`
- [x] 2.2 Guard against negative source/target IDs in relationships
- [x] 2.3 Skip cross-file relationships when either symbol ID is unresolved

## 3. Default Tool Registration

- [x] 3.1 Register `CodegraphNodeTool` and `CodegraphExploreTool` in `ToolRegistry::default_registry()` in `src/tools/mod.rs`

## 4. Verification

- [ ] 4.1 Run `cargo build` — compiles without errors
- [ ] 4.2 Run `cargo test --lib` — all tests pass
- [ ] 4.3 Run `cargo clippy --all-targets -- -D warnings` — no new warnings
- [ ] 4.4 Manual: verify codegraph tools appear in tool list and work with valid index
```

## openspec/changes/codegraph-tool-auto-registration/specs/codegraph-lazy-init/spec.md

- Source: openspec/changes/codegraph-tool-auto-registration/specs/codegraph-lazy-init/spec.md
- Lines: 1-31
- SHA256: be2abe72536d7b98ff95dcad21583e5b663b12d8fd73962759ca50b09ff10c1d

```md
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
```

