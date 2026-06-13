use crate::state::agent_phase::AgentPhase;
use crate::tui::components::subagent_tree::SubagentTree;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

const IDLE_ICON: &str = "●";
const DIM: Color = Color::Rgb(100, 100, 115);
const ACTIVE: Color = Color::Rgb(100, 200, 255);
const ERROR: Color = Color::Rgb(255, 100, 100);
#[allow(dead_code)]
const WARN: Color = Color::Rgb(255, 180, 50);

const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Render a status line:
///   ⠋ Subagent 3 active · 5/8 done (12s · ↓ 4.5k tokens · NORMAL)
///   ● Ready (↓ 1.2k tokens)
pub fn render(
    f: &mut Frame,
    area: Rect,
    phase: &AgentPhase,
    spinner_frame: u8,
    elapsed_secs: Option<u64>,
    tokens_used: usize,
    mode_label: &str,
    subagent_tree: Option<&SubagentTree>,
) {
    if area.width < 5 {
        return;
    }

    let label = phase_label(phase, subagent_tree);
    let is_active = phase.is_busy();
    let is_error = matches!(phase, AgentPhase::Errored(_));

    // ── Icon ────────────────────────────────────────────────────────────
    let icon = if is_error {
        "✗".to_string()
    } else if is_active {
        let idx = (spinner_frame as usize) % SPINNER.len();
        SPINNER[idx].to_string()
    } else {
        IDLE_ICON.to_string()
    };

    let phase_color = if is_error {
        ERROR
    } else if is_active {
        ACTIVE
    } else {
        DIM
    };

    // ── Right-side meta ─────────────────────────────────────────────────
    let mut meta_parts: Vec<String> = Vec::new();

    if let Some(s) = elapsed_secs {
        meta_parts.push(format_duration(s));
    }

    if tokens_used > 0 {
        meta_parts.push(format_tokens(tokens_used));
    }

    if is_active && !mode_label.is_empty() {
        meta_parts.push(mode_label.to_string());
    }

    let right_meta = if meta_parts.is_empty() {
        String::new()
    } else {
        format!("({})", meta_parts.join(" · "))
    };

    // ── Status line: icon + phase label + meta, left-aligned ─────────────
    let status_text = if right_meta.is_empty() {
        format!("{} {}", icon, label)
    } else {
        format!("{} {} {}", icon, label, right_meta)
    };
    let status = Span::styled(
        status_text,
        Style::default()
            .fg(phase_color)
            .add_modifier(Modifier::BOLD),
    );

    let line = Line::from(vec![Span::raw("  "), status]);

    f.render_widget(Paragraph::new(line), area);
}

fn phase_label(phase: &AgentPhase, subagent_tree: Option<&SubagentTree>) -> String {
    match phase {
        AgentPhase::Idle | AgentPhase::Completed => "Ready".to_string(),
        AgentPhase::Thinking => "Thinking…".to_string(),
        AgentPhase::PreparingTools => "Preparing tools…".to_string(),
        AgentPhase::StreamingResponse => "Streaming…".to_string(),
        AgentPhase::ExecutingTool { name } => match name.as_str() {
            "task" | "delegate" => {
                if let Some(tree) = subagent_tree {
                    if !tree.is_empty() {
                        let active = tree.active_count();
                        let done = tree.completed_count();
                        let failed = tree.failed_count();
                        let total = tree.total_count();
                        let mut label = String::new();
                        if active > 0 {
                            label.push_str(&format!("{} active", active));
                        }
                        if done > 0 || total > 0 {
                            if !label.is_empty() {
                                label.push_str(" · ");
                            }
                            label.push_str(&format!("{}/{} done", done, total));
                        }
                        if failed > 0 {
                            if !label.is_empty() {
                                label.push_str(" · ");
                            }
                            label.push_str(&format!("{} failed", failed));
                        }
                        if label.is_empty() {
                            label.push_str("Subagent running…");
                        }
                        return label;
                    }
                }
                if name == "task" {
                    "Subagent running…".to_string()
                } else {
                    "RLM Pipeline".to_string()
                }
            }
            _ => format!("Executing {}…", name),
        },
        AgentPhase::AwaitingPermission { .. } => "Permission required".to_string(),
        AgentPhase::AwaitingUserInput { .. } => "Question".to_string(),
        AgentPhase::Compacting => "Compacting…".to_string(),
        AgentPhase::Errored(_) => "Error".to_string(),
        AgentPhase::Planning => "Plan review…".to_string(),
    }
}

fn format_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{}s", secs)
    } else {
        let m = secs / 60;
        let s = secs % 60;
        format!("{}m{}s", m, s)
    }
}

fn format_tokens(tokens: usize) -> String {
    if tokens >= 1000 {
        format!("↓ {:.1}k tokens", tokens as f64 / 1000.0)
    } else {
        format!("↓ {} tokens", tokens)
    }
}
