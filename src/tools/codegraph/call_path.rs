use crate::tools::codegraph::store::IndexStore;
use crate::tools::codegraph::types::{RelKind, Relationship, Symbol};
use serde::{Deserialize, Serialize};
use std::collections::{BinaryHeap, HashMap, VecDeque};
use std::sync::Arc;

/// A directed edge in the call graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub from_id: i64,
    pub to_id: i64,
    pub from_name: String,
    pub to_name: String,
    pub rel_kind: String,
    pub location: String, // "file:line"
}

/// One hop in a call path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hop {
    pub from: String,
    pub to: String,
    pub rel: String,
    pub location: String,
}

/// A call path from root to a target symbol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallPath {
    pub depth: usize,
    pub hops: Vec<Hop>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
}

/// In-memory call graph for traversals.
#[derive(Default)]
pub struct CallGraph {
    /// source_id → Vec of outgoing edges
    pub edges: HashMap<i64, Vec<Edge>>,
    /// symbol_id → Symbol metadata for lookup
    pub symbols: HashMap<i64, Symbol>,
}

impl CallGraph {
    /// Build a call graph from the IndexStore, loading all relationships.
    pub fn build(store: &Arc<IndexStore>) -> anyhow::Result<Self> {
        let relationships = store.list_relationships()?;
        let symbols_list = store.list_symbols()?;

        let symbols: HashMap<i64, Symbol> = symbols_list
            .into_iter()
            .map(|s| (s.id.unwrap_or(0), s))
            .collect();

        let mut edges: HashMap<i64, Vec<Edge>> = HashMap::new();
        for rel in relationships {
            let from_name = symbols
                .get(&rel.source_id)
                .map(|s| s.name.clone())
                .unwrap_or_default();
            let to_name = symbols
                .get(&rel.target_id)
                .map(|s| s.name.clone())
                .unwrap_or_default();
            let edge = Edge {
                from_id: rel.source_id,
                to_id: rel.target_id,
                from_name: from_name.clone(),
                to_name: to_name.clone(),
                rel_kind: rel.rel_kind.as_str().to_string(),
                location: format!("{}:{}", rel.file_path, rel.line),
            };
            edges.entry(rel.source_id).or_default().push(edge);
        }

        Ok(Self { edges, symbols })
    }

    /// Find the symbol id for a given name (case-insensitive).
    pub fn find_symbol_id(&self, name: &str) -> Option<i64> {
        self.symbols
            .iter()
            .find(|(_, s)| s.name.eq_ignore_ascii_case(name))
            .map(|(id, _)| *id)
    }

    /// BFS from root up to max_depth, returning all call paths.
    pub fn bfs_paths(&self, root_id: i64, max_depth: usize) -> Vec<CallPath> {
        let mut result = Vec::new();
        let mut queue = VecDeque::new();

        // Initial paths: just the root symbol
        let root_name = self
            .symbols
            .get(&root_id)
            .map(|s| s.name.clone())
            .unwrap_or_default();
        let initial_path = CallPath {
            depth: 0,
            hops: Vec::new(),
            truncated: None,
        };
        queue.push_back((root_id, initial_path));

        while let Some((current_id, path)) = queue.pop_front() {
            if path.depth >= max_depth {
                let mut truncated_path = path;
                truncated_path.truncated = Some(true);
                result.push(truncated_path);
                continue;
            }

            if let Some(out_edges) = self.edges.get(&current_id) {
                for edge in out_edges {
                    let hop = Hop {
                        from: edge.from_name.clone(),
                        to: edge.to_name.clone(),
                        rel: edge.rel_kind.clone(),
                        location: edge.location.clone(),
                    };
                    let mut new_path = CallPath {
                        depth: path.depth + 1,
                        hops: {
                            let mut h = path.hops.clone();
                            h.push(hop);
                            h
                        },
                        truncated: None,
                    };
                    result.push(new_path.clone());
                    queue.push_back((edge.to_id, new_path));
                }
            }
        }

        result
    }

    /// Dijkstra shortest path from from_id to to_id.
    pub fn shortest_path(&self, from_id: i64, to_id: i64) -> Option<CallPath> {
        if from_id == to_id {
            let name = self
                .symbols
                .get(&from_id)
                .map(|s| s.name.clone())
                .unwrap_or_default();
            return Some(CallPath {
                depth: 0,
                hops: vec![Hop {
                    from: name.clone(),
                    to: name,
                    rel: "self".to_string(),
                    location: String::new(),
                }],
                truncated: None,
            });
        }

        // (cost, symbol_id, path_so_far)
        #[derive(Clone)]
        struct State {
            cost: usize,
            id: i64,
            path: Vec<Hop>,
        }
        impl PartialEq for State {
            fn eq(&self, other: &Self) -> bool {
                self.cost == other.cost
            }
        }
        impl Eq for State {}
        impl PartialOrd for State {
            fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
                Some(other.cost.cmp(&self.cost)) // reverse for min-heap
            }
        }
        impl Ord for State {
            fn cmp(&self, other: &Self) -> std::cmp::Ordering {
                other.cost.cmp(&self.cost)
            }
        }

        let mut heap = BinaryHeap::new();
        let root_name = self
            .symbols
            .get(&from_id)
            .map(|s| s.name.clone())
            .unwrap_or_default();
        heap.push(State {
            cost: 0,
            id: from_id,
            path: vec![Hop {
                from: root_name.clone(),
                to: root_name,
                rel: "self".to_string(),
                location: String::new(),
            }],
        });

        let mut visited: HashMap<i64, usize> = HashMap::new();

        while let Some(state) = heap.pop() {
            if state.id == to_id {
                let hops = if state.path.len() > 1 {
                    state.path[1..].to_vec()
                } else {
                    state.path.clone()
                };
                return Some(CallPath {
                    depth: hops.len(),
                    hops,
                    truncated: None,
                });
            }

            if let Some(best) = visited.get(&state.id) {
                if state.cost >= *best {
                    continue;
                }
            }
            visited.insert(state.id, state.cost);

            if let Some(out_edges) = self.edges.get(&state.id) {
                for edge in out_edges {
                    let new_cost = state.cost + 1;
                    if new_cost < *visited.get(&edge.to_id).unwrap_or(&usize::MAX) {
                        let mut new_path = state.path.clone();
                        new_path.push(Hop {
                            from: edge.from_name.clone(),
                            to: edge.to_name.clone(),
                            rel: edge.rel_kind.clone(),
                            location: edge.location.clone(),
                        });
                        heap.push(State {
                            cost: new_cost,
                            id: edge.to_id,
                            path: new_path,
                        });
                    }
                }
            }
        }

        None
    }
}

/// Result type for call_path tool.
#[derive(Debug, Serialize, Deserialize)]
pub struct CallPathResult {
    pub path_found: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<CallPath>,
}

impl CallPathResult {
    pub fn found(path: CallPath) -> Self {
        Self {
            path_found: true,
            reason: None,
            path: Some(path),
        }
    }

    pub fn not_found(reason: &str) -> Self {
        Self {
            path_found: false,
            reason: Some(reason.to_string()),
            path: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_graph() -> CallGraph {
        // Manually build a simple graph for testing
        let mut edges: HashMap<i64, Vec<Edge>> = HashMap::new();
        // main → init → run_async
        edges.insert(
            1,
            vec![Edge {
                from_id: 1,
                to_id: 2,
                from_name: "main".into(),
                to_name: "init".into(),
                rel_kind: "calls".into(),
                location: "src/main.rs:10".into(),
            }],
        );
        edges.insert(
            2,
            vec![Edge {
                from_id: 2,
                to_id: 3,
                from_name: "init".into(),
                to_name: "run_async".into(),
                rel_kind: "calls".into(),
                location: "src/agent.rs:20".into(),
            }],
        );

        let mut symbols = HashMap::new();
        use crate::tools::codegraph::types::Visibility;
        symbols.insert(
            1,
            Symbol {
                id: Some(1),
                name: "main".into(),
                kind: crate::tools::codegraph::types::SymbolKind::Function,
                file_path: String::new(),
                line: 0,
                col: 0,
                signature: None,
                visibility: Visibility::Private,
                parent_module: None,
                language: "rust".to_string(),
            },
        );
        symbols.insert(
            2,
            Symbol {
                id: Some(2),
                name: "init".into(),
                kind: crate::tools::codegraph::types::SymbolKind::Function,
                file_path: String::new(),
                line: 0,
                col: 0,
                signature: None,
                visibility: Visibility::Private,
                parent_module: None,
                language: "rust".to_string(),
            },
        );
        symbols.insert(
            3,
            Symbol {
                id: Some(3),
                name: "run_async".into(),
                kind: crate::tools::codegraph::types::SymbolKind::Function,
                file_path: String::new(),
                line: 0,
                col: 0,
                signature: None,
                visibility: Visibility::Private,
                parent_module: None,
                language: "rust".to_string(),
            },
        );
        symbols.insert(
            4,
            Symbol {
                id: Some(4),
                name: "orphan".into(),
                kind: crate::tools::codegraph::types::SymbolKind::Function,
                file_path: String::new(),
                line: 0,
                col: 0,
                signature: None,
                visibility: Visibility::Private,
                parent_module: None,
                language: "rust".to_string(),
            },
        );

        CallGraph { edges, symbols }
    }

    #[test]
    fn test_shortest_path_exists() {
        let graph = make_test_graph();
        let path = graph.shortest_path(1, 3);
        assert!(path.is_some());
        let p = path.unwrap();
        assert_eq!(p.depth, 2);
        assert_eq!(p.hops[0].to, "init");
        assert_eq!(p.hops[1].to, "run_async");
    }

    #[test]
    fn test_shortest_path_not_found() {
        let graph = make_test_graph();
        let path = graph.shortest_path(1, 4);
        assert!(path.is_none());
    }

    #[test]
    fn test_shortest_path_same_symbol() {
        let graph = make_test_graph();
        let path = graph.shortest_path(2, 2);
        assert!(path.is_some());
        assert_eq!(path.unwrap().depth, 0);
    }

    #[test]
    fn test_bfs_paths_depth_limit() {
        let graph = make_test_graph();
        let paths = graph.bfs_paths(1, 1);
        // main(0) → init(1) truncated
        assert!(paths.iter().any(|p| p.truncated == Some(true)));
    }
}
