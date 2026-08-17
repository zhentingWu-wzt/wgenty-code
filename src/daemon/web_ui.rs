//! Embedded Web UI static hosting (daemon-hosted-web-ui design §1).
//!
//! Compile-time embeds `web/dist` via rust-embed and serves:
//! - `GET /`         → `index.html` (`no-cache`) or inline fallback page
//! - `GET /assets/*` → hashed assets (immutable long cache)
//!
//! Static routes must live in the public router group: page load happens
//! before any token acquisition (design §1) — the browser needs the
//! HTML/JS first in order to run the auth bootstrap flow.

use crate::daemon::state::DaemonState;
use axum::{
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use rust_embed::RustEmbed;
use std::sync::Arc;

/// `web/dist` 产物树。debug 构建下 rust-embed 在运行时从磁盘读取
/// （前端开发时改动即时可见）；release 构建把字节直接烤进二进制（设计 §6）。
#[derive(RustEmbed)]
#[folder = "web/dist"]
struct WebAssets;

/// 扩展名→MIME 映射（设计 §1 组件表）：html/js/mjs/css/svg/png/ico/json/
/// wasm/woff2/map，其余一律 `application/octet-stream`。纯函数，供单元测试
/// 直接驱动。
fn mime_for(ext: &str) -> &'static str {
    match ext {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "json" => "application/json",
        "wasm" => "application/wasm",
        "woff2" => "font/woff2",
        // source map 本质是 JSON 文档
        "map" => "application/json",
        _ => "application/octet-stream",
    }
}

/// `GET /`：嵌入有 `index.html` → 200 + `text/html; charset=utf-8` +
/// `Cache-Control: no-cache`（入口 HTML 必须每次重新验证，否则发新版本后
/// 浏览器仍拿旧入口、引用已不存在的 hashed 资产）。
///
/// 无 `index.html`（未执行 `npm --prefix web run build`）→ 返回 200 内联
/// 降级页（设计 §1），提示如何构建前端。
async fn serve_index() -> Response {
    match WebAssets::get("index.html") {
        Some(asset) => (
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                (header::CACHE_CONTROL, "no-cache"),
            ],
            asset.data.into_owned(),
        )
            .into_response(),
        None => degradation_response(),
    }
}

/// 降级页 HTML（设计 §1）：`web/dist` 未构建时 `GET /` 返回的内联最小页面。
/// 纯 Rust 字符串常量、零外部依赖 —— 降级路径必须不依赖任何嵌入资产存在
/// （正因为资产缺失才走到这里）。提为纯函数供单元测试直接驱动，避免测试
/// 依赖 `web/dist` 磁盘状态（debug 构建下 rust-embed 运行时读盘）。
fn degradation_page() -> &'static str {
    r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>wgenty-code daemon</title>
</head>
<body>
<p>Web UI not bundled — run <code>npm --prefix web run build</code>, then restart the daemon.</p>
</body>
</html>"#
}

/// 降级响应：200（非 500 —— 缺前端产物是可恢复的构建前置问题，不是服务端
/// 错误）+ `text/html` + `Cache-Control: no-cache`（构建完成后立即恢复正常
/// 页面，不允许缓存降级页）。
fn degradation_response() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        degradation_page(),
    )
        .into_response()
}

/// `GET /assets/*path`：按 `assets/<path>` 查嵌入资产，MIME 按扩展名映射，
/// `Cache-Control: public, max-age=31536000, immutable`（Vite 产物文件名带
/// 内容 hash，可安全长缓存）；未命中返回 404。
async fn serve_asset(axum::extract::Path(path): axum::extract::Path<String>) -> Response {
    let path = path.trim_start_matches('/');
    let Some(asset) = WebAssets::get(&format!("assets/{path}")) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    // rsplit 保证至少产出一个元素：无扩展名时返回整段路径 → 落入默认分支
    let ext = path.rsplit('.').next().unwrap_or_default();
    (
        [
            (header::CONTENT_TYPE, mime_for(ext)),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        asset.data.into_owned(),
    )
        .into_response()
}

/// 静态路由挂入 public（health）路由组（设计 §1）：页面加载先于任何
/// token 获取。
pub(crate) fn public_router() -> Router<Arc<DaemonState>> {
    Router::new()
        .route("/", get(serve_index))
        .route("/assets/*path", get(serve_asset))
    // Task 2.1 追加：.route("/auth/bootstrap", get(bootstrap_token))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_for_covers_full_design_table() {
        assert_eq!(mime_for("html"), "text/html; charset=utf-8");
        assert_eq!(mime_for("js"), "text/javascript; charset=utf-8");
        assert_eq!(mime_for("mjs"), "text/javascript; charset=utf-8");
        assert_eq!(mime_for("css"), "text/css; charset=utf-8");
        assert_eq!(mime_for("svg"), "image/svg+xml");
        assert_eq!(mime_for("png"), "image/png");
        assert_eq!(mime_for("ico"), "image/x-icon");
        assert_eq!(mime_for("json"), "application/json");
        assert_eq!(mime_for("wasm"), "application/wasm");
        assert_eq!(mime_for("woff2"), "font/woff2");
        assert_eq!(mime_for("map"), "application/json");
    }

    #[test]
    fn mime_for_defaults_to_octet_stream() {
        assert_eq!(mime_for("exe"), "application/octet-stream");
        assert_eq!(mime_for("unknown-ext"), "application/octet-stream");
        assert_eq!(mime_for(""), "application/octet-stream");
    }

    #[tokio::test]
    async fn serve_index_without_index_html_serves_degradation_page() {
        // 直接驱动降级响应函数而非 serve_index：debug 构建下 rust-embed 运行时
        // 读盘，web/dist/index.html 是否存在会决定 serve_index 走哪个分支，
        // 单测必须确定性只测降级分支。
        let resp = degradation_response();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("text/html; charset=utf-8")
        );
        assert_eq!(
            resp.headers()
                .get(header::CACHE_CONTROL)
                .and_then(|v| v.to_str().ok()),
            Some("no-cache")
        );
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("read body");
        let html = std::str::from_utf8(&body).expect("utf-8");
        assert!(html.contains("<!doctype html>"));
        assert!(html.contains("<title>wgenty-code daemon</title>"));
        assert!(html.contains("Web UI not bundled"));
        assert!(html.contains("npm --prefix web run build"));
    }
}
