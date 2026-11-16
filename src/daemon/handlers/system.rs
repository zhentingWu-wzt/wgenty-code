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

/// GET /api/v1/models (v2) — vendor-grouped model catalog for the `/model`
/// picker: `models.providers` groups sorted by key, models sorted by name.
/// Manual, fetched, and built-in entries are deduplicated; only manual entries
/// are `configured`. With `?session_id=`, `active` follows the session's
/// synthesized selection (`resolve_session_model`) — a resolvable override's
/// (provider, model) pair is the only marked row; without it (or without the
/// param) the global `models.active_model` pair is marked.
pub async fn list_models(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<ListModelsQuery>,
) -> Json<ListModelsResponse> {
    let s = state
        .settings_handle
        .read()
        .expect("lock poisoned: settings")
        .clone();
    let active = match params.session_id.as_deref() {
        Some(session_id) => {
            state
                .resolve_session_model(session_id)
                .await
                .models
                .active_model
        }
        None => s.models.active_model.clone(),
    };

    let mut providers = Vec::with_capacity(s.models.providers.len());
    for (key, group) in &s.models.providers {
        let models = state
            .catalog
            .merged(&s, key)
            .await
            .into_iter()
            .map(|model| ProviderModelInfo {
                name: model.name.clone(),
                tier: tier_label(model.tier),
                configured: model.configured,
                active: active
                    .as_ref()
                    .is_some_and(|active| active.provider == *key && active.model == model.name),
            })
            .collect();
        providers.push(ProviderGroupInfo {
            key: key.clone(),
            display_name: group.display_name.clone().unwrap_or_else(|| key.clone()),
            protocol: group.protocol.clone(),
            models,
        });
    }
    let is_override = match params.session_id.as_deref() {
        Some(session_id) => state
            .resolve_session(session_id)
            .await
            .is_some_and(|(_, session)| session.model_override.is_some()),
        None => false,
    };
    // Stable ordering for a predictable picker.
    providers.sort_by(|a, b| a.key.cmp(&b.key));
    Json(ListModelsResponse {
        providers,
        is_override,
    })
}

/// Serialize a [`crate::config::models::ModelTier`] for the picker payload.
fn tier_label(tier: Option<crate::config::models::ModelTier>) -> Option<String> {
    tier.map(|t| {
        match t {
            crate::config::models::ModelTier::Light => "light",
            crate::config::models::ModelTier::Medium => "medium",
            crate::config::models::ModelTier::Heavy => "heavy",
        }
        .to_string()
    })
}

/// Actionable 400 body for an unknown (provider, model) pair: lists configured
/// `provider/model` entries without fetching the dynamic catalog.
fn unknown_model_message(
    models: &crate::config::models::ModelsConfig,
    provider: &str,
    model: &str,
) -> String {
    let mut available: Vec<String> = models
        .providers
        .iter()
        .flat_map(|(key, g)| g.models.iter().map(move |m| format!("{key}/{}", m.name)))
        .collect();
    available.sort();
    format!(
        "unknown model '{provider}/{model}'; available configured models: [{}]",
        available.join(", ")
    )
}

/// Apply a global vendor-grouped switch to `settings`. A dynamic selection is
/// materialized as a provider model so the persisted active pair remains
/// resolvable after restart; session-scoped switches never call this function.
async fn apply_active_model(
    settings: &mut crate::config::Settings,
    catalog: &crate::daemon::catalog::ModelCatalogService,
    provider: &str,
    model: &str,
) -> anyhow::Result<()> {
    let endpoint = catalog
        .resolve_endpoint(settings, provider, model)
        .await
        .ok_or_else(|| anyhow::anyhow!(unknown_model_message(&settings.models, provider, model)))?;
    if let Some(group) = settings.models.providers.get_mut(provider) {
        if !group.models.iter().any(|entry| entry.name == model) {
            group.models.push(crate::config::models::ProviderModel {
                name: model.to_string(),
                profile_key: None,
                tier: endpoint.tier,
                api_key: None,
                base_url: None,
                context_window: None,
                temperature: None,
            });
        }
    }
    settings.models.active_model = Some(crate::config::models::ActiveModelRef {
        provider: provider.to_string(),
        model: model.to_string(),
    });
    settings.models.active_profile = None;
    settings.models.main = endpoint;
    Ok(())
}

/// POST /api/v1/model/switch — vendor-grouped selection. A legacy `profile`
/// is first resolved by an exact migrated `profile_key` match, never by model
/// name, then follows the same canonical provider/model path.
///
/// With `session_id`: writes ONLY that session's `model_override` (targeted
/// in-lock update; no global live-handle change, no settings.json persist)
/// and broadcasts a session-scoped `ModelChanged`. Without: sets the global
/// `active_model` + `models.main`, persists to disk, and broadcasts globally
/// (the previous global semantics).
///
/// Returns 400 with an actionable message (listing configured models) when
/// the (provider, model) pair is unknown.
pub async fn switch_model(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<SwitchModelRequest>,
) -> Result<Json<SwitchModelResponse>, (StatusCode, String)> {
    let body = if let Some(profile) = body.profile.as_deref() {
        let resolved = {
            let settings = state
                .settings_handle
                .read()
                .expect("lock poisoned: settings");
            settings
                .models
                .find_provider_model_by_key(profile)
                .map(|(provider, model)| (provider, model.name.clone()))
        };
        let (provider, model) = resolved.ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                format!("unknown legacy profile '{profile}'"),
            )
        })?;
        SwitchModelRequest {
            provider,
            model,
            profile: None,
            session_id: body.session_id,
        }
    } else {
        body
    };

    // ── Session-scoped path: override only, never a global write ──────────
    if let Some(session_id) = body.session_id.as_deref() {
        // Reject ids that could escape the sessions directory (path
        // traversal / absolute paths) before any storage access — same
        // pattern as `set_permission_mode`.
        if crate::context::memory_session::validate_id(session_id).is_err() {
            return Err((StatusCode::BAD_REQUEST, "invalid session_id".into()));
        }
        // Validate against the same merged catalog used by listing and later
        // session resolution. Clone first so no settings lock spans the fetch.
        let settings = state
            .settings_handle
            .read()
            .expect("lock poisoned: settings")
            .clone();
        if state
            .catalog
            .resolve_endpoint(&settings, &body.provider, &body.model)
            .await
            .is_none()
        {
            return Err((
                StatusCode::BAD_REQUEST,
                unknown_model_message(&settings.models, &body.provider, &body.model),
            ));
        }

        let model_ref = crate::config::models::ActiveModelRef {
            provider: body.provider.clone(),
            model: body.model.clone(),
        };
        // Serialize with PUT whole-record writers; the targeted record
        // update itself mutates + persists under the manager write lock
        // (Task 5 contract — never a bare load → save).
        let _update_guard = state.session_update_lock.lock().await;
        match state.resolve_session(session_id).await {
            Some((sessions, _)) => {
                sessions
                    .update_session_record(session_id, |session| {
                        session.model_override = Some(model_ref);
                    })
                    .await
                    .map_err(|e| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("save session model override: {e}"),
                        )
                    })?;
            }
            None => {
                // Session not known to any project store yet: create it in
                // the main manager with the override applied (same shape as
                // `set_permission_mode`); the creating write bumps version.
                let mut session =
                    crate::context::memory_session::Session::with_id(session_id, None)
                        .with_model_override(model_ref);
                session.version += 1;
                state.session_manager.save(&session).await.map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("save session model override: {e}"),
                    )
                })?;
            }
        }

        let label = {
            let s = state
                .settings_handle
                .read()
                .expect("lock poisoned: settings");
            format!("{}/{}", group_display_name(&s, &body.provider), body.model)
        };
        tracing::info!(
            session_id,
            provider = %body.provider,
            model = %body.model,
            "session model override set via /model/switch"
        );
        state.broadcast_global(
            crate::daemon::global_events::GlobalEventKind::ModelChanged,
            serde_json::json!({
                "session_id": session_id,
                "provider": body.provider,
                "model_name": body.model,
            }),
        );
        return Ok(Json(SwitchModelResponse {
            success: true,
            label,
            model_name: body.model.clone(),
            provider: Some(body.provider.clone()),
        }));
    }

    // ── Global path: active_model + main + persist (previous behavior) ────
    // 1. Read current settings, switch in a clone, validate the pair exists.
    let mut settings = state
        .settings_handle
        .read()
        .expect("lock poisoned: settings")
        .clone();
    apply_active_model(&mut settings, &state.catalog, &body.provider, &body.model)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("{e:#}")))?;

    // 2. Persist so the choice survives a restart. Use the disk-load form to
    //    avoid writing back a runtime-resolved absolute working_dir.
    let mut disk = crate::config::Settings::load_from_disk().unwrap_or_else(|_| settings.clone());
    apply_active_model(&mut disk, &state.catalog, &body.provider, &body.model)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
    if let Err(e) = disk.save() {
        tracing::warn!(error = %e, "failed to persist settings.json after model switch");
    }

    let label = format!(
        "{}/{}",
        group_display_name(&settings, &body.provider),
        body.model
    );
    let model_name = settings.models.main.name.clone();

    // 3. Publish to the live handle — next chat_stream reads this.
    *state
        .settings_handle
        .write()
        .expect("lock poisoned: settings") = settings;

    tracing::info!(
        provider = %body.provider,
        model = %model_name,
        "model switched via /model/switch"
    );

    state.broadcast_global(
        crate::daemon::global_events::GlobalEventKind::ModelChanged,
        serde_json::json!({
            "provider": body.provider,
            "model_name": model_name,
        }),
    );

    Ok(Json(SwitchModelResponse {
        success: true,
        label,
        model_name,
        provider: Some(body.provider.clone()),
    }))
}

/// Group `display_name` (falling back to its key) for the response label.
fn group_display_name(settings: &crate::config::Settings, provider: &str) -> String {
    settings
        .models
        .providers
        .get(provider)
        .and_then(|g| g.display_name.clone())
        .unwrap_or_else(|| provider.to_string())
}

/// POST /api/v1/model/reset — clear a session's model override so subsequent
/// turns fall back to the global default. Idempotent: an unknown session (or
/// one without an override) still succeeds. Broadcasts a session-scoped
/// `ModelChanged` (with `reset: true` and the default model) so clients
/// refresh exactly that session's display.
pub async fn reset_session_model(
    State(state): State<Arc<DaemonState>>,
    Json(body): Json<ResetSessionModelRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // Reject traversal-shaped ids before any storage access (400, not 500).
    if crate::context::memory_session::validate_id(&body.session_id).is_err() {
        return Err((StatusCode::BAD_REQUEST, "invalid session_id".into()));
    }
    // Serialize with PUT whole-record writers (same rationale as
    // `set_permission_mode`); the targeted update mutates + persists under
    // the manager write lock.
    let _update_guard = state.session_update_lock.lock().await;
    match state.resolve_session(&body.session_id).await {
        Some((sessions, _)) => {
            sessions
                .update_session_record(&body.session_id, |session| {
                    session.model_override = None;
                })
                .await
                .map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("clear session model override: {e}"),
                    )
                })?;
        }
        None => {
            // Nothing to clear; reset stays idempotent (no record created).
        }
    }

    // The session's effective model reverts to the global default — carry it
    // in the broadcast so clients can refresh without a round trip.
    let global = {
        state
            .settings_handle
            .read()
            .expect("lock poisoned: settings")
            .clone()
    };
    let (model_name, provider) = match global.models.active_model.as_ref() {
        Some(active) => (active.model.clone(), Some(active.provider.clone())),
        None => (
            global.models.main.name.clone(),
            global.models.main.provider.clone(),
        ),
    };

    tracing::info!(session_id = %body.session_id, "session model override reset");
    state.broadcast_global(
        crate::daemon::global_events::GlobalEventKind::ModelChanged,
        serde_json::json!({
            "session_id": body.session_id,
            "provider": provider,
            "model_name": model_name,
            "reset": true,
        }),
    );
    Ok(Json(serde_json::json!({
        "success": true,
        "session_id": body.session_id,
    })))
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
