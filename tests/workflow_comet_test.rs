//! Integration tests for Comet workflow.yaml — declarative workflow definition.
//!
//! Tests verify:
//! 1. File exists and is valid YAML
//! 2. entry_commands contains comet subcommands
//! 3. hooks configured for SlashCommand / PreToolUse / UserPromptSubmit events
//! 4. all context layers have visibility: internal

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Deserialization types for workflow.yaml — used in tests only.
#[derive(Debug, Deserialize, Serialize)]
struct WorkflowYaml {
    name: String,
    entry_commands: Vec<String>,
    #[serde(default)]
    state: Option<StateConfig>,
    #[serde(default)]
    hooks: Option<HooksConfig>,
    #[serde(default)]
    context: Option<Vec<ContextLayer>>,
    #[serde(default)]
    templates: Option<TemplatesConfig>,
}

#[derive(Debug, Deserialize, Serialize)]
struct StateConfig {
    read: StateScript,
    write: StateScript,
}

#[derive(Debug, Deserialize, Serialize)]
struct StateScript {
    script: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct HooksConfig {
    #[serde(rename = "SlashCommand", default)]
    slash_command: Vec<HookAction>,
    #[serde(rename = "PreToolUse", default)]
    pre_tool_use: Vec<HookAction>,
    #[serde(rename = "UserPromptSubmit", default)]
    user_prompt_submit: Vec<HookAction>,
}

#[derive(Debug, Deserialize, Serialize)]
struct HookAction {
    action: String,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    script: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct ContextLayer {
    id: String,
    #[serde(default)]
    priority: u8,
    visibility: String,
    #[serde(default)]
    source: Option<serde_yaml::Value>,
}

#[derive(Debug, Deserialize, Serialize)]
struct TemplatesConfig {
    #[serde(rename = "phase_rules", default)]
    phase_rules: Option<BTreeMap<String, String>>,
}

fn workflow_yaml_path() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.join(".wgenty-code/skills/comet/workflow.yaml")
}

// ── RED tests (will fail until workflow.yaml is created) ──

#[test]
fn test_workflow_yaml_exists_and_valid() {
    let path = workflow_yaml_path();
    assert!(
        path.exists(),
        "workflow.yaml must exist at {}",
        path.display()
    );

    let raw = std::fs::read_to_string(&path).expect("must be able to read workflow.yaml");
    let _wf: WorkflowYaml =
        serde_yaml::from_str(&raw).expect("workflow.yaml must be valid YAML matching the schema");
}

#[test]
fn test_entry_commands_contain_comet_subcommands() {
    let path = workflow_yaml_path();
    let raw = std::fs::read_to_string(&path).expect("read workflow.yaml");
    let wf: WorkflowYaml = serde_yaml::from_str(&raw).expect("valid YAML");

    let required: &[&str] = &[
        "comet",
        "comet-open",
        "comet-design",
        "comet-build",
        "comet-verify",
        "comet-archive",
    ];
    for cmd in required {
        assert!(
            wf.entry_commands.contains(&cmd.to_string()),
            "entry_commands must contain '{}'",
            cmd
        );
    }
}

#[test]
fn test_hooks_configured_for_all_guard_events() {
    let path = workflow_yaml_path();
    let raw = std::fs::read_to_string(&path).expect("read workflow.yaml");
    let wf: WorkflowYaml = serde_yaml::from_str(&raw).expect("valid YAML");

    let hooks = wf.hooks.expect("workflow.yaml must define hooks");

    assert!(
        !hooks.slash_command.is_empty(),
        "hooks must include SlashCommand actions"
    );
    assert!(
        !hooks.pre_tool_use.is_empty(),
        "hooks must include PreToolUse actions"
    );
    assert!(
        !hooks.user_prompt_submit.is_empty(),
        "hooks must include UserPromptSubmit actions"
    );

    // SlashCommand must have an inject_context action referencing SKILL.md
    let has_skill_inject = hooks
        .slash_command
        .iter()
        .any(|a| a.action == "inject_context" && a.source.as_deref() == Some("SKILL.md"));
    assert!(
        has_skill_inject,
        "SlashCommand hooks must include inject_context with source SKILL.md"
    );

    // PreToolUse must have a phase guard script action
    let has_phase_guard = hooks.pre_tool_use.iter().any(|a| {
        a.action == "run_script"
            && a.script.as_deref().map_or(false, |s| {
                s.contains("comet-guard") || s.contains("comet-hook-guard")
            })
    });
    assert!(
        has_phase_guard,
        "PreToolUse hooks must include a run_script action for phase guard"
    );

    // UserPromptSubmit must have a rule injection action
    let has_rule_injection = hooks
        .user_prompt_submit
        .iter()
        .any(|a| a.action == "inject_rules" || a.action == "inject_context");
    assert!(
        has_rule_injection,
        "UserPromptSubmit hooks must include an inject_rules or inject_context action"
    );
}

#[test]
fn test_context_layers_have_internal_visibility() {
    let path = workflow_yaml_path();
    let raw = std::fs::read_to_string(&path).expect("read workflow.yaml");
    let wf: WorkflowYaml = serde_yaml::from_str(&raw).expect("valid YAML");

    let layers = wf
        .context
        .expect("workflow.yaml must define context layers");

    assert!(
        !layers.is_empty(),
        "context must contain at least one layer"
    );

    for layer in &layers {
        assert_eq!(
            layer.visibility, "internal",
            "context layer '{}' must have visibility: internal (no leaking to user-visible chat)",
            layer.id
        );
    }

    // Specific layers must be present
    let layer_ids: Vec<&str> = layers.iter().map(|l| l.id.as_str()).collect();
    assert!(
        layer_ids.contains(&"phase-instruction"),
        "context must include 'phase-instruction' layer"
    );
    assert!(
        layer_ids.contains(&"coordinator-reminder"),
        "context must include 'coordinator-reminder' layer"
    );
}

#[test]
fn test_templates_have_five_phase_rules_in_chinese() {
    let path = workflow_yaml_path();
    let raw = std::fs::read_to_string(&path).expect("read workflow.yaml");
    let wf: WorkflowYaml = serde_yaml::from_str(&raw).expect("valid YAML");

    let templates = wf.templates.expect("workflow.yaml must define templates");
    let phase_rules = templates
        .phase_rules
        .expect("templates must define phase_rules");

    let required_phases = ["open", "design", "build", "verify", "archive"];
    for phase in &required_phases {
        let rules = phase_rules
            .get(*phase)
            .unwrap_or_else(|| panic!("phase_rules must contain '{}'", phase));
        assert!(!rules.is_empty(), "phase_rules.{} must not be empty", phase);
        // Verify Chinese characters are present — at minimum some CJK range
        let has_chinese = rules
            .chars()
            .any(|c| c as u32 >= 0x4E00 && c as u32 <= 0x9FFF);
        assert!(
            has_chinese,
            "phase_rules.{} must contain Chinese text",
            phase
        );
    }
}

#[test]
fn test_state_read_write_scripts_configured() {
    let path = workflow_yaml_path();
    let raw = std::fs::read_to_string(&path).expect("read workflow.yaml");
    let wf: WorkflowYaml = serde_yaml::from_str(&raw).expect("valid YAML");

    let state = wf
        .state
        .expect("workflow.yaml must define state configuration");

    assert!(
        !state.read.script.is_empty(),
        "state.read.script must be configured"
    );
    assert!(
        !state.write.script.is_empty(),
        "state.write.script must be configured"
    );
    assert!(
        state.read.script.contains("current"),
        "state.read.script must reference state current command"
    );
    assert!(
        state.write.script.contains("set"),
        "state.write.script must reference state set command"
    );
}
