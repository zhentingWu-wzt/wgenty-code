//! DaemonState -- shared state for the HTTP API server.

use crate::context::session::SessionManager;
use crate::hooks::HookManager;
use crate::knowledge::loader::SkillLoader;
use crate::permissions::ToolPermissionPolicy;
use crate::state::AppState;
use crate::tasks::{TaskManagementTool, TodoState, TodoWriteTool};
use crate::teams::mailbox::TeamManager;
use crate::tools::execution::background::{BackgroundManager, BackgroundTool};
use crate::tools::meta::team_message::TeamMessageTool;
use crate::tools::{CheckpointManager, ToolExecutor, ToolRegistry};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Per-session permission rules.
struct SessionRules {
    approved: HashSet<String>,
}

impl SessionRules {
    fn new() -> Self {
        Self {
            approved: HashSet::new(),
        }
    }
}

/// Shared state for all daemon HTTP handlers.
pub struct DaemonState {
    pub app_state: AppState,
    pub tool_registry: Arc<ToolRegistry>,
    pub tool_executor: ToolExecutor,
    pub checkpoint_manager: Arc<CheckpointManager>,
    pub task_manager: Arc<TaskManagementTool>,
    pub todo_state: Arc<RwLock<TodoState>>,
    pub skill_loader: Arc<SkillLoader>,
    pub background_manager: Arc<BackgroundManager>,
    pub team_manager: Option<Arc<TeamManager>>,
    pub session_manager: SessionManager,
    sessions: Arc<RwLock<std::collections::HashMap<String, SessionRules>>>,
}

impl DaemonState {
    pub fn new(app_state: AppState) -> Self {
        let task_manager = Arc::new(TaskManagementTool::new());
        let todo_write = TodoWriteTool::new();
        let todo_state = todo_write.todo_state();
        let policy = ToolPermissionPolicy::from_settings(&app_state.settings);

        // Initialize background manager
        let bg_manager = Arc::new(BackgroundManager::new());

        // Load team manager if .team/config.json exists
        let team_manager = {
            let root = &app_state.settings.working_dir;
            TeamManager::load(root).map(Arc::new)
        };

        // Initialize skill loader (needed before registry so TaskTool can use it).
        let skill_loader = {
            let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
            let base_dirs = vec![
                home.join(".wgenty-code"),
                app_state.settings.working_dir.clone(),
            ];
            let loader = SkillLoader::load_from_dirs(&base_dirs);
            Arc::new(loader)
        };

        // Use Arc::new_cyclic so the TaskTool holds a valid Weak<ToolRegistry>
        // that points to the *final* Arc allocation — not a temporary one that
        // gets dropped (which would leave a dangling weak reference).
        let tool_registry = Arc::new_cyclic(|weak_reg| {
            let mut registry = ToolRegistry::new().with_settings(&app_state.settings);
            registry.register(Box::new(todo_write));
            registry.register(Box::new(BackgroundTool::new(bg_manager.clone())));

            // Register team message tool if team is configured
            if team_manager.is_some() {
                registry.register(Box::new(TeamMessageTool::new(team_manager.clone())));
            }

            // Register load_skill tool if skills exist
            if !skill_loader.is_empty() {
                registry.register(Box::new(
                    crate::tools::meta::load_skill::LoadSkillTool::new(skill_loader.clone()),
                ));
            }

            // TaskTool gets a Weak<ToolRegistry> that is valid for the lifetime
            // of this Arc (created by Arc::new_cyclic).
            let task_tool = crate::tools::meta::task::TaskTool::new(
                app_state.settings.clone(),
                weak_reg.clone(),
                bg_manager.clone(),
            );
            registry.register(Box::new(task_tool));

            let rlm_tool = crate::tools::meta::rlm::RlmDelegateTool::new(
                app_state.settings.clone(),
                weak_reg.clone(),
            );
            registry.register(Box::new(rlm_tool));

            let run_script_tool = crate::tools::meta::run_script::RunScriptTool::new(
                app_state.settings.clone(),
                weak_reg.clone(),
            );
            registry.register(Box::new(run_script_tool));

            registry
        });
        let checkpoint_manager = tool_registry.checkpoint_manager.clone();

        // Initialize HookManager from settings hooks configuration
        let hooks_config = app_state
            .settings
            .hooks
            .as_ref()
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let hook_manager = Arc::new(HookManager::from_settings(&hooks_config));
        let session_manager = SessionManager::new();

        Self {
            app_state,
            tool_executor: ToolExecutor::new(tool_registry.clone(), policy)
                .with_hooks(hook_manager.clone()),
            tool_registry,
            checkpoint_manager,
            task_manager,
            todo_state,
            skill_loader,
            background_manager: bg_manager,
            team_manager,
            session_manager,
            sessions: Arc::new(RwLock::new(std::collections::HashMap::new())),
        }
    }

    /// Check if a session rule is approved for a given session.
    pub async fn is_rule_approved(&self, session_id: &str, rule: &str) -> bool {
        let sessions = self.sessions.read().await;
        sessions
            .get(session_id)
            .map(|s| s.approved.contains(rule))
            .unwrap_or(false)
    }

    /// Approve a session rule.
    pub async fn approve_rule(&self, session_id: &str, rule: String) {
        let mut sessions = self.sessions.write().await;
        sessions
            .entry(session_id.to_string())
            .or_insert_with(SessionRules::new)
            .approved
            .insert(rule);
    }

    /// Remove a previously approved session rule.
    pub async fn unapprove_rule(&self, session_id: &str, rule: &str) {
        let mut sessions = self.sessions.write().await;
        if let Some(s) = sessions.get_mut(session_id) {
            s.approved.remove(rule);
        }
    }
}
