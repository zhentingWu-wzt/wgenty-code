//! Web UI 托管边界集成测试（daemon-hosted-web-ui Task 3.1）。
//!
//! 在真实 `routes::create_routers` 组装出的最终 app 上验证静态托管边界
//! （与 `mod.rs::run()` 相同组装形态：merge 后挂 `.fallback(web_ui::spa_fallback)`）：
//! - 入口 HTML `no-cache` 与 hashed 资产 `immutable` 缓存头经真实路由层叠后仍然成立
//! - fallback 与 API 404 共存：未知 `/api/` 路径得 JSON 404 而非 SPA 壳
//! - SPA 深链由 fallback 兜底回入口 HTML；非 GET 深链 405
//!
//! 为何不直接复用 `daemon_harness::spawn_daemon`：harness 的 app 组装停在
//! `health.merge(protected)`，没有挂生产在 `run()` 里挂的 SPA fallback——
//! fallback 边界必须在相同组装形态下验证才有意义。
//!
//! dist 两形态（设计 §1/§6）：CI 可能未构建过 `web/dist`（rust-embed debug
//! 构建运行时读盘），断言按 `web/dist/index.html` 是否存在分流——有真
//! index 时逐字节比对，无产物时命中内联降级页。降级页文案细节已由
//! `web_ui.rs` 单测（`serve_index_without_index_html_serves_degradation_page`）
//! 覆盖，此处只断言托管边界（200 + HTML + no-cache）。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use wgenty_code::config::Settings;
use wgenty_code::daemon::routes;
use wgenty_code::daemon::state::DaemonState;
use wgenty_code::daemon::web_ui;
use wgenty_code::state::AppState;

/// 启动生产形态的 web 托管 app（create_routers + merge + spa_fallback），
/// 返回根 URL（无 `/api/v1` 前缀——静态路由挂在根下）。
async fn spawn_web_daemon() -> (String, tempfile::TempDir) {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut settings = Settings::default();
    settings.storage.working_dir = temp.path().to_path_buf();
    let mut state = DaemonState::new(AppState::new(settings)).await;
    // 隔离项目注册表，避免读到开发者的真实 projects.json（同 harness）。
    state.projects = wgenty_code::daemon::projects::ProjectRegistry::load(
        temp.path().to_path_buf(),
        temp.path().join("projects.json"),
    );
    let state = Arc::new(state);
    let (health, protected) = routes::create_routers(state, "web-boundary-test-token".into());
    // 与 mod.rs::run() 一致的最终形态：fallback 挂在 merge 后的最终 app 上，
    // 位于 protected 组 auth route_layer 之外——静态深链公开可达。
    let app = health.merge(protected).fallback(web_ui::spa_fallback);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve web daemon");
    });
    (format!("http://{addr}"), temp)
}

/// `web/dist/index.html` 的磁盘内容（若已构建）。
fn dist_index_on_disk() -> Option<String> {
    std::fs::read_to_string(Path::new("web/dist/index.html")).ok()
}

/// 在 `web/dist/assets` 下找一个 `.js` 产物（vite hashed 输出）。
fn first_dist_js_asset() -> Option<PathBuf> {
    std::fs::read_dir(Path::new("web/dist/assets"))
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "js"))
}

/// 断言 body 是入口 HTML 的两种形态之一：真 index（与磁盘逐字节一致）或
/// 内联降级页（dist 未构建，CI 常态）。
fn assert_entry_html(body: &str) {
    if let Some(disk) = dist_index_on_disk() {
        assert_eq!(body, disk, "GET / must serve web/dist/index.html verbatim");
    } else {
        assert!(
            body.contains("Web UI not bundled"),
            "empty dist must hit the inline degradation page, got: {body}"
        );
        assert!(body.contains("npm --prefix web run build"));
    }
}

#[tokio::test]
async fn root_serves_html_entry_with_no_cache() {
    let (base, _temp) = spawn_web_daemon().await;
    let resp = reqwest::get(format!("{base}/")).await.expect("GET /");
    assert_eq!(resp.status(), reqwest::StatusCode::OK, "GET / status");
    let ct = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .expect("content-type present");
    assert!(ct.starts_with("text/html"), "GET / must be HTML, got {ct}");
    // 入口 HTML 必须每次重新验证，否则发新版后浏览器仍引用已删除的
    // hashed 资产（设计 §1）。
    assert_eq!(
        resp.headers()
            .get(reqwest::header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok()),
        Some("no-cache")
    );
    let body = resp.text().await.expect("body");
    assert_entry_html(&body);
}

#[tokio::test]
async fn hashed_js_asset_is_served_immutable() {
    // 条件用例：CI 未构建 dist 时无产物可服务（serve_asset 对未命中路径
    // 的 404 分支已由 web_ui.rs 单测覆盖），跳过而非失败。
    let Some(asset) = first_dist_js_asset() else {
        eprintln!("skipped: web/dist/assets has no build output");
        return;
    };
    let name = asset.file_name().expect("asset file name");
    let (base, _temp) = spawn_web_daemon().await;
    let resp = reqwest::get(format!("{base}/assets/{}", name.to_string_lossy()))
        .await
        .expect("GET /assets/<hash>.js");
    assert_eq!(resp.status(), reqwest::StatusCode::OK, "asset status");
    let ct = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .expect("asset content-type");
    assert!(
        ct.starts_with("text/javascript"),
        "js asset MIME must be text/javascript, got {ct}"
    );
    // 文件名带内容 hash → 可安全一年长缓存（设计 §1）。
    assert_eq!(
        resp.headers()
            .get(reqwest::header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok()),
        Some("public, max-age=31536000, immutable")
    );
    let bytes = resp.bytes().await.expect("asset bytes");
    let disk = std::fs::read(&asset).expect("read disk asset");
    assert_eq!(&bytes[..], &disk[..], "asset bytes must match disk");
}

#[tokio::test]
async fn unknown_api_path_gets_json_404_not_spa_html() {
    let (base, _temp) = spawn_web_daemon().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{base}/api/v1/nonexistent"))
        .send()
        .await
        .expect("GET unknown api path");
    assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);
    assert_eq!(
        resp.headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("application/json"),
        "unknown API path must return JSON, never the SPA shell"
    );
    let body = resp.text().await.expect("body");
    assert!(
        !body.contains('<'),
        "unknown API path must never serve HTML, got: {body}"
    );
    let v: serde_json::Value = serde_json::from_str(&body).expect("404 body is JSON");
    assert!(v.get("error").is_some(), "error field, got {v}");

    // 分支顺序（设计 §1）：/api/ 前缀判定优先于 method 检查——POST 未知
    // API 路径同样是 404 JSON，而不是深链的 405。
    let resp = client
        .post(format!("{base}/api/v1/nonexistent"))
        .send()
        .await
        .expect("POST unknown api path");
    assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);
    assert_eq!(
        resp.headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("application/json")
    );
}

#[tokio::test]
async fn spa_deep_link_serves_index_fallback() {
    let (base, _temp) = spawn_web_daemon().await;
    let resp = reqwest::get(format!("{base}/some/deep/link"))
        .await
        .expect("GET deep link");
    assert_eq!(resp.status(), reqwest::StatusCode::OK, "deep link status");
    let ct = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .expect("content-type present");
    assert!(
        ct.starts_with("text/html"),
        "fallback must be HTML, got {ct}"
    );
    // fallback 复用 serve_index：同样 no-cache。
    assert_eq!(
        resp.headers()
            .get(reqwest::header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok()),
        Some("no-cache")
    );
    let body = resp.text().await.expect("body");
    assert_entry_html(&body);
}

#[tokio::test]
async fn non_get_deep_link_is_method_not_allowed() {
    let (base, _temp) = spawn_web_daemon().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{base}/foo"))
        .send()
        .await
        .expect("POST deep link");
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::METHOD_NOT_ALLOWED,
        "fallback only backs page deep links (GET), carries no write semantics"
    );

    // 对照：同路径 GET 由 fallback 兜底为 200 入口页。
    let resp = client.get(format!("{base}/foo")).send().await.expect("GET");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
}
