use tree_sitter::{Node, Parser};

/// Wraps a tree-sitter parser configured for Rust.
pub struct CodeParser {
    parser: Parser,
}

impl CodeParser {
    pub fn new() -> Self {
        let mut parser = Parser::new();
        let lang: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
        parser
            .set_language(&lang)
            .expect("Failed to set tree-sitter Rust language");
        Self { parser }
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
}

impl Default for CodeParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_function_item() {
        let mut parser = CodeParser::new();
        let source = "pub fn hello(x: i32) -> bool { true }";
        let tree = parser.parse(source).unwrap();
        assert_eq!(tree.root_node().kind(), "source_file");
        assert_eq!(tree.root_node().child(0).unwrap().kind(), "function_item");
    }

    #[test]
    fn test_parse_struct_and_enum() {
        let mut parser = CodeParser::new();
        let source = "pub struct Point { x: i32, y: i32 }\nenum Color { Red, Green, Blue }";
        let tree = parser.parse(source).unwrap();
        assert_eq!(tree.root_node().child(0).unwrap().kind(), "struct_item");
        assert_eq!(tree.root_node().child(1).unwrap().kind(), "enum_item");
    }

    #[test]
    fn test_parse_with_syntax_error() {
        let mut parser = CodeParser::new();
        let source = "pub fn broken(x: i32 { ";
        let tree = parser.parse(source).unwrap();
        assert!(tree.root_node().has_error());
    }

    #[test]
    fn test_parse_empty_input() {
        let mut parser = CodeParser::new();
        let tree = parser.parse("").unwrap();
        assert_eq!(tree.root_node().kind(), "source_file");
        assert_eq!(tree.root_node().child_count(), 0);
    }
}
