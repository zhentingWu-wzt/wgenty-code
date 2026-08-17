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
    http::{header, StatusCode, Uri},
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

/// 挂在 mod.rs merge 后的最终 app（.fallback），不受 protected 组
/// route_layer 影响——静态深链公开可达，与 §2 "跨源读不到 token" 边界
/// 一致。
///
/// 分支顺序（设计 §1）：
/// 1. `/api/` 前缀 → 404 JSON：未知 API 路径绝不能吐 HTML——SPA 兜底页
///    会伪装成 API 响应，破坏客户端错误处理；
/// 2. 非 GET → 405：fallback 只为页面深链兜底，不承载任何写语义；
/// 3. 其余 GET → [`serve_index`]（SPA 深链兜底；单视图应用，仅兜 / 与
///    未来扩展）。
pub(crate) async fn spa_fallback(uri: Uri, method: axum::http::Method) -> Response {
    if uri.path().starts_with("/api/") {
        return (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "application/json")],
            r#"{"error":"not found"}"#,
        )
            .into_response();
    }
    if method != axum::http::Method::GET {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    serve_index().await
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

    // ---------- spa_fallback 行为（Task 1.5 路由接线） ----------

    /// 构造只挂 fallback 的最小 app 直接驱动 spa_fallback——不经过
    /// create_routers / auth 层，测试聚焦 fallback 自身的三条分支。
    /// tower 的 `util` feature 未启用（无 ServiceExt::oneshot），用原生
    /// `Service::poll_ready` + `call` 驱动；Router 的 poll_ready 恒就绪。
    async fn drive(method: &str, uri: &str) -> Response {
        use std::future::poll_fn;
        use tower::Service;

        let mut app = Router::new().fallback(spa_fallback);
        // Router 有两个 Service impl（IncomingStream / Request<B>），完全
        // 限定到 Request 消除 poll_ready 推断歧义
        poll_fn(|cx| {
            <axum::Router as Service<axum::http::Request<axum::body::Body>>>::poll_ready(
                &mut app, cx,
            )
        })
        .await
        .expect("router ready");
        app.call(
            axum::http::Request::builder()
                .method(method)
                .uri(uri)
                .body(axum::body::Body::empty())
                .expect("valid request"),
        )
        .await
        .expect("call request")
    }

    #[tokio::test]
    async fn spa_fallback_returns_404_json_for_unknown_api_paths() {
        let resp = drive("GET", "/api/v1/nonexistent").await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        // 未知 API 路径必须吐 JSON 而非 HTML——SPA 兜底页会伪装成 API
        // 响应，破坏客户端错误处理
        assert_eq!(
            resp.headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("application/json")
        );
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("read body");
        let text = std::str::from_utf8(&body).expect("utf-8 body");
        assert!(!text.contains('<'), "must not return HTML: {text}");
        assert!(text.contains("not found"));
    }

    #[tokio::test]
    async fn spa_fallback_serves_html_for_deep_links() {
        // 200 + text/html 即可：磁盘有无 web/dist/index.html 决定走 index
        // 还是降级页，两者 Content-Type 相同（任务验收只断言 200 + HTML）
        let resp = drive("GET", "/some/deep/link").await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("text/html; charset=utf-8")
        );
    }

    #[tokio::test]
    async fn spa_fallback_rejects_non_get_methods() {
        let resp = drive("POST", "/foo").await;
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    }
}
