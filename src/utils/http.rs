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

/// Parsed body of the daemon's public `GET /api/v1/health`.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct DaemonHealth {
    pub status: String,
    pub version: String,
}

/// GET `{base_url}/api/v1/health` and return the parsed body when the peer
/// is really a wgenty daemon (HTTP 200 + well-formed health body). A bare
/// TCP connect is not enough: the port may be held by a foreign process, and
/// attaching to it would surface later as auth (401) or protocol errors.
pub async fn probe_daemon_health(client: &reqwest::Client, base_url: &str) -> Option<DaemonHealth> {
    let url = format!("{}/api/v1/health", base_url.trim_end_matches('/'));
    let resp = client.get(&url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.json::<DaemonHealth>().await.ok()
}

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

/// Return a web-search client variant that does NOT follow redirects, used to
/// resolve search-engine redirect wrappers (e.g. `baidu.com/link?url=...`)
/// to their real target URLs via the `Location` header.
///
/// Shares the browser-like header set of [`web_search_client`] (Baidu serves
/// a bot-check page to requests without it) but uses a short timeout: a
/// resolution hop should cost a few hundred milliseconds, not seconds.
pub fn web_search_resolve_client() -> reqwest::Client {
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

            reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .user_agent(WEB_SEARCH_UA)
                .default_headers(headers)
                .redirect(reqwest::redirect::Policy::none())
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
///
/// NOTE: no `Accept-Encoding` header may be set here. The reqwest build has no
/// compression features enabled (`default-features = false`), so it cannot
/// decompress gzip/deflate/brotli — advertising them would make servers return
/// compressed bodies that `.text()` fails to decode ("error decoding response
/// body"). Requests are sent without the header, so servers respond plain.
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

// ── Error cause-chain formatting ─────────────────────────────────────────────

/// Format an error with its full cause chain.
///
/// reqwest's `Display` only prints the outer kind (e.g. "error decoding
/// response body") and silently drops the actual cause - timeout vs.
/// connection reset vs. HTTP/2 stream error - which lives in
/// `std::error::Error::source()`. This walks the chain so the real reason a
/// request or stream was interrupted is visible in logs and error payloads.
pub fn format_error_chain(e: &dyn std::error::Error) -> String {
    let mut s = e.to_string();
    let mut current = e.source();
    while let Some(cause) = current {
        let cause_str = cause.to_string();
        if !cause_str.is_empty() {
            s.push_str(": ");
            s.push_str(&cause_str);
        }
        current = cause.source();
    }
    s
}

/// Format an `anyhow::Error` with its full cause chain: anyhow contexts first,
/// then the `std::error::Error::source()` chain below the root cause.
pub fn format_anyhow_error_chain(e: &anyhow::Error) -> String {
    let mut s = String::new();
    for cause in e.chain() {
        let part = cause.to_string();
        if part.is_empty() {
            continue;
        }
        if !s.is_empty() {
            s.push_str(": ");
        }
        s.push_str(&part);
    }
    let mut current = e.root_cause().source();
    while let Some(cause) = current {
        let part = cause.to_string();
        if !part.is_empty() {
            s.push_str(": ");
            s.push_str(&part);
        }
        current = cause.source();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, thiserror::Error)]
    #[error("outer failure")]
    struct OuterError(#[source] InnerError);

    #[derive(Debug, thiserror::Error)]
    #[error("root cause: connection reset by peer")]
    struct InnerError;

    #[test]
    fn error_chain_includes_all_causes() {
        let err = OuterError(InnerError);
        assert_eq!(
            format_error_chain(&err),
            "outer failure: root cause: connection reset by peer"
        );
    }

    #[test]
    fn anyhow_chain_includes_contexts_and_sources() {
        let err = anyhow::Error::new(OuterError(InnerError)).context("while streaming");
        let chain = format_anyhow_error_chain(&err);
        assert!(
            chain.starts_with("while streaming: outer failure"),
            "{chain}"
        );
        assert!(
            chain.ends_with("root cause: connection reset by peer"),
            "{chain}"
        );
    }
}
