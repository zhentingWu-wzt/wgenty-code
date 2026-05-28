use crate::tui::app::{MessageRole, UIMessage};
use crate::tui::theme;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// Render the chat message list.
/// - `committed_messages` — stable history (doesn't change during streaming)
/// - `streaming_content` — current streaming tokens (frequently updated)
/// - `streaming_active` — whether we're currently receiving tokens
/// - `scroll_offset` — manual scroll position (0 = newest at bottom)
pub fn render(
    f: &mut Frame,
    area: Rect,
    committed_messages: &[UIMessage],
    streaming_content: &str,
    streaming_active: bool,
    scroll_offset: u16,
) {
    let mut lines: Vec<Line> = Vec::new();

    for msg in committed_messages {
        lines.extend(message_to_lines(msg));
    }

    // Streaming content rendered as a transient final line
    if streaming_active && !streaming_content.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("● ", Style::default().fg(theme::ROLE_ASSISTANT)),
            Span::raw(streaming_content),
        ]));
    }

    let para = Paragraph::new(Text::from(lines))
        .scroll((scroll_offset, 0));
    f.render_widget(para, area);
}

fn message_to_lines(msg: &UIMessage) -> Vec<Line<'static>> {
    match msg.role {
        MessageRole::User => {
            vec![Line::from(vec![
                Span::styled("▸ ", Style::default().fg(theme::ROLE_USER)),
                Span::raw(msg.content.clone()),
            ])]
        }
        MessageRole::Assistant => {
            let mut lines = Vec::new();
            if msg.content.is_empty() {
                lines.push(Line::from(vec![
                    Span::styled("● ", Style::default().fg(theme::ROLE_ASSISTANT)),
                    Span::raw(""),
                ]));
            } else {
                for line in msg.content.lines() {
                    lines.push(Line::from(vec![
                        Span::styled("● ", Style::default().fg(theme::ROLE_ASSISTANT)),
                        Span::raw(line.to_string()),
                    ]));
                }
            }
            lines
        }
        MessageRole::Tool => {
            let label = msg.tool_name.as_deref().unwrap_or("tool");
            let preview = if msg.content.len() > 300 {
                format!("{}...", &msg.content[..300])
            } else {
                msg.content.clone()
            };
            vec![Line::from(vec![
                Span::styled(
                    format!("⚙ {}: ", label),
                    Style::default().fg(theme::ROLE_TOOL),
                ),
                Span::styled(preview, Style::default().fg(theme::DIM)),
            ])]
        }
        MessageRole::System => {
            vec![Line::from(vec![Span::styled(
                msg.content.clone(),
                Style::default().fg(theme::DIM),
            )])]
        }
    }
}
