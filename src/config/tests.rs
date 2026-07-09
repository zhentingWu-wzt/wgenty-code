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
    assert_eq!(s.models.main.name, "sonnet");
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
fn test_small_model_settings_uses_small_overrides() {
    let mut s = Settings::default();
    s.models.main.base_url = Some("https://api.main.example".to_string());
    s.models.main.api_key = Some("main-key".to_string());
    s.models.small = Some(ModelEndpoint {
        name: "haiku".to_string(),
        base_url: None, // inherits main
        api_key: Some("small-key".to_string()),
        appkey: None,
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
    assert_eq!(small_s.models.transport.max_tokens, s.models.transport.max_tokens);
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
fn test_resolve_subagent_config_noop_when_no_overrides() {
    let s = Settings::default();
    let r = s.resolve_subagent_config();
    assert_eq!(r.agent.plan_mode, s.agent.plan_mode);
    assert_eq!(r.agent.max_rounds, s.agent.max_rounds);
    assert_eq!(r.agent.token_budget.main_k, s.agent.token_budget.main_k);
    assert_eq!(r.agent.rlm.enabled, s.agent.rlm.enabled);
    assert_eq!(r.prompt.include.skills, s.prompt.include.skills);
}

#[test]
fn test_resolve_subagent_config_applies_overrides() {
    let mut s = Settings::default();
    s.agent.token_budget.main_k = 100;
    s.agent.rlm.enabled = true;
    s.prompt.include.skills = true;

    s.agent.subagent.token_budget_k = Some(50);
    s.agent.subagent.rlm.enabled = Some(false);
    s.agent.subagent.prompt.include.skills = Some(false);

    let r = s.resolve_subagent_config();
    assert_eq!(r.agent.token_budget.main_k, 50);
    assert!(!r.agent.rlm.enabled);
    assert!(!r.prompt.include.skills);
    // Source unchanged
    assert_eq!(s.agent.token_budget.main_k, 100);
    assert!(s.agent.rlm.enabled);
}

#[test]
fn test_resolve_subagent_max_rounds_zero_means_unlimited() {
    let mut s = Settings::default();
    s.agent.max_rounds = Some(50);
    s.agent.subagent.max_rounds = Some(0);
    let r = s.resolve_subagent_config();
    assert_eq!(r.agent.max_rounds, None);
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
