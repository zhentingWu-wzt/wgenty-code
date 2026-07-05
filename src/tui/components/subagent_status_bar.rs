//! SubagentStatusBar — compact status bar above the input area.
//!
//! Shows active (Running + Pending) subagents with status icons, labels, and
//! current tool/params. The selected item is highlighted for keyboard
//! navigation (↑↓ to select, Enter to open the focus view).

use crate::agent::progress::SubagentStatus;
use crate::tui::components::subagent_tree::{SubagentNode, SubagentTree};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

/// Render the subagent status bar.
///
/// `selected_index` is the index into the active (Running + Pending) node list.
pub fn render(
    f: &mut Frame,
    area: Rect,
    tree: &SubagentTree,
    selected_index: usize,
    focused: bool,
) {
    let active: Vec<&SubagentNode> = tree
        .nodes
        .values()
        .filter(|n| {
            matches!(
                n.progress.status,
                SubagentStatus::Running | SubagentStatus::Pending
            )
        })
        .collect();

    if active.is_empty() {
        return;
    }

    let border_color = if focused {
        Color::Rgb(249, 226, 175)
    } else {
        Color::Rgb(80, 80, 100)
    };
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let lines: Vec<Line> = active
        .iter()
        .enumerate()
        .map(|(i, node)| {
            let (icon, icon_color) = status_icon(&node.progress.status);
            let label = &node.progress.label;
            let detail = match (&node.progress.current_tool, &node.progress.current_params) {
                (Some(tool), Some(params)) => format!("{}(\"{}\")", tool, params),
                (Some(tool), None) => tool.clone(),
                _ => "thinking...".to_string(),
            };

            let is_selected = i == selected_index % active.len().max(1);
            let selector = if is_selected { "▶ " } else { "  " };
            let selector_style = if is_selected {
                Style::default()
                    .fg(Color::Rgb(249, 226, 175))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Rgb(108, 112, 134))
            };

            Line::from(vec![
                Span::styled(selector, selector_style),
                Span::styled(format!("{} ", icon), Style::default().fg(icon_color)),
                Span::styled(
                    format!("{:<20} ", truncate_str(label, 20)),
                    Style::default()
                        .fg(if is_selected {
                            Color::Rgb(249, 226, 175)
                        } else {
                            Color::Rgb(180, 180, 200)
                        })
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    truncate_str(&detail, inner.width.saturating_sub(28) as usize),
                    Style::default().fg(Color::Rgb(148, 148, 165)),
                ),
            ])
        })
        .collect();

    let paragraph = Paragraph::new(lines).style(Style::default().bg(Color::Rgb(26, 26, 46)));
    f.render_widget(paragraph, inner);
}

fn status_icon(status: &SubagentStatus) -> (&'static str, Color) {
    match status {
        SubagentStatus::Running => ("⟳", Color::Rgb(137, 180, 250)),
        SubagentStatus::Pending => ("○", Color::Rgb(108, 112, 134)),
        SubagentStatus::Completed => ("✓", Color::Rgb(166, 227, 161)),
        SubagentStatus::Failed => ("✗", Color::Rgb(243, 139, 168)),
        SubagentStatus::Cancelled => ("⊘", Color::Rgb(243, 139, 168)),
    }
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max.saturating_sub(3)).collect();
        format!("{}...", truncated)
    }
}

/// Collect active (Running + Pending) REAL node IDs in the order they appear
/// in the tree. Grouping/wrapper nodes (e.g., a `delegate` 1:N wrapper stuck
/// in Running) are excluded so they don't inflate the active list. Used by
/// event handling for status bar navigation.
pub fn active_node_ids(tree: &SubagentTree) -> Vec<String> {
    let node_ids = tree.real_node_list();
    node_ids
        .into_iter()
        .filter(|id| {
            tree.nodes
                .get(id)
                .map(|n| {
                    matches!(
                        n.progress.status,
                        SubagentStatus::Running | SubagentStatus::Pending
                    )
                })
                .unwrap_or(false)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::progress::SubagentProgress;
    use crate::tui::components::subagent_tree::SubagentNode;

    fn make_node(id: &str, status: SubagentStatus) -> SubagentNode {
        SubagentNode {
            progress: SubagentProgress {
                node_id: id.to_string(),
                parent_id: None,
                label: format!("Task {}", id),
                status,
                round: None,
                max_rounds: None,
                current_tool: None,
                current_params: None,
                action_log: vec![],
                text_snapshot: None,
                started_at: 0,
                elapsed_ms: 0,
                metadata: None,
                progress_delta: None,
                token_budget_k: None,
                cumulative_tokens: 0,
                error_details: None,
                events: vec![],
                messages: vec![],
            },
            children: vec![],
        }
    }

    #[test]
    fn test_active_node_ids_filters_active() {
        let mut tree = SubagentTree::default();
        // Build a proper tree: root → [a, b, c, d]
        tree.nodes.insert(
            "root".to_string(),
            make_node("root", SubagentStatus::Completed),
        );
        tree.nodes.get_mut("root").unwrap().children = vec![
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
            "d".to_string(),
        ];
        tree.nodes
            .insert("a".to_string(), make_node("a", SubagentStatus::Running));
        tree.nodes
            .insert("b".to_string(), make_node("b", SubagentStatus::Pending));
        tree.nodes
            .insert("c".to_string(), make_node("c", SubagentStatus::Completed));
        tree.nodes
            .insert("d".to_string(), make_node("d", SubagentStatus::Failed));
        tree.root_id = Some("root".to_string());

        let active = active_node_ids(&tree);
        // root is completed (filtered out), a + b are active, c + d are not
        assert_eq!(active, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn test_active_node_ids_empty() {
        let tree = SubagentTree::default();
        assert!(active_node_ids(&tree).is_empty());
    }

    #[test]
    fn test_truncate_str_short() {
        assert_eq!(truncate_str("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_str_long() {
        assert_eq!(truncate_str("hello world", 8), "hello...");
    }
}
