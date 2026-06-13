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
