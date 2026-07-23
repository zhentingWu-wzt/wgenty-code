//! Type definitions for the TUI application layer.

use crate::api::ChatMessage;
use crate::daemon::models::LocalAgentViewResponse;
use crate::state::agent_phase::{TurnAbortReason, TurnId};
use crate::tui::client::{SessionInfo, TodoItem};
use crossterm::event::KeyEvent;
use ratatui::style::Color;

/// Agent operating mode, cycled via Shift+Tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentMode {
    Normal,
    PlanMode,
    AcceptEdits,
    Yolo,
}

impl AgentMode {
    pub fn label(&self) -> &str {
        match self {
            AgentMode::Normal => "NORMAL",
            AgentMode::PlanMode => "PLAN",
            AgentMode::AcceptEdits => "ACCEPT EDIT",
            AgentMode::Yolo => "YOLO",
        }
    }

    pub fn color(&self) -> Color {
        match self {
            AgentMode::Normal => Color::Rgb(147, 112, 219),
            AgentMode::PlanMode => Color::Rgb(255, 200, 80),
            AgentMode::AcceptEdits => Color::Rgb(80, 220, 120),
            AgentMode::Yolo => Color::Rgb(255, 90, 90),
        }
    }

    pub fn next(&self) -> Self {
        match self {
            AgentMode::Normal => AgentMode::PlanMode,
            AgentMode::PlanMode => AgentMode::AcceptEdits,
            AgentMode::AcceptEdits => AgentMode::Yolo,
            AgentMode::Yolo => AgentMode::Normal,
        }
    }

    /// Map to the daemon-side `RootPermissionMode`. `PlanMode` maps to `Normal`
    /// (no auto-approve); the TUI enforces plan-mode restrictions locally.
    pub fn to_root_permission_mode(&self) -> crate::config::agent::RootPermissionMode {
        match self {
            AgentMode::Normal | AgentMode::PlanMode => {
                crate::config::agent::RootPermissionMode::Normal
            }
            AgentMode::AcceptEdits => crate::config::agent::RootPermissionMode::AcceptEdits,
            AgentMode::Yolo => crate::config::agent::RootPermissionMode::Yolo,
        }
    }

    /// Map to sandbox [`crate::sandbox::EffectiveMode`] (Plan stays Plan).
    pub fn to_effective_mode(&self) -> crate::sandbox::EffectiveMode {
        match self {
            AgentMode::PlanMode => crate::sandbox::EffectiveMode::Plan,
            AgentMode::Normal => crate::sandbox::EffectiveMode::Normal,
            AgentMode::AcceptEdits => crate::sandbox::EffectiveMode::AcceptEdits,
            AgentMode::Yolo => crate::sandbox::EffectiveMode::Yolo,
        }
    }

    /// Prompt `sandbox_mode` string (permissions layer / Codex-aligned labels).
    ///
    /// - Plan → `read-only` (full-disk read; write tools policy-gated; net off)
    /// - Normal / AcceptEdits → `workspace-write` (full-disk read; workspace write)
    /// - Yolo → `disabled` (no OS sandbox; Full Access)
    pub fn prompt_sandbox_mode(&self) -> &'static str {
        match self {
            AgentMode::PlanMode => "read-only",
            AgentMode::Normal | AgentMode::AcceptEdits => "workspace-write",
            AgentMode::Yolo => "disabled",
        }
    }

    /// Prompt `approval_policy` string.
    ///
    /// - Yolo → `never` (auto-approve all policy Asks)
    /// - others → `on-request` (AcceptEdits still auto-approves edit tools in UI)
    pub fn prompt_approval_policy(&self) -> &'static str {
        match self {
            AgentMode::Yolo => "never",
            AgentMode::Normal | AgentMode::PlanMode | AgentMode::AcceptEdits => "on-request",
        }
    }
}

#[cfg(test)]
mod agent_mode_effective_tests {
    use super::*;
    use crate::config::agent::RootPermissionMode;
    use crate::sandbox::EffectiveMode;

    #[test]
    fn accept_edits_auto_approves_by_tool_name_not_path_rule() {
        // session_rule for writes is typically `path:…`, not the tool name.
        // AcceptEdits must key off tool_name via RootPermissionMode.
        let mode = AgentMode::AcceptEdits.to_root_permission_mode();
        assert!(mode.auto_approves("file_edit"));
        assert!(mode.auto_approves("file_write"));
        assert!(mode.auto_approves("apply_patch"));
        assert!(!mode.auto_approves("path:/tmp/foo"));
        assert!(!mode.auto_approves("exec_command"));
    }

    #[test]
    fn agent_mode_plan_to_effective_plan() {
        assert_eq!(AgentMode::PlanMode.to_effective_mode(), EffectiveMode::Plan);
        assert_eq!(
            AgentMode::PlanMode.to_root_permission_mode(),
            RootPermissionMode::Normal
        );
    }

    #[test]
    fn agent_mode_yolo_maps_both() {
        assert_eq!(AgentMode::Yolo.to_effective_mode(), EffectiveMode::Yolo);
        assert_eq!(
            AgentMode::Yolo.to_root_permission_mode(),
            RootPermissionMode::Yolo
        );
        assert_eq!(AgentMode::Yolo.prompt_sandbox_mode(), "disabled");
        assert_eq!(AgentMode::Yolo.prompt_approval_policy(), "never");
    }

    #[test]
    fn agent_mode_prompt_permissions_normal_and_plan() {
        assert_eq!(AgentMode::Normal.prompt_sandbox_mode(), "workspace-write");
        assert_eq!(AgentMode::Normal.prompt_approval_policy(), "on-request");
        assert_eq!(AgentMode::PlanMode.prompt_sandbox_mode(), "read-only");
        assert_eq!(AgentMode::PlanMode.prompt_approval_policy(), "on-request");
        assert_eq!(
            AgentMode::AcceptEdits.prompt_sandbox_mode(),
            "workspace-write"
        );
        assert_eq!(
            AgentMode::AcceptEdits.prompt_approval_policy(),
            "on-request"
        );
    }

    #[test]
    fn agent_mode_labels_assemble_into_permissions_layer() {
        // End-to-end: mode → PromptContext → permissions system message content.
        use crate::config::Settings;
        use crate::prompts::{assemble_instructions, PromptContext};

        let settings = Settings::default();
        for mode in [
            AgentMode::Normal,
            AgentMode::PlanMode,
            AgentMode::AcceptEdits,
            AgentMode::Yolo,
        ] {
            let ctx = PromptContext::new()
                .with_cwd("/tmp")
                .with_shell("zsh")
                .with_sandbox(mode.prompt_sandbox_mode())
                .with_approval(mode.prompt_approval_policy());
            let assembled = assemble_instructions(&settings, &ctx);
            let perm = assembled
                .system_messages
                .iter()
                .find_map(|m| {
                    m.content
                        .as_deref()
                        .filter(|c| c.contains("<permissions_instructions>"))
                })
                .unwrap_or_else(|| panic!("{mode:?} should inject permissions layer"));

            match mode {
                AgentMode::PlanMode => {
                    assert!(perm.contains("read-only"), "{mode:?}: {perm}");
                    assert!(perm.contains("on-request"), "{mode:?}: {perm}");
                }
                AgentMode::Normal | AgentMode::AcceptEdits => {
                    assert!(perm.contains("workspace-write"), "{mode:?}: {perm}");
                    assert!(perm.contains("on-request"), "{mode:?}: {perm}");
                }
                AgentMode::Yolo => {
                    assert!(perm.contains("disabled"), "{mode:?}: {perm}");
                    assert!(perm.contains("never"), "{mode:?}: {perm}");
                }
            }
        }
    }
}

/// A selectable option for `ask_user_question`, carrying a short label and a
/// longer description (explanation) shown beneath the label in the panel.
#[derive(Debug, Clone)]
pub struct QuestionOption {
    /// Short display label (1-5 words).
    pub label: String,
    /// Detailed explanation of this option, wrapped beneath the label.
    pub description: String,
}

/// Wraps a oneshot sender for returning question answers.
/// Manual Debug impl because `oneshot::Sender` doesn't implement Debug.
pub struct QuestionResponder(pub Option<tokio::sync::oneshot::Sender<Vec<String>>>);
impl std::fmt::Debug for QuestionResponder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("QuestionResponder").finish()
    }
}

#[derive(Debug)]
pub enum PermissionResponse {
    AllowOnce,
    AlwaysAllow,
    Deny,
}

/// Wraps a oneshot sender for returning permission decisions.
pub struct PermissionResponder(pub Option<tokio::sync::oneshot::Sender<PermissionResponse>>);
impl std::fmt::Debug for PermissionResponder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("PermissionResponder").finish()
    }
}

/// Events that drive the UI loop.
#[derive(Debug)]
pub enum AppEvent {
    /// Full key event for tui-textarea processing (CJK/IME support)
    KeyEvent(Box<KeyEvent>),
    /// User submitted input text
    Submit(String),
    /// An SSE content delta arrived
    ContentDelta(String),
    /// An SSE reasoning delta arrived
    ReasoningDelta(String),
    /// Streaming completed
    StreamDone {
        finish_reason: String,
    },
    /// LLM started generating tool calls (bridge between text and execution)
    PreparingTools,
    /// A tool call started
    ToolStart {
        name: String,
        args: serde_json::Value,
    },
    /// A tool result arrived
    ToolResult {
        name: String,
        args: serde_json::Value,
        content: String,
    },
    /// Permission is needed
    PermissionRequired {
        /// Canonical tool name (`file_edit`, `exec_command`, …) for mode auto-approve.
        tool_name: String,
        reason: String,
        /// Session rule key (`path:…`, `command:…`, `tool:…`) for AlwaysAllow storage.
        rule: String,
        responder: PermissionResponder,
    },
    /// ask_user_question was invoked
    QuestionAsked {
        question: String,
        options: Vec<QuestionOption>,
        multi_select: bool,
        responder: QuestionResponder,
    },
    /// A stream error occurred
    StreamError(String),
    /// Conversation compaction started: the transcript is being archived and
    /// summarized so the UI can show a "compacting..." indicator.
    CompactionStarted,
    /// Conversation was compacted: earlier history replaced with a model-
    /// generated summary. `summary_chars` is the char length of the summary.
    ContextCompacted {
        summary_chars: usize,
    },
    /// Connecting to the LLM API (attempt N of M)
    Connecting {
        attempt: usize,
        max_retries: usize,
    },
    /// A turn (user-input → final response) completed; start next queued input if any
    TurnComplete,
    /// A turn began processing
    TurnStarted {
        turn_id: TurnId,
    },
    /// A turn was aborted before normal completion
    TurnAborted {
        reason: TurnAbortReason,
    },
    /// Tick for periodic refresh
    Tick,
    /// Toggle session popup
    ToggleSessions,
    /// Toggle memory browser popup
    ToggleMemory,
    /// Memory list finished loading from MemoryManager
    MemoryListLoaded(Vec<crate::tui::components::memory::MemoryListItem>),
    /// Toggle task panel
    ToggleTaskPanel,
    /// Pasted text from bracketed paste
    Paste(String),
    /// Mouse scroll (positive = up, negative = down)
    MouseScrolled(i16),
    /// Ctrl+C pressed (double-press to quit)
    CtrlCPressed,
    /// Structured plan updated via update_plan tool
    PlanUpdate(serde_json::Value),
    /// User-visible system notice (e.g. per-turn reminder transcript portion).
    SystemNotice(String),
    /// Sessions loaded from daemon
    SessionListLoaded(Vec<SessionInfo>),
    /// Model history plus optional UI transcript from a loaded session.
    /// When `ui_messages` is non-empty, the TUI restores display from that track.
    HistoryLoaded {
        messages: Vec<ChatMessage>,
        ui_messages: Vec<crate::context::SessionUiMessage>,
    },
    SaveSession,
    /// Delete a session by id
    DeleteSession(String),
    /// Delete a memory by (origin, id) from the memory browser
    DeleteMemory(crate::context::MemoryOrigin, String),
    /// Toggle collapse all paragraphs
    ToggleCollapseAll,
    /// Toggle collapse latest message paragraphs
    ToggleCollapseLatest,
    /// Undo checkpoint result with diff
    UndoResult(String),
    /// Todo items updated from daemon
    TodosUpdated(Vec<TodoItem>),
    /// Settings were hot-reloaded from disk
    ConfigChanged(Box<crate::config::Settings>),
    /// A scoped agent local view (self + direct children) from the daemon.
    /// Carries the `generation` at which the polling loop was spawned so the
    /// handler can discard stale views from a previous generation (e.g. after
    /// `/clear` or a generation reset).
    AgentLocalView {
        view: Box<LocalAgentViewResponse>,
        generation: u64,
    },
    /// Background task/subagent result notification for display in chat.
    BackgroundTaskResult(String),
    /// A new task generation was established after `/clear` or shutdown
    /// cancellation. Obsolete root-direct subtrees are cancelled by the
    /// daemon; the app adopts the new generation and clears local views.
    AgentGenerationReset {
        generation: u64,
    },
    /// Descend into a direct child via its opaque navigation capability.
    /// The daemon verifies the capability and returns the child's local view.
    NavigateAgent {
        capability: String,
    },
    /// A capability-bound navigation returned a new local view. Pushes the
    /// current frame onto the back stack and replaces the loaded view.
    AgentViewNavigated(Box<LocalAgentViewResponse>),
    /// Pop the navigation back stack to restore the previous scoped view.
    NavigateAgentBack,
    /// Cross-session memory recall completed in the background at startup.
    /// Carries formatted memory lines to inject into the conversation context.
    MemoriesReady(Vec<String>),
    /// Global memory loading completed in the background at startup.
    /// Carries formatted global memory lines for the system prompt block.
    GlobalMemoriesReady(Vec<String>),
    /// Skill discovery completed in the background at startup.
    /// Carries the merged skill inventory, external skill registry, and
    /// comet workflow entry commands for the command router / completion engine.
    SkillsReady(Box<SkillsReadyData>),
}

/// Payload for [`AppEvent::SkillsReady`].
pub struct SkillsReadyData {
    /// Merged skill inventory (internal + external, filtered by exposure rules).
    pub skill_inventory: Vec<crate::prompts::SkillEntry>,
    /// Discovered external skill registry (if any).
    pub external_skill_registry: Option<std::sync::Arc<crate::knowledge::ExternalSkillRegistry>>,
    /// Comet entry commands parsed from workflow.yaml or external registry.
    pub comet_entry_commands: Vec<String>,
}

impl std::fmt::Debug for SkillsReadyData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SkillsReadyData")
            .field("skill_inventory_len", &self.skill_inventory.len())
            .field(
                "has_external_registry",
                &self.external_skill_registry.is_some(),
            )
            .field("comet_entry_commands", &self.comet_entry_commands)
            .finish()
    }
}

/// UI state for a single message in the chat view.
#[derive(Debug, Clone, PartialEq)]
pub enum MessageRole {
    User,
    Assistant,
    Tool,
    System,
}

#[derive(Debug, Clone)]
pub struct UIMessage {
    pub role: MessageRole,
    pub content: String,
    pub tool_name: Option<String>,
    pub tool_args: Option<serde_json::Value>,
    pub content_collapsed: bool,
    pub tool_collapsed: bool,
    pub tool_running: bool,
    pub diff_data: Option<DiffData>,
    pub tool_metadata: Option<serde_json::Value>,
}

impl UIMessage {
    /// Convert a live TUI row into the serializable session UI track.
    pub fn to_session_ui_message(&self) -> crate::context::SessionUiMessage {
        crate::context::SessionUiMessage {
            role: match self.role {
                MessageRole::User => "user".to_string(),
                MessageRole::Assistant => "assistant".to_string(),
                MessageRole::Tool => "tool".to_string(),
                MessageRole::System => "system".to_string(),
            },
            content: self.content.clone(),
            tool_name: self.tool_name.clone(),
            tool_args: self.tool_args.clone(),
            content_collapsed: self.content_collapsed,
            tool_collapsed: self.tool_collapsed,
            diff_data: self
                .diff_data
                .as_ref()
                .map(|d| crate::context::SessionDiffData {
                    file_path: d.file_path.clone(),
                    old_content: d.old_content.clone(),
                    new_content: d.new_content.clone(),
                }),
            tool_metadata: self.tool_metadata.clone(),
        }
    }

    /// Restore a TUI row from the persisted UI track. `tool_running` is always false.
    pub fn from_session_ui_message(msg: crate::context::SessionUiMessage) -> Self {
        let role = match msg.role.as_str() {
            "assistant" => MessageRole::Assistant,
            "tool" => MessageRole::Tool,
            "system" => MessageRole::System,
            _ => MessageRole::User,
        };
        Self {
            role,
            content: msg.content,
            tool_name: msg.tool_name,
            tool_args: msg.tool_args,
            content_collapsed: msg.content_collapsed,
            tool_collapsed: msg.tool_collapsed,
            tool_running: false,
            diff_data: msg.diff_data.map(|d| DiffData {
                file_path: d.file_path,
                old_content: d.old_content,
                new_content: d.new_content,
            }),
            tool_metadata: msg.tool_metadata,
        }
    }
}

/// Structured diff data for syntax-highlighted diff rendering in the TUI.
#[derive(Debug, Clone)]
pub struct DiffData {
    pub file_path: String,
    pub old_content: String,
    pub new_content: String,
}

/// Completion state for @ and / auto-completion.
#[derive(Debug, Clone)]
pub struct CompletionState {
    pub prefix: char,
    pub partial: String,
    pub matches: Vec<crate::tui::completion::CompletionMatch>,
    pub selected_index: usize,
    pub visible: bool,
    pub tabs: Vec<String>,
    pub active_tab: usize,
}

impl CompletionState {
    pub fn new(
        prefix: char,
        partial: String,
        matches: Vec<crate::tui::completion::CompletionMatch>,
    ) -> Self {
        let tabs = completion_tabs(prefix, &matches);
        Self {
            prefix,
            partial,
            matches,
            selected_index: 0,
            visible: true,
            tabs,
            active_tab: 0,
        }
    }

    pub fn replace_matches(&mut self, matches: Vec<crate::tui::completion::CompletionMatch>) {
        let previous_tab = self.active_tab_label().map(ToOwned::to_owned);
        self.matches = matches;
        self.tabs = completion_tabs(self.prefix, &self.matches);
        self.active_tab = previous_tab
            .and_then(|tab| self.tabs.iter().position(|candidate| candidate == &tab))
            .unwrap_or(0);
        self.selected_index = 0;
    }

    pub fn visible_matches(&self) -> Vec<&crate::tui::completion::CompletionMatch> {
        let Some(tab) = self.active_tab_label() else {
            return self.matches.iter().collect();
        };
        self.matches
            .iter()
            .filter(|item| item.category == tab)
            .collect()
    }

    pub fn active_tab_label(&self) -> Option<&str> {
        self.tabs.get(self.active_tab).map(String::as_str)
    }

    pub fn selected_match(&self) -> Option<&crate::tui::completion::CompletionMatch> {
        self.visible_matches().get(self.selected_index).copied()
    }

    pub fn move_next(&mut self) {
        let count = self.visible_matches().len();
        if count > 0 {
            self.selected_index = (self.selected_index + 1) % count;
        }
    }

    pub fn move_previous(&mut self) {
        let count = self.visible_matches().len();
        if count > 0 {
            self.selected_index = if self.selected_index == 0 {
                count - 1
            } else {
                self.selected_index - 1
            };
        }
    }

    pub fn move_to_previous_tab(&mut self) {
        if self.tabs.len() > 1 {
            self.active_tab = if self.active_tab == 0 {
                self.tabs.len() - 1
            } else {
                self.active_tab - 1
            };
            self.selected_index = 0;
        }
    }

    pub fn move_to_next_tab(&mut self) {
        if self.tabs.len() > 1 {
            self.active_tab = (self.active_tab + 1) % self.tabs.len();
            self.selected_index = 0;
        }
    }
}

fn completion_tabs(
    prefix: char,
    matches: &[crate::tui::completion::CompletionMatch],
) -> Vec<String> {
    if prefix != '/' {
        return Vec::new();
    }

    let mut tabs = Vec::new();
    for item in matches {
        if !tabs.contains(&item.category) {
            tabs.push(item.category.clone());
        }
    }
    tabs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::completion::CompletionMatch;

    fn completion_match(text: &str, category: &str) -> CompletionMatch {
        CompletionMatch {
            text: text.to_string(),
            description: String::new(),
            args_hint: None,
            category: category.to_string(),
        }
    }

    #[test]
    fn ui_message_session_track_round_trip() {
        let original = UIMessage {
            role: MessageRole::Tool,
            content: "patched".to_string(),
            tool_name: Some("apply_patch".to_string()),
            tool_args: Some(serde_json::json!({"path": "x.rs"})),
            content_collapsed: true,
            tool_collapsed: false,
            tool_running: true, // must not persist
            diff_data: Some(DiffData {
                file_path: "x.rs".to_string(),
                old_content: "a".to_string(),
                new_content: "b".to_string(),
            }),
            tool_metadata: Some(serde_json::json!({"ok": true})),
        };

        let wire = original.to_session_ui_message();
        let restored = UIMessage::from_session_ui_message(wire);

        assert_eq!(restored.role, MessageRole::Tool);
        assert_eq!(restored.content, "patched");
        assert_eq!(restored.tool_name.as_deref(), Some("apply_patch"));
        assert_eq!(restored.tool_args, original.tool_args);
        assert!(restored.content_collapsed);
        assert!(!restored.tool_collapsed);
        assert!(!restored.tool_running);
        assert_eq!(
            restored.diff_data.as_ref().map(|d| d.file_path.as_str()),
            Some("x.rs")
        );
        assert_eq!(restored.tool_metadata, original.tool_metadata);
    }

    #[test]
    fn slash_completion_state_groups_matches_into_tabs() {
        let state = CompletionState::new(
            '/',
            String::new(),
            vec![
                completion_match("clear", "Built-in"),
                completion_match("comet", "Comet"),
                completion_match("comet-build", "Comet"),
            ],
        );

        assert_eq!(state.tabs, vec!["Built-in", "Comet"]);
        assert_eq!(state.active_tab_label(), Some("Built-in"));
        assert_eq!(state.visible_matches().len(), 1);
    }

    #[test]
    fn slash_completion_state_switches_tabs_and_resets_selection() {
        let mut state = CompletionState::new(
            '/',
            String::new(),
            vec![
                completion_match("clear", "Built-in"),
                completion_match("plan", "Built-in"),
                completion_match("comet", "Comet"),
            ],
        );
        state.move_next();

        state.move_to_next_tab();

        assert_eq!(state.active_tab_label(), Some("Comet"));
        assert_eq!(state.selected_index, 0);
        assert_eq!(
            state.selected_match().map(|item| item.text.as_str()),
            Some("comet")
        );
    }

    #[test]
    fn skill_completion_state_has_no_tabs() {
        let state =
            CompletionState::new('@', String::new(), vec![completion_match("comet", "Skill")]);

        assert!(state.tabs.is_empty());
        assert_eq!(state.visible_matches().len(), 1);
    }
}

// ── Scoped agent navigation history (Task 14) ────────────────────────────────

/// One frame in the scoped agent navigation stack: the currently loaded view
/// plus which entry is selected (0 for self, 1+ for direct children).
#[derive(Debug, Clone)]
pub struct AgentViewFrame {
    pub view: LocalAgentViewResponse,
    pub selected: usize,
    pub breadcrumb_label: String,
}

/// Owned navigation state for capability-driven agent tree traversal. The
/// TUI starts at the root view and pushes frames as the user descends into
/// direct children via their navigation capability.
#[derive(Debug, Clone, Default)]
pub struct AgentNavigationState {
    pub current: Option<AgentViewFrame>,
    pub back_stack: Vec<AgentViewFrame>,
}
