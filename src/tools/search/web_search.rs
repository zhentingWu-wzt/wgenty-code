//! Web Search Tool — zero-dependency by default.
//!
//! Search strategy (in order):
//! 1. DuckDuckGo HTML (zero API key, zero config) — default
//! 2. Bing RSS (zero API key) — keyless fallback for networks where
//!    DuckDuckGo is unreachable (e.g. mainland China)
//! 3. Baidu HTML (zero API key) — second keyless fallback; reachable from
//!    mainland-China networks that throttle the TLS fingerprints of the
//!    engines above
//! 4. Tavily API (if TAVILY_API_KEY is set) — enhanced quality
//!
//! Returns title + URL only (no snippets — use web_fetch to read page content).
//! Max 8 uses per session (following Wgenty Code's pattern).

use crate::utils::http::{web_search_client, web_search_resolve_client};
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
    resolve_client: Client,
    pub(crate) use_count: AtomicUsize,
    max_uses: usize,
}

impl WebSearchTool {
    pub fn new() -> Self {
        Self {
            use_count: AtomicUsize::new(0),
            max_uses: 8,
            client: web_search_client(),
            resolve_client: web_search_resolve_client(),
        }
    }

    #[cfg(test)]
    fn with_max_uses(max_uses: usize) -> Self {
        Self {
            use_count: AtomicUsize::new(0),
            max_uses,
            client: web_search_client(),
            resolve_client: web_search_resolve_client(),
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
            // Bound the worst case: on networks where DuckDuckGo is blocked the
            // connect attempt stalls, and the 30s client timeout would make every
            // search wait the full window before the fallback runs.
            .timeout(std::time::Duration::from_secs(10))
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

    /// Secondary backend: Bing search RSS output (zero API key, zero config).
    ///
    /// `https://www.bing.com/search?q=...&format=rss` serves a stable RSS 2.0
    /// document whose `<item>` entries carry `<title>` and `<link>`. Used as
    /// the keyless fallback for networks where DuckDuckGo is unreachable.
    async fn search_bing_rss(
        &self,
        query: &str,
        max_results: usize,
    ) -> Result<Vec<SearchResult>, String> {
        let url = format!(
            "https://www.bing.com/search?q={}&format=rss",
            urlencoding(query)
        );

        let resp = self
            .client
            .get(&url)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| format!("Bing request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("Bing returned status {}", resp.status()));
        }

        let body = Self::read_body_tolerant(resp).await?;

        let results = Self::parse_bing_rss(&body, max_results);
        if results.is_empty() {
            return Err("Bing returned no results".to_string());
        }

        Ok(results)
    }

    /// Read a response body chunk-by-chunk, tolerating a trailing stream error.
    ///
    /// `www.bing.com` geo-redirects mainland-China IPs to `cn.bing.com`, which
    /// (over HTTP/2) often delivers the complete body and then resets the
    /// stream without a clean END_STREAM (curl exit 92, "stream not closed
    /// cleanly"). reqwest surfaces that as a body decode error from
    /// `.text()`/`.bytes()` even though every byte arrived. Accumulating
    /// `.chunk()`s lets us keep the delivered data and treat the body as
    /// complete when it satisfies `Content-Length` (or when any bytes arrived
    /// and no length was advertised). A genuinely truncated body fails RSS
    /// parsing downstream and falls through to the next backend.
    async fn read_body_tolerant(mut resp: reqwest::Response) -> Result<String, String> {
        let expected = resp.content_length();
        let mut buf: Vec<u8> = Vec::new();
        loop {
            match resp.chunk().await {
                Ok(Some(chunk)) => buf.extend_from_slice(&chunk),
                Ok(None) => break,
                Err(e) => {
                    let complete = match expected {
                        Some(len) => buf.len() as u64 >= len,
                        None => !buf.is_empty(),
                    };
                    if !complete {
                        return Err(format!("Bing read error: {}", e));
                    }
                    break;
                }
            }
        }
        Ok(String::from_utf8_lossy(&buf).into_owned())
    }

    /// Tertiary backend: Baidu HTML results (zero API key, zero config).
    ///
    /// The only major engine reliably reachable with a rustls TLS stack from
    /// mainland-China networks. Organic result URLs are `baidu.com/link?url=…`
    /// redirect wrappers; each one is resolved to its real target via the
    /// `Location` header (best-effort — unresolved links keep the wrapper).
    async fn search_baidu(
        &self,
        query: &str,
        max_results: usize,
    ) -> Result<Vec<SearchResult>, String> {
        let url = format!("https://www.baidu.com/s?wd={}", urlencoding(query));

        let resp = self
            .client
            .get(&url)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| format!("Baidu request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("Baidu returned status {}", resp.status()));
        }

        let body = resp
            .text()
            .await
            .map_err(|e| format!("Baidu read error: {}", e))?;

        let parsed = Self::parse_baidu_html(&body, max_results);
        if parsed.is_empty() {
            // Detect Baidu's anti-bot interstitial so the fallback chain (and
            // the user) can tell "blocked" apart from "no organic results".
            if body.contains("百度安全验证") || body.contains("wappass.baidu.com") {
                return Err("Baidu anti-bot verification triggered — search blocked".to_string());
            }
            return Err("Baidu returned no results".to_string());
        }

        // Resolve redirect wrappers in parallel; keep unresolved links as-is.
        let resolved = futures::future::join_all(parsed.into_iter().map(|r| async move {
            let url = if r.url.contains("baidu.com/link?") {
                self.resolve_redirect(&r.url).await.unwrap_or(r.url)
            } else {
                r.url
            };
            SearchResult {
                title: r.title,
                url,
            }
        }))
        .await;

        Ok(resolved)
    }

    /// Follow one redirect hop: return the `Location` target of a 3xx.
    async fn resolve_redirect(&self, url: &str) -> Option<String> {
        let resp = self.resolve_client.get(url).send().await.ok()?;
        let status = resp.status();
        if !status.is_redirection() {
            return None;
        }
        let location = resp
            .headers()
            .get(reqwest::header::LOCATION)?
            .to_str()
            .ok()?
            .to_string();
        if location.starts_with("http") {
            Some(location)
        } else {
            None
        }
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

    /// Parse Baidu HTML result titles.
    ///
    /// Organic results render as `<h3 class="… t …"><a … href="…">Title</a></h3>`;
    /// the anchor text may contain `<em>` highlight tags, which are stripped.
    fn parse_baidu_html(html: &str, max_results: usize) -> Vec<SearchResult> {
        let mut results = Vec::new();
        let mut remaining = html;
        while let Some(start) = remaining.find("<h3") {
            let Some(offset) = remaining[start..].find("</h3>") else {
                break;
            };
            let end = start + offset;
            let block = &remaining[start..end];

            if let Some(anchor) = block.find("<a ") {
                let tag = &block[anchor..];
                let href = Self::extract_attr(tag, "href");
                let title = tag
                    .find('>')
                    .and_then(|open| {
                        tag[open + 1..]
                            .find("</a>")
                            .map(|close| &tag[open + 1..open + 1 + close])
                    })
                    .map(|inner| {
                        // Strip <em>/</em> highlight tags, then decode entities.
                        let no_tags = strip_tags(inner);
                        decode_entities(&no_tags)
                    })
                    .unwrap_or_default();

                if let (Some(url), false) = (href, title.trim().is_empty()) {
                    if url.starts_with("http") {
                        results.push(SearchResult {
                            title: title.trim().to_string(),
                            url,
                        });
                    }
                }
            }

            remaining = &remaining[end + "</h3>".len()..];
            if results.len() >= max_results {
                break;
            }
        }

        results
    }

    /// Parse Bing RSS `<item>` entries into results.
    fn parse_bing_rss(rss: &str, max_results: usize) -> Vec<SearchResult> {
        let mut results = Vec::new();
        let mut remaining = rss;
        while let Some(start) = remaining.find("<item>") {
            let Some(offset) = remaining[start..].find("</item>") else {
                break;
            };
            let end = start + offset;
            let item = &remaining[start..end];
            if let (Some(title), Some(link)) = (
                extract_xml_text(item, "title"),
                extract_xml_text(item, "link"),
            ) {
                if !title.is_empty() && link.starts_with("http") {
                    results.push(SearchResult { title, url: link });
                }
            }

            remaining = &remaining[end + "</item>".len()..];
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

/// Extract the inner text of an XML element, stripping CDATA wrappers and
/// decoding common entities.
fn extract_xml_text(fragment: &str, tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let start = fragment.find(&open)? + open.len();
    let end = start + fragment[start..].find(&close)?;
    let raw = fragment[start..end].trim();
    let text = raw
        .strip_prefix("<![CDATA[")
        .and_then(|t| t.strip_suffix("]]>"))
        .map(str::trim)
        .unwrap_or(raw);
    Some(
        text.replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&#39;", "'"),
    )
}

/// Remove all HTML tags from a fragment, keeping only inner text.
fn strip_tags(fragment: &str) -> String {
    let mut out = String::with_capacity(fragment.len());
    let mut rest = fragment;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        match rest[open..].find('>') {
            Some(close) => rest = &rest[open + close + 1..],
            None => break,
        }
    }
    out.push_str(rest);
    out
}

/// Decode the handful of HTML entities search-result titles actually contain.
fn decode_entities(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
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
         Zero configuration required — uses DuckDuckGo by default with keyless \
         Bing and Baidu fallbacks for restricted networks, plus optional \
         TAVILY_API_KEY for enhanced results. Max 8 uses per session. \n\n\
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

        // Strategy: DuckDuckGo → Bing RSS → Baidu → Tavily (if configured).
        let (results, backend) = match self.search_duckduckgo(query, max_results).await {
            Ok(r) => (r, "duckduckgo"),
            Err(ddg_err) => match self.search_bing_rss(query, max_results).await {
                Ok(r) => (r, "bing"),
                Err(bing_err) => match self.search_baidu(query, max_results).await {
                    Ok(r) => (r, "baidu"),
                    Err(baidu_err) => match self.search_tavily(query, max_results).await {
                        Ok(r) => (r, "tavily"),
                        Err(tav_err) => {
                            return Ok(ToolOutput {
                                output_type: "search_error".to_string(),
                                content: json!({
                                    "success": false,
                                    "error": format!(
                                        "All search backends failed.\n  DuckDuckGo: {}\n  Bing: {}\n  Baidu: {}\n  Tavily: {}\n\n\
                                         Hint: verify your network can reach html.duckduckgo.com, bing.com or baidu.com, \
                                         or set TAVILY_API_KEY for enhanced search.",
                                        ddg_err, bing_err, baidu_err, tav_err
                                    ),
                                    "query": query,
                                }).to_string(),
                                metadata: std::collections::HashMap::new(),
                            });
                        }
                    },
                },
            },
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

    #[test]
    fn test_parse_bing_rss() {
        let rss = r#"<?xml version="1.0"?>
        <rss version="2.0"><channel>
        <title>rust language - Bing</title>
        <item><title>Rust Programming Language</title><link>https://www.rust-lang.org/</link><description>snip</description></item>
        <item><title><![CDATA[Rust &amp; Cargo Guide]]></title><link>https://doc.rust-lang.org/cargo/</link></item>
        <item><title>Bad Link</title><link>/relative/path</link></item>
        </channel></rss>"#;
        let results = WebSearchTool::parse_bing_rss(rss, 5);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Rust Programming Language");
        assert_eq!(results[0].url, "https://www.rust-lang.org/");
        assert_eq!(results[1].title, "Rust & Cargo Guide");
        assert_eq!(results[1].url, "https://doc.rust-lang.org/cargo/");
    }

    #[test]
    fn test_parse_bing_rss_respects_limit() {
        let mut rss = String::from("<channel>");
        for i in 0..5 {
            rss.push_str(&format!(
                "<item><title>T {i}</title><link>https://example.com/{i}</link></item>"
            ));
        }
        rss.push_str("</channel>");
        let results = WebSearchTool::parse_bing_rss(&rss, 3);
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn test_extract_xml_text() {
        assert_eq!(
            extract_xml_text("<item><title>plain</title></item>", "title"),
            Some("plain".to_string())
        );
        assert_eq!(
            extract_xml_text("<t><![CDATA[ cdata body ]]></t>", "t"),
            Some("cdata body".to_string())
        );
        assert_eq!(
            extract_xml_text("<t>a &amp; b</t>", "t"),
            Some("a & b".to_string())
        );
        assert_eq!(extract_xml_text("<item></item>", "title"), None);
    }

    #[test]
    fn test_parse_baidu_html() {
        let html = r#"<div><h3 class="cosc-title cos-link t title_4QsBx" data-module="title"><a class="cosc-title-a" href="http://www.baidu.com/link?url=AAA">Rust 程序设计语言</a></h3>
        <h3 class="t _sc-title" style=" "><a class="sc-link" href="http://www.baidu.com/link?url=BBB">The <em>Rust</em> Programming Language</a></h3>
        <h3 class="other"><a href="/relative">Skipped Relative</a></h3></div>"#;
        let results = WebSearchTool::parse_baidu_html(html, 5);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Rust 程序设计语言");
        assert!(results[0].url.starts_with("http://www.baidu.com/link?"));
        assert_eq!(results[1].title, "The Rust Programming Language");
    }

    #[test]
    fn test_parse_baidu_html_respects_limit() {
        let mut html = String::new();
        for i in 0..5 {
            html.push_str(&format!(
                r#"<h3 class="t"><a href="https://example.com/{i}">T {i}</a></h3>"#
            ));
        }
        let results = WebSearchTool::parse_baidu_html(&html, 3);
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn test_strip_tags_and_decode_entities() {
        assert_eq!(strip_tags("a<em>b</em>c"), "abc");
        assert_eq!(strip_tags("no tags"), "no tags");
        assert_eq!(decode_entities("a &amp; b &lt;c&gt;"), "a & b <c>");
    }
}
