//! DetailView — Full-screen event timeline for a completed/failed subagent.

use crate::agent::progress::SubagentEventType;
use crate::tui::components::subagent_panel_state::DetailViewState;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

pub struct DetailView;

impl DetailView {
    pub fn render(f: &mut Frame, area: Rect, detail: &DetailViewState) {
        let block = Block::default()
            .title(" Event Timeline ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Rgb(249, 226, 175)))
            .style(Style::default().bg(Color::Rgb(26, 26, 46)));

        let inner = block.inner(area);
        f.render_widget(block, area);

        let mut lines: Vec<Line> = Vec::new();

        // ── Transcript header ─────────────────────────────────────────
        let header_count: usize;
        if let Some(ref status) = detail.status {
            let (status_label, status_color) = match status {
                crate::agent::progress::SubagentStatus::Completed => {
                    ("COMPLETED", Color::Rgb(166, 227, 161))
                }
                crate::agent::progress::SubagentStatus::Failed => {
                    ("FAILED", Color::Rgb(243, 139, 168))
                }
                crate::agent::progress::SubagentStatus::Cancelled => {
                    ("CANCELLED", Color::Rgb(243, 139, 168))
                }
                crate::agent::progress::SubagentStatus::Running => {
                    ("RUNNING", Color::Rgb(249, 226, 175))
                }
                crate::agent::progress::SubagentStatus::Pending => {
                    ("PENDING", Color::Rgb(108, 112, 134))
                }
            };
            lines.push(Line::from(vec![
                Span::styled(" Status:  ", Style::default().fg(Color::Rgb(108, 112, 134))),
                Span::styled(
                    status_label,
                    Style::default()
                        .fg(status_color)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
            let total_secs = detail.total_elapsed_ms as f64 / 1000.0;
            lines.push(Line::from(vec![
                Span::styled(" Time:    ", Style::default().fg(Color::Rgb(108, 112, 134))),
                Span::styled(
                    format!("{:.1}s", total_secs),
                    Style::default().fg(Color::Rgb(180, 180, 200)),
                ),
            ]));
            let budget_str = detail
                .token_budget_k
                .map(|b| format!("{}k", b))
                .unwrap_or_else(|| "unlimited".to_string());
            lines.push(Line::from(vec![
                Span::styled(" Tokens:  ", Style::default().fg(Color::Rgb(108, 112, 134))),
                Span::styled(
                    format!("{}/{}", detail.cumulative_tokens, budget_str),
                    Style::default().fg(Color::Rgb(180, 180, 200)),
                ),
            ]));
            if let (Some(r), Some(mr)) = (detail.round, detail.max_rounds) {
                lines.push(Line::from(vec![
                    Span::styled(" Rounds:  ", Style::default().fg(Color::Rgb(108, 112, 134))),
                    Span::styled(
                        format!("{}/{}", r, mr),
                        Style::default().fg(Color::Rgb(180, 180, 200)),
                    ),
                ]));
            }
            if let Some(ref err) = detail.error_message {
                lines.push(Line::from(vec![
                    Span::styled(" Error:   ", Style::default().fg(Color::Rgb(108, 112, 134))),
                    Span::styled(
                        err,
                        Style::default()
                            .fg(Color::Rgb(243, 139, 168))
                            .add_modifier(Modifier::BOLD),
                    ),
                ]));
            }
            lines.push(Line::from(vec![Span::styled(
                "\u{2500}".repeat(inner.width.saturating_sub(2) as usize),
                Style::default().fg(Color::Rgb(80, 80, 100)),
            )]));
            header_count = lines.len();
        } else {
            header_count = 0;
        }

        // ── Event timeline ────────────────────────────────────────────
        let scroll = detail.scroll_offset;
        let available = (inner.height as usize).saturating_sub(header_count + 1); // +1 for help bar
        let visible_events: Vec<_> = detail.events.iter().skip(scroll).take(available).collect();

        if visible_events.is_empty() && header_count == 0 {
            lines.push(Line::from(vec![Span::styled(
                "No events recorded.",
                Style::default().fg(Color::Rgb(108, 112, 134)),
            )]));
        }

        for event in &visible_events {
            let elapsed = format!("+{:.1}s", event.elapsed_ms as f64 / 1000.0);
            match &event.event_type {
                SubagentEventType::Thought { text } => {
                    let max_w = inner.width.saturating_sub(12) as usize;
                    let preview: String = text.chars().take(max_w).collect();
                    let display = if text.len() > max_w {
                        format!("{}...", preview)
                    } else {
                        preview
                    };
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!(" {:<8} ", elapsed),
                            Style::default().fg(Color::Rgb(108, 112, 134)),
                        ),
                        Span::styled(
                            " THOUGHT ",
                            Style::default()
                                .fg(Color::Rgb(180, 180, 200))
                                .add_modifier(Modifier::DIM),
                        ),
                        Span::styled(display, Style::default().fg(Color::Rgb(180, 180, 200))),
                    ]));
                }
                SubagentEventType::Action {
                    tool_name,
                    params_summary,
                    ..
                } => {
                    let action_str = if params_summary.is_empty() {
                        tool_name.clone()
                    } else {
                        format!("{}(\"{}\")", tool_name, params_summary)
                    };
                    let max_w = inner.width.saturating_sub(12) as usize;
                    let display: String = action_str.chars().take(max_w).collect();
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!(" {:<8} ", elapsed),
                            Style::default().fg(Color::Rgb(108, 112, 134)),
                        ),
                        Span::styled(" TOOL    ", Style::default().fg(Color::Rgb(137, 180, 250))),
                        Span::styled(display, Style::default().fg(Color::Rgb(137, 180, 250))),
                    ]));
                }
                SubagentEventType::ToolResult {
                    tool_name,
                    success,
                    summary,
                } => {
                    let icon = if *success { "OK" } else { "FAIL" };
                    let color = if *success {
                        Color::Rgb(166, 227, 161)
                    } else {
                        Color::Rgb(243, 139, 168)
                    };
                    let max_w = inner.width.saturating_sub(12) as usize;
                    let display: String = summary.chars().take(max_w).collect();
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!(" {:<8} ", elapsed),
                            Style::default().fg(Color::Rgb(108, 112, 134)),
                        ),
                        Span::styled(format!(" {} ", icon), Style::default().fg(color)),
                        Span::styled(
                            format!("{}: {}", tool_name, display),
                            Style::default().fg(Color::Rgb(148, 148, 165)),
                        ),
                    ]));
                }
                SubagentEventType::Error { message, .. } => {
                    let max_w = inner.width.saturating_sub(12) as usize;
                    let display: String = message.chars().take(max_w).collect();
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!(" {:<8} ", elapsed),
                            Style::default().fg(Color::Rgb(108, 112, 134)),
                        ),
                        Span::styled(
                            " ERROR   ",
                            Style::default()
                                .fg(Color::Rgb(243, 139, 168))
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(display, Style::default().fg(Color::Rgb(243, 139, 168))),
                    ]));
                }
                SubagentEventType::Completion { status, summary } => {
                    let status_display = match status.as_str() {
                        "completed" => "COMPLETED",
                        "failed" => "FAILED",
                        _ => status,
                    };
                    let color = if status == "completed" {
                        Color::Rgb(166, 227, 161)
                    } else {
                        Color::Rgb(243, 139, 168)
                    };
                    let sum = summary.as_deref().unwrap_or("");
                    let max_w = inner.width.saturating_sub(12) as usize;
                    let display: String = sum.chars().take(max_w).collect();
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!(" {:<8} ", elapsed),
                            Style::default().fg(Color::Rgb(108, 112, 134)),
                        ),
                        Span::styled(
                            format!(" {}  ", status_display),
                            Style::default().fg(color).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(display, Style::default().fg(color)),
                    ]));
                }
            }
        }

        // Help line at bottom
        let total = detail.events.len();
        let scroll_info = if total > 0 {
            format!("({}/{})", scroll + 1, total)
        } else {
            "(no events)".to_string()
        };
        let help = Line::from(vec![
            Span::styled(
                format!(" \u{2191}\u{2195} scroll  PgUp/PgDn page  g/G top/bottom  f jump error  Esc back {}", scroll_info),
                Style::default().fg(Color::Rgb(108, 112, 134)),
            ),
        ]);

        f.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }),
            Rect {
                x: inner.x,
                y: inner.y,
                width: inner.width,
                height: inner.height.saturating_sub(1),
            },
        );
        f.render_widget(
            Paragraph::new(help),
            Rect {
                x: inner.x,
                y: inner.y + inner.height.saturating_sub(1),
                width: inner.width,
                height: 1,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::progress::{ErrorType, SubagentEvent, SubagentEventType};

    fn make_thought_event(text: &str) -> SubagentEvent {
        SubagentEvent {
            event_type: SubagentEventType::Thought {
                text: text.to_string(),
            },
            elapsed_ms: 100,
        }
    }

    fn make_action_event() -> SubagentEvent {
        SubagentEvent {
            event_type: SubagentEventType::Action {
                tool_name: "read_file".to_string(),
                params_summary: "src/main.rs".to_string(),
            },
            elapsed_ms: 200,
        }
    }

    fn make_tool_result(success: bool) -> SubagentEvent {
        SubagentEvent {
            event_type: SubagentEventType::ToolResult {
                tool_name: "read_file".to_string(),
                success,
                summary: "file read".to_string(),
            },
            elapsed_ms: 300,
        }
    }

    fn make_error_event(msg: &str) -> SubagentEvent {
        SubagentEvent {
            event_type: SubagentEventType::Error {
                message: msg.to_string(),
                error_type: ErrorType::Timeout,
            },
            elapsed_ms: 400,
        }
    }

    fn make_completion_event(status: &str) -> SubagentEvent {
        SubagentEvent {
            event_type: SubagentEventType::Completion {
                status: status.to_string(),
                summary: Some("done".to_string()),
            },
            elapsed_ms: 500,
        }
    }

    fn make_detail(
        transcript_id: &str,
        scroll_offset: usize,
        events: Vec<SubagentEvent>,
    ) -> DetailViewState {
        DetailViewState {
            transcript_id: transcript_id.to_string(),
            scroll_offset,
            events,
            loading: false,
            status: None,
            total_elapsed_ms: 0,
            cumulative_tokens: 0,
            token_budget_k: None,
            error_message: None,
            round: None,
            max_rounds: None,
        }
    }

    #[test]
    fn test_detail_view_events_types() {
        let events = vec![
            make_thought_event("analyzing code"),
            make_action_event(),
            make_tool_result(true),
            make_error_event("timeout occurred"),
            make_completion_event("completed"),
        ];

        let detail = make_detail("node1", 0, events);

        assert_eq!(detail.transcript_id, "node1");
        // The first event should be a Thought
        match &detail.events[0].event_type {
            SubagentEventType::Thought { text } => assert_eq!(text, "analyzing code"),
            _ => panic!("expected Thought"),
        }
        // The fourth event should be an Error
        match &detail.events[3].event_type {
            SubagentEventType::Error { message, .. } => assert_eq!(message, "timeout occurred"),
            _ => panic!("expected Error"),
        }
        // The fifth event should be a Completion
        match &detail.events[4].event_type {
            SubagentEventType::Completion { status, .. } => assert_eq!(status, "completed"),
            _ => panic!("expected Completion"),
        }
    }

    #[test]
    fn test_detail_view_error_jump_position() {
        let events = vec![
            make_thought_event("step 1"),
            make_action_event(),
            make_tool_result(true),
            make_error_event("something broke"),
            make_thought_event("after error"),
        ];

        // Find first error position using the same logic as event.rs
        let error_pos = events
            .iter()
            .position(|e| matches!(e.event_type, SubagentEventType::Error { .. }));
        assert_eq!(error_pos, Some(3));

        // Simulate jumping to error
        let mut scroll = 0;
        if let Some(pos) = error_pos {
            scroll = pos;
        }
        assert_eq!(scroll, 3);
    }

    #[test]
    fn test_detail_view_scroll_up_down() {
        let mut detail = make_detail("node1", 5, vec![make_thought_event("e"); 20]);

        // Up
        detail.scroll_offset = detail.scroll_offset.saturating_sub(1);
        assert_eq!(detail.scroll_offset, 4);

        // Down
        detail.scroll_offset = detail.scroll_offset.saturating_add(1);
        assert_eq!(detail.scroll_offset, 5);

        // Can't go below 0
        detail.scroll_offset = 0;
        detail.scroll_offset = detail.scroll_offset.saturating_sub(1);
        assert_eq!(detail.scroll_offset, 0);

        // Can't go past events.len() - 1 (but saturating add allows it; render clips)
        detail.scroll_offset = usize::MAX;
        detail.scroll_offset = detail.scroll_offset.saturating_add(1);
        assert_eq!(detail.scroll_offset, usize::MAX);

        // Normal scroll behavior
        detail.scroll_offset = 10;
        detail.scroll_offset = detail.scroll_offset.saturating_add(1);
        assert_eq!(detail.scroll_offset, 11);
    }

    #[test]
    fn test_detail_view_scroll_to_top_bottom() {
        let events = vec![make_thought_event("e"); 20];
        let detail = make_detail("node1", 0, events);

        // g = top = 0
        assert_eq!(detail.scroll_offset, 0);

        // G = bottom = events.len() - 1
        let bottom = detail.events.len().saturating_sub(1);
        assert_eq!(bottom, 19);
    }

    #[test]
    fn test_detail_view_empty_events() {
        let detail = make_detail("node1", 0, vec![]);
        assert!(detail.events.is_empty());
        assert_eq!(detail.scroll_offset, 0);
    }
}
