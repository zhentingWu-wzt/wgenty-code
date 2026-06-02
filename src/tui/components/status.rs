use crate::state::agent_phase::AgentPhase;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use ratatui::Frame;

const DIVIDER_COLOR: Color = Color::Rgb(60, 60, 70);

pub fn render(f: &mut Frame, area: Rect, phase: &AgentPhase, session_name: &str) {
    let label = phase_label(phase);
    let w = area.width.saturating_sub(2) as usize;
    if w > 0 {
        let bar = "─".repeat(w);
        let text = format!("  {} [ {} | {} ]", bar, session_name, label);
        f.render_widget(
            Paragraph::new(Span::styled(text, Style::default().fg(DIVIDER_COLOR))),
            area,
        );
    }
}

fn phase_label(phase: &AgentPhase) -> String {
    match phase {
        AgentPhase::Idle | AgentPhase::Completed => "Ready".to_string(),
        AgentPhase::Thinking => "Thinking...".to_string(),
        AgentPhase::StreamingResponse => "Streaming...".to_string(),
        AgentPhase::ExecutingTool { name } => format!("Executing {}", name),
        AgentPhase::AwaitingPermission { .. } => "Permission Required".to_string(),
        AgentPhase::AwaitingUserInput { .. } => "Question".to_string(),
        AgentPhase::Compacting => "Compacting...".to_string(),
        AgentPhase::Errored(_) => "Error".to_string(),
        AgentPhase::Planning => "Plan Review".to_string(),
    }
}
