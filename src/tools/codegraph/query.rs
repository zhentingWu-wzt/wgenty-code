use crate::tools::codegraph::store::{IndexStore, SymbolCallEntry};
use crate::tools::codegraph::types::*;
use crate::tools::codegraph::{audit, call_path, fuzzy};
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audit_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<String>,
}

/// Result of a `codegraph_explore` query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodegraphExploreResult {
    pub symbols: Vec<Symbol>,
    pub call_graph: Vec<RelationEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_paths: Option<Vec<call_path::CallPath>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audit_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<String>,
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
}

pub struct QueryEngine {
    store: Arc<IndexStore>,
    logger: Option<Arc<audit::AuditLogger>>,
}

impl QueryEngine {
    pub fn new(store: Arc<IndexStore>) -> Self {
        Self {
            store,
            logger: None,
        }
    }

    pub fn with_logger(mut self, logger: Arc<audit::AuditLogger>) -> Self {
        self.logger = Some(logger);
        self
    }

    fn log(
        &self,
        query_type: &str,
        params: &serde_json::Value,
        result_count: usize,
        elapsed_ms: u64,
    ) {
        if let Some(logger) = &self.logger {
            let entry = audit::AuditEntry {
                ts: chrono::Utc::now().to_rfc3339(),
                audit_id: audit::AuditLogger::generate_audit_id(),
                query_type: query_type.to_string(),
                params: params.clone(),
                result_count,
                elapsed_ms,
                source_files: vec![],
            };
            let _ = logger.log_query(&entry);
        }
    }

    /// Look up a symbol by name, returning definitions, references, and callers/callees.
    pub fn codegraph_node(&self, symbol: &str) -> anyhow::Result<CodegraphNodeResult> {
        let audit_id = Some(audit::AuditLogger::generate_audit_id());
        let symbols = self.store.get_symbol_by_name(symbol)?;
        if symbols.is_empty() {
            let suggestions = self.fuzzy_find(symbol);
            self.log(
                "codegraph_node",
                &serde_json::json!({"symbol": symbol, "fuzzy": true}),
                suggestions.len(),
                0,
            );
            return Ok(CodegraphNodeResult {
                found: false,
                symbols: vec![],
                references: vec![],
                callers: vec![],
                callees: vec![],
                suggestions,
                is_entry_point: false,
                is_leaf: false,
                audit_id,
                confidence: Some("low".to_string()),
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

        self.log(
            "codegraph_node",
            &serde_json::json!({"symbol": symbol}),
            symbols.len(),
            0,
        );

        Ok(CodegraphNodeResult {
            found: true,
            symbols,
            references: refs,
            callers: callers_list,
            callees: callees_list,
            suggestions: vec![],
            is_entry_point,
            is_leaf,
            audit_id,
            confidence: Some("high".to_string()),
        })
    }

    /// Explore symbols and their call relationships by keyword.
    pub fn codegraph_explore(&self, query: &str) -> anyhow::Result<CodegraphExploreResult> {
        let audit_id = Some(audit::AuditLogger::generate_audit_id());
        let matched = self.store.get_symbol_by_name(query).unwrap_or_default();
        let mut call_graph = Vec::new();

        // Build call graph from matched symbols
        let graph =
            call_path::CallGraph::build(&self.store).unwrap_or_else(|_| call_path::CallGraph {
                edges: Default::default(),
                symbols: Default::default(),
            });

        // Get call paths for the first matched symbol
        let call_paths = matched
            .first()
            .and_then(|s| s.id)
            .map(|id| graph.bfs_paths(id, 5));

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
        self.log(
            "codegraph_explore",
            &serde_json::json!({"query": query}),
            matched.len(),
            0,
        );
        Ok(CodegraphExploreResult {
            symbols: matched,
            call_graph,
            call_paths,
            audit_id,
            confidence: Some("medium".to_string()),
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

    /// Shortest call path between two symbols.
    pub fn call_path_query(&self, from: &str, to: &str) -> anyhow::Result<serde_json::Value> {
        let graph = call_path::CallGraph::build(&self.store)?;
        let from_id = graph.find_symbol_id(from);
        let to_id = graph.find_symbol_id(to);
        match (from_id, to_id) {
            (Some(f), Some(t)) => {
                let path = graph.shortest_path(f, t);
                let result = match path {
                    Some(p) => call_path::CallPathResult::found(p),
                    None => call_path::CallPathResult::not_found("no_connecting_path"),
                };
                Ok(serde_json::to_value(result)?)
            }
            _ => {
                let missing = if from_id.is_none() { from } else { to };
                Ok(serde_json::to_value(call_path::CallPathResult::not_found(
                    &format!("symbol '{}' not found in index", missing),
                ))?)
            }
        }
    }

    /// Batch symbol lookup (max 10).
    pub fn symbol_batch(&self, symbols: &[String]) -> anyhow::Result<Vec<CodegraphNodeResult>> {
        let mut results = Vec::new();
        for s in symbols.iter().take(10) {
            results.push(self.codegraph_node(s)?);
        }
        self.log(
            "symbol_batch",
            &serde_json::json!({"symbols": symbols}),
            results.len(),
            0,
        );
        Ok(results)
    }

    /// Module summary for a given directory path.
    pub fn module_summary(&self, module_path: &str) -> anyhow::Result<serde_json::Value> {
        let all_syms = self.store.list_symbols()?;
        let filtered: Vec<&Symbol> = all_syms
            .iter()
            .filter(|s| s.file_path.starts_with(module_path))
            .collect();

        if filtered.is_empty() {
            return Ok(serde_json::json!({
                "found": false,
                "reason": format!("no indexed files under '{}'", module_path),
                "module_path": module_path,
                "symbol_count": 0,
            }));
        }

        let pub_syms: Vec<&Symbol> = filtered
            .iter()
            .filter(|s| matches!(s.visibility, Visibility::Pub))
            .copied()
            .collect();

        let mut by_kind: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for s in &filtered {
            *by_kind.entry(s.kind.as_str().to_string()).or_default() += 1;
        }

        self.log(
            "module_summary",
            &serde_json::json!({"module_path": module_path}),
            filtered.len(),
            0,
        );

        Ok(serde_json::json!({
            "found": true,
            "module_path": module_path,
            "symbol_count": filtered.len(),
            "public_count": pub_syms.len(),
            "by_kind": by_kind,
        }))
    }

    /// Fuzzy search for similarly-named symbols (Levenshtein distance ≤ 3).
    fn fuzzy_find(&self, name: &str) -> Vec<SymbolSuggestion> {
        let Ok(all_syms) = self.store.list_symbols() else {
            return vec![];
        };
        let candidates: Vec<String> = all_syms.iter().map(|s| s.name.clone()).collect();
        let matches = fuzzy::fuzzy_search(name, &candidates, 3, 5);
        matches
            .into_iter()
            .filter_map(|m| {
                all_syms
                    .iter()
                    .find(|s| s.name == m.name)
                    .map(|sym| SymbolSuggestion {
                        name: sym.name.clone(),
                        kind: sym.kind.clone(),
                        file_path: sym.file_path.clone(),
                        line: sym.line,
                    })
            })
            .collect()
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
            language: "rust".to_string(),
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
        assert!(result.audit_id.is_some());
    }

    #[test]
    fn test_codegraph_node_not_found() {
        let (store, _dir) = setup();
        let engine = QueryEngine::new(store);
        let result = engine.codegraph_node("nonexistent").unwrap();
        assert!(!result.found);
    }
}
