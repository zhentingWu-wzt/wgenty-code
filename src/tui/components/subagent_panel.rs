//! Subagent Monitor Panel — toggleable panel (Ctrl+Shift+T) showing full tree.

use crate::agent::progress::SubagentStatus;
use super::subagent_tree::SubagentTree;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

pub fn render(f: &mut Frame, area: Rect, tree: &SubagentTree, is_executing: bool) {
    let panel = Block::default()
        .title(format!(
            " 🌳 Subagent Monitor — {} agents · {} active — Ctrl+Shift+T toggle ",
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
        .constraints([Constraint::Length(1), Constraint::Min(0), Constraint::Length(1)])
        .split(inner);

    // Summary bar
    let done = tree.count_by_status(SubagentStatus::Completed);
    let running = tree.count_by_status(SubagentStatus::Running);
    let pending = tree.count_by_status(SubagentStatus::Pending);
    let failed = tree.count_by_status(SubagentStatus::Failed);
    f.render_widget(Paragraph::new(Line::from(vec![
        Span::styled(format!(" ✅ {} done  ", done), Style::default().fg(Color::Rgb(166, 227, 161))),
        Span::styled(format!("🔄 {} running  ", running), Style::default().fg(Color::Rgb(249, 226, 175))),
        Span::styled(format!("⏳ {} pending  ", pending), Style::default().fg(Color::Rgb(108, 112, 134))),
        Span::styled(format!("❌ {} failed", failed), Style::default().fg(Color::Rgb(243, 139, 168))),
    ])), chunks[0]);

    // Tree body
    let mut tree_lines: Vec<Line> = Vec::new();
    super::chat::render_subagent_card(&mut tree_lines, tree, inner.width, is_executing, 0);
    f.render_widget(
        Paragraph::new(ratatui::text::Text::from(tree_lines)).wrap(Wrap { trim: false }),
        chunks[1],
    );

    // Info line
    let info = tree.root_id.as_ref()
        .and_then(|rid| tree.nodes.get(rid))
        .map(|root| match &root.progress.status {
            SubagentStatus::Running => format!(" ℹ️  {} — elapsed {}s", root.progress.label, root.progress.elapsed_ms / 1000),
            SubagentStatus::Completed => format!(" ✅ Completed — {} nodes in {}s", tree.nodes.len(), root.progress.elapsed_ms / 1000),
            _ => format!(" {} subagents tracked", tree.nodes.len()),
        })
        .unwrap_or_else(|| " No active subagent execution".to_string());
    f.render_widget(
        Paragraph::new(Span::styled(info, Style::default().fg(Color::Rgb(108, 112, 134)))),
        chunks[2],
    );
}
