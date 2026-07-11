use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Types of hook events
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "PascalCase")]
pub enum HookEvent {
    PreToolUse,
    PostToolUse,
    SessionStart,
    SessionEnd,
    Notification,
    /// CC-compatible: Triggered when the agent stops/completes
    Stop,
    /// CC-compatible: Triggered before user prompt is submitted
    UserPromptSubmit,
    /// CC-compatible: Triggered for permission requests
    PermissionRequest,
    /// Triggered when a slash command (e.g. /comet-design) is invoked
    SlashCommand,
}

impl std::fmt::Display for HookEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HookEvent::PreToolUse => write!(f, "PreToolUse"),
            HookEvent::PostToolUse => write!(f, "PostToolUse"),
            HookEvent::SessionStart => write!(f, "SessionStart"),
            HookEvent::SessionEnd => write!(f, "SessionEnd"),
            HookEvent::Notification => write!(f, "Notification"),
            HookEvent::Stop => write!(f, "Stop"),
            HookEvent::UserPromptSubmit => write!(f, "UserPromptSubmit"),
            HookEvent::PermissionRequest => write!(f, "PermissionRequest"),
            HookEvent::SlashCommand => write!(f, "SlashCommand"),
        }
    }
}

/// Source for injected context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ContextSource {
    Template(String),
    File(PathBuf),
    Inline(String),
}

/// Visibility of an injected context layer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LayerVisibility {
    Internal,
    Visible,
}

/// An option presented to the user in AskUser action
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserOption {
    pub label: String,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Actions that a hook can perform
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HookAction {
    Command {
        command: String,
        timeout_secs: u64,
    },
    InjectContext {
        source: ContextSource,
        priority: u8,
        visibility: LayerVisibility,
    },
    AskUser {
        question: String,
        options: Vec<UserOption>,
    },
}

/// A single hook definition from settings.json
#[derive(Debug, Clone, Serialize)]
pub struct HookDefinition {
    /// The event that triggers this hook
    pub event: HookEvent,
    /// CC-compatible: matcher for filtering hook execution.
    /// None/"" = match all, "ToolA|ToolB" = pipe-separated tool names.
    #[serde(default)]
    pub matcher: Option<String>,
    /// Optional workflow state condition (e.g. "build", "design").
    /// Hook only fires when Comet is in this state, if set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when_state: Option<String>,
    /// Actions to execute when the hook fires
    pub actions: Vec<HookAction>,
}

impl<'de> Deserialize<'de> for HookDefinition {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct HookDefHelper {
            event: HookEvent,
            #[serde(default)]
            matcher: Option<String>,
            #[serde(default)]
            when_state: Option<String>,
            #[serde(default)]
            command: Option<String>,
            #[serde(default)]
            timeout_secs: Option<u64>,
            #[serde(default)]
            // Accepted for forward compatibility with future
            // hook schemas; not yet used by the current engine.
            #[allow(dead_code)]
            hook_type: Option<String>,
            #[serde(default)]
            actions: Option<Vec<HookAction>>,
        }
        let helper = HookDefHelper::deserialize(deserializer)?;
        let actions = match helper.actions {
            Some(a) if !a.is_empty() => a,
            _ => vec![HookAction::Command {
                command: helper.command.unwrap_or_default(),
                timeout_secs: helper.timeout_secs.unwrap_or(30),
            }],
        };
        Ok(HookDefinition {
            event: helper.event,
            matcher: helper.matcher,
            when_state: helper.when_state,
            actions,
        })
    }
}

/// Context passed to hooks via stdin (JSON)
#[derive(Debug, Clone, Serialize)]
pub struct HookContext {
    pub event: String,
    pub tool_name: Option<String>,
    pub tool_input: Option<serde_json::Value>,
    pub tool_result: Option<String>,
    pub session_id: Option<String>,
    pub working_directory: String,
    pub timestamp: String,
    /// Current Comet workflow phase (open/design/build/verify/archive), if any.
    /// Deprecated: use `workflow_state` instead.
    pub comet_phase: Option<String>,
    /// Generic workflow state (replaces comet_phase for the runtime).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow_state: Option<String>,
    /// Key-value variables for hook context (e.g., from slash commands).
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub variables: HashMap<String, String>,
}

impl HookContext {
    /// Set the comet_phase field and return self (builder pattern).
    #[deprecated(note = "Use `with_workflow_state` instead")]
    pub fn with_comet_phase(mut self, phase: Option<String>) -> Self {
        self.workflow_state = phase.clone();
        self.comet_phase = phase;
        self
    }

    /// Set the workflow_state (and comet_phase for backward compat) field and return self.
    pub fn with_workflow_state(mut self, state: Option<String>) -> Self {
        self.workflow_state = state.clone();
        self.comet_phase = state;
        self
    }

    /// Set the workflow_state field and return self (shorthand alias).
    pub fn with_state(mut self, state: Option<String>) -> Self {
        self.workflow_state = state.clone();
        self.comet_phase = state;
        self
    }
}

/// Result returned from a hook via stdout (JSON)
#[derive(Debug, Clone, Deserialize)]
pub struct HookResult {
    #[serde(default)]
    pub continue_execution: bool, // true = proceed, false = block
    #[serde(default)]
    pub reason: Option<String>, // reason if blocked
}

/// User's answer to an AskUser action
#[derive(Debug, Clone)]
pub struct UserAnswer {
    pub selected: Vec<String>,
}

/// Outcome of executing a single hook
#[derive(Debug, Clone)]
pub struct HookOutcome {
    pub def: HookDefinition,
    pub continue_execution: bool,
    pub reason: Option<String>,
    pub injected_content: Option<String>,
    pub user_answer: Option<UserAnswer>,
    // 新增：当 outcome 来自 InjectContext 时填充
    pub injection_priority: Option<u8>,
    pub injection_visibility: Option<LayerVisibility>,
}

/// A normalized injection fragment derived from one or more `HookOutcome`s.
/// Consumers (e.g. the `<system-reminder>` channel) collect these via
/// [`collect_injections`] and render them in priority order.
#[derive(Debug, Clone)]
pub struct InjectedFragment {
    pub content: String,
    pub priority: u8,
    pub visibility: LayerVisibility,
    pub source_label: String,
}

/// Collect `InjectedFragment`s from a slice of `HookOutcome`s.
///
/// - Outcomes without `injected_content` (or with empty content) are skipped.
/// - Priority defaults to `50` and visibility to `Visible` when not provided.
/// - Fragments are sorted by `priority` ascending. The sort is stable, so
///   ties preserve the original outcome order.
/// - `source_label` is `"hook:UserPromptSubmit:<idx>"` where `idx` is the
///   zero-based index of the outcome in the input slice.
pub fn collect_injections(outcomes: &[HookOutcome]) -> Vec<InjectedFragment> {
    let mut out: Vec<InjectedFragment> = outcomes
        .iter()
        .enumerate()
        .filter_map(|(idx, oc)| {
            let content = oc.injected_content.as_ref()?;
            if content.is_empty() {
                return None;
            }
            Some(InjectedFragment {
                content: content.clone(),
                priority: oc.injection_priority.unwrap_or(50),
                visibility: oc
                    .injection_visibility
                    .clone()
                    .unwrap_or(LayerVisibility::Visible),
                source_label: format!("hook:UserPromptSubmit:{idx}"),
            })
        })
        .collect();
    out.sort_by_key(|f| f.priority); // stable sort: ties 保留传入顺序
    out
}
