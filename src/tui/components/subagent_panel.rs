//! Subagent Monitor Panel — interactive panel (Ctrl+Shift+T) with node expand.
//!
//! Keyboard navigation: j/k or ↑/↓ to select nodes, Enter to expand/collapse,
//! g/G to jump to first/last, Esc to close. Expanded nodes show the full
//! think→call→think→call event timeline.

use super::subagent_panel_state::SubagentPanelState;
use super::subagent_tree::SubagentTree;
use crate::agent::progress::{ErrorType, SubagentEventType, SubagentStatus};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

pub fn render(
    f: &mut Frame,
    area: Rect,
    tree: &SubagentTree,
    state: &SubagentPanelState,
    _is_executing: bool,
) {
    let panel = Block::default()
        .title(format!(
            " 🌳 Subagent Monitor — {} agents · {} active — Esc close ",
            tree.nodes.len(),
            tree.count_by_status(SubagentStatus::Running),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(203, 166, 247)))
        .style(Style::default().bg(Color::Rgb(26, 26, 46)));

    let inner = panel.inner(area);
    f.render_widget(panel, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // summary bar
            Constraint::Min(0),    // tree body
            Constraint::Length(1), // help bar
        ])
        .split(inner);

    // Summary bar
    let done = tree.count_by_status(SubagentStatus::Completed);
    let running = tree.count_by_status(SubagentStatus::Running);
    let pending = tree.count_by_status(SubagentStatus::Pending);
    let failed = tree.count_by_status(SubagentStatus::Failed);
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!(" ✅ {} done  ", done),
                Style::default().fg(Color::Rgb(166, 227, 161)),
            ),
            Span::styled(
                format!("🔄 {} running  ", running),
                Style::default().fg(Color::Rgb(249, 226, 175)),
            ),
            Span::styled(
                format!("⏳ {} pending  ", pending),
                Style::default().fg(Color::Rgb(108, 112, 134)),
            ),
            Span::styled(
                format!("❌ {} failed", failed),
                Style::default().fg(Color::Rgb(243, 139, 168)),
            ),
        ])),
        chunks[0],
    );

    // Tree body with expand support
    let mut tree_lines: Vec<Line> = Vec::new();
    let selected_id = state.selected_node_id(tree);
    render_tree_with_expand(
        &mut tree_lines,
        tree,
        state,
        tree.root_id.as_deref(),
        0,
        4u16,
        selected_id.as_deref(),
    );
    f.render_widget(
        Paragraph::new(ratatui::text::Text::from(tree_lines)).wrap(Wrap { trim: false }),
        chunks[1],
    );

    // Help bar
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " ↑↓ navigate  ",
                Style::default().fg(Color::Rgb(108, 112, 134)),
            ),
            Span::styled(
                "Enter expand  ",
                Style::default().fg(Color::Rgb(108, 112, 134)),
            ),
            Span::styled(
                "g/G top/bottom  ",
                Style::default().fg(Color::Rgb(108, 112, 134)),
            ),
            Span::styled("Esc close", Style::default().fg(Color::Rgb(108, 112, 134))),
        ])),
        chunks[2],
    );
}

/// Recursively render tree nodes with selection highlight and expand.
fn render_tree_with_expand(
    lines: &mut Vec<Line>,
    tree: &SubagentTree,
    state: &SubagentPanelState,
    node_id: Option<&str>,
    depth: u16,
    base_indent: u16,
    selected_id: Option<&str>,
) {
    let Some(nid) = node_id else {
        return;
    };
    let Some(node) = tree.nodes.get(nid) else {
        return;
    };

    let indent = base_indent + depth * 2;
    let is_selected = selected_id == Some(nid);
    let is_expanded = state.is_expanded(nid);
    let prefix = if is_expanded { "▶" } else { "▸" };
    let indent_str = " ".repeat(indent as usize);

    let icon = match node.progress.status {
        SubagentStatus::Pending => "⏳",
        SubagentStatus::Running => "🔄",
        SubagentStatus::Completed => "✅",
        SubagentStatus::Failed => "❌",
        SubagentStatus::Cancelled => "🚫",
    };

    let color = match node.progress.status {
        SubagentStatus::Running => Color::Rgb(249, 226, 175),
        SubagentStatus::Completed => Color::Rgb(166, 227, 161),
        SubagentStatus::Failed | SubagentStatus::Cancelled => Color::Rgb(243, 139, 168),
        SubagentStatus::Pending => Color::Rgb(108, 112, 134),
    };

    let select_style = if is_selected {
        Style::default()
            .fg(Color::Rgb(203, 166, 247))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(color)
    };

    // ── Node header line ─────────────────────────────────────────────
    let elapsed_secs = node.progress.elapsed_ms as f64 / 1000.0;
    let status_detail = match node.progress.status {
        SubagentStatus::Running => match (node.progress.round, node.progress.max_rounds) {
            (Some(r), Some(mr)) => format!("round {}/{} · {:.1}s", r, mr, elapsed_secs),
            _ => format!("{:.1}s", elapsed_secs),
        },
        SubagentStatus::Completed => {
            let mut s = node
                .progress
                .round
                .map(|r| format!("{} rounds", r))
                .unwrap_or_default();
            if !s.is_empty() {
                s.push_str(" · ");
            }
            s.push_str(&format!("{:.1}s", elapsed_secs));
            if let Some(ref meta) = node.progress.metadata {
                if let Some(tc) = meta.token_count {
                    if tc >= 1000 {
                        s.push_str(&format!(" · {:.1}k tokens", tc as f64 / 1000.0));
                    } else {
                        s.push_str(&format!(" · {} tokens", tc));
                    }
                }
            }
            s
        }
        _ => String::new(),
    };

    let label = if status_detail.is_empty() {
        format!(" {}", node.progress.label)
    } else {
        format!(" {} — {}", node.progress.label, status_detail)
    };

    lines.push(Line::from(vec![
        Span::styled(
            format!("{}{} ", indent_str, prefix),
            Style::default().fg(Color::Rgb(108, 112, 134)),
        ),
        Span::styled(icon, select_style),
        Span::styled(label, select_style),
    ]));

    // ── Expanded: show full event timeline ──────────────────────────
    if is_expanded {
        let event_indent = " ".repeat((indent + 2) as usize);
        let events: Vec<&crate::agent::progress::SubagentEvent> =
            node.progress.action_log.iter().collect();

        if events.is_empty() {
            lines.push(Line::from(vec![Span::styled(
                format!("{}│", event_indent),
                Style::default().fg(Color::Rgb(80, 80, 100)),
            )]));
            lines.push(Line::from(vec![Span::styled(
                format!("{}💭 thinking…", event_indent),
                Style::default().fg(Color::Rgb(108, 112, 134)),
            )]));
            lines.push(Line::from(vec![Span::styled(
                format!("{}▼", event_indent),
                Style::default().fg(Color::Rgb(80, 80, 100)),
            )]));
        } else {
            for (i, event) in events.iter().enumerate() {
                let connector = if i == events.len() - 1 { "▼" } else { "│" };
                match &event.event_type {
                    SubagentEventType::Thought { text } => {
                        let preview: String = text.chars().take(140).collect();
                        let display = if text.len() > 140 {
                            format!("{}…", preview)
                        } else {
                            preview
                        };
                        lines.push(Line::from(vec![Span::styled(
                            format!("{}{}", event_indent, connector),
                            Style::default().fg(Color::Rgb(80, 80, 100)),
                        )]));
                        lines.push(Line::from(vec![Span::styled(
                            format!(
                                "{}💭 {}  ({:.1}s)",
                                event_indent,
                                display,
                                event.elapsed_ms as f64 / 1000.0
                            ),
                            Style::default().fg(Color::Rgb(180, 180, 200)),
                        )]));
                    }
                    SubagentEventType::Action {
                        tool_name,
                        params_summary,
                    } => {
                        let action_str = if params_summary.is_empty() {
                            format!("{}", tool_name)
                        } else {
                            format!("{}(\"{}\")", tool_name, params_summary)
                        };
                        lines.push(Line::from(vec![Span::styled(
                            format!("{}{}", event_indent, connector),
                            Style::default().fg(Color::Rgb(80, 80, 100)),
                        )]));
                        lines.push(Line::from(vec![Span::styled(
                            format!(
                                "{}▸ {}  ({:.1}s)",
                                event_indent,
                                action_str,
                                event.elapsed_ms as f64 / 1000.0
                            ),
                            Style::default().fg(Color::Rgb(137, 180, 250)),
                        )]));
                    }
                    SubagentEventType::ToolResult {
                        tool_name,
                        success,
                        summary,
                    } => {
                        let icon = if *success { "✓" } else { "✗" };
                        lines.push(Line::from(vec![Span::styled(
                            format!("{}{}", event_indent, connector),
                            Style::default().fg(Color::Rgb(80, 80, 100)),
                        )]));
                        let color = if *success {
                            Color::Rgb(120, 200, 120)
                        } else {
                            Color::Rgb(220, 100, 100)
                        };
                        lines.push(Line::from(vec![Span::styled(
                            format!(
                                "{}{} {} {}  ({:.1}s)",
                                event_indent,
                                icon,
                                tool_name,
                                summary,
                                event.elapsed_ms as f64 / 1000.0
                            ),
                            Style::default().fg(color),
                        )]));
                    }
                    SubagentEventType::Error {
                        message,
                        error_type,
                    } => {
                        let err_type = match error_type {
                            ErrorType::Timeout => "TIMEOUT",
                            ErrorType::BudgetExceeded { .. } => "BUDGET",
                            ErrorType::Stuck { .. } => "STUCK",
                            ErrorType::ToolError { .. } => "TOOL_ERR",
                            ErrorType::ParseError { .. } => "PARSE_ERR",
                            ErrorType::Unknown => "ERROR",
                        };
                        lines.push(Line::from(vec![Span::styled(
                            format!("{}{}", event_indent, connector),
                            Style::default().fg(Color::Rgb(80, 80, 100)),
                        )]));
                        lines.push(Line::from(vec![Span::styled(
                            format!(
                                "{}⚠ [{}] {}  ({:.1}s)",
                                event_indent,
                                err_type,
                                message,
                                event.elapsed_ms as f64 / 1000.0
                            ),
                            Style::default().fg(Color::Rgb(255, 150, 50)),
                        )]));
                    }
                    SubagentEventType::Completion { status, summary } => {
                        let summary_str = summary.as_deref().unwrap_or("");
                        lines.push(Line::from(vec![Span::styled(
                            format!("{}{}", event_indent, connector),
                            Style::default().fg(Color::Rgb(80, 80, 100)),
                        )]));
                        lines.push(Line::from(vec![Span::styled(
                            format!(
                                "{}🏁 {} {}  ({:.1}s)",
                                event_indent,
                                status,
                                summary_str,
                                event.elapsed_ms as f64 / 1000.0
                            ),
                            Style::default().fg(Color::Rgb(140, 200, 255)),
                        )]));
                    }
                }
            }
        }
        lines.push(Line::default()); // blank line after expanded section
    }

    // ── Collapsed: show compact info ────────────────────────────────
    if node.progress.status == SubagentStatus::Running {
        let detail_indent = " ".repeat((indent + 2) as usize);
        // Current tool
        if let Some(ref tool) = node.progress.current_tool {
            let tool_label = if let Some(ref params) = node.progress.current_params {
                if params.is_empty() {
                    format!("executing: {}", tool)
                } else {
                    format!("executing: {}(\"{}\")", tool, params)
                }
            } else {
                format!("executing: {}", tool)
            };
            lines.push(Line::from(vec![Span::styled(
                format!("{}└─ 🛠 {}", detail_indent, tool_label),
                Style::default().fg(Color::Rgb(137, 180, 250)),
            )]));
        }

        // Text snapshot
        if let Some(ref snapshot) = node.progress.text_snapshot {
            let preview: String = snapshot.chars().take(100).collect();
            let display = if snapshot.len() > 100 {
                format!("{}…", preview)
            } else {
                preview
            };
            lines.push(Line::from(vec![Span::styled(
                format!("{}   💬 {}", detail_indent, display),
                Style::default().fg(Color::Rgb(150, 150, 165)),
            )]));
        } else {
            lines.push(Line::from(vec![Span::styled(
                format!("{}   💭 thinking…", detail_indent),
                Style::default().fg(Color::Rgb(150, 150, 165)),
            )]));
        }

        // Recent 3 Action events
        let recent: Vec<_> = node
            .progress
            .action_log
            .iter()
            .filter(|e| matches!(e.event_type, SubagentEventType::Action { .. }))
            .rev()
            .take(3)
            .collect();
        for event in recent.iter().rev() {
            if let SubagentEventType::Action {
                tool_name,
                params_summary,
            } = &event.event_type
            {
                let action_str = if params_summary.is_empty() {
                    format!("{}", tool_name)
                } else {
                    format!("{}(\"{}\")", tool_name, params_summary)
                };
                lines.push(Line::from(vec![Span::styled(
                    format!("{}   ▸ {}", detail_indent, action_str),
                    Style::default().fg(Color::Rgb(108, 112, 134)),
                )]));
            }
        }
    }

    // Recurse into children
    for child_id in &node.children {
        render_tree_with_expand(
            lines,
            tree,
            state,
            Some(child_id),
            depth + 1,
            base_indent,
            selected_id,
        );
    }
}
