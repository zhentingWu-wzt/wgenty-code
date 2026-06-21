//! Daemon authentication — bearer-token middleware.
//!
//! On startup a random 32‑char hex token is generated and printed to stdout.
//! All endpoints except `GET /api/v1/health` require the header
//! `Authorization: Bearer <token>`.

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

/// Axum middleware that requires `Authorization: Bearer <expected_token>`.
pub async fn require_auth(
    axum::extract::State(token): axum::extract::State<String>,
    request: Request,
    next: Next,
) -> Response {
    let auth_header = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    if auth_header == Some(&token) {
        next.run(request).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            "unauthorized: missing or invalid bearer token",
        )
            .into_response()
    }
}
