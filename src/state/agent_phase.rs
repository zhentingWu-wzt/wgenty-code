//! Agent state machine — formal Phase, Turn lifecycle, and ReviewDecision types.
//!
//! Replaces the ad-hoc `status: String` field with a compile-time-checked enum
//! and introduces explicit TurnStarted / TurnComplete / TurnAborted events
//! so the agent loop can propagate cancellation and timeout signals.


// ── Agent Phase ──────────────────────────────────────────────────────────

/// Formal agent lifecycle phase, replacing the raw `status: String`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentPhase {
    /// Nothing in progress; waiting for user input.
    Idle,
    /// Streaming LLM response chunks back to the UI.
    StreamingResponse,
    /// Agent is thinking / between tool executions.
    Thinking,
    /// LLM is generating tool calls; tools will execute shortly.
    PreparingTools,
    /// A tool is currently executing on the daemon.
    ExecutingTool { name: String },
    /// Awaiting user response to a permission prompt.
    AwaitingPermission { tool: String, rule: String },
    /// Awaiting user response to an ask_user_question prompt.
    AwaitingUserInput { question: String },
    /// Conversation history is being compacted (may call LLM).
    Compacting,
    /// Turn completed normally.
    Completed,
    /// Turn ended with an error.
    Errored(String),
    /// Plan mode: agent has generated a plan, awaiting user review.
    Planning,
}

impl AgentPhase {
    /// Human-readable label for the status bar.
    pub fn label(&self) -> &str {
        match self {
            AgentPhase::Idle => "idle",
            AgentPhase::StreamingResponse => "streaming",
            AgentPhase::Thinking => "thinking",
            AgentPhase::PreparingTools => "preparing tools...",
            AgentPhase::ExecutingTool { name } => return name.as_str(),
            AgentPhase::AwaitingPermission { .. } => "permission required",
            AgentPhase::AwaitingUserInput { .. } => "question",
            AgentPhase::Compacting => "compacting",
            AgentPhase::Completed => "idle",
            AgentPhase::Errored(_) => "error",
            AgentPhase::Planning => "plan review",
        }
    }

    /// Whether the phase is a "busy" state (non-idle, non-error).
    pub fn is_busy(&self) -> bool {
        !matches!(self, AgentPhase::Idle | AgentPhase::Completed | AgentPhase::Errored(_) | AgentPhase::Planning)
    }
}

impl Default for AgentPhase {
    fn default() -> Self {
        AgentPhase::Idle
    }
}

// ── Turn Abort Reason ────────────────────────────────────────────────────

/// Why a turn was aborted (not a normal completion).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnAbortReason {
    /// User interrupted (Ctrl+C / cancel).
    Interrupted,
    /// Agent loop timed out.
    TimedOut,
    /// Exceeded max tool-use rounds.
    MaxRoundsExceeded,
    /// Stream/API error that wasn't recoverable.
    StreamError,
    /// Turn was replaced by a newer queued input.
    Replaced,
}

// ── Review Decision ──────────────────────────────────────────────────────

/// User decision for a permission request.
/// Replaces the simple PermissionResponse with richer semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewDecision {
    /// Allow this one execution.
    Approved,
    /// Allow for the remainder of this session.
    ApprovedForSession,
    /// User explicitly denied.
    Denied(String),
    /// Timed out waiting for user response.
    TimedOut,
}

// ── Turn ID ───────────────────────────────────────────────────────────────

/// Unique identifier for a single user-input → agent-response cycle.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TurnId(pub String);

impl TurnId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
}

impl std::fmt::Display for TurnId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
