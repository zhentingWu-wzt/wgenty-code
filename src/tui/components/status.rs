use crate::tui::theme;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// Render the status bar.
pub fn render(f: &mut Frame, area: Rect, status: &str, session_name: &str) {
    let label = status_label(status);
    let text = Span::styled(
        format!(" {} | {}", session_name, label),
        Style::default().fg(theme::DIM),
    );
    f.render_widget(Paragraph::new(text), area);
}

fn status_label(status: &str) -> &str {
    match status {
        "idle" => "Ready",
        "thinking" => "Thinking...",
        "streaming" => "Streaming...",
        s if s.starts_with("executing") => s,
        _ => status,
    }
}
