//! SubagentStatusBar — compact status bar above the input area.
//!
//! Shows nonterminal subagents with status icons, labels, and
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
/// `selected_index` indexes the unified list ["main", ...active] where
/// index 0 is the "main" placeholder and 1..N are active subagents.
pub fn render(
    f: &mut Frame,
    area: Rect,
    tree: &SubagentTree,
    selected_index: usize,
    focused: bool,
) {
    let active_ids = active_node_ids(tree);
    let active: Vec<&SubagentNode> = active_ids
        .iter()
        .filter_map(|id| tree.nodes.get(id))
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

    // Unified list: ["main", ...active]. wrap_len = N+1 (main + subagents).
    let wrap_len = active.len() + 1;
    let sel = selected_index % wrap_len;

    let mut lines: Vec<Line> = Vec::with_capacity(wrap_len);

    // "main" entry at absolute index 0.
    {
        let is_selected = sel == 0;
        let selector = if is_selected { "▶ " } else { "  " };
        let selector_style = if is_selected {
            Style::default()
                .fg(Color::Rgb(249, 226, 175))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Rgb(108, 112, 134))
        };
        lines.push(Line::from(vec![
            Span::styled(selector, selector_style),
            // Spacer to align label with subagent rows (icon + space).
            Span::raw("  "),
            Span::styled(
                format!("{:<20} ", "main"),
                Style::default()
                    .fg(if is_selected {
                        Color::Rgb(249, 226, 175)
                    } else {
                        Color::Rgb(180, 180, 200)
                    })
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    }

    // Subagent entries: absolute index = i + 1.
    for (i, node) in active.iter().enumerate() {
        let abs_index = i + 1;
        let (icon, icon_color) = status_icon(&node.progress.status);
        let label = &node.progress.label;
        let detail = match (&node.progress.current_tool, &node.progress.current_params) {
            (Some(tool), Some(params)) => format!("{}(\"{}\")", tool, params),
            (Some(tool), None) => tool.clone(),
            _ => "thinking...".to_string(),
        };

        let is_selected = sel == abs_index;
        let selector = if is_selected { "▶ " } else { "  " };
        let selector_style = if is_selected {
            Style::default()
                .fg(Color::Rgb(249, 226, 175))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Rgb(108, 112, 134))
        };

        lines.push(Line::from(vec![
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
        ]));
    }

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
        SubagentStatus::WaitingForChildren => ("◌", Color::Rgb(137, 180, 250)),
        SubagentStatus::Finalizing => ("◆", Color::Rgb(166, 227, 161)),
        SubagentStatus::Cancelling => ("◐", Color::Rgb(243, 139, 168)),
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

/// Collect nonterminal REAL node IDs in the order they appear
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
                .map(|node| !node.progress.status.is_terminal())
                .unwrap_or(false)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::progress::SubagentProgress;
    use crate::tui::components::subagent_tree::SubagentNode;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn test_lifecycle_status_icons() {
        assert_eq!(
            status_icon(&SubagentStatus::WaitingForChildren),
            ("◌", Color::Rgb(137, 180, 250))
        );
        assert_eq!(
            status_icon(&SubagentStatus::Finalizing),
            ("◆", Color::Rgb(166, 227, 161))
        );
        assert_eq!(
            status_icon(&SubagentStatus::Cancelling),
            ("◐", Color::Rgb(243, 139, 168))
        );
    }

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
    fn test_new_nonterminal_states_are_visible_and_navigable() {
        for status in [
            SubagentStatus::WaitingForChildren,
            SubagentStatus::Finalizing,
            SubagentStatus::Cancelling,
        ] {
            let mut tree = SubagentTree::default();
            tree.nodes
                .insert("active".to_string(), make_node("active", status.clone()));
            tree.root_id = Some("active".to_string());

            assert_eq!(active_node_ids(&tree), vec!["active".to_string()]);

            let backend = TestBackend::new(80, 4);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal
                .draw(|frame| render(frame, frame.area(), &tree, 1, true))
                .unwrap();
            let rendered = terminal.backend().to_string();

            assert!(
                rendered.contains("Task active"),
                "{status:?} must render in the status bar"
            );
        }
    }

    #[test]
    fn test_terminal_states_are_not_navigable_or_rendered() {
        for status in [
            SubagentStatus::Completed,
            SubagentStatus::Failed,
            SubagentStatus::Cancelled,
        ] {
            let mut tree = SubagentTree::default();
            tree.nodes.insert(
                "terminal".to_string(),
                make_node("terminal", status.clone()),
            );
            tree.root_id = Some("terminal".to_string());

            assert!(active_node_ids(&tree).is_empty());

            let backend = TestBackend::new(80, 4);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal
                .draw(|frame| render(frame, frame.area(), &tree, 0, false))
                .unwrap();
            let rendered = terminal.backend().to_string();

            assert!(!rendered.contains("Task terminal"));
        }
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
