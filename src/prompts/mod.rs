//! Prompt Assembly Module — layered instruction injection.
//!
//! Layers (in order, each optional with graceful degradation):
//!   1. base_instructions   — static role + behavior (from prompts/base.md)
//!   2. permissions         — sandbox mode + approval policy (dynamic)
//!   3. developer           — user-custom instructions (from Settings)
//!   4. environment         — cwd, shell, date, timezone
//!
//! Note: AGENTS.md and WGENTY.md (both user and project-level) are
//! delivered via the per-turn <system-reminder> channel built by
//! `build_user_turn_reminder`, NOT via this system prompt cascade.
//! `PromptContext::wgenty_md_sections` and `agents_md_sections` are
//! still populated for the reminder builder to consume.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::api::ChatMessage;
use crate::config::Settings;
use crate::runtime::context::ContextAssembler;
use crate::runtime::hooks::{InjectedFragment, LayerVisibility};
use chrono::Local;

/// Pre-compiled base instructions (embedded at compile time).
const BASE_INSTRUCTIONS: &str = include_str!("base.md");

/// Opening preamble for the `<system-reminder>` channel injected at user turn.
const REMINDER_PREAMBLE_OPENING: &str =
    "As you answer the user's questions, you can use the following context:\n\
     # wgentyMd\n\
     Codebase and user instructions are shown below. Be sure to adhere to\n\
     these instructions. IMPORTANT: These instructions OVERRIDE any default\n\
     behavior and you MUST follow them exactly as written.\n";

/// Closing preamble for the `<system-reminder>` channel.
const REMINDER_PREAMBLE_CLOSING: &str =
    "      IMPORTANT: this context may or may not be relevant to your tasks.\n\
     \x20     You should not respond to this context unless it is highly relevant\n\
     \x20     to your task.";

// Attribution description strings for project-level file-backed segments.
const PROJECT_INSTRUCTIONS_DESC: &str = "project instructions, checked into the codebase";
const PROJECT_AGENTS_DESC: &str = "project agent conventions, checked into the codebase";
const HOOK_INJECTION_DESC: &str = "dynamic hook injection";

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
#[derive(Clone)]
pub struct PromptContext {
    pub cwd: String,
    pub shell: String,
    pub sandbox_mode: Option<String>,
    pub approval_policy: Option<String>,
    pub collaboration_mode: Option<String>,
    pub agents_md_sections: Vec<String>,
    /// User-global instructions from `~/.wgenty-code/WGENTY.md`.
    /// Read once at startup and cached here to avoid per-turn disk I/O.
    pub user_global_instructions: Option<(PathBuf, String)>,
    /// User-global rule files from `~/.wgenty-code/rules/*.md`, sorted by name.
    /// Read once at startup and cached here to avoid per-turn disk I/O.
    pub user_global_rules: Vec<(PathBuf, String)>,
    /// Skills discoverable by the agent. Layer 1: name + description only.
    pub skills_inventory: Vec<SkillEntry>,
    /// Sections from the project's WGENTY.md (split by `---`). Layer 8.
    pub wgenty_md_sections: Vec<String>,
    /// Absolute path to the project root; used by reminder builders to render
    /// attribution headers (e.g. `Contents of <abs-path> (description):`).
    pub project_root: Option<PathBuf>,
    /// Generic runtime context assembler. Replaces hardcoded comet phase injection.
    pub context_assembler: Option<Arc<ContextAssembler>>,
    /// Pre-formatted memory lines for cross-session recall.
    /// Each entry is a single system-message line (e.g. "- [decision] Use Jaccard for dedup").
    pub memories: Vec<String>,
    /// Pre-formatted global memory lines injected every turn (soft cap 50).
    /// Each entry is a single line like "- [Preference] 始终用中文回复".
    pub global_memories: Vec<String>,
    /// Override the home directory used to locate user-global files
    /// (`~/.wgenty-code/WGENTY.md` and `~/.wgenty-code/rules/*`). When `None`,
    /// the real home is resolved via `dirs::home_dir()` (which does NOT honor
    /// `USERPROFILE`/`HOME` env vars on Windows). Tests set this to a temp dir
    /// so user-global content is deterministic and cross-platform.
    pub home_override: Option<PathBuf>,
    /// CodeGraph availability (sync probe result) for guidance injection.
    pub codegraph_state: Option<crate::mcp::codegraph::CodegraphInstallState>,
}

impl fmt::Debug for PromptContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PromptContext")
            .field("cwd", &self.cwd)
            .field("shell", &self.shell)
            .field("sandbox_mode", &self.sandbox_mode)
            .field("approval_policy", &self.approval_policy)
            .field("collaboration_mode", &self.collaboration_mode)
            .field("agents_md_sections", &self.agents_md_sections)
            .field(
                "user_global_instructions",
                &self.user_global_instructions.is_some(),
            )
            .field("user_global_rules", &self.user_global_rules.len())
            .field("skills_inventory", &self.skills_inventory)
            .field("wgenty_md_sections", &self.wgenty_md_sections)
            .field("project_root", &self.project_root)
            .field(
                "context_assembler",
                &self.context_assembler.as_ref().map(|_| "ContextAssembler"),
            )
            .field("memories", &self.memories)
            .field("global_memories", &self.global_memories)
            .field("home_override", &self.home_override)
            .field("codegraph_state", &self.codegraph_state)
            .finish()
    }
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
            user_global_instructions: None,
            user_global_rules: Vec::new(),
            skills_inventory: Vec::new(),
            wgenty_md_sections: Vec::new(),
            project_root: None,
            context_assembler: None,
            memories: Vec::new(),
            global_memories: Vec::new(),
            home_override: None,
            codegraph_state: None,
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

    /// Set cached user-global instructions (`~/.wgenty-code/WGENTY.md`).
    pub fn with_user_global_instructions(
        mut self,
        instructions: Option<(PathBuf, String)>,
    ) -> Self {
        self.user_global_instructions = instructions;
        self
    }

    /// Set cached user-global rule files (`~/.wgenty-code/rules/*.md`).
    pub fn with_user_global_rules(mut self, rules: Vec<(PathBuf, String)>) -> Self {
        self.user_global_rules = rules;
        self
    }

    pub fn with_project_root(mut self, path: PathBuf) -> Self {
        self.project_root = Some(path);
        self
    }

    pub fn with_memories(mut self, memories: Vec<String>) -> Self {
        self.memories = memories;
        self
    }

    /// Set pre-formatted global memory lines (injected every turn).
    pub fn with_global_memories(mut self, memories: Vec<String>) -> Self {
        self.global_memories = memories;
        self
    }

    /// Override the home directory used to locate user-global files. See
    /// [`PromptContext::home_override`].
    pub fn with_home_override(mut self, home: PathBuf) -> Self {
        self.home_override = Some(home);
        self
    }

    /// Set the CodeGraph availability state for guidance injection.
    pub fn with_codegraph_state(
        mut self,
        state: crate::mcp::codegraph::CodegraphInstallState,
    ) -> Self {
        self.codegraph_state = Some(state);
        self
    }
}

/// Output of [`build_user_turn_reminder`].
///
/// `to_model` includes every collected segment and every injected fragment
/// (regardless of visibility). `to_transcript` is `Some` only when at least
/// one injected hook fragment has [`LayerVisibility::Visible`] — file-backed
/// segments (WGENTY.md / AGENTS.md / rules) are model instructions, never
/// shown in the transcript.
#[derive(Debug, Clone)]
pub struct ReminderOutput {
    pub to_model: String,
    pub to_transcript: Option<String>,
}

/// Render the attribution header line for a single source block inside the
/// `<system-reminder>` channel.
fn render_attribution_header(absolute_path: &Path, description: &str) -> String {
    format!("Contents of {} ({}):", absolute_path.display(), description)
}

/// Build the `<system-reminder>` channel injected at the start of each user
/// turn. Returns `None` when neither file-based segments nor hook injections
/// produced any content.
pub fn build_user_turn_reminder(
    ctx: &PromptContext,
    hook_injections: &[InjectedFragment],
) -> Option<ReminderOutput> {
    // ── Collect file-backed segments (project-level only; user globals are in Layers 7/8) ──
    struct Segment {
        path: PathBuf,
        description: &'static str,
        content: String,
    }
    let mut segments: Vec<Segment> = Vec::new();

    if !ctx.wgenty_md_sections.is_empty() {
        let path = ctx
            .project_root
            .as_ref()
            .map(|p| p.join("WGENTY.md"))
            .unwrap_or_else(|| PathBuf::from("WGENTY.md"));
        let content = ctx.wgenty_md_sections.join("\n\n");
        segments.push(Segment {
            path,
            description: PROJECT_INSTRUCTIONS_DESC,
            content,
        });
    }
    if !ctx.agents_md_sections.is_empty() {
        let path = ctx
            .project_root
            .as_ref()
            .map(|p| p.join("AGENTS.md"))
            .unwrap_or_else(|| PathBuf::from("AGENTS.md"));
        let content = ctx.agents_md_sections.join("\n\n");
        segments.push(Segment {
            path,
            description: PROJECT_AGENTS_DESC,
            content,
        });
    }

    if segments.is_empty() && hook_injections.is_empty() {
        return None;
    }

    // ── Render dual-track output ───────────────────────────────────────────
    let mut to_model = String::from("<system-reminder>\n");
    let mut to_transcript = String::from("<system-reminder>\n");
    to_model.push_str(REMINDER_PREAMBLE_OPENING);
    to_transcript.push_str(REMINDER_PREAMBLE_OPENING);

    for seg in &segments {
        let header = render_attribution_header(&seg.path, seg.description);
        let block = format!("\n{}\n\n{}\n", header, seg.content);
        to_model.push_str(&block);
    }

    let mut transcript_has_hook = false;
    for frag in hook_injections {
        let header = format!(
            "Contents of {} ({}):",
            frag.source_label, HOOK_INJECTION_DESC
        );
        let block = format!("\n{}\n\n{}\n", header, frag.content);
        to_model.push_str(&block);
        if matches!(frag.visibility, LayerVisibility::Visible) {
            to_transcript.push_str(&block);
            transcript_has_hook = true;
        }
    }

    to_model.push('\n');
    to_model.push_str(REMINDER_PREAMBLE_CLOSING);
    to_model.push_str("\n</system-reminder>");

    to_transcript.push('\n');
    to_transcript.push_str(REMINDER_PREAMBLE_CLOSING);
    to_transcript.push_str("\n</system-reminder>");

    Some(ReminderOutput {
        to_model,
        to_transcript: if transcript_has_hook {
            Some(to_transcript)
        } else {
            None
        },
    })
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

    // ── Layer 1b: Generic Runtime Context (replaces hardcoded comet injection) ─
    if let Some(ref assembler) = context.context_assembler {
        let assembled = assembler.assemble("", &HashMap::new());
        for instruction in &assembled.internal_instructions {
            system_messages.push(ChatMessage::system(instruction));
        }
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

    // ── Layer 5b: Recalled Cross-Session Memories ──────────────────────
    if !context.memories.is_empty() {
        let memory_lines = context.memories.join("\n");
        system_messages.push(ChatMessage::system(format!(
            "<relevant_memories>\n{}\n</relevant_memories>",
            memory_lines
        )));
    }

    // ── Layer 5c: Global Memories (injected every turn, soft cap 50) ──
    if !context.global_memories.is_empty() {
        let global_lines = context.global_memories.join("\n");
        system_messages.push(ChatMessage::system(format!(
            "<global-memory>\n{}\n</global-memory>",
            global_lines
        )));
    }

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

    // ── Layer 7: User Global Instructions ──────────────────────────
    if let Some((ref path, ref content)) = context.user_global_instructions {
        system_messages.push(ChatMessage::system(format!(
            "<user_global_instructions path=\"{}\">\n{}\n</user_global_instructions>",
            path.display(),
            content.trim()
        )));
    }

    // ── Layer 8: User Global Rules ─────────────────────────────────
    for (ref path, ref content) in &context.user_global_rules {
        system_messages.push(ChatMessage::system(format!(
            "<user_global_rules path=\"{}\">\n{}\n</user_global_rules>",
            path.display(),
            content.trim()
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
            "disabled" | "danger-full-access" => {
                include_str!("permissions/sandbox_disabled.md").to_string()
            }
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
    let codegraph_line = ctx
        .codegraph_state
        .map(|s| format!("\n  <codegraph>{}</codegraph>", s.guidance_hint()))
        .unwrap_or_default();

    format!(
        "<environment_context>\n  <cwd>{cwd}</cwd>\n  <shell>{shell}</shell>\n  <current_date>{date}</current_date>\n  <timezone>{timezone}</timezone>{codegraph_line}\n</environment_context>",
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
    use std::path::PathBuf;

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
    fn environment_layer_includes_codegraph_state() {
        let settings = Settings::default();
        let ctx = PromptContext::new()
            .with_cwd("/tmp")
            .with_shell("zsh")
            .with_codegraph_state(crate::mcp::CodegraphInstallState::NotInitialized);

        let instructions = assemble_instructions(&settings, &ctx);
        // Match the real env layer (Layer 5) via a runtime-generated marker;
        // base instructions also mention `<environment_context>` in prose.
        let env = instructions
            .system_messages
            .iter()
            .find(|m| {
                m.content
                    .as_deref()
                    .is_some_and(|c| c.contains("<current_date>"))
            })
            .expect("environment layer present");
        let content = env.content.as_deref().unwrap();
        assert!(content.contains("<codegraph>"), "got: {content}");
        assert!(content.contains("not_initialized"));
        assert!(content.contains("codegraph init"));
    }

    #[test]
    fn environment_layer_omits_codegraph_when_none() {
        let settings = Settings::default();
        let ctx = PromptContext::new().with_cwd("/tmp").with_shell("zsh");

        let instructions = assemble_instructions(&settings, &ctx);
        let env = instructions
            .system_messages
            .iter()
            .find(|m| {
                m.content
                    .as_deref()
                    .is_some_and(|c| c.contains("<current_date>"))
            })
            .expect("environment layer present");
        let content = env.content.as_deref().unwrap();
        assert!(!content.contains("<codegraph>"), "got: {content}");
    }

    #[test]
    fn environment_layer_ready_state() {
        let settings = Settings::default();
        let ctx = PromptContext::new()
            .with_cwd("/tmp")
            .with_shell("zsh")
            .with_codegraph_state(crate::mcp::CodegraphInstallState::Ready);

        let instructions = assemble_instructions(&settings, &ctx);
        let env = instructions
            .system_messages
            .iter()
            .find(|m| {
                m.content
                    .as_deref()
                    .is_some_and(|c| c.contains("<current_date>"))
            })
            .expect("environment layer present");
        let content = env.content.as_deref().unwrap();
        assert!(content.contains("<codegraph>ready"));
    }

    #[test]
    fn test_assemble_with_empty_memories_no_injection() {
        let settings = Settings::default();
        let ctx = PromptContext::new()
            .with_cwd("/tmp")
            .with_shell("zsh")
            .with_memories(Vec::new());

        let instructions = assemble_instructions(&settings, &ctx);
        let has_memories = instructions.system_messages.iter().any(|m| {
            m.content
                .as_deref()
                .is_some_and(|c| c.contains("<relevant_memories>"))
        });
        assert!(
            !has_memories,
            "empty memories should not inject extra system message"
        );
    }

    #[test]
    fn test_assemble_with_memories_between_layer_5_and_6() {
        let mut settings = Settings::default();
        settings.prompt.include.skills = true; // ensure Layer 6 exists

        let ctx = PromptContext::new()
            .with_cwd("/tmp")
            .with_shell("zsh")
            .with_skills(vec![SkillEntry {
                name: "test-skill".into(),
                description: "A test skill".into(),
            }])
            .with_memories(vec![
                "- [decision] Use Jaccard for dedup".to_string(),
                "- [knowledge] Project uses Rust".to_string(),
            ]);

        let instructions = assemble_instructions(&settings, &ctx);
        let messages = &instructions.system_messages;

        // Find positions of environment marker, memories marker, and skills marker
        let env_pos = messages
            .iter()
            .position(|m| {
                m.content
                    .as_deref()
                    .is_some_and(|c| c.contains("<environment_context>"))
            })
            .expect("Layer 5 (Environment) should be present");

        let mem_pos = messages
            .iter()
            .position(|m| {
                m.content
                    .as_deref()
                    .is_some_and(|c| c.contains("<relevant_memories>"))
            })
            .expect("Memories should be present when non-empty");

        let skills_pos = messages
            .iter()
            .position(|m| {
                m.content
                    .as_deref()
                    .is_some_and(|c| c.contains("Available skills"))
            })
            .expect("Layer 6 (Skills) should be present when skills enabled");

        assert!(
            env_pos < mem_pos,
            "Memories should come after Environment (Layer 5)"
        );
        assert!(
            mem_pos < skills_pos,
            "Memories should come before Skills (Layer 6)"
        );

        // Verify content
        let mem_content = messages[mem_pos].content.as_deref().unwrap();
        assert!(mem_content.contains("Use Jaccard for dedup"));
        assert!(mem_content.contains("Project uses Rust"));
    }

    #[test]
    fn test_assemble_with_empty_global_memories_no_injection() {
        let settings = Settings::default();
        let ctx = PromptContext::new()
            .with_cwd("/tmp")
            .with_shell("zsh")
            .with_global_memories(Vec::new());

        let instructions = assemble_instructions(&settings, &ctx);
        let has_global = instructions.system_messages.iter().any(|m| {
            m.content
                .as_deref()
                .is_some_and(|c| c.contains("<global-memory>"))
        });
        assert!(
            !has_global,
            "empty global memories should not inject extra system message"
        );
    }

    #[test]
    fn test_assemble_with_global_memories_in_system_prompt() {
        let settings = Settings::default();

        let ctx = PromptContext::new()
            .with_cwd("/tmp")
            .with_shell("zsh")
            .with_global_memories(vec![
                "- [preference] Always reply in Chinese".to_string(),
                "- [knowledge] User works on Rust projects".to_string(),
            ]);

        let instructions = assemble_instructions(&settings, &ctx);
        let messages = &instructions.system_messages;

        let global_pos = messages
            .iter()
            .position(|m| {
                m.content
                    .as_deref()
                    .is_some_and(|c| c.contains("<global-memory>"))
            })
            .expect("Global memory block should be present when non-empty");

        let content = messages[global_pos].content.as_deref().unwrap();
        assert!(content.contains("Always reply in Chinese"));
        assert!(content.contains("User works on Rust projects"));
        assert!(content.contains("<global-memory>"));
        assert!(content.contains("</global-memory>"));
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
        let perm = permissions_layer_text(&instructions)
            .expect("permissions layer should be present when sandbox/approval set");
        assert!(perm.contains("workspace-write"));
        assert!(perm.contains("Approval policy is currently never"));
    }

    /// Extract the permissions system-message body, if any.
    fn permissions_layer_text(instructions: &AssembledInstructions) -> Option<&str> {
        instructions.system_messages.iter().find_map(|m| {
            m.content.as_deref().filter(|c| c.contains("<permissions_instructions>"))
        })
    }

    #[test]
    fn permissions_layer_normal_workspace_write_on_request() {
        // Normal / AcceptEdits prompt labels.
        let settings = Settings::default();
        let ctx = PromptContext::new()
            .with_cwd("/tmp")
            .with_shell("zsh")
            .with_sandbox("workspace-write")
            .with_approval("on-request");

        let instructions = assemble_instructions(&settings, &ctx);
        let perm = permissions_layer_text(&instructions).expect("permissions layer");
        assert!(
            perm.contains("`sandbox_mode` is `workspace-write`"),
            "expected workspace-write sandbox copy, got: {perm}"
        );
        assert!(
            perm.contains("Approval policy is currently on-request"),
            "expected on-request approval copy, got: {perm}"
        );
        assert!(!perm.contains("Approval policy is currently never"));
    }

    #[test]
    fn permissions_layer_plan_read_only_on_request() {
        let settings = Settings::default();
        let ctx = PromptContext::new()
            .with_cwd("/tmp")
            .with_shell("zsh")
            .with_sandbox("read-only")
            .with_approval("on-request");

        let instructions = assemble_instructions(&settings, &ctx);
        let perm = permissions_layer_text(&instructions).expect("permissions layer");
        assert!(
            perm.contains("`sandbox_mode` is `read-only`") || perm.contains("read-only"),
            "expected read-only sandbox copy, got: {perm}"
        );
        assert!(
            perm.contains("across the disk")
                || perm.contains("File writes and edits via tools are blocked")
                || perm.contains("read-only"),
            "expected Codex-aligned read-only body (full-disk read), got: {perm}"
        );
        assert!(perm.contains("Approval policy is currently on-request"));
    }

    #[test]
    fn permissions_layer_workspace_write_describes_full_disk_read() {
        // Codex workspace-write: unrestricted reads, workspace-scoped writes.
        let settings = Settings::default();
        let ctx = PromptContext::new()
            .with_cwd("/tmp")
            .with_shell("zsh")
            .with_sandbox("workspace-write")
            .with_approval("on-request");
        let instructions = assemble_instructions(&settings, &ctx);
        let perm = permissions_layer_text(&instructions).expect("permissions layer");
        assert!(
            perm.contains("across the disk") || perm.contains("outside the workspace"),
            "workspace-write copy must not claim workspace-only reads: {perm}"
        );
        assert!(
            !perm.contains("Reading files outside the workspace requires approval"),
            "stale path-scoped-read copy still present: {perm}"
        );
    }

    #[test]
    fn permissions_layer_yolo_disabled_never() {
        // Yolo / Full Access: OS sandbox off + auto-approve.
        let settings = Settings::default();
        let ctx = PromptContext::new()
            .with_cwd("/tmp")
            .with_shell("zsh")
            .with_sandbox("disabled")
            .with_approval("never");

        let instructions = assemble_instructions(&settings, &ctx);
        let perm = permissions_layer_text(&instructions).expect("permissions layer");
        assert!(
            perm.contains("disabled") || perm.contains("danger-full-access"),
            "expected disabled sandbox copy, got: {perm}"
        );
        assert!(
            perm.contains("without OS-level seatbelt")
                || perm.contains("sandboxing is disabled"),
            "expected full-access body, got: {perm}"
        );
        assert!(perm.contains("Approval policy is currently never"));
    }

    #[test]
    fn permissions_layer_danger_full_access_alias() {
        // Codex-style alias should reuse the same disabled copy.
        let settings = Settings::default();
        let ctx = PromptContext::new()
            .with_cwd("/tmp")
            .with_shell("zsh")
            .with_sandbox("danger-full-access")
            .with_approval("never");

        let instructions = assemble_instructions(&settings, &ctx);
        let perm = permissions_layer_text(&instructions).expect("permissions layer");
        assert!(
            perm.contains("disabled") || perm.contains("danger-full-access"),
            "danger-full-access should map to disabled template, got: {perm}"
        );
        assert!(perm.contains("Approval policy is currently never"));
    }

    #[test]
    fn permissions_layer_unknown_sandbox_and_approval_fallback() {
        let settings = Settings::default();
        let ctx = PromptContext::new()
            .with_cwd("/tmp")
            .with_shell("zsh")
            .with_sandbox("custom-mode")
            .with_approval("custom-policy");

        let instructions = assemble_instructions(&settings, &ctx);
        let perm = permissions_layer_text(&instructions).expect("permissions layer");
        assert!(perm.contains("Sandbox mode: custom-mode"));
        assert!(perm.contains("Approval policy: custom-policy"));
    }

    #[test]
    fn build_permissions_layer_none_when_unset() {
        let ctx = PromptContext::new().with_cwd("/tmp").with_shell("zsh");
        assert!(build_permissions_layer(&ctx).is_none());
    }

    #[test]
    fn build_permissions_layer_sandbox_only() {
        let ctx = PromptContext::new()
            .with_cwd("/tmp")
            .with_shell("zsh")
            .with_sandbox("workspace-write");
        let text = build_permissions_layer(&ctx).expect("sandbox alone is enough");
        assert!(text.contains("<permissions_instructions>"));
        assert!(text.contains("workspace-write"));
        assert!(!text.contains("Approval policy"));
    }

    #[test]
    fn build_permissions_layer_approval_only() {
        let ctx = PromptContext::new()
            .with_cwd("/tmp")
            .with_shell("zsh")
            .with_approval("on-request");
        let text = build_permissions_layer(&ctx).expect("approval alone is enough");
        assert!(text.contains("Approval policy is currently on-request"));
        assert!(!text.contains("sandbox_mode"));
    }

    #[test]
    fn test_prompt_context_project_root_default_none() {
        let ctx = PromptContext::new();
        assert!(ctx.project_root.is_none());
    }

    #[test]
    fn test_prompt_context_with_project_root_sets_field() {
        let ctx = PromptContext::new().with_project_root(PathBuf::from("/tmp/proj"));
        assert_eq!(ctx.project_root, Some(PathBuf::from("/tmp/proj")));
    }

    #[test]
    fn test_build_user_turn_reminder_callable() {
        // Smoke test: function exists and returns without panicking.
        // Detailed behavior tests come in Tasks 2.4–2.8.
        let ctx = PromptContext::new();
        let _result = build_user_turn_reminder(&ctx, &[]);
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

    // ============================================================
    // U9 (Task 4.4) — Hard-cut verification: Layer 7/8 removed
    // ============================================================
    #[test]
    fn assemble_instructions_no_layer_7_8() {
        // After the system-reminder-channel hard-cut, AGENTS.md and WGENTY.md
        // sections must NOT appear as system messages — they go through the
        // <system-reminder> channel instead.
        let settings = Settings::default();
        let ctx = PromptContext::new()
            .with_cwd("/tmp")
            .with_shell("zsh")
            .with_wgenty_md(vec!["wgenty body".into()])
            .with_agents_md(vec!["agents body".into()]);

        let instructions = assemble_instructions(&settings, &ctx);

        for msg in &instructions.system_messages {
            let content = msg.content.as_deref().unwrap_or_default();
            // Match the EXACT Layer 7 header format: "# AGENTS.md\n\n<body>".
            // Plain substring "# AGENTS.md" would false-positive on base.md's
            // "## AGENTS.md and repository conventions" heading.
            assert!(
                !content.contains("# AGENTS.md\n\n"),
                "AGENTS.md Layer 7 header should NOT appear in system messages after hard-cut"
            );
            assert!(
                !content.contains("# WGENTY.md — 项目规则与约定"),
                "WGENTY.md section header should NOT appear in system messages"
            );
            // Bonus: actual content strings should also be absent (the data exits via reminder, not here)
            assert!(
                !content.contains("wgenty body"),
                "wgenty body content should NOT be pushed to system messages"
            );
            assert!(
                !content.contains("agents body"),
                "agents body content should NOT be pushed to system messages"
            );
        }
    }
}

#[cfg(test)]
mod reminder_tests {
    use super::*;
    use serial_test::serial;
    use tempfile::TempDir;

    /// Set up a fake home with .wgenty-code/WGENTY.md and rules/*.md files.
    /// Returns TempDir — keep alive for test duration.
    fn make_fake_home(user_wgenty: Option<&str>, rules: &[(&str, &str)]) -> TempDir {
        let tmp = TempDir::new().unwrap();
        let wgenty_dir = tmp.path().join(".wgenty-code");
        std::fs::create_dir_all(&wgenty_dir).unwrap();
        if let Some(content) = user_wgenty {
            std::fs::write(wgenty_dir.join("WGENTY.md"), content).unwrap();
        }
        if !rules.is_empty() {
            let rules_dir = wgenty_dir.join("rules");
            std::fs::create_dir_all(&rules_dir).unwrap();
            for (name, content) in rules {
                std::fs::write(rules_dir.join(name), content).unwrap();
            }
        }
        tmp
    }

    // ============================================================
    // U1 — complete snapshot (2 project sources in reminder; user globals in Layers 7/8)
    // ============================================================
    #[test]
    #[serial]
    fn reminder_full_four_sources_snapshot() {
        let project_root = TempDir::new().unwrap();
        let ctx = PromptContext::new()
            .with_user_global_instructions(Some((
                PathBuf::from("/fake/home/.wgenty/instructions.md"),
                "USER_WGENTY_CONTENT".to_string(),
            )))
            .with_user_global_rules(vec![
                (
                    PathBuf::from("/fake/home/.wgenty/rules/a.md"),
                    "RULE_A".to_string(),
                ),
                (
                    PathBuf::from("/fake/home/.wgenty/rules/b.md"),
                    "RULE_B".to_string(),
                ),
            ])
            .with_wgenty_md(vec!["PROJECT_WGENTY".to_string()])
            .with_agents_md(vec!["PROJECT_AGENTS".to_string()])
            .with_project_root(project_root.path().to_path_buf());

        // ── System prompt: user instructions + rules in Layers 7/8 ─────
        let settings = Settings::default();
        let assembled = assemble_instructions(&settings, &ctx);
        let sys_text = assembled
            .system_messages
            .iter()
            .filter_map(|msg| msg.content.as_deref())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            sys_text.contains("USER_WGENTY_CONTENT"),
            "Layer 7: user instructions"
        );
        assert!(sys_text.contains("RULE_A"), "Layer 8: rule a.md");
        assert!(sys_text.contains("RULE_B"), "Layer 8: rule b.md");
        assert!(
            sys_text.contains("<user_global_instructions"),
            "Layer 7 tag"
        );
        assert!(sys_text.contains("<user_global_rules"), "Layer 8 tag");

        // ── Reminder: project WGENTY.md + AGENTS.md only ─────────────
        let result = build_user_turn_reminder(&ctx, &[]).expect("reminder should be Some");

        // Skeleton
        assert!(
            result.to_model.starts_with("<system-reminder>\n"),
            "missing opener: {}",
            result.to_model.chars().take(50).collect::<String>()
        );
        assert!(
            result.to_model.contains("# wgentyMd"),
            "missing # wgentyMd marker"
        );
        assert!(
            result
                .to_model
                .contains("IMPORTANT: These instructions OVERRIDE"),
            "missing OVERRIDE preamble"
        );
        assert!(
            result
                .to_model
                .contains("IMPORTANT: this context may or may not be relevant"),
            "missing closing preamble"
        );
        assert!(
            result.to_model.trim_end().ends_with("</system-reminder>"),
            "missing closer"
        );

        // 2 project content markers
        assert!(result.to_model.contains("PROJECT_WGENTY"));
        assert!(result.to_model.contains("PROJECT_AGENTS"));

        // 2 description tags (project-level only; user globals now in Layers 7/8)
        assert!(result
            .to_model
            .contains("project instructions, checked into the codebase"));
        assert!(result
            .to_model
            .contains("project agent conventions, checked into the codebase"));

        // Ordering: project WGENTY.md → project AGENTS.md
        let pos_proj_w = result.to_model.find("PROJECT_WGENTY").unwrap();
        let pos_proj_a = result.to_model.find("PROJECT_AGENTS").unwrap();
        assert!(
            pos_proj_w < pos_proj_a,
            "project WGENTY should precede project AGENTS"
        );
    }

    // ============================================================
    // U2 — missing user WGENTY → no orphan header
    // ============================================================
    #[test]
    #[serial]
    fn reminder_missing_user_wgenty_no_empty_header() {
        // No user WGENTY.md, no rules — only project sections
        let home = make_fake_home(None, &[]);
        let project_root = TempDir::new().unwrap();

        {
            let ctx = PromptContext::new()
                .with_home_override(home.path().to_path_buf())
                .with_wgenty_md(vec!["PROJECT_WGENTY".to_string()])
                .with_agents_md(vec!["PROJECT_AGENTS".to_string()])
                .with_project_root(project_root.path().to_path_buf());

            let result = build_user_turn_reminder(&ctx, &[]).expect("project sections present");

            // User-global description should NOT appear (no user WGENTY, no rules)
            assert!(
                !result
                    .to_model
                    .contains("user's private global instructions"),
                "should not include user-global description when no user files"
            );
            // Project descriptions still present
            assert!(result
                .to_model
                .contains("project instructions, checked into the codebase"));
            assert!(result
                .to_model
                .contains("project agent conventions, checked into the codebase"));

            // No orphan attribution header — empty path + empty description
            // would render "Contents of  ():" with double-space; must never occur.
            assert!(
                !result.to_model.contains("Contents of  ("),
                "orphan attribution header with empty path detected"
            );
        }
    }

    // ============================================================
    // U3 — all missing → None
    // ============================================================
    #[test]
    #[serial]
    fn reminder_all_missing_returns_none() {
        let home = make_fake_home(None, &[]);

        {
            let ctx = PromptContext::new().with_home_override(home.path().to_path_buf()); // no project sections, no project_root
            assert!(
                build_user_turn_reminder(&ctx, &[]).is_none(),
                "all-missing should return None"
            );
        }
    }

    // ============================================================
    // U5 (Task 2.6) — Absolute paths in attribution
    // ============================================================
    #[test]
    #[serial]
    fn reminder_absolute_paths_in_attribution() {
        let home = make_fake_home(Some("X"), &[("a.md", "Y")]);
        let project_root = TempDir::new().unwrap();

        {
            let ctx = PromptContext::new()
                .with_home_override(home.path().to_path_buf())
                .with_wgenty_md(vec!["P".to_string()])
                .with_agents_md(vec!["Q".to_string()])
                .with_project_root(project_root.path().to_path_buf());

            let result = build_user_turn_reminder(&ctx, &[]).unwrap();

            // Every "Contents of <path> (...)" line must have an absolute path.
            // Use Path::is_absolute rather than starts_with('/') so this holds on
            // Windows too, where absolute paths look like "C:\...".
            for line in result.to_model.lines() {
                if let Some(rest) = line.strip_prefix("Contents of ") {
                    // Path is everything up to the last " ("
                    let path_end = rest
                        .rfind(" (")
                        .expect("attribution line should contain ' ('");
                    let path = &rest[..path_end];
                    assert!(
                        std::path::Path::new(path).is_absolute(),
                        "attribution path should be absolute, got: {path:?}"
                    );
                }
            }
        }
    }

    // ============================================================
    // U4 (Task 2.7) — Rules alphabetical order (now in Layer 8 of system prompt)
    // ============================================================
    #[test]
    fn reminder_user_rules_alphabetical_order() {
        // Rules are now cached and delivered via Layer 8 in assemble_instructions.
        // Verify they appear in the same order as stored in user_global_rules.
        let ctx = PromptContext::new().with_user_global_rules(vec![
            (PathBuf::from("/fake/rules/a.md"), "AAA".to_string()),
            (PathBuf::from("/fake/rules/b.md"), "BBB".to_string()),
            (PathBuf::from("/fake/rules/c.md"), "CCC".to_string()),
        ]);

        let settings = Settings::default();
        let assembled = assemble_instructions(&settings, &ctx);
        let sys_text = assembled
            .system_messages
            .iter()
            .filter_map(|msg| msg.content.as_deref())
            .collect::<Vec<_>>()
            .join("\n");

        let pos_a = sys_text.find("AAA").unwrap();
        let pos_b = sys_text.find("BBB").unwrap();
        let pos_c = sys_text.find("CCC").unwrap();
        assert!(pos_a < pos_b, "a.md should precede b.md in Layer 8");
        assert!(pos_b < pos_c, "b.md should precede c.md in Layer 8");
    }

    // ============================================================
    // U6 (Task 2.8 part 1) — Hook priority sorting
    // ============================================================
    #[test]
    fn reminder_hook_priority_sorting() {
        // Empty fs setup — but hook_injections feed directly without needing $HOME.
        // Still mark non-serial since we don't touch HOME here.
        let ctx = PromptContext::new().with_wgenty_md(vec!["P".to_string()]); // ensures non-None result

        let frags = vec![
            InjectedFragment {
                content: "PRI_30".into(),
                priority: 30,
                visibility: LayerVisibility::Visible,
                source_label: "hook:UserPromptSubmit:0".into(),
            },
            InjectedFragment {
                content: "PRI_10".into(),
                priority: 10,
                visibility: LayerVisibility::Visible,
                source_label: "hook:UserPromptSubmit:1".into(),
            },
            InjectedFragment {
                content: "PRI_20".into(),
                priority: 20,
                visibility: LayerVisibility::Visible,
                source_label: "hook:UserPromptSubmit:2".into(),
            },
        ];

        // Note: builder receives fragments as-is. Caller (production path) uses
        // collect_injections which sorts. For this test, sort fragments ourselves
        // to verify the builder PRESERVES priority order from the input slice.
        let mut sorted = frags.clone();
        sorted.sort_by_key(|f| f.priority);

        let result = build_user_turn_reminder(&ctx, &sorted).unwrap();

        let pos_10 = result.to_model.find("PRI_10").unwrap();
        let pos_20 = result.to_model.find("PRI_20").unwrap();
        let pos_30 = result.to_model.find("PRI_30").unwrap();
        assert!(pos_10 < pos_20);
        assert!(pos_20 < pos_30);
    }

    // ============================================================
    // U7 (Task 2.8 part 2) — Internal visibility excludes transcript
    // ============================================================
    #[test]
    fn reminder_internal_visibility_excludes_transcript() {
        let ctx = PromptContext::new().with_wgenty_md(vec!["P_CONTENT".to_string()]);

        let frags = vec![
            InjectedFragment {
                content: "INTERNAL_PAYLOAD".into(),
                priority: 50,
                visibility: LayerVisibility::Internal,
                source_label: "hook:UserPromptSubmit:0".into(),
            },
            InjectedFragment {
                content: "VISIBLE_MAKER".into(),
                priority: 60,
                visibility: LayerVisibility::Visible,
                source_label: "hook:UserPromptSubmit:1".into(),
            },
        ];

        let result = build_user_turn_reminder(&ctx, &frags).unwrap();

        assert!(
            result.to_model.contains("INTERNAL_PAYLOAD"),
            "to_model MUST contain internal hook content"
        );
        let transcript = result
            .to_transcript
            .expect("visible hook → transcript Some");
        assert!(
            !transcript.contains("INTERNAL_PAYLOAD"),
            "to_transcript MUST NOT contain internal hook content"
        );
        assert!(
            !transcript.contains("P_CONTENT"),
            "to_transcript MUST NOT contain file-backed segments"
        );
    }

    // ============================================================
    // U8 (Task 2.8 part 3) — Visible hook in both outputs
    // ============================================================
    #[test]
    fn reminder_visible_hook_in_both_outputs() {
        let ctx = PromptContext::new().with_wgenty_md(vec!["P_CONTENT".to_string()]);

        let frags = vec![InjectedFragment {
            content: "VISIBLE_PAYLOAD".into(),
            priority: 50,
            visibility: LayerVisibility::Visible,
            source_label: "hook:UserPromptSubmit:0".into(),
        }];

        let result = build_user_turn_reminder(&ctx, &frags).unwrap();
        assert!(result.to_model.contains("VISIBLE_PAYLOAD"));
        let transcript = result.to_transcript.expect("content → transcript Some");
        assert!(transcript.contains("VISIBLE_PAYLOAD"));
    }
}
