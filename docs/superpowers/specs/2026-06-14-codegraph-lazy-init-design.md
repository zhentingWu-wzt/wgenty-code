---
comet_change: codegraph-tool-auto-registration
role: technical-design
canonical_spec: openspec
archived-with: 2026-06-13-codegraph-tool-auto-registration
status: final
---

# Technical Design: CodeGraph Lazy Initialization

## Overview

Replace pre-built `Arc<QueryEngine>` dependency with `OnceLock`-based lazy init. Tools auto-register, gracefully degrade without index.

## Implementation

### 1. Lazy Engine (`src/tools/codegraph/tools.rs`)

```rust
static ENGINE: OnceLock<Arc<QueryEngine>> = OnceLock::new();

fn get_engine() -> Result<Arc<QueryEngine>, ToolError> {
    if let Some(e) = ENGINE.get() { return Ok(e.clone()); }
    let db_path = cwd.join(".codegraph").join("index.db");
    if !db_path.exists() {
        return Err(ToolError { message: "No codegraph index found...", .. });
    }
    let store = IndexStore::open(&cwd)?;
    let engine = Arc::new(QueryEngine::new(Arc::new(store)));
    let _ = ENGINE.set(engine.clone());
    Ok(engine)
}
```

- `CodegraphNodeTool` and `CodegraphExploreTool` become unit structs (no fields)
- `execute()` calls `get_engine()?` for each invocation
- First call opens index; subsequent calls reuse cached engine

### 2. Indexer Guards (`src/tools/codegraph/indexer.rs`)

- Unresolved references: check `symbol_id_map.get()` before insert, skip on None
- Negative IDs: `if rel.source_id < 0 || rel.target_id < 0 { continue; }`
- Cross-file: require both source and target resolved, skip otherwise

### 3. Default Registration (`src/tools/mod.rs`)

```rust
registry.register(Box::new(CodegraphNodeTool::new()));
registry.register(Box::new(CodegraphExploreTool::new()));
```

## Edge Cases

| Case | Handling |
|------|----------|
| No .codegraph/ directory | Friendly error message |
| Index corrupted | IndexStore::open error propagated |
| Concurrent first calls | OnceLock ensures only one init |
| Unresolved tree-sitter refs | Skipped, not inserted |
