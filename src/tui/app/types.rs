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
        reason: String,
        rule: String,
        responder: PermissionResponder,
    },
    /// ask_user_question was invoked
    QuestionAsked {
        question: String,
        options: Vec<String>,
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
    HistoryLoaded(Vec<ChatMessage>),
    SaveSession,
    /// Delete a session by id
    DeleteSession(String),
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
