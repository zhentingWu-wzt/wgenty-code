use crate::state::agent_phase::AgentPhase;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

const IDLE_ICON: &str = "●";
const DIM: Color = Color::Rgb(100, 100, 115);
const ACTIVE: Color = Color::Rgb(100, 200, 255);
const ERROR: Color = Color::Rgb(255, 100, 100);
const MUTED: Color = Color::Rgb(130, 130, 150);

const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Render a Claude Code-style status line:
///   ⠋ Thinking… (12s · ↓ 4.5k tokens · NORMAL)
///   ● Ready (↓ 1.2k tokens)
pub fn render(
    f: &mut Frame,
    area: Rect,
    phase: &AgentPhase,
    session_name: &str,
    spinner_frame: u8,
    elapsed_secs: Option<u64>,
    tokens_used: usize,
    mode_label: &str,
) {
    let area_w = area.width as usize;
    if area_w < 5 {
        return;
    }

    let label = phase_label(phase);
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

    // ── Left: session name ──────────────────────────────────────────────
    let left = Span::styled(
        format!("  {}", session_name),
        Style::default().fg(MUTED),
    );

    // ── Center: icon + phase label ──────────────────────────────────────
    let center_text = if right_meta.is_empty() {
        format!("{} {}", icon, label)
    } else {
        format!("{} {} {}", icon, label, right_meta)
    };
    let center = Span::styled(center_text, Style::default().fg(phase_color).add_modifier(Modifier::BOLD));

    // ── Assemble line ───────────────────────────────────────────────────
    let left_width = session_name.len() + 2;
    let center_width = icon.chars().count()
        + 1 // space
        + label.len()
        + if right_meta.is_empty() { 0 } else { 1 + right_meta.len() }; // space + meta
    let padding = area_w.saturating_sub(left_width + center_width);

    let line = Line::from(vec![left, Span::raw(" ".repeat(padding)), center]);

    f.render_widget(Paragraph::new(line), area);
}

fn phase_label(phase: &AgentPhase) -> String {
    match phase {
        AgentPhase::Idle | AgentPhase::Completed => "Ready".to_string(),
        AgentPhase::Thinking => "Thinking…".to_string(),
        AgentPhase::PreparingTools => "Preparing tools…".to_string(),
        AgentPhase::StreamingResponse => "Streaming…".to_string(),
        AgentPhase::ExecutingTool { name } => match name.as_str() {
            "task" => "Subagent running…".to_string(),
            "delegate" => "RLM Pipeline".to_string(),
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
