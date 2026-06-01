use crate::state::agent_phase::AgentPhase;
use crate::tui::theme;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// Render the status bar.
pub fn render(f: &mut Frame, area: Rect, phase: &AgentPhase, session_name: &str) {
    let label = phase_label(phase);
    let text = Span::styled(
        format!(" {} | {}", session_name, label),
        Style::default().fg(theme::DIM),
    );
    f.render_widget(Paragraph::new(text), area);
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
    }
}
