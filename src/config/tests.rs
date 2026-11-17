use super::*;

#[test]
fn test_rlm_settings_default_all_enabled() {
    let rlm = RlmSettings::default();
    assert!(rlm.enabled);
    assert!(rlm.delegate_tool);
    assert!(rlm.auto_routing);
    assert!(rlm.retry_enabled);
    assert_eq!(rlm.max_replan_cycles, 2);
    assert_eq!(rlm.jaccard_threshold, 0.8);
}

#[test]
#[serial_test::serial]
fn set_rejects_keys_outside_the_settings_schema() {
    // serde silently drops unknown fields, so a typo'd path must be caught
    // by the round-trip check instead of "succeeding" while the real key
    // lives elsewhere (top-level exec_session.* vs agent.exec_session.*).
    let dir = tempfile::tempdir().expect("tempdir");
    std::env::set_var("WGENTY_HOME", dir.path());
    let err = Settings::set("exec_session.auto_retry_max", "5").unwrap_err();
    assert!(
        format!("{err}").contains("unknown setting key"),
        "got: {err:#}"
    );

    Settings::set("agent.exec_session.auto_retry_max", "5")
        .expect("the real nested path must keep working");
    std::env::remove_var("WGENTY_HOME");
}

/// Write an old-format (legacy `main`/`small`/`planner`) settings.json under
/// a fake home and return its path-bearing dir. Caller must clean up
/// `WGENTY_HOME` afterwards.
fn seed_legacy_settings_home() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let cfg_dir = dir.path().join(".wgenty-code");
    std::fs::create_dir_all(&cfg_dir).expect("create cfg dir");
    std::fs::write(
        cfg_dir.join("settings.json"),
        r#"{
            "models": {
                "main": {"name": "glm-5.3", "base_url": "https://relay.example", "api_key": "k-main"},
                "small": {"name": "haiku"},
                "planner": {"name": "planner-x", "base_url": "https://planner.example"}
            }
        }"#,
    )
    .expect("seed settings");
    std::env::set_var("WGENTY_HOME", dir.path());
    dir
}

/// Restore the pre-test `WGENTY_HOME` (None → remove). Tests mutating the
/// global must run `#[serial_test::serial]` — the env var is process-wide.
fn restore_wgenty_home(prev: Option<std::ffi::OsString>) {
    match prev {
        Some(v) => std::env::set_var("WGENTY_HOME", v),
        None => std::env::remove_var("WGENTY_HOME"),
    }
}

#[test]
#[serial_test::serial]
fn load_from_disk_migrates_legacy_model_fields() {
    let prev = std::env::var_os("WGENTY_HOME");
    let _dir = seed_legacy_settings_home();
    let s = Settings::load_from_disk().expect("load");

    // The load path runs both migrations in sequence: legacy main/small/
    // planner → profiles (covered by `migrate_legacy` unit tests) → the
    // vendor-grouped `providers` view.
    assert_eq!(s.models.active_profile.as_deref(), Some("main"));
    assert!(s.models.profiles.is_empty(), "profiles migrated on load");
    assert_eq!(s.models.main.name, "glm-5.3", "runtime cache resynced");

    // main + small (light tier) are grouped by host under the relay host.
    let relay = &s.models.providers["relay.example"];
    assert_eq!(
        relay
            .models
            .iter()
            .map(|m| m.name.as_str())
            .collect::<Vec<_>>(),
        ["glm-5.3", "haiku"]
    );
    assert_eq!(
        relay.models[1].tier,
        Some(crate::config::models::ModelTier::Light)
    );

    // planner → role ref survives; the planner endpoint is grouped by its
    // own host ("planner.example").
    assert_eq!(s.models.roles.planner.as_deref(), Some("planner"));
    assert!(s.models.providers["planner.example"]
        .models
        .iter()
        .any(|m| m.name == "planner-x"));

    // Legacy slots consumed; save→reload round-trips the new format without
    // synthesizing another profile or changing the active provider model.
    assert!(s.models.small.is_none() && s.models.planner.is_none());
    let providers = serde_json::to_value(&s.models.providers).expect("serialize providers");
    let active_model = s.models.active_model.clone();
    s.save().expect("save");
    let re = Settings::load_from_disk().expect("reload");
    assert_eq!(re.models.active_profile.as_deref(), Some("main"));
    assert_eq!(re.models.active_model, active_model);
    assert!(re.models.profiles.is_empty());
    assert_eq!(re.models.main.name, "glm-5.3");
    assert_eq!(
        re.models.main.base_url.as_deref(),
        Some("https://relay.example")
    );
    assert_eq!(re.models.main.api_key.as_deref(), Some("k-main"));
    assert_eq!(
        serde_json::to_value(&re.models.providers).expect("serialize reloaded providers"),
        providers
    );
    restore_wgenty_home(prev);
}

#[test]
#[serial_test::serial]
fn grouped_load_and_reload_preserve_api_key_precedence_through_api_client() {
    let env_names = [
        "WGENTY_HOME",
        "KIMI_API_KEY",
        "DEEPSEEK_API_KEY",
        "ANTHROPIC_API_KEY",
        "DASHSCOPE_API_KEY",
    ];
    let previous: Vec<_> = env_names
        .iter()
        .map(|name| (*name, std::env::var_os(name)))
        .collect();
    let dir = tempfile::tempdir().expect("tempdir");
    let cfg_dir = dir.path().join(".wgenty-code");
    std::fs::create_dir_all(&cfg_dir).expect("create cfg dir");
    std::env::set_var("WGENTY_HOME", dir.path());
    std::env::set_var("KIMI_API_KEY", "test-provider-env-key");
    std::env::set_var("DEEPSEEK_API_KEY", "test-global-fallback-key");
    std::env::remove_var("ANTHROPIC_API_KEY");
    std::env::remove_var("DASHSCOPE_API_KEY");

    let cases = [
        (Some("test-model-key"), Some("test-provider-key"), "test-model-key"),
        (None, Some("test-provider-key"), "test-provider-key"),
        (None, None, "test-provider-env-key"),
    ];
    for (model_key, provider_key, expected) in cases {
        let model = serde_json::json!({
            "name": "kimi-for-test",
            "api_key": model_key,
        });
        let group = serde_json::json!({
            "base_url": "https://api.kimi.com/coding/v1",
            "api_key": provider_key,
            "models": [model],
        });
        std::fs::write(
            cfg_dir.join("settings.json"),
            serde_json::to_vec(&serde_json::json!({
                "models": {
                    "providers": {"kimi": group},
                    "active_model": {"provider": "kimi", "model": "kimi-for-test"}
                }
            }))
            .unwrap(),
        )
        .expect("seed grouped settings");

        let loaded = Settings::load().expect("load grouped settings");
        let reloaded = Settings::reload().expect("reload grouped settings");
        assert_eq!(
            crate::api::ApiClient::new(loaded).get_api_key().as_deref(),
            Some(expected)
        );
        assert_eq!(
            crate::api::ApiClient::new(reloaded)
                .get_api_key()
                .as_deref(),
            Some(expected)
        );
    }

    for (name, value) in previous {
        match value {
            Some(value) => std::env::set_var(name, value),
            None => std::env::remove_var(name),
        }
    }
}

#[test]
#[serial_test::serial]
fn legacy_env_only_active_profile_uses_provider_env_through_api_client() {
    let env_names = ["WGENTY_HOME", "KIMI_API_KEY", "DEEPSEEK_API_KEY"];
    let previous: Vec<_> = env_names
        .iter()
        .map(|name| (*name, std::env::var_os(name)))
        .collect();
    let dir = tempfile::tempdir().expect("tempdir");
    let cfg_dir = dir.path().join(".wgenty-code");
    std::fs::create_dir_all(&cfg_dir).expect("create cfg dir");
    std::env::set_var("WGENTY_HOME", dir.path());
    std::env::set_var("KIMI_API_KEY", "test-provider-env-key");
    std::env::set_var("DEEPSEEK_API_KEY", "test-global-fallback-key");
    std::fs::write(
        cfg_dir.join("settings.json"),
        serde_json::to_vec(&serde_json::json!({
            "models": {
                "profiles": {
                    "kimi-env": {
                        "name": "kimi-for-test",
                        "base_url": "https://api.kimi.com/coding/v1"
                    }
                },
                "active_profile": "kimi-env"
            }
        }))
        .unwrap(),
    )
    .expect("seed env-only legacy profile");

    let loaded = Settings::load().expect("load env-only legacy profile");
    let reloaded = Settings::reload().expect("reload migrated profile");
    assert_eq!(
        crate::api::ApiClient::new(loaded).get_api_key().as_deref(),
        Some("test-provider-env-key")
    );
    assert_eq!(
        crate::api::ApiClient::new(reloaded)
            .get_api_key()
            .as_deref(),
        Some("test-provider-env-key")
    );

    for (name, value) in previous {
        match value {
            Some(value) => std::env::set_var(name, value),
            None => std::env::remove_var(name),
        }
    }
}

#[test]
fn transport_max_concurrent_llm_requests_defaults_to_eight_and_parses_zero() {
    let s = Settings::default();
    assert_eq!(s.models.transport.max_concurrent_llm_requests, 8);

    let raw = r#"
        {
          "max_tokens": 4096,
          "timeout": 120,
          "streaming": true,
          "beta_headers": [],
          "max_concurrent_llm_requests": 0
        }
    "#;
    let transport: crate::config::models::TransportConfig =
        serde_json::from_str(raw).expect("parse transport config");
    assert_eq!(transport.max_concurrent_llm_requests, 0);

    // Legacy configs without the key keep working via serde default.
    let legacy = r#"
        {
          "max_tokens": 4096,
          "timeout": 120,
          "streaming": true,
          "beta_headers": []
        }
    "#;
    let transport: crate::config::models::TransportConfig =
        serde_json::from_str(legacy).expect("parse legacy transport config");
    assert_eq!(transport.max_concurrent_llm_requests, 8);
}

#[test]
#[serial_test::serial]
fn set_models_main_name_remaps_onto_active_profile_without_clobber() {
    let prev = std::env::var_os("WGENTY_HOME");
    let _dir = seed_legacy_settings_home();

    // Legacy path: must patch the migrated "main" profile object — keeping
    // sibling fields like base_url — instead of replacing `models.main`.
    Settings::set("models.main.name", "\"glm-5.2\"").expect("set");

    let s = Settings::load_from_disk().expect("reload");
    let relay = &s.models.providers["relay.example"];
    let main_model = relay
        .models
        .iter()
        .find(|m| m.profile_key.as_deref() == Some("main"))
        .expect("main provider model exists");
    assert_eq!(main_model.name, "glm-5.2");
    assert_eq!(
        relay.base_url, "https://relay.example",
        "sibling fields survive the legacy-path set"
    );
    assert_eq!(s.models.active_profile.as_deref(), Some("main"));
    assert_eq!(
        s.models.active_model,
        Some(ActiveModelRef {
            provider: "relay.example".into(),
            model: "glm-5.2".into(),
        })
    );
    restore_wgenty_home(prev);
}

#[test]
#[serial_test::serial]
fn set_provider_model_rejects_unknown_fields() {
    let prev = std::env::var_os("WGENTY_HOME");
    let _dir = seed_legacy_settings_home();

    let err = Settings::set("models.main.nam", "\"glm-5.2\"").unwrap_err();
    assert!(
        format!("{err}").contains("unknown setting key"),
        "got: {err:#}"
    );
    restore_wgenty_home(prev);
}

#[test]
#[serial_test::serial]
fn set_models_main_whole_object_updates_active_provider_model() {
    let prev = std::env::var_os("WGENTY_HOME");
    let _dir = seed_legacy_settings_home();

    Settings::set(
        "models.main",
        r#"{
            "name": "glm-whole",
            "profile_key": "main",
            "tier": null,
            "api_key": "k-whole",
            "base_url": "https://relay.example/whole",
            "context_window": null,
            "temperature": null
        }"#,
    )
    .expect("set whole active provider model");

    let s = Settings::load_from_disk().expect("reload");
    let main_model = s.models.providers["relay.example"]
        .models
        .iter()
        .find(|m| m.profile_key.as_deref() == Some("main"))
        .expect("main provider model exists");
    assert_eq!(main_model.name, "glm-whole");
    assert_eq!(main_model.api_key.as_deref(), Some("k-whole"));
    assert_eq!(
        s.models.active_model,
        Some(ActiveModelRef {
            provider: "relay.example".into(),
            model: "glm-whole".into(),
        })
    );
    restore_wgenty_home(prev);
}

#[test]
#[serial_test::serial]
fn set_provider_group_field_preserves_generic_validation() {
    let prev = std::env::var_os("WGENTY_HOME");
    let dir = tempfile::tempdir().expect("tempdir");
    let cfg_dir = dir.path().join(".wgenty-code");
    std::fs::create_dir_all(&cfg_dir).expect("create cfg dir");
    std::fs::write(
        cfg_dir.join("settings.json"),
        r#"{
            "models": {
                "providers": {
                    "zhipu": {
                        "base_url": "https://api.z.ai/api/coding/paas/v4",
                        "models": [{"name": "glm-5.3"}]
                    }
                },
                "active_model": {"provider": "zhipu", "model": "glm-5.3"}
            }
        }"#,
    )
    .expect("seed settings");
    std::env::set_var("WGENTY_HOME", dir.path());

    Settings::set("models.providers.zhipu.display_name", "\"Zhipu AI\"")
        .expect("set provider group field");
    let err = Settings::set("models.providers.zhipu.display_nam", "\"typo\"").unwrap_err();
    assert!(
        format!("{err}").contains("unknown setting key"),
        "got: {err:#}"
    );
    let s = Settings::load_from_disk().expect("reload");
    assert_eq!(
        s.models.providers["zhipu"].display_name.as_deref(),
        Some("Zhipu AI")
    );
    restore_wgenty_home(prev);
}

#[test]
#[serial_test::serial]
fn legacy_main_set_after_switch_by_model_name_targets_active_model() {
    let prev = std::env::var_os("WGENTY_HOME");
    let _dir = seed_legacy_settings_home();
    let mut s = Settings::load_from_disk().expect("load");
    s.switch_to_profile("haiku").expect("switch by model name");
    s.save().expect("save switched settings");

    Settings::set("models.main.context_window", "12345").expect("set active model");

    let s = Settings::load_from_disk().expect("reload");
    assert!(s.models.profiles.is_empty());
    assert_eq!(
        s.models.active_model,
        Some(ActiveModelRef {
            provider: "relay.example".into(),
            model: "haiku".into(),
        })
    );
    let active = s.models.providers["relay.example"]
        .models
        .iter()
        .find(|m| m.name == "haiku")
        .expect("active provider model exists");
    assert_eq!(active.context_window, Some(12345));
    assert_eq!(s.models.main.name, "haiku");
    restore_wgenty_home(prev);
}

#[test]
#[serial_test::serial]
fn set_models_small_name_keeps_light_role() {
    let prev = std::env::var_os("WGENTY_HOME");
    let _dir = seed_legacy_settings_home();
    // "small" light profile already exists post-migration → set targets it.
    Settings::set("models.small.name", "\"haiku-2\"").expect("set");
    let s = Settings::load_from_disk().expect("reload");
    let small = s.models.providers["relay.example"]
        .models
        .iter()
        .find(|m| m.profile_key.as_deref() == Some("small"))
        .expect("small provider model exists");
    assert_eq!(small.name, "haiku-2");
    assert_eq!(small.tier, Some(crate::config::models::ModelTier::Light));
    restore_wgenty_home(prev);
}

#[test]
fn test_rlm_settings_deserialize_partial() {
    let json = r#"{"enabled": false}"#;
    let rlm: RlmSettings = serde_json::from_str(json).unwrap();
    assert!(!rlm.enabled);
    assert!(rlm.delegate_tool);
    assert!(rlm.auto_routing);
    assert!(rlm.retry_enabled);
    assert_eq!(rlm.max_replan_cycles, 2);
    assert_eq!(rlm.jaccard_threshold, 0.8);
}

#[test]
fn test_rlm_settings_deserialize_full() {
    let json = r#"{
            "enabled": false,
            "delegate_tool": false,
            "auto_routing": false,
            "retry_enabled": false,
            "max_replan_cycles": 0,
            "jaccard_threshold": 0.95
        }"#;
    let rlm: RlmSettings = serde_json::from_str(json).unwrap();
    assert!(!rlm.enabled);
    assert!(!rlm.delegate_tool);
    assert!(!rlm.auto_routing);
    assert!(!rlm.retry_enabled);
    assert_eq!(rlm.max_replan_cycles, 0);
    assert!((rlm.jaccard_threshold - 0.95).abs() < 1e-9);
}

#[test]
fn test_settings_default_includes_rlm() {
    let settings = Settings::default();
    assert!(settings.agent.rlm.enabled);
    assert!(settings.agent.rlm.delegate_tool);
    assert!(settings.agent.rlm.auto_routing);
}

#[test]
fn test_rlm_deserialize_in_settings() {
    let json = r#"{
            "models": {
                "transport": {"max_tokens": 4096, "timeout": 120, "streaming": true, "beta_headers": []},
                "main": {"name": "test"}
            },
            "agent": {
                "rlm": {"enabled": false, "delegate_tool": false}
            },
            "storage": {
                "working_dir": ".",
                "memory": {"enabled": false, "path": ".", "consolidation_interval": 24, "max_memories": 100}
            },
            "plugins": {"enabled": false, "dir": ".", "auto_update": false}
        }"#;
    let settings: Settings = serde_json::from_str(json).unwrap();
    assert!(!settings.agent.rlm.enabled);
    assert!(!settings.agent.rlm.delegate_tool);
    // Unspecified rlm fields use defaults
    assert!(settings.agent.rlm.auto_routing);
    assert!(settings.agent.rlm.retry_enabled);
    assert_eq!(settings.agent.rlm.max_replan_cycles, 2);
}

#[test]
fn test_prompt_includes_default_all_true() {
    let s = Settings::default();
    assert!(s.prompt.include.permissions);
    assert!(s.prompt.include.developer);
    assert!(s.prompt.include.collaboration);
    assert!(s.prompt.include.environment);
    assert!(s.prompt.include.skills);
}

#[test]
fn test_models_default_no_small_or_planner() {
    let s = Settings::default();
    assert_eq!(s.models.main.name, "deepseek-v4-pro");
    assert!(s.models.small.is_none());
    assert!(s.models.planner.is_none());
}

#[test]
fn test_models_small_inherits_when_url_absent() {
    let json = r#"{
            "models": {
                "main": {"name": "sonnet", "base_url": "https://api.example.com", "api_key": "main-key"},
                "small": {"name": "haiku"}
            }
        }"#;
    let s: Settings = serde_json::from_str(json).unwrap();
    let small = s.models.small.as_ref().unwrap();
    assert_eq!(small.name, "haiku");
    // Inheritance is the consumer's job — see small_model_settings
    assert!(small.base_url.is_none());
    assert!(small.api_key.is_none());
}

#[test]
fn providers_deserialize_from_grouped_json() {
    let json = r#"{
        "providers": {
            "zhipu": {
                "display_name": "智谱",
                "base_url": "https://api.z.ai/api/coding/paas/v4",
                "api_key": "sk-z",
                "models": [
                    {"name": "glm-5.3"},
                    {"name": "glm-5.3-flash", "tier": "light"}
                ]
            }
        },
        "active_model": {"provider": "zhipu", "model": "glm-5.3"}
    }"#;
    let cfg: ModelsConfig = serde_json::from_str(json).expect("parse");
    let zg = cfg.providers.get("zhipu").expect("group");
    assert_eq!(zg.base_url, "https://api.z.ai/api/coding/paas/v4");
    assert_eq!(zg.models.len(), 2);
    assert_eq!(zg.models[1].tier, Some(ModelTier::Light));
    assert_eq!(cfg.active_model.as_ref().unwrap().model, "glm-5.3");
}

#[test]
fn providers_absent_keeps_old_config_parsing() {
    // 旧配置（无 providers 字段）反序列化不报错，providers 为空。
    let json = r#"{"main": {"name": "glm-5.3"}}"#;
    let cfg: ModelsConfig = serde_json::from_str(json).expect("legacy ok");
    assert!(cfg.providers.is_empty());
    assert!(cfg.active_model.is_none());
}

struct EnvVarGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, previous }
    }

    fn remove(key: &'static str) -> Self {
        let previous = std::env::var_os(key);
        std::env::remove_var(key);
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

#[test]
#[serial_test::serial]
fn resolve_endpoint_prefers_model_override_then_group_then_env() {
    let _env = EnvVarGuard::set("ZHIPU_API_KEY", "test-env-key");
    let mut cfg = ModelsConfig::default();
    cfg.providers.insert(
        "zhipu".into(),
        ProviderGroup {
            display_name: None,
            base_url: "https://group.zhipu.test".into(),
            api_key: Some("test-group-key".into()),
            protocol: Some("openai".into()),
            models: vec![ProviderModel {
                name: "glm-test".into(),
                profile_key: None,
                tier: Some(ModelTier::Light),
                api_key: Some("test-model-key".into()),
                base_url: Some("https://model.zhipu.test".into()),
                context_window: Some(123_456),
                temperature: Some(0.2),
            }],
        },
    );

    let resolved = cfg.resolve_endpoint("zhipu", "glm-test").expect("model");
    assert_eq!(resolved.name, "glm-test");
    assert_eq!(resolved.api_key.as_deref(), Some("test-model-key"));
    assert_eq!(
        resolved.base_url.as_deref(),
        Some("https://model.zhipu.test")
    );
    assert_eq!(resolved.provider.as_deref(), Some("openai"));
    assert_eq!(resolved.context_window, Some(123_456));
    assert_eq!(resolved.temperature, Some(0.2));
    assert_eq!(resolved.tier, Some(ModelTier::Light));

    let model = &mut cfg.providers.get_mut("zhipu").unwrap().models[0];
    model.api_key = None;
    model.base_url = None;
    let resolved = cfg.resolve_endpoint("zhipu", "glm-test").expect("model");
    assert_eq!(resolved.api_key.as_deref(), Some("test-group-key"));
    assert_eq!(
        resolved.base_url.as_deref(),
        Some("https://group.zhipu.test")
    );

    cfg.providers.get_mut("zhipu").unwrap().api_key = None;
    let resolved = cfg.resolve_endpoint("zhipu", "glm-test").expect("model");
    assert_eq!(resolved.api_key.as_deref(), Some("test-env-key"));
}

#[test]
#[serial_test::serial]
fn resolve_endpoint_uses_global_env_fallback_chain() {
    let _deepseek = EnvVarGuard::remove("DEEPSEEK_API_KEY");
    let _anthropic = EnvVarGuard::remove("ANTHROPIC_API_KEY");
    let _dashscope = EnvVarGuard::remove("DASHSCOPE_API_KEY");
    let mut cfg = ModelsConfig::default();
    cfg.providers.insert(
        "custom".into(),
        ProviderGroup {
            display_name: None,
            base_url: "https://custom.test".into(),
            api_key: None,
            protocol: None,
            models: vec![ProviderModel {
                name: "custom-model".into(),
                profile_key: None,
                tier: None,
                api_key: None,
                base_url: None,
                context_window: None,
                temperature: None,
            }],
        },
    );

    let _dashscope_key = EnvVarGuard::set("DASHSCOPE_API_KEY", "test-dashscope-key");
    let resolved = cfg
        .resolve_endpoint("custom", "custom-model")
        .expect("model");
    assert_eq!(resolved.api_key.as_deref(), Some("test-dashscope-key"));

    let _anthropic_key = EnvVarGuard::set("ANTHROPIC_API_KEY", "test-anthropic-key");
    let resolved = cfg
        .resolve_endpoint("custom", "custom-model")
        .expect("model");
    assert_eq!(resolved.api_key.as_deref(), Some("test-anthropic-key"));

    let _deepseek_key = EnvVarGuard::set("DEEPSEEK_API_KEY", "test-deepseek-key");
    let resolved = cfg
        .resolve_endpoint("custom", "custom-model")
        .expect("model");
    assert_eq!(resolved.api_key.as_deref(), Some("test-deepseek-key"));
}

#[test]
fn resolve_endpoint_returns_none_for_unknown_provider_or_model() {
    let mut cfg = ModelsConfig::default();
    cfg.providers.insert(
        "deepseek".into(),
        ProviderGroup {
            display_name: None,
            base_url: "https://api.deepseek.com".into(),
            api_key: None,
            protocol: None,
            models: Vec::new(),
        },
    );
    assert!(cfg.resolve_endpoint("missing", "model").is_none());
    assert!(cfg.resolve_endpoint("deepseek", "missing").is_none());
}

#[test]
fn provider_env_key_maps_supported_vendors() {
    assert_eq!(provider_env_key("kimi").as_deref(), Some("KIMI_API_KEY"));
    assert_eq!(provider_env_key("zhipu").as_deref(), Some("ZHIPU_API_KEY"));
    assert_eq!(
        provider_env_key("deepseek").as_deref(),
        Some("DEEPSEEK_API_KEY")
    );
    assert_eq!(
        provider_env_key("anthropic").as_deref(),
        Some("ANTHROPIC_API_KEY")
    );
    assert_eq!(
        provider_env_key("openai").as_deref(),
        Some("OPENAI_API_KEY")
    );
    assert_eq!(provider_env_key("custom"), None);
}

#[test]
fn endpoint_for_tier_cross_provider_deterministic() {
    let group = |model_name: &str| ProviderGroup {
        display_name: None,
        base_url: format!("https://{model_name}.test"),
        api_key: Some(format!("key-{model_name}")),
        protocol: None,
        models: vec![ProviderModel {
            name: model_name.into(),
            profile_key: None,
            tier: Some(ModelTier::Light),
            api_key: None,
            base_url: None,
            context_window: None,
            temperature: None,
        }],
    };
    let mut cfg = ModelsConfig::default();
    cfg.providers.insert("b".into(), group("m1"));
    cfg.providers.insert("a".into(), group("m2"));

    let selected = cfg.endpoint_for_tier(ModelTier::Light);
    assert_eq!(selected.name, "m2");
    assert_eq!(selected.base_url.as_deref(), Some("https://m2.test"));
    assert_eq!(selected.api_key.as_deref(), Some("key-m2"));
    assert_eq!(
        serde_json::to_value(&selected).unwrap(),
        serde_json::to_value(cfg.endpoint_for_tier(ModelTier::Light)).unwrap()
    );

    let mut reversed = ModelsConfig::default();
    reversed.providers.insert("a".into(), group("m2"));
    reversed.providers.insert("b".into(), group("m1"));
    assert_eq!(
        serde_json::to_value(selected).unwrap(),
        serde_json::to_value(reversed.endpoint_for_tier(ModelTier::Light)).unwrap()
    );
}

#[test]
fn has_tier_recognizes_provider_model_tiers() {
    let mut cfg = ModelsConfig::default();
    cfg.providers.insert(
        "custom".into(),
        ProviderGroup {
            display_name: None,
            base_url: "https://custom.test".into(),
            api_key: Some("test-key".into()),
            protocol: None,
            models: vec![
                ProviderModel {
                    name: "light-model".into(),
                    profile_key: None,
                    tier: Some(ModelTier::Light),
                    api_key: None,
                    base_url: None,
                    context_window: None,
                    temperature: None,
                },
                ProviderModel {
                    name: "heavy-model".into(),
                    profile_key: None,
                    tier: Some(ModelTier::Heavy),
                    api_key: None,
                    base_url: None,
                    context_window: None,
                    temperature: None,
                },
            ],
        },
    );

    assert!(cfg.profiles.is_empty());
    assert!(cfg.has_tier(ModelTier::Light));
    assert!(cfg.has_tier(ModelTier::Heavy));
    assert!(!cfg.has_tier(ModelTier::Medium));
    assert_eq!(cfg.endpoint_for_tier(ModelTier::Light).name, "light-model");
}

#[test]
fn endpoint_for_tier_migration_preserves_choice() {
    let mut cfg: ModelsConfig = serde_json::from_str(&legacy_profiles_json()).unwrap();
    let before = cfg.endpoint_for_tier(ModelTier::Light);
    assert_eq!(before.name, "glm-5.3-flash");

    cfg.migrate_providers().unwrap();
    let after = cfg.endpoint_for_tier(ModelTier::Light);
    assert_eq!(after.name, "glm-5.3-flash");
    assert_eq!(
        serde_json::to_value(before).unwrap(),
        serde_json::to_value(after).unwrap()
    );
}

#[test]
fn test_small_model_settings_uses_small_overrides() {
    let mut s = Settings::default();
    s.models.main.base_url = Some("https://api.main.example".to_string());
    s.models.main.api_key = Some("main-key".to_string());
    s.models.small = Some(ModelEndpoint {
        name: "haiku".to_string(),
        base_url: None, // inherits main
        api_key: Some("small-key".to_string()),
        appkey: None,
        provider: None,
        ..Default::default()
    });
    let small_s = s.small_model_settings();
    assert_eq!(small_s.models.main.name, "haiku");
    assert_eq!(
        small_s.models.main.base_url,
        Some("https://api.main.example".to_string())
    ); // unchanged
    assert_eq!(small_s.models.main.api_key, Some("small-key".to_string())); // overridden
                                                                            // transport.max_tokens is inherited from the shared config (no longer
                                                                            // forced to 2048), so small uses the same output budget as the main model.
    assert_eq!(
        small_s.models.transport.max_tokens,
        s.models.transport.max_tokens
    );
}

#[test]
fn test_subagent_overrides_default_none() {
    let s = Settings::default();
    let ov = &s.agent.subagent;
    assert!(ov.token_budget_k.is_none());
    assert!(ov.max_rounds.is_none());
    assert!(ov.plan_mode.is_none());
    assert!(ov.rlm.enabled.is_none());
    assert!(ov.rlm.delegate_tool.is_none());
    assert!(ov.rlm.auto_routing.is_none());
    assert!(ov.rlm.retry_enabled.is_none());
    assert!(ov.rlm.max_replan_cycles.is_none());
    assert!(ov.rlm.jaccard_threshold.is_none());
    assert!(ov.prompt.include.permissions.is_none());
    assert!(ov.prompt.include.developer.is_none());
    assert!(ov.prompt.include.collaboration.is_none());
    assert!(ov.prompt.include.environment.is_none());
    assert!(ov.prompt.include.skills.is_none());
    assert!(ov.prompt.developer_instructions.is_none());
    assert!(ov.prompt.collaboration_mode.is_none());
    assert!(ov.prompt.model_instructions_file.is_none());
}

#[test]
fn test_resolve_max_rounds_mappings() {
    // None → default cap.
    assert_eq!(resolve_max_rounds(None), DEFAULT_MAX_ROUNDS);
    // Some(0) → unlimited.
    assert_eq!(resolve_max_rounds(Some(0)), usize::MAX);
    // Some(n) → n.
    assert_eq!(resolve_max_rounds(Some(7)), 7);
}

#[test]
fn test_agent_effective_max_rounds() {
    let mut s = Settings::default();
    assert_eq!(s.agent.effective_max_rounds(), DEFAULT_MAX_ROUNDS);
    s.agent.max_rounds = Some(0);
    assert_eq!(s.agent.effective_max_rounds(), usize::MAX);
    s.agent.max_rounds = Some(300);
    assert_eq!(s.agent.effective_max_rounds(), 300);
}

#[test]
fn test_subagent_effective_max_rounds_inherits_agent() {
    let mut s = Settings::default();
    // Both unset → default.
    assert_eq!(s.subagent_effective_max_rounds(), DEFAULT_MAX_ROUNDS);
    // Subagent unset inherits agent value.
    s.agent.max_rounds = Some(300);
    assert_eq!(s.subagent_effective_max_rounds(), 300);
    // Subagent override wins; Some(0) = unlimited.
    s.agent.subagent.max_rounds = Some(0);
    assert_eq!(s.subagent_effective_max_rounds(), usize::MAX);
    s.agent.subagent.max_rounds = Some(5);
    assert_eq!(s.subagent_effective_max_rounds(), 5);
    // Agent unlimited propagates too.
    s.agent.subagent.max_rounds = None;
    s.agent.max_rounds = Some(0);
    assert_eq!(s.subagent_effective_max_rounds(), usize::MAX);
}

#[test]
fn test_agent_stream_max_retries_default_and_serde() {
    // Programmatic default.
    assert_eq!(
        Settings::default().agent.stream_max_retries,
        DEFAULT_STREAM_MAX_RETRIES
    );
    // Missing field in settings.json → serde default (not 0).
    let json = r#"{"plan_mode": false}"#;
    let cfg: AgentConfig = serde_json::from_str(json).expect("deserialize");
    assert_eq!(cfg.stream_max_retries, DEFAULT_STREAM_MAX_RETRIES);
    // Explicit value round-trips.
    let mut s = Settings::default();
    s.agent.stream_max_retries = 5;
    let round: Settings = serde_json::from_value(serde_json::to_value(&s).unwrap()).unwrap();
    assert_eq!(round.agent.stream_max_retries, 5);
}

#[test]
fn test_agent_stream_idle_timeout_default_and_serde() {
    // Programmatic default.
    assert_eq!(
        Settings::default().agent.stream_idle_timeout_secs,
        DEFAULT_STREAM_IDLE_TIMEOUT_SECS
    );
    // Missing field in settings.json → serde default (not 0).
    let json = r#"{"plan_mode": false}"#;
    let cfg: AgentConfig = serde_json::from_str(json).expect("deserialize");
    assert_eq!(
        cfg.stream_idle_timeout_secs,
        DEFAULT_STREAM_IDLE_TIMEOUT_SECS
    );
    // Explicit value round-trips.
    let mut s = Settings::default();
    s.agent.stream_idle_timeout_secs = 180;
    let round: Settings = serde_json::from_value(serde_json::to_value(&s).unwrap()).unwrap();
    assert_eq!(round.agent.stream_idle_timeout_secs, 180);
    // `0` resolves to the default via the effective accessor.
    s.agent.stream_idle_timeout_secs = 0;
    assert_eq!(
        s.agent.effective_stream_idle_timeout_secs(),
        DEFAULT_STREAM_IDLE_TIMEOUT_SECS
    );
}

#[test]
fn test_set_dotted_path_nested_field() {
    use serde_json::Value;
    let s = Settings::default();
    let mut json = serde_json::to_value(&s).unwrap();
    let parts: &[&str] = &["agent", "subagent", "max_depth"];
    fn walk_set(n: &mut Value, p: &[&str], v: Value) {
        let (h, r) = p.split_first().unwrap();
        if r.is_empty() {
            n.as_object_mut().unwrap().insert(h.to_string(), v);
        } else {
            let nx = n
                .as_object_mut()
                .unwrap()
                .entry(h.to_string())
                .or_insert(Value::Object(Default::default()));
            walk_set(nx, r, v);
        }
    }
    walk_set(&mut json, parts, Value::Number(7.into()));
    let new: Settings = serde_json::from_value(json).unwrap();
    assert_eq!(new.agent.subagent.max_depth, 7);
}

#[test]
fn test_set_dotted_path_unknown_field_fails_validation() {
    use serde_json::Value;
    let s = Settings::default();
    let mut json = serde_json::to_value(&s).unwrap();
    json.as_object_mut()
        .unwrap()
        .insert("nonexistent_top".to_string(), Value::Bool(true));
    // serde_json by default tolerates extra fields; document behavior here.
    let r: Result<Settings, _> = serde_json::from_value(json);
    assert!(
        r.is_ok(),
        "extra fields are tolerated by default; if rejection is desired, add deny_unknown_fields"
    );
}

/// Mirrors the budget-fallback chain in src/tools/meta/task.rs.
fn resolve_token_budget_k(s: &Settings, caller: Option<usize>) -> usize {
    caller
        .or(s.agent.subagent.token_budget_k)
        .or((s.agent.token_budget.subagent_default_k > 0)
            .then_some(s.agent.token_budget.subagent_default_k))
        .unwrap_or(s.agent.token_budget.main_k)
}

#[test]
fn test_subagent_token_budget_fallback_chain() {
    let mut s = Settings::default();
    s.agent.token_budget.main_k = 100;

    // Level 4: only main_k set
    assert_eq!(resolve_token_budget_k(&s, None), 100);

    // Level 3: subagent_default_k > 0 wins over main_k
    s.agent.token_budget.subagent_default_k = 50;
    assert_eq!(resolve_token_budget_k(&s, None), 50);

    // Level 3 ignored when subagent_default_k == 0
    s.agent.token_budget.subagent_default_k = 0;
    assert_eq!(resolve_token_budget_k(&s, None), 100);

    // Level 2: subagent override beats subagent_default and main
    s.agent.token_budget.subagent_default_k = 50;
    s.agent.subagent.token_budget_k = Some(30);
    assert_eq!(resolve_token_budget_k(&s, None), 30);

    // Level 1: caller-explicit beats everything
    assert_eq!(resolve_token_budget_k(&s, Some(7)), 7);
}

/// Expand leading `~/` path placeholders so the checked-in template can stay
/// machine-independent while still matching `Settings::default()` paths.
fn expand_tilde_paths(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(s) => {
            if let Some(rest) = s.strip_prefix("~/") {
                if let Some(home) = dirs::home_dir() {
                    // Keep `/` separators so Windows PathBuf defaults and the
                    // string-form `default_transcript_db_path` compare cleanly.
                    *s = format!("{}/{}", home.to_string_lossy(), rest);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                expand_tilde_paths(item);
            }
        }
        serde_json::Value::Object(map) => {
            for item in map.values_mut() {
                expand_tilde_paths(item);
            }
        }
        _ => {}
    }
}

/// Normalize path separators in JSON strings so Windows `PathBuf` (`\`) and
/// template/`format!` paths (`/`) compare as the same logical path.
fn normalize_path_separators(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(s) if s.contains('\\') => {
            *s = s.replace('\\', "/");
        }
        serde_json::Value::Array(items) => {
            for item in items {
                normalize_path_separators(item);
            }
        }
        serde_json::Value::Object(map) => {
            for item in map.values_mut() {
                normalize_path_separators(item);
            }
        }
        _ => {}
    }
}

/// `settings.json.template` is a human-readable snapshot of `Settings::default()`.
/// It is NOT loaded at runtime (first-run still uses `Settings::default()`), but
/// must stay structurally in sync so docs/onboarding do not drift.
#[test]
fn settings_json_template_matches_settings_default() {
    let raw = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/settings.json.template"
    ));
    let mut template_val: serde_json::Value =
        serde_json::from_str(raw).expect("settings.json.template must be valid JSON");
    expand_tilde_paths(&mut template_val);

    // Template must deserialize as Settings (schema check).
    let template_settings: Settings = serde_json::from_value(template_val)
        .expect("settings.json.template must deserialize as Settings");

    // Compare against a sanitized Default: env-derived secrets/urls are not part
    // of the checked-in template (always null). Re-serialize both Settings so
    // f32 defaults (0.3 / 0.01) share the same binary representation.
    let mut default_settings = Settings::default();
    default_settings.models.main.api_key = None;
    default_settings.models.main.base_url = None;
    default_settings.models.main.provider = None;

    let mut template_val =
        serde_json::to_value(&template_settings).expect("serialize template Settings");
    let mut default_val =
        serde_json::to_value(&default_settings).expect("serialize Settings::default");
    normalize_path_separators(&mut template_val);
    normalize_path_separators(&mut default_val);

    assert_eq!(
        template_val, default_val,
        "settings.json.template drifted from Settings::default(); rewrite the template"
    );

    // Spot-check critical product defaults that previously drifted in docs/template.
    assert_eq!(template_settings.agent.subagent.max_depth, 1);
    assert_eq!(template_settings.agent.subagent.timeout_secs, 1800);
    assert_eq!(template_settings.models.context_window, 200_000);
    assert_eq!(template_settings.models.main.name, "deepseek-v4-pro");
}

#[test]
fn resolve_working_dir_makes_dot_absolute() {
    let mut s = Settings::default();
    assert_eq!(s.storage.working_dir, PathBuf::from("."));
    s.resolve_working_dir();
    assert!(
        s.storage.working_dir.is_absolute(),
        "resolve_working_dir should bind '.' to an absolute project root"
    );
    assert!(
        s.storage.working_dir != std::path::Path::new("."),
        "resolved working_dir must not remain relative '.'"
    );
}

#[test]
fn reconcile_routing_disables_models_routing_when_auto_routing_off() {
    // Legacy kill switch off → models.routing.enabled forced off (AND logic).
    let mut s = Settings::default();
    s.agent.rlm.auto_routing = false;
    assert!(s.models.routing.enabled, "default routing should be on");
    s.reconcile_routing();
    assert!(
        !s.models.routing.enabled,
        "auto_routing=false must disable models.routing"
    );
}

#[test]
fn reconcile_routing_keeps_models_routing_on_when_both_on() {
    let mut s = Settings::default();
    s.agent.rlm.auto_routing = true;
    s.models.routing.enabled = true;
    s.reconcile_routing();
    assert!(s.models.routing.enabled);
}

#[test]
fn reconcile_routing_off_independent_of_models_routing() {
    // If user only disabled models.routing (new switch), auto_routing stays on
    // but routing stays off — the AND is commutative here.
    let mut s = Settings::default();
    s.models.routing.enabled = false;
    s.reconcile_routing();
    assert!(!s.models.routing.enabled);
}

#[cfg(test)]
mod fallback_config_tests {
    use super::*;
    use crate::config::agent::SubagentLimits;

    #[test]
    fn fallback_models_default_empty() {
        let limits = SubagentLimits::default();
        assert!(limits.fallback_models.is_empty());
    }

    #[test]
    fn fallback_models_loaded_from_config() {
        let toml = r#"
[agent.subagent]
max_depth = 2
max_concurrent = 5
timeout_secs = 1800
fallback_models = ["claude-sonnet-4", "gpt-4o"]
"#;
        let settings: Settings = toml::from_str(toml).unwrap();
        assert_eq!(
            settings.agent.subagent.fallback_models,
            vec!["claude-sonnet-4".to_string(), "gpt-4o".to_string()]
        );
    }

    #[test]
    fn fallback_model_settings_only_overrides_name() {
        let mut settings = Settings::default();
        settings.models.main.name = "deepseek-reasoner".to_string();
        settings.models.main.base_url = Some("https://api.deepseek.com".to_string());
        settings.models.main.api_key = Some("sk-deepseek".to_string());

        let fallback = settings.fallback_model_settings("claude-sonnet-4");
        assert_eq!(fallback.models.main.name, "claude-sonnet-4");
        // base_url / api_key preserved (reuse original endpoint)
        assert_eq!(
            fallback.models.main.base_url,
            Some("https://api.deepseek.com".to_string())
        );
        assert_eq!(
            fallback.models.main.api_key,
            Some("sk-deepseek".to_string())
        );
    }

    #[test]
    fn select_fallback_model_picks_first_different() {
        let mut settings = Settings::default();
        settings.models.main.name = "deepseek-reasoner".to_string();
        settings.agent.subagent.fallback_models = vec![
            "deepseek-reasoner".to_string(),
            "claude-sonnet-4".to_string(),
            "gpt-4o".to_string(),
        ];
        assert_eq!(
            settings.select_fallback_model("deepseek-reasoner"),
            Some("claude-sonnet-4")
        );
    }

    #[test]
    fn select_fallback_model_none_when_empty() {
        let settings = Settings::default();
        assert_eq!(settings.select_fallback_model("any-model"), None);
    }

    #[test]
    fn select_fallback_model_none_when_all_same() {
        let mut settings = Settings::default();
        settings.agent.subagent.fallback_models = vec!["deepseek-reasoner".to_string()];
        assert_eq!(settings.select_fallback_model("deepseek-reasoner"), None);
    }

    #[test]
    fn subagent_trace_context_char_limit_defaults_to_2000() {
        let trace = SubagentTraceConfig::default();
        assert_eq!(trace.context_char_limit, 2000);
        assert_eq!(trace.sink, TraceSinkMode::Both);
        assert!(trace.dir.is_none());
    }

    #[test]
    fn subagent_trace_context_char_limit_is_configurable() {
        let json = r#"{"context_char_limit": 4096, "sink": "off"}"#;
        let trace: SubagentTraceConfig = serde_json::from_str(json).unwrap();
        assert_eq!(trace.context_char_limit, 4096);
        assert_eq!(trace.sink, TraceSinkMode::Off);
    }
}

/// Legacy `models.profiles` fixture mirroring a real settings.json: a kimi
/// profile plus two z.ai profiles sharing host and key (one light-tier).
fn legacy_profiles_json() -> String {
    r#"{
        "main": {"name": "glm-5.3", "base_url": "https://api.z.ai/api/coding/paas/v4", "api_key": "sk-z"},
        "profiles": {
            "kimi": {"name": "k3", "base_url": "https://api.kimi.com/coding/v1", "api_key": "sk-k"},
            "glm": {"name": "glm-5.3", "base_url": "https://api.z.ai/api/coding/paas/v4", "api_key": "sk-z"},
            "glm-flash": {"name": "glm-5.3-flash", "base_url": "https://api.z.ai/api/coding/paas/v4", "api_key": "sk-z", "tier": "light"}
        },
        "active_profile": "glm"
    }"#
    .to_string()
}

#[test]
fn migrate_profiles_groups_by_host() {
    let mut cfg: ModelsConfig = serde_json::from_str(&legacy_profiles_json()).unwrap();
    cfg.migrate_providers().unwrap();
    assert_eq!(cfg.providers.len(), 2, "kimi + zhipu");
    let zg = cfg.providers.get("zhipu").expect("zhipu group");
    assert_eq!(
        zg.models
            .iter()
            .map(|m| m.name.as_str())
            .collect::<Vec<_>>(),
        ["glm-5.3", "glm-5.3-flash"]
    );
    assert_eq!(zg.api_key.as_deref(), Some("sk-z"));
    assert_eq!(
        cfg.active_model,
        Some(ActiveModelRef {
            provider: "zhipu".into(),
            model: "glm-5.3".into()
        })
    );
    // 迁移后 profiles 置空（写回时不序列化旧结构）。
    assert!(cfg.profiles.is_empty());
}

#[test]
fn migrate_is_idempotent() {
    let mut cfg: ModelsConfig = serde_json::from_str(&legacy_profiles_json()).unwrap();
    cfg.migrate_providers().unwrap();
    let once = serde_json::to_string(&cfg.providers).unwrap();
    cfg.migrate_providers().unwrap();
    let twice = serde_json::to_string(&cfg.providers).unwrap();
    assert_eq!(once, twice);
}

#[test]
fn migrate_unknown_host_uses_host_as_group_name() {
    let mut cfg: ModelsConfig = serde_json::from_str(
        r#"{"profiles": {"x": {"name": "m", "base_url": "https://llm.internal.io/v1"}}}"#,
    )
    .unwrap();
    cfg.migrate_providers().unwrap();
    assert!(cfg.providers.contains_key("llm.internal.io"));
}

#[test]
fn migrate_invalid_url_uses_custom_group() {
    let mut cfg: ModelsConfig =
        serde_json::from_str(r#"{"profiles": {"x": {"name": "m", "base_url": "not a url"}}}"#)
            .unwrap();
    cfg.migrate_providers().unwrap();
    assert!(cfg.providers.contains_key("custom"));
}

#[test]
fn migrate_merges_into_existing_providers_without_duplicates() {
    // Pre-existing group (new format already partially adopted) + legacy
    // profiles: same-host profiles merge into the group, a same-named model
    // is skipped instead of duplicated.
    let mut cfg: ModelsConfig = serde_json::from_str(
        r#"{
            "profiles": {
                "glm": {"name": "glm-5.3", "base_url": "https://api.z.ai/api/coding/paas/v4", "api_key": "sk-z"},
                "glm-flash": {"name": "glm-5.3-flash", "base_url": "https://api.z.ai/api/coding/paas/v4", "api_key": "sk-z"}
            },
            "providers": {
                "zhipu": {"base_url": "https://api.z.ai/api/coding/paas/v4", "api_key": "sk-z",
                          "models": [{"name": "glm-5.3"}]}
            }
        }"#,
    )
    .unwrap();
    cfg.migrate_providers().unwrap();
    let zg = cfg.providers.get("zhipu").expect("zhipu group");
    assert_eq!(
        zg.models
            .iter()
            .map(|m| m.name.as_str())
            .collect::<Vec<_>>(),
        ["glm-5.3", "glm-5.3-flash"],
        "existing model kept, new one appended, no duplicate"
    );
    assert!(cfg.profiles.is_empty());
}

#[test]
#[serial_test::serial]
fn load_from_disk_rejects_conflicting_same_name_profiles_without_rewriting() {
    let previous = std::env::var_os("WGENTY_HOME");
    let dir = tempfile::tempdir().expect("tempdir");
    let cfg_dir = dir.path().join(".wgenty-code");
    std::fs::create_dir_all(&cfg_dir).expect("create cfg dir");
    std::env::set_var("WGENTY_HOME", dir.path());
    let first = serde_json::json!({
        "name": "shared-model",
        "base_url": "https://api.deepseek.com/v1",
        "api_key": "test-first-key",
        "provider": "openai",
        "tier": "light"
    });
    let changes = [
        ("api_key", serde_json::json!("test-second-key")),
        ("base_url", serde_json::json!("https://api.deepseek.com/other/v1")),
        ("tier", serde_json::json!("heavy")),
        ("provider", serde_json::json!("anthropic")),
    ];

    for (field, value) in changes {
        let mut second = first.clone();
        second[field] = value;
        let original = serde_json::to_string_pretty(&serde_json::json!({
            "models": {"profiles": {"a-first": first, "b-second": second}}
        }))
        .unwrap();
        let path = cfg_dir.join("settings.json");
        std::fs::write(&path, &original).expect("seed conflicting profiles");

        let result = Settings::load_from_disk();
        let after = std::fs::read_to_string(&path).expect("read settings after migration");
        assert!(
            result.is_err(),
            "{field} conflict must not migrate successfully"
        );
        assert_eq!(after, original, "{field} conflict rewrote the old file");
    }
    restore_wgenty_home(previous);
}

#[test]
#[serial_test::serial]
fn load_from_disk_rejects_second_merge_conflict_without_rewriting() {
    let previous = std::env::var_os("WGENTY_HOME");
    let dir = tempfile::tempdir().expect("tempdir");
    let cfg_dir = dir.path().join(".wgenty-code");
    std::fs::create_dir_all(&cfg_dir).expect("create cfg dir");
    std::env::set_var("WGENTY_HOME", dir.path());
    let original = serde_json::to_string_pretty(&serde_json::json!({
        "models": {
            "profiles": {
                "legacy": {
                    "name": "shared-model",
                    "base_url": "https://api.deepseek.com/v1",
                    "api_key": "test-legacy-key"
                }
            },
            "providers": {
                "deepseek": {
                    "base_url": "https://api.deepseek.com/v1",
                    "api_key": "test-existing-key",
                    "models": [{"name": "shared-model"}]
                }
            }
        }
    }))
    .unwrap();
    let path = cfg_dir.join("settings.json");
    std::fs::write(&path, &original).expect("seed second-merge conflict");

    let result = Settings::load_from_disk();
    let after = std::fs::read_to_string(&path).expect("read settings after migration");
    restore_wgenty_home(previous);

    assert!(result.is_err(), "second merge silently discarded a key");
    assert_eq!(after, original, "second merge rewrote the old file");
}

#[test]
fn vendor_for_host_maps_builtin_vendors() {
    assert_eq!(vendor_for_host("api.kimi.com"), Some("kimi"));
    assert_eq!(vendor_for_host("api.moonshot.cn"), Some("kimi"));
    assert_eq!(vendor_for_host("api.z.ai"), Some("zhipu"));
    assert_eq!(vendor_for_host("open.bigmodel.cn"), Some("zhipu"));
    assert_eq!(vendor_for_host("api.deepseek.com"), Some("deepseek"));
    assert_eq!(vendor_for_host("api.openai.com"), Some("openai"));
    assert_eq!(vendor_for_host("api.anthropic.com"), Some("anthropic"));
    assert_eq!(vendor_for_host("llm.internal.io"), None);
}

#[test]
fn vendor_for_host_requires_domain_boundaries() {
    assert_eq!(vendor_for_host("notkimi.com"), None);
    assert_eq!(vendor_for_host("api.openai.com.evil.test"), None);
    assert_eq!(vendor_for_host("notz.ai"), None);
    assert_eq!(vendor_for_host("notanthropic.com"), None);
}

#[test]
#[serial_test::serial]
fn load_from_disk_migrates_profiles_to_providers_and_writes_bak() {
    let prev = std::env::var_os("WGENTY_HOME");
    let dir = tempfile::tempdir().expect("tempdir");
    let cfg_dir = dir.path().join(".wgenty-code");
    std::fs::create_dir_all(&cfg_dir).expect("create cfg dir");
    let settings_path = cfg_dir.join("settings.json");
    let original = format!("{{\"models\": {}}}", legacy_profiles_json());
    std::fs::write(&settings_path, &original).expect("seed settings");
    std::env::set_var("WGENTY_HOME", dir.path());

    let s = Settings::load_from_disk().expect("load");
    restore_wgenty_home(prev);

    // Migrated in memory: providers grouped, profiles gone.
    assert!(s.models.profiles.is_empty());
    assert!(s.models.providers.contains_key("zhipu"));
    assert!(s.models.providers.contains_key("kimi"));
    assert!(s.models.active_model.is_some());

    // Backup written next to settings.json as settings.json.bak-<date>,
    // preserving the untouched legacy content.
    let bak = std::fs::read_dir(&cfg_dir)
        .expect("read cfg dir")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .find(|n| n.starts_with("settings.json.bak-"))
        .expect("a settings.json.bak-<date> file must exist");
    let bak_content = std::fs::read_to_string(cfg_dir.join(&bak)).expect("read bak");
    assert_eq!(bak_content, original, "backup keeps the pre-migration file");

    // Rewritten settings.json no longer carries legacy profiles.
    let rewritten: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&settings_path).expect("read settings"))
            .expect("parse rewritten");
    assert!(rewritten["models"].get("profiles").is_none());
    assert!(rewritten["models"]["providers"].is_object());
}

#[test]
#[serial_test::serial]
fn load_from_disk_does_not_overwrite_same_day_bak() {
    let prev = std::env::var_os("WGENTY_HOME");
    let dir = tempfile::tempdir().expect("tempdir");
    let cfg_dir = dir.path().join(".wgenty-code");
    std::fs::create_dir_all(&cfg_dir).expect("create cfg dir");
    let settings_path = cfg_dir.join("settings.json");
    std::fs::write(
        &settings_path,
        format!("{{\"models\": {}}}", legacy_profiles_json()),
    )
    .expect("seed settings");
    std::env::set_var("WGENTY_HOME", dir.path());

    Settings::load_from_disk().expect("first load migrates");
    let bak_name = std::fs::read_dir(&cfg_dir)
        .expect("read cfg dir")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .find(|n| n.starts_with("settings.json.bak-"))
        .expect("bak exists after first load");
    let marker = "sentinel — do not overwrite";
    std::fs::write(cfg_dir.join(&bak_name), marker).expect("overwrite bak with sentinel");
    // Force a second migration pass over a legacy-shaped file.
    std::fs::write(
        &settings_path,
        format!("{{\"models\": {}}}", legacy_profiles_json()),
    )
    .expect("reseed legacy settings");
    Settings::load_from_disk().expect("second load migrates again");
    restore_wgenty_home(prev);

    let bak_content = std::fs::read_to_string(cfg_dir.join(&bak_name)).expect("read bak");
    assert_eq!(bak_content, marker, "same-day bak must not be overwritten");
}
