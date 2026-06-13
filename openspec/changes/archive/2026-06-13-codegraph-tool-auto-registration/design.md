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
