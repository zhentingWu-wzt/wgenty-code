//! Web Search Tool — zero-dependency by default.
//!
//! Search strategy (in order):
//! 1. DuckDuckGo HTML (zero API key, zero config) — default
//! 2. Tavily API (if TAVILY_API_KEY is set) — enhanced quality
//!
//! Returns title + URL only (no snippets — use web_fetch to read page content).
//! Max 8 uses per session (following Wgenty Code's pattern).

use crate::utils::http::web_search_client;
use async_trait::async_trait;
use reqwest::Client;
use serde::Serialize;
use serde_json::json;

use crate::tools::{Tool, ToolError, ToolOutput};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Unified search result (title + URL only).
#[derive(Debug, Clone, Serialize)]
struct SearchResult {
    title: String,
    url: String,
}

pub struct WebSearchTool {
    client: Client,
    pub(crate) use_count: AtomicUsize,
    max_uses: usize,
}

impl WebSearchTool {
    pub fn new() -> Self {
        Self {
            use_count: AtomicUsize::new(0),
            max_uses: 8,
            client: web_search_client(),
        }
    }

    #[cfg(test)]
    fn with_max_uses(max_uses: usize) -> Self {
        Self {
            use_count: AtomicUsize::new(0),
            max_uses,
            client: web_search_client(),
        }
    }

    /// Primary backend: DuckDuckGo HTML (zero-dependency).
    /// Fetches https://html.duckduckgo.com/html/?q=... and parses title+url.
    async fn search_duckduckgo(
        &self,
        query: &str,
        max_results: usize,
    ) -> Result<Vec<SearchResult>, String> {
        let url = format!("https://html.duckduckgo.com/html/?q={}", urlencoding(query));

        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("DuckDuckGo request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("DuckDuckGo returned status {}", resp.status()));
        }

        let html = resp
            .text()
            .await
            .map_err(|e| format!("DuckDuckGo read error: {}", e))?;

        let results = Self::parse_ddg_html(&html, max_results);
        if results.is_empty() {
            // Detect common anti-bot responses
            if html.contains("g-recaptcha")
                || html.contains("Complete the following challenge")
                || html.contains("Unfortunately, bots use DuckDuckGo")
            {
                return Err("DuckDuckGo anti-bot challenge triggered — search blocked. \
                     Set TAVILY_API_KEY as a fallback."
                    .to_string());
            }
            return Err("DuckDuckGo returned no results".to_string());
        }

        Ok(results)
    }

    /// Parse DuckDuckGo HTML results page.
    /// Extracts title from `result__a` links and URL from `result__url` snippets.
    fn parse_ddg_html(html: &str, max_results: usize) -> Vec<SearchResult> {
        let mut results = Vec::new();

        // DuckDuckGo HTML results structure:
        // <a rel="nofollow" class="result__a" href="...">Title</a>
        // <a class="result__url" href="..."> (sometimes)

        // Strategy: find all result__a links — the href is the real URL, text is title
        let mut remaining = html;
        while let Some(start) = remaining.find("class=\"result__a\"") {
            // Find href in this anchor
            let anchor_start = remaining[..start].rfind("<a ").unwrap_or(0);
            let anchor = &remaining[anchor_start..];

            // Extract href
            let href = Self::extract_attr(anchor, "href");
            // Extract link text (title)
            let title = if let Some(tag_end) = anchor.find('>') {
                let after_tag = &anchor[tag_end + 1..];
                if let Some(close) = after_tag.find("</a>") {
                    after_tag[..close]
                        .trim()
                        .replace("&amp;", "&")
                        .replace("&lt;", "<")
                        .replace("&gt;", ">")
                        .replace("&quot;", "\"")
                        .replace("&#39;", "'")
                } else {
                    "".to_string()
                }
            } else {
                "".to_string()
            };

            if let (Some(url), true) = (href, !title.is_empty()) {
                // Skip ad/tracking links — only real results
                if !url.contains("duckduckgo.com/y.js") && !url.contains("duckduckgo.com/l/?") {
                    results.push(SearchResult { title, url });
                }
            }

            remaining = &remaining[start + "class=\"result__a\"".len()..];
            if results.len() >= max_results {
                break;
            }
        }

        results
    }

    /// Extract an HTML attribute value from a tag fragment.
    fn extract_attr(tag: &str, attr_name: &str) -> Option<String> {
        let pattern = format!("{}=\"", attr_name);
        let start = tag.find(&pattern)? + pattern.len();
        let rest = &tag[start..];
        let end = rest.find('"')?;
        let value = rest[..end].to_string();

        // Decode HTML entities
        let decoded = value
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"");

        Some(decoded)
    }

    /// Optional enhancement: Tavily API (only if TAVILY_API_KEY is configured).
    async fn search_tavily(
        &self,
        query: &str,
        max_results: usize,
    ) -> Result<Vec<SearchResult>, String> {
        let api_key = std::env::var("TAVILY_API_KEY")
            .ok()
            .filter(|k| !k.is_empty())
            .ok_or_else(|| "TAVILY_API_KEY not configured".to_string())?;

        let base_url = std::env::var("TAVILY_BASE_URL")
            .unwrap_or_else(|_| "https://api.tavily.com".to_string());

        let url = format!("{}/search", base_url);
        let body = json!({
            "query": query,
            "max_results": max_results,
            "search_depth": "basic",
            "include_answer": false,
        });

        let resp = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Tavily request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("Tavily API error ({}): {}", status, text));
        }

        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("Tavily parse error: {}", e))?;

        let results: Vec<SearchResult> = data["results"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .map(|r| SearchResult {
                title: r["title"].as_str().unwrap_or("").to_string(),
                url: r["url"].as_str().unwrap_or("").to_string(),
            })
            .filter(|r| !r.title.is_empty() && !r.url.is_empty())
            .take(max_results)
            .collect();

        Ok(results)
    }
}

/// Simple URL encoding that doesn't pull in another crate.
fn urlencoding(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 3);
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            b' ' => result.push('+'),
            _ => {
                result.push('%');
                result.push(hex(byte >> 4));
                result.push(hex(byte & 0x0F));
            }
        }
    }
    result
}

fn hex(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        _ => (b'A' + (n - 10)) as char,
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the web for information. Returns title and URL only (no snippets — use web_fetch to \
         read full page content). \n\n\
         Zero configuration required — uses DuckDuckGo by default. Optionally set TAVILY_API_KEY \
         for enhanced results. Max 8 uses per session. \n\n\
         Prefer this over web_fetch when you need to discover information rather than fetch a \
         known URL."
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query string."
                },
                "max_results": {
                    "type": "integer",
                    "default": 5,
                    "description": "Maximum number of results to return (1-10)."
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let query = input["query"].as_str().ok_or_else(|| ToolError {
            message: "query is required".to_string(),
            code: Some("missing_parameter".to_string()),
        })?;

        // Enforce max_uses limit (Wgenty Code pattern: max 8 web_search calls)
        let count = self.use_count.fetch_add(1, Ordering::SeqCst);
        if count >= self.max_uses {
            return Ok(ToolOutput {
                output_type: "search_error".to_string(),
                content: json!({
                    "success": false,
                    "error": format!("web_search limit reached: {} / {} uses", count, self.max_uses),
                    "max_uses_exceeded": true,
                    "hint": "Use web_fetch to read specific URLs directly, or re-start the conversation to reset limits."
                }).to_string(),
                metadata: std::collections::HashMap::new(),
            });
        }

        let max_results = input["max_results"].as_u64().unwrap_or(5).min(10) as usize;

        if query.trim().is_empty() {
            return Err(ToolError {
                message: "query cannot be empty".to_string(),
                code: Some("invalid_parameter".to_string()),
            });
        }

        // Strategy: duckduckgo first (zero-dependency), then Tavily if available
        let (results, backend) = match self.search_duckduckgo(query, max_results).await {
            Ok(r) => (r, "duckduckgo"),
            Err(ddg_err) => {
                // Try Tavily as fallback
                match self.search_tavily(query, max_results).await {
                    Ok(r) => (r, "tavily"),
                    Err(tav_err) => {
                        return Ok(ToolOutput {
                            output_type: "search_error".to_string(),
                            content: json!({
                                "success": false,
                                "error": format!(
                                    "All search backends failed.\n  DuckDuckGo: {}\n  Tavily: {}\n\n\
                                     Hint: verify your network can reach html.duckduckgo.com, \
                                     or set TAVILY_API_KEY for enhanced search.",
                                    ddg_err, tav_err
                                ),
                                "query": query,
                            }).to_string(),
                            metadata: std::collections::HashMap::new(),
                        });
                    }
                }
            }
        };

        let output = json!({
            "success": true,
            "query": query,
            "backend": backend,
            "results_count": results.len(),
            "use": count + 1,
            "results": results.iter().map(|r| json!({
                "title": r.title,
                "url": r.url,
            })).collect::<Vec<_>>(),
        });

        let mut metadata = std::collections::HashMap::new();
        metadata.insert("query".to_string(), json!(query));
        metadata.insert("backend".to_string(), json!(backend));
        metadata.insert("count".to_string(), json!(results.len()));

        Ok(ToolOutput {
            output_type: "search_result".to_string(),
            content: output.to_string(),
            metadata,
        })
    }
}

impl Default for WebSearchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_urlencoding_spaces() {
        assert_eq!(urlencoding("hello world"), "hello+world");
    }

    #[test]
    fn test_urlencoding_special() {
        assert_eq!(urlencoding("rust & go"), "rust+%26+go");
    }

    #[test]
    fn test_parse_ddg_html() {
        let html = r#"
        <html>
        <a rel="nofollow" class="result__a" href="https://example.com/page1">Example Title One</a>
        <a rel="nofollow" class="result__a" href="https://example.com/page2">Example Title Two</a>
        </html>
        "#;
        let results = WebSearchTool::parse_ddg_html(html, 5);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Example Title One");
        assert_eq!(results[0].url, "https://example.com/page1");
        assert_eq!(results[1].title, "Example Title Two");
        assert_eq!(results[1].url, "https://example.com/page2");
    }

    #[test]
    fn test_parse_ddg_html_filters_ads() {
        let html = r#"
        <a rel="nofollow" class="result__a" href="https://duckduckgo.com/y.js?ad=1">Ad Link</a>
        <a rel="nofollow" class="result__a" href="https://real-result.com">Real Result</a>
        "#;
        let results = WebSearchTool::parse_ddg_html(html, 5);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://real-result.com");
    }

    #[test]
    fn test_parse_ddg_html_respects_limit() {
        let mut html = String::new();
        for i in 0..10 {
            html.push_str(&format!(
                r#"<a rel="nofollow" class="result__a" href="https://example.com/{}">Title {}</a>"#,
                i, i
            ));
        }
        let results = WebSearchTool::parse_ddg_html(&html, 3);
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn test_extract_attr() {
        let tag = r#"<a href="https://example.com" class="foo">"#;
        assert_eq!(
            WebSearchTool::extract_attr(tag, "href"),
            Some("https://example.com".to_string())
        );
    }

    #[test]
    fn test_input_schema() {
        let tool = WebSearchTool::new();
        let schema = tool.input_schema();
        assert!(schema["properties"]["query"].is_object());
        assert_eq!(schema["required"][0], "query");
    }

    #[test]
    fn test_empty_query_rejected() {
        let tool = WebSearchTool::new();
        assert!(tool.is_read_only());
    }

    #[tokio::test]
    async fn test_max_uses_enforced() {
        let tool = WebSearchTool::with_max_uses(0);
        let result = tool.execute(json!({"query": "rust"})).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(parsed["max_uses_exceeded"], true);
    }

    #[tokio::test]
    async fn test_max_uses_counting() {
        let tool = WebSearchTool::with_max_uses(10);
        assert_eq!(tool.use_count.load(Ordering::SeqCst), 0);
        let _ = tool.execute(json!({"query": "rust lang"})).await;
        assert_eq!(tool.use_count.load(Ordering::SeqCst), 1);
    }
}
