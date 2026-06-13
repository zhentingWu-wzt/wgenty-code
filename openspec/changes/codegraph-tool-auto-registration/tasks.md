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
