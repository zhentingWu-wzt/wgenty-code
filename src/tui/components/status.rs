use crate::state::agent_phase::AgentPhase;
use crate::tui::components::subagent_tree::SubagentTree;
use ratatui::layout::{Alignment, Rect};
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
#[allow(clippy::too_many_arguments)]
pub fn render(
    f: &mut Frame,
    area: Rect,
    phase: &AgentPhase,
    spinner_frame: u8,
    elapsed_secs: Option<u64>,
    input_tokens: usize,
    output_tokens: usize,
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

    let turn_token_str = format_turn_tokens(input_tokens, output_tokens);
    if !turn_token_str.is_empty() {
        meta_parts.push(turn_token_str);
    }

    if is_active && !mode_label.is_empty() {
        meta_parts.push(mode_label.to_string());
    }

    // Subagent token budget info in status bar meta line
    if let Some(tree) = subagent_tree {
        if !tree.is_empty() {
            let total_tokens = tree.total_tokens();
            if total_tokens > 0 {
                meta_parts.push(format!("{:.1}k tokens", total_tokens as f64 / 1000.0));
            }
        }
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

    // Right-aligned shortcut hint (only when idle, to avoid clutter while
    // the agent is busy). Skipped on narrow terminals.
    if !is_active && !is_error && area.width >= 60 {
        let hint = "Ctrl+O expand last · Ctrl+E expand all · Ctrl+S sessions · Ctrl+T tasks";
        let hint_para = Paragraph::new(Line::from(Span::styled(hint, Style::default().fg(DIM))))
            .alignment(Alignment::Right);
        // Reserve right portion of the same line for the hint.
        let hint_width = (hint.chars().count() as u16 + 2).min(area.width);
        let hint_area = Rect {
            x: area.x + area.width.saturating_sub(hint_width),
            y: area.y,
            width: hint_width,
            height: 1,
        };
        f.render_widget(hint_para, hint_area);
    }
}

fn phase_label(phase: &AgentPhase, subagent_tree: Option<&SubagentTree>) -> String {
    match phase {
        AgentPhase::Idle | AgentPhase::Completed => "Ready".to_string(),
        AgentPhase::Thinking => "Thinking…".to_string(),
        AgentPhase::Connecting {
            attempt,
            max_retries,
        } => {
            if *max_retries > 1 {
                format!("Connecting (attempt {}/{})…", attempt, max_retries)
            } else {
                "Connecting…".to_string()
            }
        }
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
        AgentPhase::WaitingForInteraction => "Awaiting input…".to_string(),
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

/// Format a single token count with k-suffix (e.g. "1.6k").
fn fmt_single(n: usize) -> String {
    if n >= 1000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        format!("{} tokens", n)
    }
}

/// Format per-turn token display: "↑ N · ↓ Mk"
/// Omits input part if zero, omits output part if zero.
fn format_turn_tokens(input: usize, output: usize) -> String {
    let mut parts: Vec<String> = Vec::new();
    if input > 0 {
        parts.push(format!("↑ {}", fmt_single(input)));
    }
    if output > 0 {
        parts.push(format!("↓ {}", fmt_single(output)));
    }
    parts.join(" · ")
}
