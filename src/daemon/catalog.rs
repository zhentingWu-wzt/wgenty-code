use crate::api::provider::resolve_provider;
use crate::config::models::ModelTier;
use crate::config::Settings;
use serde::Deserialize;
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

const CATALOG_TTL: Duration = Duration::from_secs(300);

/// Built-in model fallbacks, merged with provider responses and manual settings.
pub const BUILTIN_CATALOGS: &[(&str, &[&str])] = &[
    ("kimi", &["kimi-k2.5", "kimi-k2-thinking", "kimi-latest"]),
    ("zhipu", &["glm-5.3", "glm-5.3-flash", "glm-5.3-air"]),
    (
        "deepseek",
        &["deepseek-v4-pro", "deepseek-chat", "deepseek-reasoner"],
    ),
    ("openai", &["gpt-5.2", "gpt-5.2-codex", "gpt-5-mini"]),
    (
        "anthropic",
        &["claude-sonnet-4-5", "claude-opus-4-5", "claude-haiku-4-5"],
    ),
];

/// One deduplicated row in a provider's effective model catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergedModel {
    pub name: String,
    pub tier: Option<ModelTier>,
    pub configured: bool,
}

#[derive(Clone, PartialEq, Eq)]
struct CatalogIdentity {
    base_url: String,
    protocol: String,
    credential_fingerprint: u64,
}

impl CatalogIdentity {
    fn new(base_url: &str, protocol: &str, api_key: &str) -> Self {
        let mut hasher = DefaultHasher::new();
        api_key.hash(&mut hasher);
        Self {
            base_url: normalize_base_url(base_url),
            protocol: protocol.to_string(),
            credential_fingerprint: hasher.finish(),
        }
    }
}

fn normalize_base_url(base_url: &str) -> String {
    base_url.trim().trim_end_matches('/').to_string()
}

#[derive(Clone)]
struct CatalogEntry {
    identity: CatalogIdentity,
    fetched: Option<Vec<String>>,
    at: Instant,
}

/// Lazily fetched provider catalogs with a five-minute TTL and per-provider
/// singleflight. Cache-map locks are synchronous and never held across await;
/// only the provider-specific singleflight lock spans the HTTP request.
pub struct ModelCatalogService {
    http: reqwest::Client,
    cache: RwLock<HashMap<String, CatalogEntry>>,
    provider_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl ModelCatalogService {
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            http,
            cache: RwLock::new(HashMap::new()),
            provider_locks: Mutex::new(HashMap::new()),
        }
    }

    /// Merge manual entries (which win and carry their tier), fetched IDs, and
    /// the static built-in fallback into deterministic name order.
    pub async fn merged(&self, settings: &Settings, provider_key: &str) -> Vec<MergedModel> {
        let fetched = self.fetched(settings, provider_key).await;
        let mut models = BTreeMap::new();
        if let Some(group) = settings.models.providers.get(provider_key) {
            for model in &group.models {
                models.insert(
                    model.name.clone(),
                    MergedModel {
                        name: model.name.clone(),
                        tier: model.tier,
                        configured: true,
                    },
                );
            }
        }
        if let Some(fetched) = fetched {
            for name in fetched {
                models.entry(name.clone()).or_insert(MergedModel {
                    name,
                    tier: None,
                    configured: false,
                });
            }
        }
        if let Some(builtin) = BUILTIN_CATALOGS
            .iter()
            .find_map(|(provider, models)| (*provider == provider_key).then_some(*models))
        {
            for name in builtin {
                models.entry((*name).to_string()).or_insert(MergedModel {
                    name: (*name).to_string(),
                    tier: None,
                    configured: false,
                });
            }
        }
        models.into_values().collect()
    }

    /// Resolve either a manual endpoint or a model proven to exist in the
    /// merged catalog. Unconfigured entries inherit group URL and credentials.
    pub async fn resolve_endpoint(
        &self,
        settings: &Settings,
        provider_key: &str,
        model: &str,
    ) -> Option<crate::config::models::ModelEndpoint> {
        if let Some(endpoint) = settings.models.resolve_endpoint(provider_key, model) {
            return Some(endpoint);
        }
        let merged = self.merged(settings, provider_key).await;
        let catalog_model = merged.into_iter().find(|entry| entry.name == model)?;
        let mut endpoint = settings
            .models
            .resolve_unconfigured_endpoint(provider_key, model)?;
        endpoint.tier = catalog_model.tier;
        Some(endpoint)
    }

    fn fresh(&self, provider_key: &str, identity: &CatalogIdentity) -> Option<Option<Vec<String>>> {
        self.cache
            .read()
            .expect("catalog cache lock poisoned")
            .get(provider_key)
            .filter(|entry| entry.identity == *identity && entry.at.elapsed() < CATALOG_TTL)
            .map(|entry| entry.fetched.clone())
    }

    async fn fetched(&self, settings: &Settings, provider_key: &str) -> Option<Vec<String>> {
        let group = settings.models.providers.get(provider_key)?;
        let provider = resolve_provider(&group.base_url, group.protocol.as_deref());
        if !provider.is_openai_compat() || group.base_url.trim().is_empty() {
            return None;
        }
        let api_key = settings.models.resolve_provider_api_key(provider_key)?;
        let identity = CatalogIdentity::new(&group.base_url, provider.name(), &api_key);
        if let Some(fresh) = self.fresh(provider_key, &identity) {
            return fresh;
        }

        let provider_lock = {
            let mut locks = self
                .provider_locks
                .lock()
                .expect("catalog provider locks poisoned");
            locks
                .entry(provider_key.to_string())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        let _guard = provider_lock.lock().await;
        if let Some(fresh) = self.fresh(provider_key, &identity) {
            return fresh;
        }

        let previous = self
            .cache
            .read()
            .expect("catalog cache lock poisoned")
            .get(provider_key)
            .filter(|entry| entry.identity == identity)
            .cloned();
        let fetched = self
            .fetch_models(&identity.base_url, &api_key, provider_key)
            .await;
        let entry = CatalogEntry {
            identity,
            fetched: fetched.or_else(|| previous.and_then(|entry| entry.fetched)),
            at: Instant::now(),
        };
        let result = entry.fetched.clone();
        self.cache
            .write()
            .expect("catalog cache lock poisoned")
            .insert(provider_key.to_string(), entry);
        result
    }

    async fn fetch_models(
        &self,
        base_url: &str,
        api_key: &str,
        provider_key: &str,
    ) -> Option<Vec<String>> {
        let url = format!("{}/models", base_url.trim_end_matches('/'));
        let response = match self.http.get(url).bearer_auth(api_key).send().await {
            Ok(response) => response,
            Err(_) => {
                tracing::warn!(provider = provider_key, "model catalog request failed");
                return None;
            }
        };
        if !response.status().is_success() {
            tracing::warn!(
                provider = provider_key,
                status = %response.status(),
                "model catalog request returned an error"
            );
            return None;
        }
        let payload = match response.json::<OpenAiModelsResponse>().await {
            Ok(payload) => payload,
            Err(_) => {
                tracing::warn!(
                    provider = provider_key,
                    "model catalog response was invalid"
                );
                return None;
            }
        };
        Some(payload.data.into_iter().map(|model| model.id).collect())
    }
}

#[derive(Deserialize)]
struct OpenAiModelsResponse {
    data: Vec<OpenAiModel>,
}

#[derive(Deserialize)]
struct OpenAiModel {
    id: String,
}

#[cfg(test)]
pub(crate) mod test_support {
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::Json;
    use axum::{routing::get, Router};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, RwLock};
    use std::time::Duration;

    #[derive(Clone)]
    pub(crate) struct MockReply {
        pub(crate) status: u16,
        pub(crate) models: Vec<String>,
    }

    struct MockState {
        reply: RwLock<MockReply>,
        delay: Duration,
        hits: AtomicUsize,
        authorizations: RwLock<Vec<Option<String>>>,
    }

    pub(crate) struct MockCatalogServer {
        pub(crate) base_url: String,
        state: Arc<MockState>,
        task: tokio::task::JoinHandle<()>,
    }

    impl MockCatalogServer {
        pub(crate) async fn start(status: u16, models: &[&str]) -> Self {
            Self::start_with_delay(status, models, Duration::ZERO).await
        }

        pub(crate) async fn start_with_delay(
            status: u16,
            models: &[&str],
            delay: Duration,
        ) -> Self {
            let state = Arc::new(MockState {
                reply: RwLock::new(MockReply {
                    status,
                    models: models.iter().map(|model| (*model).to_string()).collect(),
                }),
                delay,
                hits: AtomicUsize::new(0),
                authorizations: RwLock::new(Vec::new()),
            });
            let app = Router::new()
                .route("/models", get(serve_models))
                .with_state(state.clone());
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind local catalog mock");
            let address = listener.local_addr().expect("catalog mock address");
            let task = tokio::spawn(async move {
                axum::serve(listener, app)
                    .await
                    .expect("local catalog mock serves");
            });
            Self {
                base_url: format!("http://{address}"),
                state,
                task,
            }
        }

        pub(crate) fn set_reply(&self, status: u16, models: &[&str]) {
            *self.state.reply.write().expect("mock reply lock") = MockReply {
                status,
                models: models.iter().map(|model| (*model).to_string()).collect(),
            };
        }

        pub(crate) fn hits(&self) -> usize {
            self.state.hits.load(Ordering::SeqCst)
        }

        pub(crate) fn authorizations(&self) -> Vec<Option<String>> {
            self.state
                .authorizations
                .read()
                .expect("mock authorization lock")
                .clone()
        }
    }

    impl Drop for MockCatalogServer {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    async fn serve_models(
        State(state): State<Arc<MockState>>,
        headers: HeaderMap,
    ) -> (StatusCode, Json<serde_json::Value>) {
        state.hits.fetch_add(1, Ordering::SeqCst);
        state
            .authorizations
            .write()
            .expect("mock authorization lock")
            .push(
                headers
                    .get(axum::http::header::AUTHORIZATION)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string),
            );
        if !state.delay.is_zero() {
            tokio::time::sleep(state.delay).await;
        }
        let reply = state.reply.read().expect("mock reply lock").clone();
        let data: Vec<serde_json::Value> = reply
            .models
            .iter()
            .map(|id| {
                serde_json::json!({
                    "id": id,
                    "object": "model",
                    "created": 1_700_000_000,
                    "owned_by": "local-test-provider"
                })
            })
            .collect();
        (
            StatusCode::from_u16(reply.status).expect("valid mock status"),
            Json(serde_json::json!({"object": "list", "data": data})),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{MergedModel, ModelCatalogService};
    use crate::config::models::{ModelTier, ProviderGroup, ProviderModel};
    use crate::config::Settings;
    use std::sync::Arc;
    use std::time::Duration;

    fn settings(provider: &str, base_url: &str, protocol: &str, api_key: Option<&str>) -> Settings {
        let mut settings = Settings::default();
        settings.models.providers.insert(
            provider.to_string(),
            ProviderGroup {
                display_name: None,
                base_url: base_url.to_string(),
                api_key: api_key.map(str::to_string),
                protocol: Some(protocol.to_string()),
                models: vec![ProviderModel {
                    name: if provider == "anthropic" {
                        "claude-sonnet-4-5"
                    } else {
                        "glm-5.3"
                    }
                    .to_string(),
                    profile_key: None,
                    tier: Some(ModelTier::Heavy),
                    api_key: None,
                    base_url: None,
                    context_window: None,
                    temperature: None,
                }],
            },
        );
        settings
    }

    fn by_name<'a>(models: &'a [MergedModel], name: &str) -> &'a MergedModel {
        models
            .iter()
            .find(|model| model.name == name)
            .unwrap_or_else(|| panic!("missing merged model {name}"))
    }

    #[tokio::test]
    async fn catalog_merges_fetched_builtin_and_manual() {
        let server = super::test_support::MockCatalogServer::start(
            200,
            &["glm-5.3", "glm-5.3-flash", "new-model"],
        )
        .await;
        let settings = settings("zhipu", &server.base_url, "openai", Some("test-key"));
        let service = ModelCatalogService::new(reqwest::Client::new());

        let merged = service.merged(&settings, "zhipu").await;

        let names: Vec<&str> = merged.iter().map(|model| model.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["glm-5.3", "glm-5.3-air", "glm-5.3-flash", "new-model"]
        );
        assert!(by_name(&merged, "glm-5.3").configured);
        assert_eq!(by_name(&merged, "glm-5.3").tier, Some(ModelTier::Heavy));
        assert!(!by_name(&merged, "glm-5.3-air").configured);
        assert!(!by_name(&merged, "glm-5.3-flash").configured);
        assert!(!by_name(&merged, "new-model").configured);
        assert_eq!(server.hits(), 1);
        assert_eq!(
            server.authorizations(),
            vec![Some("Bearer test-key".to_string())]
        );
    }

    #[tokio::test]
    async fn catalog_failure_falls_back_and_caches() {
        let server = super::test_support::MockCatalogServer::start(500, &[]).await;
        let settings = settings("zhipu", &server.base_url, "openai", Some("test-key"));
        let service = ModelCatalogService::new(reqwest::Client::new());

        let fallback = service.merged(&settings, "zhipu").await;
        let names: Vec<&str> = fallback.iter().map(|model| model.name.as_str()).collect();
        assert_eq!(names, vec!["glm-5.3", "glm-5.3-air", "glm-5.3-flash"]);
        assert!(by_name(&fallback, "glm-5.3").configured);

        server.set_reply(200, &["new-model"]);
        let cached = service.merged(&settings, "zhipu").await;
        assert_eq!(cached.len(), fallback.len());
        assert!(!cached.iter().any(|model| model.name == "new-model"));
        assert_eq!(server.hits(), 1, "failure result must obey the same TTL");
    }

    #[tokio::test]
    async fn catalog_ttl_cache_hit_avoids_refetch() {
        let server = super::test_support::MockCatalogServer::start(200, &["first-model"]).await;
        let settings = settings("zhipu", &server.base_url, "openai", Some("test-key"));
        let service = ModelCatalogService::new(reqwest::Client::new());

        let first = service.merged(&settings, "zhipu").await;
        assert!(first.iter().any(|model| model.name == "first-model"));
        server.set_reply(200, &["second-model"]);
        let second = service.merged(&settings, "zhipu").await;

        assert!(second.iter().any(|model| model.name == "first-model"));
        assert!(!second.iter().any(|model| model.name == "second-model"));
        assert_eq!(server.hits(), 1);
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn catalog_skips_anthropic_and_missing_key() {
        let server = super::test_support::MockCatalogServer::start(200, &["new-model"]).await;
        let anthropic = settings("anthropic", &server.base_url, "anthropic", Some("test-key"));
        let service = ModelCatalogService::new(reqwest::Client::new());
        let merged = service.merged(&anthropic, "anthropic").await;
        assert!(!merged.iter().any(|model| model.name == "new-model"));
        assert_eq!(server.hits(), 0);

        let variables = [
            "DEEPSEEK_API_KEY",
            "ANTHROPIC_API_KEY",
            "DASHSCOPE_API_KEY",
            "ZHIPU_API_KEY",
        ];
        let saved: Vec<(&str, Option<std::ffi::OsString>)> = variables
            .iter()
            .map(|key| (*key, std::env::var_os(key)))
            .collect();
        for key in variables {
            std::env::remove_var(key);
        }
        let no_key = settings("zhipu", &server.base_url, "openai", None);
        let merged = service.merged(&no_key, "zhipu").await;
        for (key, value) in saved {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
        assert!(!merged.iter().any(|model| model.name == "new-model"));
        assert_eq!(server.hits(), 0);
    }

    #[tokio::test]
    async fn catalog_openai_compat_alias_fetches_models() {
        let server = super::test_support::MockCatalogServer::start(200, &["alias-model"]).await;
        let settings = settings("zhipu", &server.base_url, "openai-compat", Some("test-key"));
        let service = ModelCatalogService::new(reqwest::Client::new());

        let merged = service.merged(&settings, "zhipu").await;

        assert!(merged.iter().any(|model| model.name == "alias-model"));
        assert_eq!(server.hits(), 1);
    }

    #[tokio::test]
    async fn catalog_openai_compatible_alias_fetches_models() {
        let server = super::test_support::MockCatalogServer::start(200, &["alias-model"]).await;
        let settings = settings(
            "zhipu",
            &server.base_url,
            "openai_compatible",
            Some("test-key"),
        );
        let service = ModelCatalogService::new(reqwest::Client::new());

        let merged = service.merged(&settings, "zhipu").await;

        assert!(merged.iter().any(|model| model.name == "alias-model"));
        assert_eq!(server.hits(), 1);
    }

    #[tokio::test]
    async fn catalog_unknown_protocol_uses_url_fallback() {
        let server = super::test_support::MockCatalogServer::start(200, &["fallback-model"]).await;
        let settings = settings(
            "zhipu",
            &server.base_url,
            "unknown-protocol",
            Some("test-key"),
        );
        let service = ModelCatalogService::new(reqwest::Client::new());

        let merged = service.merged(&settings, "zhipu").await;

        assert!(merged.iter().any(|model| model.name == "fallback-model"));
        assert_eq!(server.hits(), 1);
    }

    #[tokio::test]
    async fn catalog_different_url_misses_cache_and_drops_stale_models() {
        let server_a = super::test_support::MockCatalogServer::start(200, &["a-only-model"]).await;
        let server_b = super::test_support::MockCatalogServer::start(500, &[]).await;
        let settings_a = settings("zhipu", &server_a.base_url, "openai", Some("key-a"));
        let settings_b = settings("zhipu", &server_b.base_url, "openai", Some("key-b"));
        let service = ModelCatalogService::new(reqwest::Client::new());

        let first = service.merged(&settings_a, "zhipu").await;
        assert!(first.iter().any(|model| model.name == "a-only-model"));
        let second = service.merged(&settings_b, "zhipu").await;

        assert_eq!(server_b.hits(), 1, "a new URL must not reuse A's cache");
        assert!(!second.iter().any(|model| model.name == "a-only-model"));
    }

    #[tokio::test]
    async fn catalog_different_key_misses_cache_and_drops_stale_models() {
        let server = super::test_support::MockCatalogServer::start(200, &["a-only-model"]).await;
        let settings_a = settings("zhipu", &server.base_url, "openai", Some("key-a"));
        let settings_b = settings("zhipu", &server.base_url, "openai", Some("key-b"));
        let service = ModelCatalogService::new(reqwest::Client::new());

        let first = service.merged(&settings_a, "zhipu").await;
        assert!(first.iter().any(|model| model.name == "a-only-model"));
        server.set_reply(500, &[]);
        let second = service.merged(&settings_b, "zhipu").await;

        assert_eq!(server.hits(), 2, "a new key must not reuse A's cache");
        assert!(!second.iter().any(|model| model.name == "a-only-model"));
    }

    #[tokio::test]
    async fn catalog_singleflight_coalesces_provider_requests() {
        let server = super::test_support::MockCatalogServer::start_with_delay(
            200,
            &["new-model"],
            Duration::from_millis(50),
        )
        .await;
        let settings = Arc::new(settings(
            "zhipu",
            &server.base_url,
            "openai",
            Some("test-key"),
        ));
        let service = Arc::new(ModelCatalogService::new(reqwest::Client::new()));

        let mut tasks = Vec::new();
        for _ in 0..8 {
            let service = service.clone();
            let settings = settings.clone();
            tasks.push(tokio::spawn(async move {
                service.merged(&settings, "zhipu").await
            }));
        }
        for task in tasks {
            let merged = task.await.expect("catalog task");
            assert!(merged.iter().any(|model| model.name == "new-model"));
        }
        assert_eq!(server.hits(), 1, "concurrent misses must share one fetch");
    }
}
