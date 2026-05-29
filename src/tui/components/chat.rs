use crate::tui::app::{MessageRole, UIMessage};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

const USER_COLOR: Color = Color::Rgb(255, 140, 66);
const ASSISTANT_COLOR: Color = Color::Rgb(147, 112, 219);
const ASSISTANT_HEADER_COLOR: Color = Color::Rgb(200, 150, 255);
const TEXT_COLOR: Color = Color::Rgb(220, 220, 230);
const DIM_COLOR: Color = Color::Rgb(100, 100, 110);
const TOOL_COLOR: Color = Color::Rgb(100, 200, 255);

/// Render the chat message list.
/// - `user_scrolled` — false = auto-scroll to bottom; true = use `scroll_offset` as-is
/// - `scroll_offset` — ratatui-native: lines to skip from top (0 = oldest, max = newest)
pub fn render(
    f: &mut Frame,
    area: Rect,
    committed_messages: &[UIMessage],
    streaming_content: &str,
    streaming_active: bool,
    scroll_offset: u16,
    user_scrolled: bool,
) {
    let mut lines: Vec<Line> = Vec::new();

    for msg in committed_messages {
        lines.extend(message_to_lines(msg, area.width));
    }

    // Streaming content as a transient final block
    if streaming_active && !streaming_content.is_empty() {
        let max_w = area.width.saturating_sub(4) as usize;
        lines.push(border_top(" Wgenty ▐", ASSISTANT_HEADER_COLOR, max_w));
        for line in streaming_content.lines() {
            push_wrapped(&mut lines, line, "│ ", ASSISTANT_COLOR, TEXT_COLOR, max_w);
        }
        lines.push(border_bottom(ASSISTANT_COLOR, max_w));
        lines.push(Line::raw(""));
    }

    let total_lines = lines.len() as u16;
    let viewport = area.height;

    // If auto-scrolling, snap to bottom. Otherwise use the user's scroll offset.
    let actual_scroll = if user_scrolled {
        scroll_offset.min(total_lines.saturating_sub(viewport))
    } else {
        total_lines.saturating_sub(viewport)
    };

    let para = Paragraph::new(Text::from(lines)).scroll((actual_scroll, 0));
    f.render_widget(para, area);
}

fn message_to_lines(msg: &UIMessage, width: u16) -> Vec<Line<'static>> {
    let max_w = width.saturating_sub(4) as usize;

    match msg.role {
        MessageRole::User => {
            let mut lines = vec![border_top(" You ", USER_COLOR, max_w)];
            for line in msg.content.lines() {
                push_wrapped(&mut lines, line, "│ ", USER_COLOR, Color::White, max_w);
            }
            lines.push(border_bottom(USER_COLOR, max_w));
            lines.push(Line::raw(""));
            lines
        }
        MessageRole::Assistant => {
            let mut lines = vec![border_top(" Wgenty ", ASSISTANT_HEADER_COLOR, max_w)];
            if msg.content.is_empty() {
                lines.push(Line::from(Span::styled(
                    "│ ",
                    Style::default().fg(ASSISTANT_COLOR),
                )));
            } else if msg.content_collapsed {
                render_collapsed(&mut lines, &msg.content, "│ ", ASSISTANT_COLOR, max_w);
            } else {
                for line in msg.content.lines() {
                    push_wrapped(&mut lines, line, "│ ", ASSISTANT_COLOR, TEXT_COLOR, max_w);
                }
            }
            lines.push(border_bottom(ASSISTANT_COLOR, max_w));
            lines.push(Line::raw(""));
            lines
        }
        MessageRole::Tool => {
            if msg.content.is_empty() {
                // ToolStart placeholder — show "running..." indicator
                let label = msg.tool_name.as_deref().unwrap_or("tool");
                vec![Line::from(vec![Span::styled(
                    format!("⚙ {}: running...", label),
                    Style::default().fg(TOOL_COLOR),
                )])]
            } else if msg.tool_collapsed {
                let label = msg.tool_name.as_deref().unwrap_or("Tool");
                let mut lines = vec![border_top(&format!(" {} ", label), TOOL_COLOR, max_w)];
                render_collapsed(&mut lines, &msg.content, "│ ", TOOL_COLOR, max_w);
                lines.push(border_bottom(TOOL_COLOR, max_w));
                lines.push(Line::raw(""));
                lines
            } else {
                // ToolResult — content already formatted by format_tool_result
                msg.content
                    .lines()
                    .map(|line| {
                        Line::from(Span::styled(
                            line.to_string(),
                            Style::default().fg(DIM_COLOR),
                        ))
                    })
                    .collect()
            }
        }
        MessageRole::System => {
            msg.content
                .lines()
                .map(|line| {
                    Line::from(Span::styled(
                        line.to_string(),
                        Style::default().fg(DIM_COLOR),
                    ))
                })
                .collect()
        }
    }
}

/// Render a collapsed paragraph: first 3 lines + "... (N lines total, collapsed)" indicator.
fn render_collapsed(
    lines_buf: &mut Vec<Line<'static>>,
    content: &str,
    prefix: &str,
    prefix_color: Color,
    max_w: usize,
) {
    let preview_lines: Vec<&str> = content.lines().take(3).collect();
    let total_lines = content.lines().count();
    for line in &preview_lines {
        push_wrapped(lines_buf, line, prefix, prefix_color, DIM_COLOR, max_w);
    }
    let indicator = format!("   ... ({} lines total, collapsed)", total_lines);
    lines_buf.push(Line::from(Span::styled(
        indicator,
        Style::default().fg(DIM_COLOR),
    )));
}

fn border_top(label: &str, color: Color, max_w: usize) -> Line<'static> {
    let label_len = label.chars().count();
    let right_len = max_w.saturating_sub(label_len);
    Line::from(Span::styled(
        format!("┌{}┌{}", label, "─".repeat(right_len)),
        Style::default().fg(color),
    ))
}

fn border_bottom(color: Color, max_w: usize) -> Line<'static> {
    Line::from(Span::styled(
        format!("└{}", "─".repeat(max_w)),
        Style::default().fg(color),
    ))
}

fn push_wrapped(
    lines: &mut Vec<Line<'static>>,
    text: &str,
    prefix: &str,
    prefix_color: Color,
    text_color: Color,
    max_w: usize,
) {
    let prefix_len = prefix.chars().count();
    let content_w = max_w.saturating_sub(prefix_len);

    if text.is_empty() {
        lines.push(Line::from(Span::styled(
            prefix.to_string(),
            Style::default().fg(prefix_color),
        )));
        return;
    }

    let prefix_owned = prefix.to_string();
    let chars: Vec<char> = text.chars().collect();
    let mut start = 0;

    while start < chars.len() {
        let end = (start + content_w).min(chars.len());
        let end = if end < chars.len() {
            let mut break_at = end;
            for i in (start..end).rev() {
                if chars[i] == ' ' {
                    break_at = i;
                    break;
                }
            }
            if break_at == start {
                end
            } else {
                break_at
            }
        } else {
            end
        };

        let chunk: String = chars[start..end].iter().collect();
        lines.push(Line::from(vec![
            Span::styled(prefix_owned.clone(), Style::default().fg(prefix_color)),
            Span::styled(chunk, Style::default().fg(text_color)),
        ]));

        start = if end < chars.len() && chars[end] == ' ' {
            end + 1
        } else {
            end
        };
    }
}
