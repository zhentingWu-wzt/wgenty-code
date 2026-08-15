//! Daemon authentication — bearer-token middleware.
//!
//! On startup a random 32‑char hex token is generated and printed to stdout.
//! All endpoints except `GET /api/v1/health` require the header
//! `Authorization: Bearer <token>`. Browser WebSocket APIs cannot set
//! headers, so credentials may alternatively arrive as the `?token=`
//! query parameter (sse-to-websocket design §3.1). The middleware accepts
//! that fallback so the query-token WS handshake (`GET /api/v1/ws`, which
//! is registered behind this middleware like every other protected route)
//! is not rejected before the in-handler ws auth runs; the ws handler
//! (`ws_push::authorize_ws`) enforces the same check independently.

use axum::{
    extract::Request,
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};

/// Generates a random 32‑char hex token using UUID v4 (no `-` separators).
pub fn generate_api_token() -> String {
    uuid::Uuid::new_v4().to_string().replace('-', "")
}

/// Extracts the presented credential from a request: the
/// `Authorization: Bearer` header first, falling back to the `?token=`
/// query parameter. Query values are matched verbatim (no percent-decoding)
/// because daemon tokens are plain hex, so callers never need encoding.
pub(crate) fn presented_token(request: &Request) -> Option<&str> {
    let header_token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    header_token.or_else(|| query_token(request))
}

/// Extracts the `token` query parameter, if present.
fn query_token(request: &Request) -> Option<&str> {
    request.uri().query()?.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == "token").then_some(value)
    })
}

/// Credential check shared by the middleware and the WebSocket handler.
/// A daemon whose expected token is not initialized (empty) rejects every
/// credential — including an empty `?token=` — so the guard cannot be
/// bypassed by omitting the value.
pub(crate) fn token_matches(expected: &str, presented: Option<&str>) -> bool {
    !expected.is_empty() && presented == Some(expected)
}

/// The shared 401 rejection; byte-identical wherever auth is enforced so WS
/// handshake failures look exactly like any other protected route's.
pub(crate) fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        "unauthorized: missing or invalid bearer token",
    )
        .into_response()
}

/// Axum middleware that requires `Authorization: Bearer <expected_token>`
/// (or the `?token=` query fallback).
pub async fn require_auth(
    axum::extract::State(token): axum::extract::State<String>,
    request: Request,
    next: Next,
) -> Response {
    if token_matches(&token, presented_token(&request)) {
        next.run(request).await
    } else {
        unauthorized()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request as HttpRequest;
    use axum::routing::get;
    use axum::Router;
    use tower::Service as _;

    fn request(uri: &str, bearer: Option<&str>) -> Request {
        let mut builder = HttpRequest::builder().method("GET").uri(uri);
        if let Some(bearer) = bearer {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {bearer}"));
        }
        builder
            .body(axum::body::Body::empty())
            .expect("test request")
    }

    #[test]
    fn presented_token_prefers_header_over_query() {
        let req = request("/api/v1/ws?token=qtok", Some("htok"));
        assert_eq!(presented_token(&req), Some("htok"));
    }

    #[test]
    fn presented_token_falls_back_to_query() {
        assert_eq!(
            presented_token(&request("/api/v1/ws?token=qtok", None)),
            Some("qtok")
        );
        // Other query params never satisfy the fallback.
        assert_eq!(
            presented_token(&request("/api/v1/ws?session_id=s1", None)),
            None
        );
        // Explicitly empty `?token=` yields Some("") — rejected by the
        // empty-expected guard rather than treated as "no credential".
        assert_eq!(
            presented_token(&request("/api/v1/ws?token=", None)),
            Some("")
        );
        assert_eq!(presented_token(&request("/api/v1/ws", None)), None);
    }

    #[test]
    fn token_matches_guards_empty_expected() {
        assert!(token_matches("tok", Some("tok")));
        assert!(!token_matches("tok", Some("other")));
        assert!(!token_matches("tok", None));
        // Uninitialized daemon: even an empty credential is rejected.
        assert!(!token_matches("", Some("")));
        assert!(!token_matches("", None));
    }

    /// 中间件层同构：query token 放行；无效凭证 401 且响应体与受保护路由一致。
    #[tokio::test]
    async fn require_auth_accepts_query_token_and_rejects_isomorphically() {
        let mut app = Router::new()
            .route("/api/v1/anything", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(
                "tok123".to_string(),
                require_auth,
            ));

        let res = app
            .call(request("/api/v1/anything?token=tok123", None))
            .await
            .expect("middleware router");
        assert_eq!(res.status(), StatusCode::OK);

        let res = app
            .call(request("/api/v1/anything?token=wrong", None))
            .await
            .expect("middleware router");
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        let body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .expect("read body");
        assert_eq!(
            body.as_ref(),
            b"unauthorized: missing or invalid bearer token"
        );
    }
}
