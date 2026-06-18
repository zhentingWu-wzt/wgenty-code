//! Type definitions for the TUI application layer.

use crate::agent::progress::SubagentProgress;
use crate::api::ChatMessage;
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
    /// A subagent progress update from daemon polling.
    SubagentUpdate(Box<SubagentProgress>),
    /// Toggle the subagent monitor panel.
    ToggleSubagentPanel,
    /// Retry a failed/cancelled subagent by node_id.
    RetrySubagent(String),
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
}
