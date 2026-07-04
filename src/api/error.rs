//! API error formatting and network error wrapping.
//!
//! Provides user-friendly error messages for common failure modes
//! (timeout, DNS, TLS, connection refused) and parses upstream API
//! error responses into concise `"code: message"` strings.

/// Wrap a reqwest error with a user-friendly message for network failures.
pub(crate) fn wrap_network_error(err: reqwest::Error, api_name: &str) -> anyhow::Error {
    let msg = err.to_string().to_lowercase();
    if err.is_timeout() {
        anyhow::anyhow!(
            "⚠️  Connection timed out while contacting {api_name} API. \
             Please check your network connection and try again."
        )
    } else if err.is_connect() {
        anyhow::anyhow!(
            "⚠️  Cannot connect to {api_name} API — the server may be unreachable. \
             Check your network, VPN, or proxy settings."
        )
    } else if msg.contains("dns") || msg.contains("resolve") || msg.contains("name") {
        anyhow::anyhow!(
            "⚠️  DNS resolution failed for {api_name} API endpoint. \
             Verify your DNS settings and internet connection."
        )
    } else if msg.contains("tls") || msg.contains("ssl") || msg.contains("certificate") {
        anyhow::anyhow!(
            "⚠️  TLS/SSL error connecting to {api_name} API. \
             Check your system certificates or proxy configuration. (details: {err})"
        )
    } else if msg.contains("refused") {
        anyhow::anyhow!(
            "⚠️  Connection refused by {api_name} API server. \
             The API server may be down or blocked by a firewall."
        )
    } else {
        anyhow::anyhow!(
            "⚠️  Network error contacting {api_name} API: {err}. \
             Please check your internet connection."
        )
    }
}

/// Format an upstream API error response into a concise, human-readable message.
///
/// Recognises the `{"error":{"code":...,"message":...}}` shape used by
/// OpenAI-/Volcengine-compatible endpoints (and Anthropic's
/// `{"type":"error","error":{"type":...,"message":...}}` variant), returning
/// `"{code}: {message}"` so a misconfigured model surfaces as e.g.
/// `UnsupportedModel: The requested model does not support the coding plan
/// feature.` instead of a double-escaped JSON blob. Falls back to
/// `"API error ({status}): {body}"` when the body is not parseable, so no
/// detail is lost on unexpected shapes.
///
/// The raw status + body are also emitted to the dev log (`tracing::warn!`)
/// so the full upstream response stays available for debugging even though
/// the user-facing surface stays clean.
pub(crate) fn format_api_error(status: reqwest::StatusCode, body: &str) -> String {
    tracing::warn!(status = %status, body = %body, "upstream API error");
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
        if let Some(err) = v.get("error") {
            let code = err
                .get("code")
                .and_then(|c| c.as_str())
                .or_else(|| err.get("type").and_then(|c| c.as_str()))
                .filter(|s| !s.is_empty());
            let message = err
                .get("message")
                .and_then(|m| m.as_str())
                .filter(|s| !s.is_empty());
            if let Some(message) = message {
                if let Some(code) = code {
                    return format!("{}: {}", code, message);
                }
                return message.to_string();
            }
        }
    }
    format!("API error ({}): {}", status, body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_api_error_parses_openai_compat_error_body() {
        // Real Volcengine Ark reply for an unsupported model on the coding
        // endpoint — must surface as a clean "code: message" string, not a
        // double-escaped JSON blob.
        let body = r#"{"error":{"code":"UnsupportedModel","message":"The requested model does not support the coding plan feature.","param":"","type":""}}"#;
        let msg = format_api_error(reqwest::StatusCode::NOT_FOUND, body);
        assert_eq!(
            msg,
            "UnsupportedModel: The requested model does not support the coding plan feature."
        );
    }

    #[test]
    fn format_api_error_falls_back_to_type_for_anthropic_shape() {
        // Anthropic uses {"type":"error","error":{"type":...,"message":...}}
        // (no `code` field) — fall back to `error.type` for the prefix.
        let body = r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#;
        let msg = format_api_error(reqwest::StatusCode::UNAUTHORIZED, body);
        assert_eq!(msg, "authentication_error: invalid x-api-key");
    }

    #[test]
    fn format_api_error_falls_back_to_raw_when_unparseable() {
        // Non-JSON body (e.g. an HTML 502 page) — keep status + body verbatim
        // so no detail is lost on unexpected shapes.
        let body = "<html>Bad Gateway</html>";
        let msg = format_api_error(reqwest::StatusCode::BAD_GATEWAY, body);
        assert_eq!(msg, "API error (502 Bad Gateway): <html>Bad Gateway</html>");
    }
}
