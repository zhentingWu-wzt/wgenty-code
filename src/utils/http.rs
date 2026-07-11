//! Shared HTTP client factory.
//!
//! Tools and CLI commands that need a one-off `reqwest::Client` should use
//! these helpers instead of calling `Client::builder()` directly. This avoids
//! creating redundant connection pools and ensures consistent timeout / UA
//! defaults across the codebase.
//!
//! Note: `api::ApiClient` and `tui::DaemonClient` intentionally build their
//! own clients with provider-specific timeouts and auth headers - they are
//! exempt from this factory.

use std::sync::OnceLock;
/// Default user-agent string for outgoing requests.
const DEFAULT_UA: &str = "wgenty-code/1.0";
const WEB_SEARCH_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

/// Return the shared general-purpose client with no total timeout.
///
/// This preserves `reqwest::Client::new()` semantics for callers that manage
/// their own request deadline while still reusing one connection pool.
pub fn default_client() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .user_agent(DEFAULT_UA)
                .build()
                .unwrap_or_default()
        })
        .clone()
}

/// Return the shared web-search client with a browser-like user-agent, Accept
/// headers, and a 30-second total timeout.
///
/// DuckDuckGo (and other search engines) require browser-like Accept /
/// Accept-Language headers to avoid serving a CAPTCHA page. Without them, the
/// server classifies the request as bot traffic even when the user-agent string
/// looks legitimate.
pub fn web_search_client() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert(
                reqwest::header::ACCEPT,
                reqwest::header::HeaderValue::from_static(
                    "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
                ),
            );
            headers.insert(
                reqwest::header::ACCEPT_LANGUAGE,
                reqwest::header::HeaderValue::from_static("en-US,en;q=0.9"),
            );
            headers.insert(
                reqwest::header::ACCEPT_ENCODING,
                reqwest::header::HeaderValue::from_static("gzip, deflate, br"),
            );
            headers.insert(
                reqwest::header::DNT,
                reqwest::header::HeaderValue::from_static("1"),
            );

            // Sec-Fetch headers — modern browsers send these; missing them is a bot signal
            headers.insert(
                reqwest::header::HeaderName::from_static("sec-fetch-site"),
                reqwest::header::HeaderValue::from_static("none"),
            );
            headers.insert(
                reqwest::header::HeaderName::from_static("sec-fetch-mode"),
                reqwest::header::HeaderValue::from_static("navigate"),
            );
            headers.insert(
                reqwest::header::HeaderName::from_static("sec-fetch-dest"),
                reqwest::header::HeaderValue::from_static("document"),
            );
            headers.insert(
                reqwest::header::HeaderName::from_static("sec-fetch-user"),
                reqwest::header::HeaderValue::from_static("?1"),
            );

            reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .user_agent(WEB_SEARCH_UA)
                .default_headers(headers)
                .build()
                .unwrap_or_default()
        })
        .clone()
}
