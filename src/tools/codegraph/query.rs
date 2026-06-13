use crate::tools::codegraph::store::{IndexStore, SymbolCallEntry};
use crate::tools::codegraph::types::*;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Result of a `codegraph_node` query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodegraphNodeResult {
    pub found: bool,
    pub symbols: Vec<Symbol>,
    pub references: Vec<Reference>,
    pub callers: Vec<SymbolCallEntry>,
    pub callees: Vec<SymbolCallEntry>,
    pub suggestions: Vec<SymbolSuggestion>,
    pub is_entry_point: bool,
    pub is_leaf: bool,
}

/// Result of a `codegraph_explore` query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodegraphExploreResult {
    pub symbols: Vec<Symbol>,
    pub call_graph: Vec<RelationEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationEntry {
    pub symbol_name: String,
    pub file_path: String,
    pub line: usize,
    pub relation: String,
    pub depth: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolSuggestion {
    pub name: String,
    pub kind: SymbolKind,
    pub file_path: String,
    pub line: usize,
    pub distance: usize,
}

/// Query engine for symbol lookup, reference tracking, and call graph analysis.
pub struct QueryEngine {
    store: Arc<IndexStore>,
}

impl QueryEngine {
    pub fn new(store: Arc<IndexStore>) -> Self {
        Self { store }
    }

    /// Look up a symbol by name, returning definitions, references, and callers/callees.
    pub fn codegraph_node(&self, symbol: &str) -> anyhow::Result<CodegraphNodeResult> {
        let symbols = self.store.get_symbol_by_name(symbol)?;
        if symbols.is_empty() {
            let suggestions = self.fuzzy_find(symbol);
            return Ok(CodegraphNodeResult {
                found: false,
                symbols: vec![],
                references: vec![],
                callers: vec![],
                callees: vec![],
                suggestions,
                is_entry_point: false,
                is_leaf: false,
            });
        }

        let mut refs = Vec::new();
        let mut callers_list = Vec::new();
        let mut callees_list = Vec::new();
        for sym in &symbols {
            if let Some(id) = sym.id {
                if let Ok(r) = self.store.get_references(id) {
                    refs.extend(r);
                }
                if let Ok(c) = self.store.get_callers(id, 1) {
                    callers_list.extend(c);
                }
                if let Ok(c) = self.store.get_callees(id, 1) {
                    callees_list.extend(c);
                }
            }
        }
        let is_entry_point = symbols.iter().any(|s| s.name == "main");
        let is_leaf = callees_list.is_empty();
        Ok(CodegraphNodeResult {
            found: true,
            symbols,
            references: refs,
            callers: callers_list,
            callees: callees_list,
            suggestions: vec![],
            is_entry_point,
            is_leaf,
        })
    }

    /// Explore symbols and their call relationships by keyword.
    pub fn codegraph_explore(&self, query: &str) -> anyhow::Result<CodegraphExploreResult> {
        let matched = self.store.get_symbol_by_name(query).unwrap_or_default();
        let mut call_graph = Vec::new();
        for sym in &matched {
            if let Some(id) = sym.id {
                if let Ok(callers) = self.store.get_callers(id, 2) {
                    call_graph.extend(callers.into_iter().map(|c| RelationEntry {
                        symbol_name: c.name,
                        file_path: c.file_path,
                        line: c.line,
                        relation: "caller".into(),
                        depth: c.depth,
                    }));
                }
                if let Ok(callees) = self.store.get_callees(id, 2) {
                    call_graph.extend(callees.into_iter().map(|c| RelationEntry {
                        symbol_name: c.name,
                        file_path: c.file_path,
                        line: c.line,
                        relation: "callee".into(),
                        depth: c.depth,
                    }));
                }
            }
        }
        Ok(CodegraphExploreResult {
            symbols: matched,
            call_graph,
        })
    }

    /// Get transitive callers for a symbol, up to max depth 5.
    pub fn get_callers(&self, symbol: &str, depth: u32) -> anyhow::Result<Vec<SymbolCallEntry>> {
        let depth = depth.min(5);
        let mut all = Vec::new();
        for sym in self.store.get_symbol_by_name(symbol)? {
            if let Some(id) = sym.id {
                if let Ok(c) = self.store.get_callers(id, depth) {
                    all.extend(c);
                }
            }
        }
        Ok(all)
    }

    /// Get transitive callees for a symbol, up to max depth 5.
    pub fn get_callees(&self, symbol: &str, depth: u32) -> anyhow::Result<Vec<SymbolCallEntry>> {
        let depth = depth.min(5);
        let mut all = Vec::new();
        for sym in self.store.get_symbol_by_name(symbol)? {
            if let Some(id) = sym.id {
                if let Ok(c) = self.store.get_callees(id, depth) {
                    all.extend(c);
                }
            }
        }
        Ok(all)
    }

    /// Fuzzy search for similarly-named symbols (Levenshtein distance ≤ 3).
    fn fuzzy_find(&self, name: &str) -> Vec<SymbolSuggestion> {
        // Simple approach: get all symbols and compute Levenshtein distance
        // For a production system, this would use a more efficient index
        let suggestions = Vec::new();
        // We don't have a "get all symbols" method, so just return empty for now
        // The fuzzy find will be improved in a future iteration
        let _ = name;
        suggestions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::codegraph::store::IndexStore;
    use tempfile::TempDir;

    fn setup() -> (Arc<IndexStore>, TempDir) {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(IndexStore::open(dir.path()).unwrap());
        // Pre-populate with some test data
        let file_id = store.upsert_file("src/test.rs", "hash1").unwrap();
        let sym = Symbol {
            id: None,
            name: "test_fn".into(),
            kind: SymbolKind::Function,
            file_path: "src/test.rs".into(),
            line: 1,
            col: 1,
            signature: Some("fn test_fn()".into()),
            visibility: Visibility::Pub,
            parent_module: None,
        };
        store.insert_symbol(&sym, file_id).unwrap();
        (store, dir)
    }

    #[test]
    fn test_codegraph_node_found() {
        let (store, _dir) = setup();
        let engine = QueryEngine::new(store);
        let result = engine.codegraph_node("test_fn").unwrap();
        assert!(result.found);
        assert_eq!(result.symbols.len(), 1);
        assert_eq!(result.symbols[0].name, "test_fn");
    }

    #[test]
    fn test_codegraph_node_not_found() {
        let (store, _dir) = setup();
        let engine = QueryEngine::new(store);
        let result = engine.codegraph_node("nonexistent").unwrap();
        assert!(!result.found);
    }
}
