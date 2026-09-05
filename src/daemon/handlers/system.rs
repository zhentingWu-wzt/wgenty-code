//! System, config and model handlers.

use super::*;

pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

// ── Shutdown ─────────────────────────────────────────────────────────────────

/// POST /api/v1/shutdown — request a graceful daemon shutdown. Backs
/// `wgenty-code daemon stop`; protected by the same bearer token as every
/// other non-health endpoint. The server's shutdown future listens on
/// `shutdown_notify` and runs the normal cleanup (token/discovery files).
pub async fn shutdown(State(state): State<Arc<DaemonState>>) -> Json<serde_json::Value> {
    tracing::info!("shutdown requested via API");
    state.shutdown_notify.notify_one();
    Json(serde_json::json!({ "shutting_down": true }))
}

// ── Config ───────────────────────────────────────────────────────────────────

pub async fn get_config(State(state): State<Arc<DaemonState>>) -> Json<ConfigResponse> {
    // Read from the live handle so a `/model` switch is reflected here too.
    let s = state
        .settings_handle
        .read()
        .expect("lock poisoned: settings");
    Json(ConfigResponse {
        model: s.models.main.name.clone(),
        api_base: s.models.main.endpoint_base_url(),
        max_tokens: s.models.transport.max_tokens,
        timeout: s.models.transport.timeout,
        streaming: s.models.transport.streaming,
        context_window: s.models.context_window,
    })
}

/// `PUT /api/v1/config` — partial update of transport-level settings.
///
/// Mirrors the `switch_model` write pattern: validate in a clone → persist via
/// `load_from_disk()` + `save()` (to keep relative working_dir) → overwrite the
/// live handle → broadcast a global event. Only `max_tokens`, `timeout`,
/// `streaming`, and `api_base` are editable; api_key/appkey are never accepted.
pub async fn update_config(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<UpdateConfigRequest>,
) -> Result<Json<ConfigResponse>, (StatusCode, String)> {
    // 1. Validate fields in a clone of the live settings.
    let mut settings = state
        .settings_handle
        .read()
        .expect("lock poisoned: settings")
        .clone();

    // 0 = omit `max_tokens` from provider requests (provider default output
    // limit); any positive value caps output explicitly.
    if let Some(mt) = body.max_tokens {
        settings.models.transport.max_tokens = mt;
    }
    if let Some(t) = body.timeout {
        if t == 0 {
            return Err((StatusCode::BAD_REQUEST, "timeout must be > 0".into()));
        }
        settings.models.transport.timeout = t;
    }
    if let Some(s) = body.streaming {
        settings.models.transport.streaming = s;
    }
    if let Some(ref base) = body.api_base {
        settings.models.main.base_url = if base.trim().is_empty() {
            None
        } else {
            Some(base.clone())
        };
    }

    // 2. Persist via disk-load form (avoids writing resolved absolute working_dir).
    let mut disk = crate::config::Settings::load_from_disk().unwrap_or_else(|_| settings.clone());
    disk.models.transport = settings.models.transport.clone();
    if body.api_base.is_some() {
        disk.models.main.base_url = settings.models.main.base_url.clone();
    }
    if let Err(e) = disk.save() {
        tracing::warn!(error = %e, "failed to persist settings.json after config update");
    }

    // 3. Overwrite the live handle so the next request sees the change.
    *state
        .settings_handle
        .write()
        .expect("lock poisoned: settings") = settings;

    tracing::info!("config updated via PUT /config");

    // 4. Broadcast so connected SSE clients refresh.
    state.broadcast_global(
        crate::daemon::global_events::GlobalEventKind::ConfigChanged,
        serde_json::json!({}),
    );

    // 5. Return the new ConfigResponse (no api_key).
    let s = state
        .settings_handle
        .read()
        .expect("lock poisoned: settings");
    Ok(Json(ConfigResponse {
        model: s.models.main.name.clone(),
        api_base: s.models.main.endpoint_base_url(),
        max_tokens: s.models.transport.max_tokens,
        timeout: s.models.transport.timeout,
        streaming: s.models.transport.streaming,
        context_window: s.models.context_window,
    }))
}

/// GET /api/v1/models - list switchable model profiles for the `/model` picker.
/// Always includes the currently active one (marked `active: true`). If
/// `models.profiles` is empty, returns an empty list (picker can show a hint).
pub async fn list_models(State(state): State<Arc<DaemonState>>) -> Json<ListModelsResponse> {
    let s = state
        .settings_handle
        .read()
        .expect("lock poisoned: settings");

    // The active profile is whichever profile is currently installed in
    // `models.main` (`switch_to_profile` copies a profile's full endpoint into
    // `main`). Matching by model name -- rather than the persisted
    // `active_profile` key -- keeps the picker correct when `main` was changed
    // out-of-band (manual config edit, env override, or a fresh config where
    // `active_profile` is still `None` but `main` already matches a profile).
    // When several profiles share the same model name, the persisted
    // `active_profile` key disambiguates.
    let main_name = s.models.main.name.as_str();
    let active_profile_key = s.models.active_profile.as_deref();
    let matching: Vec<&String> = s
        .models
        .profiles
        .iter()
        .filter(|(_, ep)| ep.name == main_name)
        .map(|(k, _)| k)
        .collect();
    let active_key: Option<&str> = match matching.len() {
        0 => None,
        1 => Some(matching[0].as_str()),
        _ => active_profile_key
            .filter(|k| matching.iter().any(|m| m.as_str() == *k))
            .or_else(|| matching.first().map(|k| k.as_str())),
    };

    let mut profiles: Vec<ModelProfileInfo> = s
        .models
        .profiles
        .iter()
        .map(|(key, ep)| ModelProfileInfo {
            key: key.clone(),
            label: ep.display_name.clone().unwrap_or_else(|| ep.name.clone()),
            model_name: ep.name.clone(),
            provider: ep.provider.clone(),
            tier: ep.tier.map(|t| {
                match t {
                    crate::config::models::ModelTier::Light => "light",
                    crate::config::models::ModelTier::Medium => "medium",
                    crate::config::models::ModelTier::Heavy => "heavy",
                }
                .to_string()
            }),
            active: active_key == Some(key.as_str()),
        })
        .collect();
    // Stable, alphabetical ordering for a predictable picker.
    profiles.sort_by(|a, b| a.key.cmp(&b.key));
    Json(ListModelsResponse { profiles })
}

/// POST /api/v1/model/switch - activate a named profile. Copies the profile
/// endpoint into `models.main`, records `active_profile`, persists to disk,
/// and updates the live handle so the next chat turn uses the new model.
///
/// Returns 400 with an actionable message (listing available profiles) when
/// the profile key is unknown.
pub async fn switch_model(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<SwitchModelRequest>,
) -> Result<Json<SwitchModelResponse>, (StatusCode, String)> {
    // 1. Read current settings, switch in a clone, validate the profile exists.
    let mut settings = state
        .settings_handle
        .read()
        .expect("lock poisoned: settings")
        .clone();
    settings
        .switch_to_profile(&body.profile)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("{e:#}")))?;

    // 2. Persist so the choice survives a restart. Use the disk-load form to
    //    avoid writing back a runtime-resolved absolute working_dir.
    let mut disk = crate::config::Settings::load_from_disk().unwrap_or_else(|_| settings.clone());
    disk.switch_to_profile(&body.profile)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
    if let Err(e) = disk.save() {
        tracing::warn!(error = %e, "failed to persist settings.json after model switch");
    }

    let label = settings.main_model_label();
    let model_name = settings.models.main.name.clone();
    let provider = settings.models.main.provider.clone();

    // 3. Publish to the live handle — next chat_stream reads this.
    *state
        .settings_handle
        .write()
        .expect("lock poisoned: settings") = settings;

    tracing::info!(
        profile = %body.profile,
        model = %model_name,
        provider = ?provider,
        "model switched via /model"
    );

    state.broadcast_global(
        crate::daemon::global_events::GlobalEventKind::ModelChanged,
        serde_json::json!({
            "profile": body.profile,
            "model_name": model_name,
            "provider": provider,
        }),
    );

    Ok(Json(SwitchModelResponse {
        success: true,
        profile: body.profile,
        label,
        model_name,
        provider,
    }))
}

// ── Chat / Stream ────────────────────────────────────────────────────────────

/// SSE keepalive endpoint for thin clients. Register on connect, unregister
/// on disconnect, triggering graceful daemon shutdown when all clients leave.
pub async fn client_heartbeat(
    State(state): State<Arc<DaemonState>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let registered = state.active_clients.register_client();
    if !registered {
        // Daemon is already shutting down; return an empty stream that
        // immediately closes so the client knows to reconnect later.
        let (tx, rx) = mpsc::unbounded_channel::<Result<Event, Infallible>>();
        let shutdown_event = Event::default()
            .event("shutting_down")
            .data("daemon is shutting down");
        let _ = tx.send(Ok(shutdown_event));
        drop(tx);
        return Sse::new(UnboundedReceiverStream::new(rx));
    }

    tracing::info!(
        clients = state.active_clients.client_count(),
        "thin client connected"
    );

    let (tx, rx) = mpsc::unbounded_channel::<Result<Event, Infallible>>();
    let active_clients = state.active_clients.clone();

    // Background task: send periodic pings. When the client disconnects
    // (rx dropped), this task exits and the Drop guard unregisters the client.
    tokio::spawn(async move {
        let status = Event::default()
            .event("status")
            .data(serde_json::json!({"clients": active_clients.client_count()}).to_string());
        let _ = tx.send(Ok(status));

        // Periodic keepalive pings. When the client disconnects (rx dropped),
        // `tx.send` fails immediately, so we detect disconnection quickly.
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        // Skip the first tick (interval fires immediately).
        interval.tick().await;
        loop {
            interval.tick().await;
            if tx.is_closed() {
                break;
            }
            let ping = Event::default().comment("ping");
            if tx.send(Ok(ping)).is_err() {
                break;
            }
        }

        // Client disconnected — unregister.
        active_clients.unregister_client();
        tracing::info!(
            clients = active_clients.client_count(),
            "thin client disconnected"
        );
    });

    Sse::new(UnboundedReceiverStream::new(rx)).keep_alive(KeepAlive::default())
}
