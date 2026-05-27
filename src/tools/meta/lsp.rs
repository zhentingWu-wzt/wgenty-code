//! LSP-like Symbol Tools — Go-to-definition, find-references via language-aware search
//!
//! Rather than requiring a running LSP server, this uses language-specific regex
//! patterns to locate symbol definitions and references. This gives ~80% of LSP
//! value for 10% of the complexity.

use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;
use regex::Regex;
use std::collections::HashMap;
use std::path::PathBuf;

/// Language-specific symbol patterns for definition matching
fn def_patterns() -> Vec<(&'static str, &'static str, &'static str)> {
    // (extension, symbol_pattern, reference_pattern)
    vec![
        // Rust
        ("rs", r"\b(fn|struct|enum|trait|impl|mod|type|const|static)\s+{symbol}\b", r"\b{symbol}\b"),
        // Go
        ("go", r"\b(func|type|var|const)\s+{symbol}\b", r"\b{symbol}\b"),
        // Python
        ("py", r"\b(def|class)\s+{symbol}\b", r"\b{symbol}\b"),
        // JavaScript / TypeScript
        ("js", r"\b(function|class|const|let|var)\s+{symbol}\b", r"\b{symbol}\b"),
        ("ts", r"\b(function|class|const|let|var|interface|type|enum)\s+{symbol}\b", r"\b{symbol}\b"),
        ("tsx", r"\b(function|class|const|let|var|interface|type|enum)\s+{symbol}\b", r"\b{symbol}\b"),
        ("jsx", r"\b(function|class|const|let|var)\s+{symbol}\b", r"\b{symbol}\b"),
        // Java
        ("java", r"\b(class|interface|enum|record)\s+{symbol}\b", r"\b{symbol}\b"),
        // C / C++
        ("c", r"\b(\w+\s+)?{symbol}\s*\([^)]*\)\s*\{", r"\b{symbol}\b"),
        ("h", r"\b(\w+\s+)?{symbol}\s*\([^)]*\)\s*;", r"\b{symbol}\b"),
        ("cpp", r"\b(\w+\s+)?{symbol}\s*\([^)]*\)\s*\{", r"\b{symbol}\b"),
        ("hpp", r"\b(\w+\s+)?{symbol}\s*\([^)]*\)\s*;", r"\b{symbol}\b"),
        // Shell
        ("sh", r"\b(function\s+)?{symbol}\s*\(\s*\)", r"\b{symbol}\b"),
        // Ruby
        ("rb", r"\b(def|class|module)\s+{symbol}\b", r"\b{symbol}\b"),
        // Swift
        ("swift", r"\b(func|class|struct|enum|protocol|let|var)\s+{symbol}\b", r"\b{symbol}\b"),
        // Kotlin
        ("kt", r"\b(fun|class|object|interface|val|var)\s+{symbol}\b", r"\b{symbol}\b"),
        // Markdown (for completeness)
        ("md", r"^#+\s.*{symbol}", r"{symbol}"),
        // TOML / config
        ("toml", r"^{symbol}\s*=", r"{symbol}"),
        ("yaml", r"^{symbol}\s*:", r"{symbol}"),
        ("yml", r"^{symbol}\s*:", r"{symbol}"),
    ]
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct SymbolLocation {
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub context: String,
}

pub struct LspTool;

impl LspTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for LspTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for LspTool {
    fn name(&self) -> &str {
        "lsp"
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn description(&self) -> &str {
        "Find symbol definitions and references using language-aware search. Supports two operations: 'definition' (find where a symbol is defined) and 'references' (find all usages of a symbol)."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "description": "Operation: 'definition' or 'references'",
                    "enum": ["definition", "references"]
                },
                "symbol": {
                    "type": "string",
                    "description": "Symbol name to find (e.g. function name, type name)"
                },
                "path": {
                    "type": "string",
                    "description": "File or directory to search in (defaults to current directory)"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of results (default: 20)",
                    "default": 20
                }
            },
            "required": ["operation", "symbol"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let operation = input["operation"].as_str().ok_or_else(|| ToolError {
            message: "operation is required (definition or references)".to_string(),
            code: Some("missing_parameter".to_string()),
        })?;
        let symbol = input["symbol"].as_str().ok_or_else(|| ToolError {
            message: "symbol is required".to_string(),
            code: Some("missing_parameter".to_string()),
        })?;
        let base_path = input["path"].as_str().unwrap_or(".");
        let max_results = input["max_results"].as_u64().unwrap_or(20) as usize;

        let base = PathBuf::from(base_path);

        match operation {
            "definition" => find_definitions(&base, symbol, max_results),
            "references" => find_references(&base, symbol, max_results),
            _ => Err(ToolError {
                message: format!("Unknown operation: {}. Use 'definition' or 'references'", operation),
                code: Some("invalid_operation".to_string()),
            }),
        }
    }
}

fn find_definitions(base: &std::path::Path, symbol: &str, max_results: usize) -> Result<ToolOutput, ToolError> {
    let mut results: Vec<SymbolLocation> = Vec::new();

    for (ext, def_pat, _) in def_patterns() {
        let pattern_str = def_pat.replace("{symbol}", &regex::escape(symbol));
        let re = match Regex::new(&pattern_str) {
            Ok(r) => r,
            Err(_) => continue,
        };

        let entries: Vec<_> = walkdir::WalkDir::new(base)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .and_then(|x| x.to_str())
                    .map(|x| x == ext)
                    .unwrap_or(false)
            })
            .collect();

        for entry in entries {
            if results.len() >= max_results {
                break;
            }
            if let Ok(content) = std::fs::read_to_string(entry.path()) {
                for (line_num, line) in content.lines().enumerate() {
                    if let Some(mat) = re.find(line) {
                        results.push(SymbolLocation {
                            file: entry.path().display().to_string(),
                            line: line_num + 1,
                            column: mat.start() + 1,
                            context: line.trim().to_string(),
                        });
                        if results.len() >= max_results {
                            break;
                        }
                    }
                }
            }
        }
    }

    format_results("definition", symbol, results, max_results)
}

fn find_references(base: &std::path::Path, symbol: &str, max_results: usize) -> Result<ToolOutput, ToolError> {
    let escaped = regex::escape(symbol);
    let pattern_str = format!(r"\b{}\b", escaped);
    let word_re = Regex::new(&pattern_str).map_err(|e| ToolError {
        message: format!("Regex error: {}", e),
        code: Some("regex_error".to_string()),
    })?;

    // Exclude definition lines to avoid mixing defs and refs
    let mut def_locations: Vec<(String, usize)> = Vec::new();
    for (ext, def_pat, _) in def_patterns() {
        let dp = def_pat.replace("{symbol}", &escaped);
        if let Ok(re) = Regex::new(&dp) {
            let entries: Vec<_> = walkdir::WalkDir::new(base)
                .follow_links(false)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.path()
                        .extension()
                        .and_then(|x| x.to_str())
                        .map(|x| x == ext)
                        .unwrap_or(false)
                })
                .collect();

            for entry in entries {
                if let Ok(content) = std::fs::read_to_string(entry.path()) {
                    for (line_num, line) in content.lines().enumerate() {
                        if re.is_match(line) {
                            def_locations.push((entry.path().display().to_string(), line_num + 1));
                        }
                    }
                }
            }
        }
    }

    let def_set: HashMap<String, std::collections::HashSet<usize>> = {
        let mut map = HashMap::new();
        for (file, line) in &def_locations {
            map.entry(file.clone()).or_insert_with(std::collections::HashSet::new).insert(*line);
        }
        map
    };

    let mut results: Vec<SymbolLocation> = Vec::new();

    let all_entries: Vec<_> = walkdir::WalkDir::new(base)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .collect();

    for entry in all_entries {
        if results.len() >= max_results {
            break;
        }
        let file_path = entry.path().display().to_string();
        if let Ok(content) = std::fs::read_to_string(entry.path()) {
            for (line_num, line) in content.lines().enumerate() {
                if let Some(mat) = word_re.find(line) {
                    // Skip definition lines
                    let is_def = def_set.get(&file_path).map(|lines| lines.contains(&(line_num + 1))).unwrap_or(false);
                    if is_def {
                        continue;
                    }

                    results.push(SymbolLocation {
                        file: file_path.clone(),
                        line: line_num + 1,
                        column: mat.start() + 1,
                        context: line.trim().to_string(),
                    });
                    if results.len() >= max_results {
                        break;
                    }
                }
            }
        }
    }

    format_results("references", symbol, results, max_results)
}

fn format_results(op: &str, symbol: &str, results: Vec<SymbolLocation>, max: usize) -> Result<ToolOutput, ToolError> {
    let label = if op == "definition" { "definitions" } else { "references" };
    let mut lines = vec![format!("Found {} {} for `{}`:\n", results.len().min(max), label, symbol)];

    for loc in &results {
        lines.push(format!(
            "  {}:{}:{}  {}",
            loc.file, loc.line, loc.column, loc.context
        ));
    }

    let mut metadata = HashMap::new();
    metadata.insert("count".to_string(), serde_json::json!(results.len()));
    metadata.insert("operation".to_string(), serde_json::json!(op));
    metadata.insert("symbol".to_string(), serde_json::json!(symbol));

    Ok(ToolOutput {
        output_type: "text".to_string(),
        content: lines.join("\n"),
        metadata,
    })
}
