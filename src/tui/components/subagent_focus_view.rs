//! SubagentFocusView — full-screen event timeline for a subagent.
//!
//! Evolved from the old `DetailView` + `SubagentPanelState` design. The main
//! chat window stays clean; subagent progress is shown via a compact status
//! bar, and pressing Enter opens this full-screen focus view with the complete
//! event timeline.

use crate::agent::progress::{SubagentEvent, SubagentEventType, SubagentStatus};
use crate::tui::components::subagent_tree::SubagentTree;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

/// Which area of the focus view is currently focused (for keyboard input).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusArea {
    Timeline,
    Selector,
}

/// State for the full-screen subagent focus view.
#[derive(Debug, Clone)]
pub struct FocusViewState {
    pub node_id: String,
    pub label: String,
    pub events: Vec<SubagentEvent>,
    pub status: SubagentStatus,
    pub elapsed_ms: u64,
    pub cumulative_tokens: u64,
    pub token_budget_k: Option<u64>,
    pub round: Option<usize>,
    pub max_rounds: Option<usize>,
    pub error_message: Option<String>,
    pub current_tool: Option<String>,
    pub current_params: Option<String>,
    pub scroll_offset: usize,
    pub auto_scroll: bool,
    pub active_area: FocusArea,
    pub selector_index: usize,
}

impl FocusViewState {
    /// Build a `FocusViewState` from a node in the tree.
    /// Returns `None` if the node doesn't exist.
    pub fn build(node_id: &str, tree: &SubagentTree) -> Option<Self> {
        let node = tree.nodes.get(node_id)?;
        let p = &node.progress;
        Some(Self {
            node_id: node_id.to_string(),
            label: p.label.clone(),
            events: p.events.clone(),
            status: p.status.clone(),
            elapsed_ms: p.elapsed_ms,
            cumulative_tokens: p.cumulative_tokens,
            token_budget_k: p.token_budget_k,
            round: p.round,
            max_rounds: p.max_rounds,
            error_message: p.error_details.as_ref().map(|e| e.message.clone()),
            current_tool: p.current_tool.clone(),
            current_params: p.current_params.clone(),
            scroll_offset: 0,
            auto_scroll: true,
            active_area: FocusArea::Timeline,
            selector_index: 0,
        })
    }

    /// Rebuild cached data from the tree, preserving UI state.
    /// When `auto_scroll` is true, `scroll_offset` is reset to 0 so the
    /// timeline stays pinned to the latest events.
    pub fn rebuild(&mut self, tree: &SubagentTree) {
        if let Some(node) = tree.nodes.get(&self.node_id) {
            let p = &node.progress;
            self.label = p.label.clone();
            self.events = p.events.clone();
            self.status = p.status.clone();
            self.elapsed_ms = p.elapsed_ms;
            self.cumulative_tokens = p.cumulative_tokens;
            self.current_tool = p.current_tool.clone();
            self.current_params = p.current_params.clone();
            self.error_message = p.error_details.as_ref().map(|e| e.message.clone());
            self.round = p.round;
            self.max_rounds = p.max_rounds;
            if self.auto_scroll {
                self.scroll_offset = 0;
            }
        }
    }
}

/// Full-screen focus view renderer.
pub struct FocusView;

impl FocusView {
    /// Render the focus view full-screen.
    pub fn render(f: &mut Frame, area: Rect, state: &FocusViewState, tree: &SubagentTree) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(8), // header
                Constraint::Min(5),    // timeline
                Constraint::Length(6), // selector
                Constraint::Length(1), // help
            ])
            .split(area);

        let active_border = Style::default().fg(Color::Rgb(249, 226, 175));
        let inactive_border = Style::default().fg(Color::Rgb(80, 80, 100));

        // ── Header ────────────────────────────────────────────────────
        let header_border = if state.active_area == FocusArea::Timeline {
            active_border
        } else {
            inactive_border
        };
        let header_block = Block::default()
            .title(" Subagent Focus ")
            .borders(Borders::ALL)
            .border_style(header_border)
            .style(Style::default().bg(Color::Rgb(26, 26, 46)));
        let header_inner = header_block.inner(chunks[0]);
        f.render_widget(header_block, chunks[0]);

        let (status_label, status_color) = status_display(&state.status);
        let total_secs = state.elapsed_ms as f64 / 1000.0;
        let budget_str = state
            .token_budget_k
            .map(|b| format!("{}k", b))
            .unwrap_or_else(|| "unlimited".to_string());

        let mut header_lines: Vec<Line> = vec![
            Line::from(vec![
                Span::styled(" Task:    ", Style::default().fg(Color::Rgb(108, 112, 134))),
                Span::styled(
                    state.label.clone(),
                    Style::default()
                        .fg(Color::Rgb(249, 226, 175))
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled(" Status:  ", Style::default().fg(Color::Rgb(108, 112, 134))),
                Span::styled(
                    status_label,
                    Style::default()
                        .fg(status_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(
                    format!("{:.1}s", total_secs),
                    Style::default().fg(Color::Rgb(180, 180, 200)),
                ),
                Span::raw("  "),
                Span::styled(
                    format!("{}/{} tokens", state.cumulative_tokens, budget_str),
                    Style::default().fg(Color::Rgb(180, 180, 200)),
                ),
            ]),
        ];

        if let (Some(r), Some(mr)) = (state.round, state.max_rounds) {
            header_lines.push(Line::from(vec![
                Span::styled(" Rounds:  ", Style::default().fg(Color::Rgb(108, 112, 134))),
                Span::styled(
                    format!("{}/{}", r, mr),
                    Style::default().fg(Color::Rgb(180, 180, 200)),
                ),
            ]));
        }

        if let Some(ref err) = state.error_message {
            header_lines.push(Line::from(vec![
                Span::styled(" Error:   ", Style::default().fg(Color::Rgb(108, 112, 134))),
                Span::styled(
                    err,
                    Style::default()
                        .fg(Color::Rgb(243, 139, 168))
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
        }

        f.render_widget(Paragraph::new(header_lines), header_inner);

        // ── Timeline ──────────────────────────────────────────────────
        let timeline_border = if state.active_area == FocusArea::Timeline {
            active_border
        } else {
            inactive_border
        };
        let timeline_block = Block::default()
            .title(" Event Timeline ")
            .borders(Borders::ALL)
            .border_style(timeline_border)
            .style(Style::default().bg(Color::Rgb(26, 26, 46)));
        let timeline_inner = timeline_block.inner(chunks[1]);
        f.render_widget(timeline_block, chunks[1]);

        let timeline_lines = build_timeline_lines(state, timeline_inner);
        f.render_widget(Paragraph::new(timeline_lines), timeline_inner);

        // ── Selector ──────────────────────────────────────────────────
        let selector_border = if state.active_area == FocusArea::Selector {
            active_border
        } else {
            inactive_border
        };
        let selector_block = Block::default()
            .title(" Subagents ")
            .borders(Borders::ALL)
            .border_style(selector_border)
            .style(Style::default().bg(Color::Rgb(26, 26, 46)));
        let selector_inner = selector_block.inner(chunks[2]);
        f.render_widget(selector_block, chunks[2]);

        let selector_lines = build_selector_lines(state, tree, selector_inner);
        f.render_widget(Paragraph::new(selector_lines), selector_inner);

        // ── Help bar ──────────────────────────────────────────────────
        let total = state.events.len();
        let scroll_info = if total > 0 {
            format!("({}/{})", state.scroll_offset + 1, total)
        } else {
            "(no events)".to_string()
        };
        let help_text = match state.active_area {
            FocusArea::Timeline => format!(
                " \u{2191}\u{2193} scroll  Tab selector  Esc back  {}",
                scroll_info
            ),
            FocusArea::Selector => {
                " \u{2191}\u{2193} navigate  Enter switch  Tab timeline  Esc back".to_string()
            }
        };
        f.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                help_text,
                Style::default().fg(Color::Rgb(108, 112, 134)),
            )])),
            chunks[3],
        );
    }
}

fn status_display(status: &SubagentStatus) -> (&'static str, Color) {
    match status {
        SubagentStatus::Completed => ("COMPLETED", Color::Rgb(166, 227, 161)),
        SubagentStatus::Failed => ("FAILED", Color::Rgb(243, 139, 168)),
        SubagentStatus::Cancelled => ("CANCELLED", Color::Rgb(243, 139, 168)),
        SubagentStatus::Running => ("RUNNING", Color::Rgb(249, 226, 175)),
        SubagentStatus::Pending => ("PENDING", Color::Rgb(108, 112, 134)),
    }
}

fn build_timeline_lines(state: &FocusViewState, inner: Rect) -> Vec<Line<'static>> {
    let mut lines: Vec<Line> = Vec::new();
    let available = inner.height as usize;
    let scroll = state.scroll_offset;

    let visible_events: Vec<&SubagentEvent> =
        state.events.iter().skip(scroll).take(available).collect();

    if visible_events.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            "No events recorded.",
            Style::default().fg(Color::Rgb(108, 112, 134)),
        )]));
        return lines;
    }

    for event in &visible_events {
        let elapsed = format!("+{:.1}s", event.elapsed_ms as f64 / 1000.0);
        match &event.event_type {
            SubagentEventType::Thought { text } => {
                let max_w = inner.width.saturating_sub(12) as usize;
                let display = truncate(text, max_w);
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
                let display = truncate(&action_str, max_w);
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
                let (icon, color) = if *success {
                    ("OK", Color::Rgb(166, 227, 161))
                } else {
                    ("FAIL", Color::Rgb(243, 139, 168))
                };
                let max_w = inner.width.saturating_sub(12) as usize;
                let display = truncate(summary, max_w);
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
                let display = truncate(message, max_w);
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
                let status_display_str = match status.as_str() {
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
                let display = truncate(sum, max_w);
                lines.push(Line::from(vec![
                    Span::styled(
                        format!(" {:<8} ", elapsed),
                        Style::default().fg(Color::Rgb(108, 112, 134)),
                    ),
                    Span::styled(
                        format!(" {}  ", status_display_str),
                        Style::default().fg(color).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(display, Style::default().fg(color)),
                ]));
            }
        }
    }

    lines
}

fn build_selector_lines(
    state: &FocusViewState,
    tree: &SubagentTree,
    inner: Rect,
) -> Vec<Line<'static>> {
    let node_ids = tree.node_list();
    let available = inner.height as usize;
    let scroll = 0usize; // selector scrolls are simple for now

    node_ids
        .iter()
        .skip(scroll)
        .take(available)
        .enumerate()
        .map(|(i, node_id)| {
            let is_current = node_id == &state.node_id;
            let is_selected = i + scroll == state.selector_index;
            let node = tree.nodes.get(node_id);
            let (icon, icon_color) = if let Some(n) = node {
                selector_status_icon(&n.progress.status)
            } else {
                ("?", Color::Rgb(108, 112, 134))
            };
            let label = node
                .map(|n| n.progress.label.clone())
                .unwrap_or_else(|| node_id.clone());

            let selector = if is_selected { "▶ " } else { "  " };
            let current_marker = if is_current { " ●" } else { "  " };
            let label_color = if is_current {
                Color::Rgb(249, 226, 175)
            } else if is_selected {
                Color::Rgb(137, 180, 250)
            } else {
                Color::Rgb(180, 180, 200)
            };
            let max_w = inner.width.saturating_sub(8) as usize;
            let display = truncate(&label, max_w);

            Line::from(vec![
                Span::styled(selector, Style::default().fg(Color::Rgb(249, 226, 175))),
                Span::styled(format!("{} ", icon), Style::default().fg(icon_color)),
                Span::styled(
                    display,
                    Style::default()
                        .fg(label_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    current_marker,
                    Style::default().fg(Color::Rgb(249, 226, 175)),
                ),
            ])
        })
        .collect()
}

fn selector_status_icon(status: &SubagentStatus) -> (&'static str, Color) {
    match status {
        SubagentStatus::Running => ("⟳", Color::Rgb(137, 180, 250)),
        SubagentStatus::Pending => ("○", Color::Rgb(108, 112, 134)),
        SubagentStatus::Completed => ("✓", Color::Rgb(166, 227, 161)),
        SubagentStatus::Failed => ("✗", Color::Rgb(243, 139, 168)),
        SubagentStatus::Cancelled => ("⊘", Color::Rgb(243, 139, 168)),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max.saturating_sub(3)).collect();
        format!("{}...", truncated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::progress::{SubagentEventType, SubagentProgress};
    use crate::tui::components::subagent_tree::SubagentNode;

    fn make_node(node_id: &str, events: Vec<SubagentEvent>) -> SubagentNode {
        SubagentNode {
            progress: SubagentProgress {
                node_id: node_id.to_string(),
                parent_id: None,
                label: format!("Node {}", node_id),
                status: SubagentStatus::Running,
                round: Some(2),
                max_rounds: Some(5),
                current_tool: Some("file_read".to_string()),
                current_params: Some("src/main.rs".to_string()),
                action_log: vec![],
                text_snapshot: None,
                started_at: 0,
                elapsed_ms: 1500,
                metadata: None,
                progress_delta: None,
                token_budget_k: Some(10),
                cumulative_tokens: 500,
                error_details: None,
                events,
            },
            children: vec![],
        }
    }

    fn make_event(text: &str) -> SubagentEvent {
        SubagentEvent {
            event_type: SubagentEventType::Thought {
                text: text.to_string(),
            },
            elapsed_ms: 100,
        }
    }

    #[test]
    fn test_build_from_node() {
        let mut tree = SubagentTree::default();
        let events = vec![make_event("hello"), make_event("world")];
        tree.nodes
            .insert("n1".to_string(), make_node("n1", events.clone()));
        tree.root_id = Some("n1".to_string());

        let state = FocusViewState::build("n1", &tree).expect("should build from existing node");
        assert_eq!(state.node_id, "n1");
        assert_eq!(state.label, "Node n1");
        assert_eq!(state.events.len(), 2);
        assert_eq!(state.status, SubagentStatus::Running);
        assert_eq!(state.elapsed_ms, 1500);
        assert_eq!(state.cumulative_tokens, 500);
        assert_eq!(state.token_budget_k, Some(10));
        assert_eq!(state.round, Some(2));
        assert_eq!(state.max_rounds, Some(5));
        assert_eq!(state.current_tool.as_deref(), Some("file_read"));
        assert_eq!(state.current_params.as_deref(), Some("src/main.rs"));
        assert!(state.error_message.is_none());
        assert_eq!(state.scroll_offset, 0);
        assert!(state.auto_scroll);
        assert_eq!(state.active_area, FocusArea::Timeline);
        assert_eq!(state.selector_index, 0);
    }

    #[test]
    fn test_build_missing_node_returns_none() {
        let tree = SubagentTree::default();
        assert!(FocusViewState::build("nonexistent", &tree).is_none());
    }

    #[test]
    fn test_rebuild_preserves_ui_state() {
        let mut tree = SubagentTree::default();
        tree.nodes
            .insert("n1".to_string(), make_node("n1", vec![make_event("a")]));
        tree.root_id = Some("n1".to_string());

        let mut state = FocusViewState::build("n1", &tree).unwrap();
        // Simulate user interaction: scrolled and switched to selector
        state.auto_scroll = false;
        state.scroll_offset = 3;
        state.active_area = FocusArea::Selector;
        state.selector_index = 1;

        // Update tree data
        tree.nodes.get_mut("n1").unwrap().progress.elapsed_ms = 3000;
        tree.nodes.get_mut("n1").unwrap().progress.cumulative_tokens = 999;

        state.rebuild(&tree);

        // Data refreshed
        assert_eq!(state.elapsed_ms, 3000);
        assert_eq!(state.cumulative_tokens, 999);
        // UI state preserved when auto_scroll is false
        assert_eq!(state.scroll_offset, 3);
        assert_eq!(state.active_area, FocusArea::Selector);
        assert_eq!(state.selector_index, 1);
    }

    #[test]
    fn test_rebuild_auto_scroll_resets() {
        let mut tree = SubagentTree::default();
        tree.nodes
            .insert("n1".to_string(), make_node("n1", vec![make_event("a")]));
        tree.root_id = Some("n1".to_string());

        let mut state = FocusViewState::build("n1", &tree).unwrap();
        // auto_scroll is true by default; set a non-zero scroll_offset
        state.scroll_offset = 5;
        assert!(state.auto_scroll);

        state.rebuild(&tree);

        // scroll_offset reset to 0 because auto_scroll is true
        assert_eq!(state.scroll_offset, 0);
    }

    #[test]
    fn test_rebuild_missing_node_noop() {
        let mut tree = SubagentTree::default();
        tree.nodes
            .insert("n1".to_string(), make_node("n1", vec![make_event("a")]));
        tree.root_id = Some("n1".to_string());

        let mut state = FocusViewState::build("n1", &tree).unwrap();
        // Point to a node that doesn't exist
        state.node_id = "ghost".to_string();
        let original_elapsed = state.elapsed_ms;

        state.rebuild(&tree);

        // Nothing changed
        assert_eq!(state.elapsed_ms, original_elapsed);
    }

    #[test]
    fn test_build_with_error_details() {
        use crate::agent::progress::{ErrorInfo, ErrorType};

        let mut tree = SubagentTree::default();
        let mut node = make_node("n1", vec![]);
        node.progress.status = SubagentStatus::Failed;
        node.progress.error_details = Some(ErrorInfo {
            error_type: ErrorType::Timeout,
            message: "timed out after 30s".to_string(),
            last_tool: None,
            last_params: None,
            round: 3,
            retryable: true,
        });
        tree.nodes.insert("n1".to_string(), node);
        tree.root_id = Some("n1".to_string());

        let state = FocusViewState::build("n1", &tree).unwrap();
        assert_eq!(state.status, SubagentStatus::Failed);
        assert_eq!(state.error_message.as_deref(), Some("timed out after 30s"));
    }
}
