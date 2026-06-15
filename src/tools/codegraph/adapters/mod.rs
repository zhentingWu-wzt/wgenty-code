//! Language adapter trait and implementations for multi-language code indexing.
//!
//! Each adapter encapsulates language-specific parsing and extraction logic,
//! allowing the indexer to handle Rust, Java, Python (and future languages)
//! through a uniform interface.

pub mod java;
pub mod python;
pub mod rust;

use crate::tools::codegraph::types::{Reference, Relationship, Symbol};
use tree_sitter::{Language, Parser, Tree};

/// A language adapter knows how to parse source code and extract
/// symbols, references, and relationships for a specific language.
pub trait LanguageAdapter: Send + Sync {
    /// The language name (e.g., "rust", "java", "python").
    fn language(&self) -> &'static str;

    /// File extensions this adapter handles (e.g., &["rs"], &["java"], &["py"]).
    fn file_extensions(&self) -> &[&str];

    /// Return the tree-sitter `Language` for parser configuration.
    fn tree_sitter_language(&self) -> Language;

    /// Create a fresh parser configured for this language.
    fn create_parser(&self) -> Parser {
        let mut parser = Parser::new();
        parser
            .set_language(&self.tree_sitter_language())
            .expect("Failed to set tree-sitter language");
        parser
    }

    /// Parse source text into a concrete syntax tree.
    fn parse(&self, source: &str) -> anyhow::Result<Tree> {
        let mut parser = self.create_parser();
        parser
            .parse(source, None)
            .ok_or_else(|| anyhow::anyhow!("Failed to parse {} source", self.language()))
    }

    /// Extract symbols from the parsed AST.
    fn extract_symbols(&self, tree: &Tree, source: &str, file_path: &str) -> Vec<Symbol>;

    /// Extract references from the parsed AST.
    /// `symbols` must already be extracted (used for ID mapping).
    fn extract_references(
        &self,
        tree: &Tree,
        source: &str,
        symbols: &[Symbol],
    ) -> Vec<Reference>;

    /// Extract relationships from the parsed AST.
    /// `symbols` must already be extracted (used for source/target ID mapping).
    fn extract_relationships(
        &self,
        tree: &Tree,
        source: &str,
        symbols: &[Symbol],
    ) -> Vec<Relationship>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::codegraph::types::{RelKind, RefKind, Symbol, SymbolKind, Visibility};
    use tree_sitter::{Language, Tree};

    /// A minimal mock adapter to verify the trait is usable.
    struct MockAdapter;

    impl LanguageAdapter for MockAdapter {
        fn language(&self) -> &'static str {
            "mock"
        }

        fn file_extensions(&self) -> &[&str] {
            &["mock"]
        }

        fn tree_sitter_language(&self) -> Language {
            tree_sitter_rust::LANGUAGE.into()
        }

        fn extract_symbols(
            &self,
            _tree: &Tree,
            _source: &str,
            file_path: &str,
        ) -> Vec<Symbol> {
            vec![Symbol {
                id: Some(0),
                name: "mock_fn".into(),
                kind: SymbolKind::Function,
                file_path: file_path.to_string(),
                line: 1,
                col: 1,
                signature: Some("fn mock_fn()".into()),
                visibility: Visibility::Pub,
                parent_module: None,
                language: self.language().to_string(),
            }]
        }

        fn extract_references(
            &self,
            _tree: &Tree,
            _source: &str,
            _symbols: &[Symbol],
        ) -> Vec<Reference> {
            vec![Reference {
                id: None,
                symbol_id: 0,
                file_path: String::new(),
                line: 2,
                col: 5,
                ref_kind: RefKind::Call,
                context: Some("other_fn".into()),
            }]
        }

        fn extract_relationships(
            &self,
            _tree: &Tree,
            _source: &str,
            symbols: &[Symbol],
        ) -> Vec<Relationship> {
            if symbols.is_empty() {
                return vec![];
            }
            vec![Relationship {
                id: None,
                source_id: symbols[0].id.unwrap_or(0),
                target_id: -1,
                rel_kind: RelKind::Calls,
                file_path: String::new(),
                line: 2,
                confidence: crate::tools::codegraph::types::Confidence::Low,
            }]
        }
    }

    #[test]
    fn test_trait_language_name() {
        let adapter = MockAdapter;
        assert_eq!(adapter.language(), "mock");
    }

    #[test]
    fn test_trait_file_extensions() {
        let adapter = MockAdapter;
        assert_eq!(adapter.file_extensions(), &["mock"]);
    }

    #[test]
    fn test_trait_parse_simple_source() {
        let adapter = MockAdapter;
        let source = "fn hello() {}";
        let tree = adapter.parse(source).unwrap();
        assert!(tree.root_node().child_count() > 0, "parsed tree should have children");
    }

    #[test]
    fn test_trait_parse_error_on_invalid_syntax() {
        let adapter = MockAdapter;
        let source = "fn broken( {";
        let tree = adapter.parse(source).unwrap();
        assert!(tree.root_node().has_error(), "invalid syntax should produce error nodes");
    }

    #[test]
    fn test_trait_extract_symbols_returns_language_field() {
        let adapter = MockAdapter;
        let source = "fn hello() {}";
        let tree = adapter.parse(source).unwrap();
        let symbols = adapter.extract_symbols(&tree, source, "test.mock");
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "mock_fn");
        assert_eq!(symbols[0].language, "mock");
    }

    #[test]
    fn test_trait_extract_references() {
        let adapter = MockAdapter;
        let source = "fn hello() {}";
        let tree = adapter.parse(source).unwrap();
        let symbols = adapter.extract_symbols(&tree, source, "test.mock");
        let refs = adapter.extract_references(&tree, source, &symbols);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].ref_kind, RefKind::Call);
    }

    #[test]
    fn test_trait_extract_relationships() {
        let adapter = MockAdapter;
        let source = "fn hello() {}";
        let tree = adapter.parse(source).unwrap();
        let symbols = adapter.extract_symbols(&tree, source, "test.mock");
        let rels = adapter.extract_relationships(&tree, source, &symbols);
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].rel_kind, RelKind::Calls);
    }
}
