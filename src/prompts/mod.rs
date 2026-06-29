//! Prompt Assembly Module — layered instruction injection.
//!
//! Layers (in order, each optional with graceful degradation):
//!   1. base_instructions   — static role + behavior (from prompts/base.md)
//!   2. permissions         — sandbox mode + approval policy (dynamic)
//!   3. developer           — user-custom instructions (from Settings)
//!   4. environment         — cwd, shell, date, timezone
//!   5. agents_md           — repo-level AGENTS.md conventions

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::api::ChatMessage;
use crate::config::Settings;
use crate::runtime::context::ContextAssembler;
use crate::runtime::hooks::{InjectedFragment, LayerVisibility};
use crate::utils::project::{read_user_global_instructions, read_user_global_rules};
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

// Attribution description strings, one per source.
const USER_INSTRUCTIONS_DESC: &str = "user's private global instructions for all projects";
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
    /// Skills discoverable by the agent. Layer 1: name + description only.
    pub skills_inventory: Vec<SkillEntry>,
    /// Sections from the project's WGENTY.md (split by `---`). Layer 8.
    pub wgenty_md_sections: Vec<String>,
    /// Absolute path to the project root; used by reminder builders to render
    /// attribution headers (e.g. `Contents of <abs-path> (description):`).
    pub project_root: Option<PathBuf>,
    /// Generic runtime context assembler. Replaces hardcoded comet phase injection.
    pub context_assembler: Option<Arc<ContextAssembler>>,
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
            .field("skills_inventory", &self.skills_inventory)
            .field("wgenty_md_sections", &self.wgenty_md_sections)
            .field("project_root", &self.project_root)
            .field(
                "context_assembler",
                &self.context_assembler.as_ref().map(|_| "ContextAssembler"),
            )
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
            skills_inventory: Vec::new(),
            wgenty_md_sections: Vec::new(),
            project_root: None,
            context_assembler: None,
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

    pub fn with_project_root(mut self, path: PathBuf) -> Self {
        self.project_root = Some(path);
        self
    }
}

/// Output of [`build_user_turn_reminder`].
///
/// `to_model` includes every collected segment and every injected fragment
/// (regardless of visibility). `to_transcript` is `Some` only when there is
/// any content suitable for transcript display — i.e. file-based segments
/// were present OR at least one hook fragment was [`LayerVisibility::Visible`].
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
    // ── Collect file-backed segments ───────────────────────────────────────
    struct Segment {
        path: PathBuf,
        description: &'static str,
        content: String,
    }
    let mut segments: Vec<Segment> = Vec::new();

    if let Some((path, content)) = read_user_global_instructions() {
        segments.push(Segment {
            path,
            description: USER_INSTRUCTIONS_DESC,
            content,
        });
    }
    for (path, content) in read_user_global_rules() {
        segments.push(Segment {
            path,
            description: USER_INSTRUCTIONS_DESC,
            content,
        });
    }
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
        to_transcript.push_str(&block);
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

    let transcript_has_content = !segments.is_empty() || transcript_has_hook;
    Some(ReminderOutput {
        to_model,
        to_transcript: if transcript_has_content {
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
}

#[cfg(test)]
mod reminder_tests {
    use super::*;
    use serial_test::serial;
    use std::path::Path;
    use tempfile::TempDir;

    /// Test helper: temporarily set $HOME, run closure, restore.
    /// Must be used with #[serial] to prevent races between tests.
    fn with_fake_home<F: FnOnce() -> R, R>(home: &Path, f: F) -> R {
        let prev = std::env::var_os("HOME");
        std::env::set_var("HOME", home);
        let result = f();
        match prev {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        result
    }

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
    // U1 — complete snapshot (4 sources)
    // ============================================================
    #[test]
    #[serial]
    fn reminder_full_four_sources_snapshot() {
        let home = make_fake_home(
            Some("USER_WGENTY_CONTENT"),
            &[("a.md", "RULE_A"), ("b.md", "RULE_B")],
        );
        let project_root = TempDir::new().unwrap();

        with_fake_home(home.path(), || {
            let ctx = PromptContext::new()
                .with_wgenty_md(vec!["PROJECT_WGENTY".to_string()])
                .with_agents_md(vec!["PROJECT_AGENTS".to_string()])
                .with_project_root(project_root.path().to_path_buf());

            let result = build_user_turn_reminder(&ctx, &[]).expect("reminder should be Some");

            // Skeleton
            assert!(
                result.to_model.starts_with("<system-reminder>\n"),
                "missing opener: {}",
                &result.to_model[..50.min(result.to_model.len())]
            );
            assert!(
                result.to_model.contains("# wgentyMd"),
                "missing # wgentyMd marker"
            );
            assert!(
                result.to_model.contains("IMPORTANT: These instructions OVERRIDE"),
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

            // 4 content markers
            assert!(result.to_model.contains("USER_WGENTY_CONTENT"));
            assert!(result.to_model.contains("RULE_A"));
            assert!(result.to_model.contains("RULE_B"));
            assert!(result.to_model.contains("PROJECT_WGENTY"));
            assert!(result.to_model.contains("PROJECT_AGENTS"));

            // 4 description tags
            assert!(result
                .to_model
                .contains("user's private global instructions for all projects"));
            assert!(result
                .to_model
                .contains("project instructions, checked into the codebase"));
            assert!(result
                .to_model
                .contains("project agent conventions, checked into the codebase"));

            // Ordering: user-global WGENTY → rules/a.md → rules/b.md → project WGENTY.md → project AGENTS.md
            let pos_user_w = result.to_model.find("USER_WGENTY_CONTENT").unwrap();
            let pos_a = result.to_model.find("RULE_A").unwrap();
            let pos_b = result.to_model.find("RULE_B").unwrap();
            let pos_proj_w = result.to_model.find("PROJECT_WGENTY").unwrap();
            let pos_proj_a = result.to_model.find("PROJECT_AGENTS").unwrap();
            assert!(pos_user_w < pos_a, "user WGENTY should precede rules");
            assert!(pos_a < pos_b, "rules should be alphabetical");
            assert!(pos_b < pos_proj_w, "rules should precede project WGENTY");
            assert!(
                pos_proj_w < pos_proj_a,
                "project WGENTY should precede project AGENTS"
            );
        });
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

        with_fake_home(home.path(), || {
            let ctx = PromptContext::new()
                .with_wgenty_md(vec!["PROJECT_WGENTY".to_string()])
                .with_agents_md(vec!["PROJECT_AGENTS".to_string()])
                .with_project_root(project_root.path().to_path_buf());

            let result = build_user_turn_reminder(&ctx, &[]).expect("project sections present");

            // User-global description should NOT appear (no user WGENTY, no rules)
            assert!(
                !result.to_model.contains("user's private global instructions"),
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
        });
    }

    // ============================================================
    // U3 — all missing → None
    // ============================================================
    #[test]
    #[serial]
    fn reminder_all_missing_returns_none() {
        let home = make_fake_home(None, &[]);

        with_fake_home(home.path(), || {
            let ctx = PromptContext::new(); // no project sections, no project_root
            assert!(
                build_user_turn_reminder(&ctx, &[]).is_none(),
                "all-missing should return None"
            );
        });
    }
}
