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

/// 是否嵌入了 index.html（web/dist 是否已构建）——供 run() 启动日志选用打印形态。
pub fn has_index() -> bool {
    WebAssets::get("index.html").is_some()
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

/// host:port 是否在白名单内（设计 §2）。接收已解析出的 authority 部分
/// （无 scheme、无路径）。
///
/// 白名单两档：
/// - 始终允许：`127.0.0.1:<port>`、`localhost:<port>`（`[::1]` 归 LAN 档）
/// - `lan_exposed`（`--host 0.0.0.0` 等非回环绑定）追加：任何**字面量私网
///   IP**（v4 10/8、172.16/12、192.168/16、169.254/16；v6 ULA fc00::/7、
///   链路本地 fe80::/10）——手机通过局域网 IP 访问嵌入式 Web UI 的场景。
///
/// 域名一律拒绝：DNS rebinding 攻击者把 `attacker.example` 解析到本机 IP，
/// 但浏览器发出的 Host/Origin 仍携带域名而非 IP，无法冒充字面量私网 IP。
fn is_allowed_host_port(host_port: &str, port: u16, lan_exposed: bool) -> bool {
    let Some((host, port_str)) = host_port.rsplit_once(':') else {
        return false;
    };
    if port_str != port.to_string() {
        return false;
    }
    if host == "127.0.0.1" || host == "localhost" {
        return true;
    }
    if !lan_exposed {
        return false;
    }
    // Strip IPv6 bracket form (`[fe80::1]` → `fe80::1`) then require a literal
    // private/loopback address.
    let bare = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    match bare.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
        Ok(std::net::IpAddr::V6(ip)) => {
            let seg = ip.segments()[0];
            ip.is_loopback()
                || (seg & 0xfe00) == 0xfc00 // unique-local fc00::/7
                || (seg & 0xffc0) == 0xfe80 // link-local fe80::/10
        }
        Err(_) => false,
    }
}

/// 设计 §2：三检查全部通过才放行。
/// 1. Origin 头存在 → 其 host（含端口）必须 ∈ {127.0.0.1:<port>, localhost:<port>}
/// 2. Sec-Fetch-Site 头存在 → 必须 ∈ {same-origin, none}（none 覆盖用户直接打开 URL）
/// 3. Host 头必须 ∈ {127.0.0.1:<port>, localhost:<port>} —— 防 DNS rebinding
///
/// 头不存在的维度放行（1、2 条）——非浏览器客户端（curl）不发这些头；但
/// Host 缺失或不匹配即拒：没有 Host 无法确认请求真正访问的目标。纯函数，
/// 供四方向单元测试直接驱动，无需构造 DaemonState。
fn is_same_origin_request(
    origin: Option<&str>,
    sec_fetch_site: Option<&str>,
    host: Option<&str>,
    port: u16,
    lan_exposed: bool,
) -> bool {
    // 检查 3（必查）：Host 防 DNS rebinding
    let Some(host) = host else { return false };
    if !is_allowed_host_port(host, port, lan_exposed) {
        return false;
    }
    // 检查 1（可选）：Origin 的 host:port 必须同源。Origin 形如
    // `http://127.0.0.1:8371`，取 `://` 之后的 authority；防御性截掉
    // 可能出现的 path/query（规范里 Origin 不带 path，但解析不依赖这点）。
    if let Some(origin) = origin {
        let authority = origin
            .rsplit_once("://")
            .map(|(_, rest)| rest)
            .unwrap_or(origin);
        let authority = authority.split(['/', '?', '#']).next().unwrap_or_default();
        if !is_allowed_host_port(authority, port, lan_exposed) {
            return false;
        }
    }
    // 检查 2（可选）：Fetch Metadata 标记。same-origin = 浏览器认定同源
    // 请求；none = 无来源（地址栏直接输入 / 书签 / 新标签页）。
    if let Some(site) = sec_fetch_site {
        if site != "same-origin" && site != "none" {
            return false;
        }
    }
    true
}

/// `GET /auth/bootstrap`（设计 §2）：页面加载后前端用同源请求换取启动
/// bearer token。通过 → 200 `{"token": ...}` + `Cache-Control: no-store`
/// （token 绝不能进任何缓存）；拒绝 → 403 JSON，token 不出。
///
/// 挂 public 组而非 protected 组：请求本身不带 token（它就是来领 token
/// 的），身份保障完全由 [`is_same_origin_request`] 三重谓词提供——跨源
/// 攻击者既过不了浏览器同源策略，也伪造不出匹配的 Origin/Host 组合。
async fn bootstrap_token(
    headers: axum::http::HeaderMap,
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
) -> Response {
    let same_origin = state
        .current_bind()
        // bind 在 run() bind 成功后设置；路由已可达却读不到端口属于
        // 初始化窗口异常，fail-closed
        .map(|(port, lan_exposed)| {
            is_same_origin_request(
                headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()),
                headers.get("sec-fetch-site").and_then(|v| v.to_str().ok()),
                headers.get(header::HOST).and_then(|v| v.to_str().ok()),
                port,
                lan_exposed,
            )
        })
        .unwrap_or(false);
    if !same_origin {
        return (
            StatusCode::FORBIDDEN,
            [(header::CONTENT_TYPE, "application/json")],
            r#"{"error":"cross-origin request rejected"}"#,
        )
            .into_response();
    }
    let token = state.current_api_token();
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        format!(r#"{{"token":"{token}"}}"#),
    )
        .into_response()
}

/// 静态路由挂入 public（health）路由组（设计 §1）：页面加载先于任何
/// token 获取。
pub(crate) fn public_router() -> Router<Arc<DaemonState>> {
    Router::new()
        .route("/", get(serve_index))
        .route("/assets/*path", get(serve_asset))
        .route("/auth/bootstrap", get(bootstrap_token))
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
pub async fn spa_fallback(uri: Uri, method: axum::http::Method) -> Response {
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

    // ---------- has_index（Task 3.2 启动日志） ----------

    #[test]
    fn has_index_matches_embedded_index_presence() {
        // 环境无关恒等式：无论本机 web/dist 是否已构建，两者必须一致
        assert_eq!(has_index(), WebAssets::get("index.html").is_some());
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

    // ---------- is_same_origin_request 谓词（Task 2.1，设计 §2） ----------

    const PORT: u16 = 8371;

    #[test]
    fn same_origin_predicate_allows_same_origin_requests() {
        // 完整同源三检查全过
        assert!(is_same_origin_request(
            Some("http://127.0.0.1:8371"),
            Some("same-origin"),
            Some("127.0.0.1:8371"),
            PORT,
            false,
        ));
        // localhost 变体同样放行
        assert!(is_same_origin_request(
            Some("http://localhost:8371"),
            Some("same-origin"),
            Some("localhost:8371"),
            PORT,
            false,
        ));
        // Sec-Fetch-Site=none：用户直接在地址栏打开 URL，属合法首次加载
        assert!(is_same_origin_request(
            Some("http://127.0.0.1:8371"),
            Some("none"),
            Some("127.0.0.1:8371"),
            PORT,
            false,
        ));
    }

    #[test]
    fn same_origin_predicate_allows_missing_optional_headers() {
        // 非浏览器客户端（curl 等）不发 Origin / Sec-Fetch-Site：头缺失的
        // 维度放行，但 Host 仍然必须校验
        assert!(is_same_origin_request(
            None,
            None,
            Some("127.0.0.1:8371"),
            PORT,
            false
        ));
        assert!(is_same_origin_request(
            None,
            None,
            Some("localhost:8371"),
            PORT,
            false
        ));
        // 只有 Origin 缺失
        assert!(is_same_origin_request(
            None,
            Some("same-origin"),
            Some("127.0.0.1:8371"),
            PORT,
            false,
        ));
        // 只有 Sec-Fetch-Site 缺失
        assert!(is_same_origin_request(
            Some("http://localhost:8371"),
            None,
            Some("localhost:8371"),
            PORT,
            false,
        ));
    }

    #[test]
    fn same_origin_predicate_rejects_cross_origin() {
        // 恶意站点发起的跨源请求：Origin host 不在白名单
        assert!(!is_same_origin_request(
            Some("http://evil.example"),
            Some("cross-site"),
            Some("127.0.0.1:8371"),
            PORT,
            false,
        ));
        // 即使其余头全过，Origin 单独越界即拒
        assert!(!is_same_origin_request(
            Some("http://evil.example"),
            Some("same-origin"),
            Some("127.0.0.1:8371"),
            PORT,
            false,
        ));
    }

    #[test]
    fn same_origin_predicate_rejects_dns_rebinding_host() {
        // DNS rebinding：浏览器把 attacker.example 解析到 127.0.0.1，Host
        // 头暴露真实访问目标 → Host 不在白名单即拒
        assert!(!is_same_origin_request(
            Some("http://attacker.example"),
            Some("same-origin"),
            Some("attacker.example"),
            PORT,
            false,
        ));
        // Host 单独越界（Origin 合法）也拒
        assert!(!is_same_origin_request(
            Some("http://127.0.0.1:8371"),
            Some("same-origin"),
            Some("attacker.example"),
            PORT,
            false,
        ));
        // Host 缺失即拒：无法确认访问目标
        assert!(!is_same_origin_request(
            Some("http://127.0.0.1:8371"),
            Some("same-origin"),
            None,
            PORT,
            false,
        ));
    }

    #[test]
    fn same_origin_predicate_rejects_cross_site_fetch_metadata() {
        // Sec-Fetch-Site=cross-site：浏览器明确标记跨站导航/请求
        assert!(!is_same_origin_request(
            Some("http://127.0.0.1:8371"),
            Some("cross-site"),
            Some("127.0.0.1:8371"),
            PORT,
            false,
        ));
        // same-site 也不行：同站≠同源，端口不同即不同源
        assert!(!is_same_origin_request(
            Some("http://127.0.0.1:8371"),
            Some("same-site"),
            Some("127.0.0.1:8371"),
            PORT,
            false,
        ));
    }

    #[test]
    fn same_origin_predicate_lan_mode_admits_private_ip_hosts() {
        // --host 0.0.0.0：手机经局域网 IP（192.168.x.x 等）访问嵌入式 Web UI
        assert!(is_same_origin_request(
            Some("http://192.168.1.5:8371"),
            Some("same-origin"),
            Some("192.168.1.5:8371"),
            PORT,
            true,
        ));
        // 地址栏直接打开（none）
        assert!(is_same_origin_request(
            None,
            Some("none"),
            Some("10.0.0.8:8371"),
            PORT,
            true,
        ));
        // 其他私网段与 IPv6 ULA / [::1]
        assert!(is_same_origin_request(
            None,
            None,
            Some("172.16.0.2:8371"),
            PORT,
            true,
        ));
        assert!(is_same_origin_request(
            None,
            None,
            Some("[fd00::1234]:8371"),
            PORT,
            true,
        ));
        assert!(is_same_origin_request(
            None,
            None,
            Some("[::1]:8371"),
            PORT,
            true,
        ));
        // 回环白名单在 LAN 模式下依然有效
        assert!(is_same_origin_request(
            None,
            None,
            Some("localhost:8371"),
            PORT,
            true,
        ));
    }

    #[test]
    fn same_origin_predicate_lan_mode_still_rejects_hostnames_and_public_ips() {
        // DNS rebinding：域名（哪怕解析到本机）不能冒充字面量 IP
        assert!(!is_same_origin_request(
            Some("http://attacker.example:8371"),
            Some("same-origin"),
            Some("attacker.example:8371"),
            PORT,
            true,
        ));
        // 公网 IP 不放行：--host 0.0.0.0 面向局域网，不鼓励暴露公网
        assert!(!is_same_origin_request(
            None,
            None,
            Some("8.8.8.8:8371"),
            PORT,
            true,
        ));
        // 端口不匹配
        assert!(!is_same_origin_request(
            None,
            None,
            Some("192.168.1.5:9999"),
            PORT,
            true,
        ));
        // 跨源 fetch 元数据标记在 LAN 模式下照旧拒绝
        assert!(!is_same_origin_request(
            Some("http://192.168.1.5:8371"),
            Some("cross-site"),
            Some("192.168.1.5:8371"),
            PORT,
            true,
        ));
        // 回环模式（默认）下私网 IP 不放行：白名单必须显式开启
        assert!(!is_same_origin_request(
            None,
            None,
            Some("192.168.1.5:8371"),
            PORT,
            false,
        ));
    }

    // ---------- /auth/bootstrap HTTP 层（Task 2.2，设计 §2） ----------

    const BOOTSTRAP_TEST_TOKEN: &str = "bootstrap-http-layer-token";

    /// 组装真实路由测 `/auth/bootstrap`（跟随 ws_push 测试先例）：走
    /// `create_routers` 而非只挂 public_router——同时验证端点在 public 组
    /// 真实可达、未被 protected 组的 auth 层拦截（页面加载先于 token
    /// 获取，请求本身不带 Authorization）。state 预置 api token + bind
    /// port，模拟 run() 完成 bind 后的初始化状态——bind_port 未设时处理
    /// 器 fail-closed，测试就测不到放行路径了。projects registry 指到
    /// 临时目录，隔离开发者的真实 projects.json。
    async fn bootstrap_app() -> Router {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.keep();
        let mut settings = crate::config::Settings::default();
        settings.storage.working_dir = root.clone();
        let mut state = DaemonState::new(crate::state::AppState::new(settings)).await;
        state.projects = crate::daemon::projects::ProjectRegistry::load(
            root.clone(),
            root.join("projects.json"),
        );
        let state = Arc::new(state);
        state.set_bind(PORT, false);
        state.set_api_token(BOOTSTRAP_TEST_TOKEN.to_string());
        let (health, protected) =
            crate::daemon::routes::create_routers(state, BOOTSTRAP_TEST_TOKEN.to_string());
        health.merge(protected)
    }

    /// 带任意头驱动 app（tower 的 `util` feature 未启用，无
    /// ServiceExt::oneshot，跟随本模块 drive 先例用原生 poll_ready + call）。
    async fn drive_with_headers(
        app: &mut Router,
        method: &str,
        uri: &str,
        headers: &[(&'static str, &str)],
    ) -> Response {
        use std::future::poll_fn;
        use tower::Service;

        let mut builder = axum::http::Request::builder().method(method).uri(uri);
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        poll_fn(|cx| {
            <axum::Router as Service<axum::http::Request<axum::body::Body>>>::poll_ready(app, cx)
        })
        .await
        .expect("router ready");
        app.call(
            builder
                .body(axum::body::Body::empty())
                .expect("valid request"),
        )
        .await
        .expect("call request")
    }

    /// 放行路径：同源头组合（Origin/Sec-Fetch-Site/Host 三检查全过）→ 200、
    /// 响应体含 token 字段且值是 state 预置的 api token、
    /// Cache-Control: no-store（token 绝不能进任何缓存）。
    #[tokio::test]
    async fn bootstrap_endpoint_returns_token_to_same_origin_request() {
        let mut app = bootstrap_app().await;
        let resp = drive_with_headers(
            &mut app,
            "GET",
            "/auth/bootstrap",
            &[
                ("origin", "http://127.0.0.1:8371"),
                ("sec-fetch-site", "same-origin"),
                ("host", "127.0.0.1:8371"),
            ],
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers()
                .get(header::CACHE_CONTROL)
                .and_then(|v| v.to_str().ok()),
            Some("no-store")
        );
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("read body");
        let text = std::str::from_utf8(&body).expect("utf-8 body");
        assert!(
            text.contains("\"token\""),
            "响应体必须含 token 字段: {text}"
        );
        assert!(
            text.contains(BOOTSTRAP_TEST_TOKEN),
            "token 值必须是 state 预置的 api token: {text}"
        );
    }

    /// 拒绝路径：跨源头组合（Origin 越界，即使 Host 合法、Fetch Metadata
    /// 标记 cross-site）→ 403 + 固定错误 JSON、响应体无 token。
    #[tokio::test]
    async fn bootstrap_endpoint_rejects_cross_origin_request() {
        let mut app = bootstrap_app().await;
        let resp = drive_with_headers(
            &mut app,
            "GET",
            "/auth/bootstrap",
            &[
                ("origin", "http://evil.example"),
                ("sec-fetch-site", "cross-site"),
                ("host", "127.0.0.1:8371"),
            ],
        )
        .await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("read body");
        let text = std::str::from_utf8(&body).expect("utf-8 body");
        assert_eq!(text, r#"{"error":"cross-origin request rejected"}"#);
        assert!(
            !text.contains("token"),
            "拒绝路径响应体不得含 token: {text}"
        );
    }
}
