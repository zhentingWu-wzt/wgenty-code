//! Prompt Assembly Module — layered instruction injection.
//!
//! Layers (in order, each optional with graceful degradation):
//!   1. base_instructions   — static role + behavior (from prompts/base.md)
//!   2. permissions         — sandbox mode + approval policy (dynamic)
//!   3. developer           — user-custom instructions (from Settings)
//!   4. environment         — cwd, shell, date, timezone
//!   5. agents_md           — repo-level AGENTS.md conventions

use crate::api::ChatMessage;
use crate::config::Settings;
use chrono::Local;

/// Pre-compiled base instructions (embedded at compile time).
const BASE_INSTRUCTIONS: &str = include_str!("base.md");

/// Assembled layered instructions for a single turn.
#[derive(Debug, Clone)]
pub struct AssembledInstructions {
    pub system_messages: Vec<ChatMessage>,
}

/// A single skill entry for system prompt injection (Layer 1).
/// Contains name and one-line description only; full body is loaded
/// on demand by the agent via the `load_skill` tool (Layer 2).
#[derive(Debug, Clone)]
pub struct SkillEntry {
    pub name: String,
    pub description: String,
}

/// Context needed for dynamic layers (permissions + environment).
#[derive(Debug, Clone)]
pub struct PromptContext {
    pub cwd: String,
    pub shell: String,
    pub sandbox_mode: Option<String>,
    pub approval_policy: Option<String>,
    pub collaboration_mode: Option<String>,
    pub agents_md_sections: Vec<String>,
    /// Skills discoverable by the agent. Layer 1: name + description only.
    pub skills_inventory: Vec<SkillEntry>,
    /// Sections from the project's WGENTY.md (split by `---`). Layer 8.
    pub wgenty_md_sections: Vec<String>,
}

impl Default for PromptContext {
    fn default() -> Self {
        Self::new()
    }
}

impl PromptContext {
    pub fn new() -> Self {
        Self {
            cwd: String::new(),
            shell: String::new(),
            sandbox_mode: None,
            approval_policy: None,
            collaboration_mode: None,
            agents_md_sections: Vec::new(),
            skills_inventory: Vec::new(),
            wgenty_md_sections: Vec::new(),
        }
    }

    pub fn with_cwd(mut self, cwd: impl Into<String>) -> Self {
        self.cwd = cwd.into();
        self
    }

    pub fn with_shell(mut self, shell: impl Into<String>) -> Self {
        self.shell = shell.into();
        self
    }

    pub fn with_sandbox(mut self, mode: impl Into<String>) -> Self {
        self.sandbox_mode = Some(mode.into());
        self
    }

    pub fn with_approval(mut self, policy: impl Into<String>) -> Self {
        self.approval_policy = Some(policy.into());
        self
    }

    pub fn with_collaboration(mut self, mode: impl Into<String>) -> Self {
        self.collaboration_mode = Some(mode.into());
        self
    }

    pub fn with_skills(mut self, skills: Vec<SkillEntry>) -> Self {
        self.skills_inventory = skills;
        self
    }

    pub fn with_wgenty_md(mut self, sections: Vec<String>) -> Self {
        self.wgenty_md_sections = sections;
        self
    }

    pub fn with_agents_md(mut self, sections: Vec<String>) -> Self {
        self.agents_md_sections = sections;
        self
    }
}

/// Assembles the full layered instructions from config + context.
/// Each layer gracefully degrades: if a layer has no content, it is skipped.
pub fn assemble_instructions(
    settings: &Settings,
    context: &PromptContext,
) -> AssembledInstructions {
    let mut system_messages: Vec<ChatMessage> = Vec::new();

    // ── Layer 1: Base Instructions ──────────────────────────────────────
    system_messages.push(ChatMessage::system(BASE_INSTRUCTIONS));

    // ── Layer 1b: Comet Phase Awareness (active OpenSpec change) ────────
    let working_dir = std::path::Path::new(&context.cwd);
    if let Some(comet_state) = crate::comet::CometState::read(working_dir) {
        system_messages.push(ChatMessage::system(comet_state.phase_instruction()));
    }
    if crate::comet::CometGuard::is_coordinator_mode(working_dir) {
        system_messages.push(ChatMessage::system(
            crate::comet::CometGuard::coordinator_reminder(),
        ));
    }

    // ── Layer 2: Permissions (dynamic) ──────────────────────────────────
    let perm_text = build_permissions_layer(context);
    if let Some(text) = perm_text {
        system_messages.push(ChatMessage::system(text));
    }

    // ── Layer 3: Developer Instructions (from settings) ─────────────────
    if let Some(ref dev_instr) = settings.prompt.developer_instructions {
        if !dev_instr.trim().is_empty() {
            system_messages.push(ChatMessage::system(format!(
                "<developer_instructions>\n{}\n</developer_instructions>",
                dev_instr.trim()
            )));
        }
    }

    // ── Layer 4: Collaboration Mode (from settings) ──────────────────
    let collab_text = build_collaboration_layer(context);
    if let Some(text) = collab_text {
        system_messages.push(ChatMessage::system(text));
    }

    // ── Layer 5: Environment Context (dynamic) ──────────────────────────
    let env_text = build_environment_layer(context);
    system_messages.push(ChatMessage::system(env_text));

    // ── Layer 6: Skills (discoverable + on-demand via load_skill tool) ──
    if settings.prompt.include.skills && !context.skills_inventory.is_empty() {
        let mut skills_lines = Vec::new();
        for skill in &context.skills_inventory {
            skills_lines.push(format!("- `{}`: {}", skill.name, skill.description));
        }
        system_messages.push(ChatMessage::system(format!(
            "## Available skills

The following skills are available. Use the `load_skill` tool to read a skill's full instructions when needed.

{}",
            skills_lines.join("
")
        )));
    }

    // ── Layer 7: AGENTS.md Convention ───────────────────────────────────
    if !context.agents_md_sections.is_empty() {
        let agents_text = context.agents_md_sections.join("\n\n");
        system_messages.push(ChatMessage::system(format!(
            "# AGENTS.md\n\n{}",
            agents_text
        )));
    }

    // ── Layer 8: WGENTY.md 项目事实 ──────────────────────────────────
    if !context.wgenty_md_sections.is_empty() {
        let wgenty_text = context.wgenty_md_sections.join("\n\n");
        system_messages.push(ChatMessage::system(format!(
            "# WGENTY.md — 项目规则与约定\n\n{}",
            wgenty_text
        )));
    }

    AssembledInstructions { system_messages }
}

/// Build the permissions layer text. Returns None if no sandbox/approval info.
fn build_permissions_layer(ctx: &PromptContext) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();

    if let Some(ref mode) = ctx.sandbox_mode {
        let sandbox_text = match mode.as_str() {
            "workspace-write" => include_str!("permissions/sandbox_workspace_write.md").to_string(),
            "read-only" => include_str!("permissions/sandbox_read_only.md").to_string(),
            other => format!("Sandbox mode: {other}"),
        };
        parts.push(sandbox_text);
    }

    if let Some(ref policy) = ctx.approval_policy {
        let approval_text = match policy.as_str() {
            "never" => include_str!("permissions/approval_never.md").to_string(),
            "on-request" => include_str!("permissions/approval_on_request.md").to_string(),
            other => format!("Approval policy: {other}"),
        };
        parts.push(approval_text);
    }

    if parts.is_empty() {
        return None;
    }

    Some(format!(
        "<permissions_instructions>\n{}\n</permissions_instructions>",
        parts.join("\n\n")
    ))
}

/// Build the collaboration mode layer text. Returns None if no mode set.
fn build_collaboration_layer(ctx: &PromptContext) -> Option<String> {
    let mode = ctx.collaboration_mode.as_deref()?;
    let text = match mode {
        "plan" => include_str!("collaboration/plan.md"),
        "execute" => include_str!("collaboration/execute.md"),
        "pair_programming" => include_str!("collaboration/pair_programming.md"),
        _ => return None,
    };
    Some(format!(
        "<collaboration_mode>
{}
</collaboration_mode>",
        text.trim()
    ))
}

/// Build the environment context layer text.
fn build_environment_layer(ctx: &PromptContext) -> String {
    let now = Local::now();
    let date = now.format("%Y-%m-%d").to_string();
    let timezone = now.format("%Z").to_string();

    format!(
        "<environment_context>\n  <cwd>{cwd}</cwd>\n  <shell>{shell}</shell>\n  <current_date>{date}</current_date>\n  <timezone>{timezone}</timezone>\n</environment_context>",
        cwd = ctx.cwd,
        shell = ctx.shell,
    )
}

/// Return the /init command prompt text for LLM-based codebase analysis.
pub fn get_init_prompt() -> &'static str {
    include_str!("init_instructions.md")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_assemble_base_only() {
        let settings = Settings::default();
        let ctx = PromptContext::new().with_cwd("/tmp").with_shell("zsh");

        let instructions = assemble_instructions(&settings, &ctx);
        // Should have: base + environment = at least 2 messages
        assert!(instructions.system_messages.len() >= 2); // base + env
                                                          // First message is the base instructions
        assert_eq!(instructions.system_messages[0].role, "system");
    }

    #[test]
    fn test_assemble_with_permissions() {
        let settings = Settings::default();
        let ctx = PromptContext::new()
            .with_cwd("/tmp")
            .with_shell("zsh")
            .with_sandbox("workspace-write")
            .with_approval("never");

        let instructions = assemble_instructions(&settings, &ctx);
        assert!(instructions.system_messages.len() >= 3); // base + permissions + env
    }

    #[test]
    fn test_graceful_degradation_no_permissions() {
        let settings = Settings::default();
        let ctx = PromptContext::new().with_cwd("/tmp").with_shell("zsh");
        // No sandbox/approval set

        let instructions = assemble_instructions(&settings, &ctx);
        // No permissions layer injected
        let has_permissions = instructions.system_messages.iter().any(|m| {
            m.content
                .as_deref()
                .is_some_and(|c| c.contains("<permissions_instructions>"))
        });
        assert!(!has_permissions);
    }
}
