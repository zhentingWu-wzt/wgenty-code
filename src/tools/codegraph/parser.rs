use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tree_sitter::{Node, Parser};

/// Wraps a tree-sitter parser configured for a specific language.
pub struct CodeParser {
    parser: Parser,
    language_name: String,
}

impl CodeParser {
    pub fn new(language: tree_sitter::Language, language_name: &str) -> Self {
        let mut parser = Parser::new();
        parser
            .set_language(&language)
            .unwrap_or_else(|_| panic!("Failed to set tree-sitter language: {}", language_name));
        Self {
            parser,
            language_name: language_name.to_string(),
        }
    }

    pub fn parse(&mut self, source: &str) -> anyhow::Result<tree_sitter::Tree> {
        let tree = self
            .parser
            .parse(source, None)
            .ok_or_else(|| anyhow::anyhow!("Failed to parse source"))?;
        Ok(tree)
    }

    pub fn root_node<'a>(&self, tree: &'a tree_sitter::Tree) -> Node<'a> {
        tree.root_node()
    }

    pub fn language_name(&self) -> &str {
        &self.language_name
    }
}

/// A pool of language-specific parsers, keyed by language name.
/// Parsers are created lazily and cached for reuse.
pub struct ParserPool {
    parsers: HashMap<String, Arc<Mutex<CodeParser>>>,
}

impl ParserPool {
    pub fn new() -> Self {
        Self {
            parsers: HashMap::new(),
        }
    }

    /// Get or create a parser for the given language.
    pub fn get_or_create(&mut self, language: tree_sitter::Language, name: &str) -> Arc<Mutex<CodeParser>> {
        self.parsers
            .entry(name.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(CodeParser::new(language, name))))
            .clone()
    }

    /// Map a file extension to a language name.
    pub fn language_for_extension(ext: &str) -> Option<&'static str> {
        match ext {
            "rs" => Some("rust"),
            "java" => Some("java"),
            "py" => Some("python"),
            _ => None,
        }
    }

    /// Get a parser by file extension (creates if needed).
    pub fn get_for_extension(
        &mut self,
        ext: &str,
    ) -> Option<Arc<Mutex<CodeParser>>> {
        match ext {
            "rs" => Some(self.get_or_create(tree_sitter_rust::LANGUAGE.into(), "rust")),
            "java" => Some(self.get_or_create(tree_sitter_java::LANGUAGE.into(), "java")),
            "py" => Some(self.get_or_create(tree_sitter_python::LANGUAGE.into(), "python")),
            _ => None,
        }
    }
}

impl Default for ParserPool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_code_parser_with_language() {
        let mut parser = CodeParser::new(tree_sitter_rust::LANGUAGE.into(), "rust");
        let tree = parser.parse("fn main() {}").unwrap();
        assert!(!tree.root_node().has_error());
        assert_eq!(parser.language_name(), "rust");
    }

    #[test]
    fn test_parser_pool_caches_parsers() {
        let mut pool = ParserPool::new();
        let p1 = pool.get_or_create(tree_sitter_rust::LANGUAGE.into(), "rust");
        let p2 = pool.get_or_create(tree_sitter_rust::LANGUAGE.into(), "rust");
        // Same key should return the cached parser
        assert!(Arc::ptr_eq(&p1, &p2));
    }

    #[test]
    fn test_parser_pool_different_languages() {
        let mut pool = ParserPool::new();
        let rust_parser = pool.get_or_create(tree_sitter_rust::LANGUAGE.into(), "rust");
        let java_parser = pool.get_or_create(tree_sitter_java::LANGUAGE.into(), "java");
        // Different languages should get different parsers
        assert!(!Arc::ptr_eq(&rust_parser, &java_parser));
        assert_eq!(rust_parser.lock().unwrap().language_name(), "rust");
        assert_eq!(java_parser.lock().unwrap().language_name(), "java");
    }

    #[test]
    fn test_language_for_extension() {
        assert_eq!(ParserPool::language_for_extension("rs"), Some("rust"));
        assert_eq!(ParserPool::language_for_extension("java"), Some("java"));
        assert_eq!(ParserPool::language_for_extension("py"), Some("python"));
        assert_eq!(ParserPool::language_for_extension("js"), None);
        assert_eq!(ParserPool::language_for_extension("unknown"), None);
    }

    #[test]
    fn test_get_for_extension() {
        let mut pool = ParserPool::new();
        assert!(pool.get_for_extension("rs").is_some());
        assert!(pool.get_for_extension("java").is_some());
        assert!(pool.get_for_extension("py").is_some());
        assert!(pool.get_for_extension("js").is_none());
    }

    #[test]
    fn test_parse_rust_with_pool() {
        let mut pool = ParserPool::new();
        let parser = pool.get_for_extension("rs").unwrap();
        let mut p = parser.lock().unwrap();
        let tree = p.parse("fn hello() {}").unwrap();
        assert_eq!(tree.root_node().kind(), "source_file");
    }

    #[test]
    fn test_parse_java_with_pool() {
        let mut pool = ParserPool::new();
        let parser = pool.get_for_extension("java").unwrap();
        let mut p = parser.lock().unwrap();
        let tree = p.parse("class Hello {}").unwrap();
        assert!(!tree.root_node().has_error());
    }

    #[test]
    fn test_parse_python_with_pool() {
        let mut pool = ParserPool::new();
        let parser = pool.get_for_extension("py").unwrap();
        let mut p = parser.lock().unwrap();
        let tree = p.parse("def hello():\n    pass\n").unwrap();
        assert!(!tree.root_node().has_error());
    }
}
