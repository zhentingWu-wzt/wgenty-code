use std::path::PathBuf;

/// Maximum allowed depth for nested skill loading.
pub const MAX_NESTED_SKILL_DEPTH: usize = 8;

/// Outcome of a policy hook evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    /// Operation is permitted.
    Allow,
    /// Operation is permitted with a diagnostic warning.
    Warn { message: String },
    /// Operation is denied.
    Deny { message: String },
}

/// Record of a loaded skill during a session or turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedSkillRecord {
    pub name: String,
    pub source_path: PathBuf,
    pub base_dir: PathBuf,
    pub args: Option<String>,
    pub parent: Option<String>,
    pub depth: usize,
    pub turn_id: usize,
}

/// Accumulates loaded skill records and enforces depth limits.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoadedSkillContext {
    records: Vec<LoadedSkillRecord>,
}

impl LoadedSkillContext {
    /// Record a skill load. Returns `true` if the record is new (name+source_path not already seen),
    /// `false` if it was already loaded.
    pub fn record_load(&mut self, record: LoadedSkillRecord) -> bool {
        if self.records.iter().any(|existing| {
            existing.name == record.name && existing.source_path == record.source_path
        }) {
            return false;
        }
        self.records.push(record);
        true
    }

    /// Snapshot of currently loaded records.
    pub fn records(&self) -> &[LoadedSkillRecord] {
        &self.records
    }

    /// Whether the requested nested depth is allowed.
    pub fn depth_allowed(&self, requested_depth: usize) -> bool {
        requested_depth <= MAX_NESTED_SKILL_DEPTH
    }
}

/// Event payload for `before_skill_load`.
#[derive(Debug, Clone)]
pub struct SkillLoadEvent {
    pub skill_name: String,
    pub args: Option<String>,
    pub depth: usize,
    pub loaded_context: LoadedSkillContext,
}

/// Event payload for `before_nested_skill_call`.
#[derive(Debug, Clone)]
pub struct NestedSkillCallEvent {
    pub parent: Option<String>,
    pub child: String,
    pub depth: usize,
    pub loaded_context: LoadedSkillContext,
}

/// Event payload for `before_tool_call_observed`.
#[derive(Debug, Clone)]
pub struct ToolCallObservedEvent {
    pub tool_name: String,
    pub loaded_context: LoadedSkillContext,
}

/// Trait for skill lifecycle policy hooks.
pub trait SkillPolicy: Send + Sync {
    /// Called before a skill is resolved and loaded.
    fn before_skill_load(&self, _event: &SkillLoadEvent) -> PolicyDecision {
        PolicyDecision::Allow
    }

    /// Called before a nested skill is invoked.
    fn before_nested_skill_call(&self, _event: &NestedSkillCallEvent) -> PolicyDecision {
        PolicyDecision::Allow
    }

    /// Called when a tool call is observed in the context of loaded skills.
    fn before_tool_call_observed(&self, _event: &ToolCallObservedEvent) -> PolicyDecision {
        PolicyDecision::Allow
    }
}

/// Default permissive policy — allows all operations and emits no denials.
#[derive(Debug, Default)]
pub struct DefaultAllowPolicy;

impl SkillPolicy for DefaultAllowPolicy {}
