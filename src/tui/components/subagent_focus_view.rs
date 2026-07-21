//! SubagentFocusView — full-screen event timeline for a subagent.
//!
//! Evolved from the old `DetailView` + `SubagentPanelState` design. The main
//! chat window stays clean; subagent progress is shown via a compact status
//! bar, and pressing Enter opens this full-screen focus view with the complete
//! event timeline.

use crate::agent::progress::SubagentStatus;
use crate::api::ChatMessage;
use crate::tui::app::{MessageRole, UIMessage};
use crate::tui::components::chat::message_to_lines;
use crate::tui::components::subagent_tree::SubagentTree;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;
use std::collections::{HashMap, HashSet};
use std::time::Instant;

/// Completed subagents are removed from the selector after this delay (seconds).
pub const COMPLETED_REMOVE_DELAY_SECS: u64 = 10;

/// Real subagent node IDs visible in the selector: grouping nodes are excluded
/// (via `real_node_list`), and completed nodes past the removal delay are
/// excluded — except the currently-viewed node, which is always kept so its
/// timeline remains accessible.
pub fn visible_node_ids(
    tree: &SubagentTree,
    completed_at: &HashMap<String, Instant>,
    now: Instant,
    current_node_id: &str,
) -> Vec<String> {
    tree.real_node_list()
        .into_iter()
        .filter(|id| {
            if id == current_node_id {
                return true;
            }
            match completed_at.get(id) {
                Some(t) => now.duration_since(*t).as_secs() < COMPLETED_REMOVE_DELAY_SECS,
                None => true,
            }
        })
        .collect()
}

/// Convert a sequence of `ChatMessage`s into `UIMessage`s for conversation-style
/// rendering in the subagent focus view.
///
/// The conversion follows a two-step algorithm:
///
/// **Step A — Build result map**: Scan messages and collect tool results keyed
/// by `tool_call_id`.
///
/// **Step B — Iterate and convert**: Scan messages again, generating `UIMessage`s.
/// Tool results that have been merged into an assistant's tool call are consumed
/// so they don't produce duplicate entries.
pub fn chat_messages_to_ui_messages(messages: &[ChatMessage]) -> Vec<UIMessage> {
    // Step A: build result map
    let mut result_map: HashMap<&str, &ChatMessage> = HashMap::new();
    for msg in messages {
        if msg.role == "tool" {
            if let Some(ref tcid) = msg.tool_call_id {
                result_map.insert(tcid.as_str(), msg);
            }
        }
    }

    // Step B: iterate and convert
    let mut consumed: HashSet<String> = HashSet::new();
    let mut ui_messages: Vec<UIMessage> = Vec::new();

    let empty_defaults = || UIMessage {
        role: MessageRole::User,
        content: String::new(),
        tool_name: None,
        tool_args: None,
        tool_collapsed: false,
        content_collapsed: false,
        tool_running: false,
        diff_data: None,
        tool_metadata: None,
    };

    for msg in messages {
        match msg.role.as_str() {
            "system" => {} // skip
            "user" => {
                ui_messages.push(UIMessage {
                    role: MessageRole::User,
                    content: msg.content.clone().unwrap_or_default(),
                    ..empty_defaults()
                });
            }
            "assistant" => {
                if let Some(ref content) = msg.content {
                    if !content.is_empty() {
                        ui_messages.push(UIMessage {
                            role: MessageRole::Assistant,
                            content: content.clone(),
                            ..empty_defaults()
                        });
                    }
                }

                if let Some(ref tool_calls) = msg.tool_calls {
                    for tc in tool_calls {
                        let args: serde_json::Value = serde_json::from_str(&tc.function.arguments)
                            .unwrap_or(serde_json::Value::Null);

                        let tool_metadata =
                            Some(serde_json::json!({"tool_call_id": tc.id.clone()}));

                        if let Some(result_msg) = result_map.get(tc.id.as_str()) {
                            let result_content = result_msg.content.clone().unwrap_or_default();
                            let diff_data = crate::tui::util::extract_diff_data(
                                &tc.function.name,
                                &args,
                                &result_content,
                            );

                            consumed.insert(tc.id.clone());

                            ui_messages.push(UIMessage {
                                role: MessageRole::Tool,
                                content: result_content,
                                tool_name: Some(tc.function.name.clone()),
                                tool_args: Some(args),
                                tool_collapsed: true,
                                content_collapsed: false,
                                tool_running: false,
                                diff_data,
                                tool_metadata,
                            });
                        } else {
                            ui_messages.push(UIMessage {
                                role: MessageRole::Tool,
                                content: String::new(),
                                tool_name: Some(tc.function.name.clone()),
                                tool_args: Some(args),
                                tool_collapsed: false,
                                content_collapsed: false,
                                tool_running: true,
                                diff_data: None,
                                tool_metadata,
                            });
                        }
                    }
                }
            }
            "tool" => {
                if let Some(ref tcid) = msg.tool_call_id {
                    if !consumed.contains(tcid.as_str()) {
                        consumed.insert(tcid.clone());
                        ui_messages.push(UIMessage {
                            role: MessageRole::Tool,
                            content: msg.content.clone().unwrap_or_default(),
                            tool_name: Some(tcid.clone()),
                            tool_args: None,
                            tool_collapsed: true,
                            content_collapsed: false,
                            tool_running: false,
                            diff_data: None,
                            tool_metadata: None,
                        });
                    }
                }
            }
            _ => {} // unknown roles skipped
        }
    }

    ui_messages
}

/// State for the full-screen subagent focus view.
#[derive(Debug, Clone)]
pub struct FocusViewState {
    pub node_id: String,
    pub label: String,
    pub messages: Vec<ChatMessage>,
    pub collapsed_tool_ids: HashSet<String>,
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
    pub selector_index: usize,
}

impl FocusViewState {
    /// Build a `FocusViewState` from a node in the tree.
    /// Returns `None` if the node doesn't exist. The selector cursor is
    /// initialized to align with `node_id` (D2): `real_node_list` position +1
    /// (the +1 accounts for the "main" entry at index 0).
    pub fn build(node_id: &str, tree: &SubagentTree) -> Option<Self> {
        let node = tree.nodes.get(node_id)?;
        let p = &node.progress;
        let pos = tree.real_node_list().iter().position(|id| id == node_id);
        let selector_index = pos.map(|i| i + 1).unwrap_or(0);
        Some(Self {
            node_id: node_id.to_string(),
            label: p.label.clone(),
            messages: p.messages.clone(),
            collapsed_tool_ids: HashSet::new(),
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
            selector_index,
        })
    }

    /// Rebuild cached data from the tree, preserving UI state.
    /// When `auto_scroll` is true, `scroll_offset` is reset to 0 so the
    /// timeline stays pinned to the latest events.
    pub fn rebuild(&mut self, tree: &SubagentTree) {
        if let Some(node) = tree.nodes.get(&self.node_id) {
            let p = &node.progress;
            self.label = p.label.clone();
            self.messages = p.messages.clone();
            // collapsed_tool_ids preserved — stale tool_call IDs from
            // old messages are harmless (they won't match new messages)
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

    /// Toggle fold/expand of all tool calls in the timeline.
    /// If all are collapsed (empty set), expand all; otherwise collapse all.
    /// Shared by the `t` key and `Ctrl+E` while the focus view is open.
    pub fn toggle_fold_all(&mut self) {
        let ui_msgs = chat_messages_to_ui_messages(&self.messages);
        if self.collapsed_tool_ids.is_empty() {
            for msg in &ui_msgs {
                if msg.role == MessageRole::Tool {
                    if let Some(ref meta) = msg.tool_metadata {
                        if let Some(tid) = meta.get("tool_call_id").and_then(|v| v.as_str()) {
                            self.collapsed_tool_ids.insert(tid.to_string());
                        }
                    }
                }
            }
        } else {
            self.collapsed_tool_ids.clear();
        }
    }

    /// Toggle fold/expand of the last tool call in the timeline.
    /// Bound to `Ctrl+O` while the focus view is open.
    pub fn toggle_fold_latest(&mut self) {
        let ui_msgs = chat_messages_to_ui_messages(&self.messages);
        for msg in ui_msgs.iter().rev() {
            if msg.role == MessageRole::Tool {
                if let Some(ref meta) = msg.tool_metadata {
                    if let Some(tid) = meta.get("tool_call_id").and_then(|v| v.as_str()) {
                        let tid = tid.to_string();
                        if self.collapsed_tool_ids.contains(&tid) {
                            self.collapsed_tool_ids.remove(&tid);
                        } else {
                            self.collapsed_tool_ids.insert(tid);
                        }
                        break;
                    }
                }
            }
        }
    }
}

/// Full-screen focus view renderer.
pub struct FocusView;

impl FocusView {
    /// Render the focus view full-screen.
    pub fn render(
        f: &mut Frame,
        area: Rect,
        state: &FocusViewState,
        tree: &SubagentTree,
        completed_at: &HashMap<String, Instant>,
        now: Instant,
        spinner_frame: u8,
    ) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(8), // header
                Constraint::Min(5),    // timeline
                Constraint::Length(8), // selector
                Constraint::Length(1), // help
            ])
            .split(area);

        let active_border = Style::default().fg(Color::Rgb(249, 226, 175));
        let inactive_border = Style::default().fg(Color::Rgb(80, 80, 100));

        // ── Header ────────────────────────────────────────────────────
        let header_border = inactive_border;
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
        let timeline_border = inactive_border;
        let timeline_block = Block::default()
            .title(" Conversation ")
            .borders(Borders::ALL)
            .border_style(timeline_border)
            .style(Style::default().bg(Color::Rgb(26, 26, 46)));
        let timeline_inner = timeline_block.inner(chunks[1]);
        f.render_widget(timeline_block, chunks[1]);

        let timeline_lines = build_conversation_lines(state, timeline_inner, spinner_frame);
        f.render_widget(Paragraph::new(timeline_lines), timeline_inner);

        // ── Selector ──────────────────────────────────────────────────
        let selector_border = active_border;
        let selector_block = Block::default()
            .title(" Subagents ")
            .borders(Borders::ALL)
            .border_style(selector_border)
            .style(Style::default().bg(Color::Rgb(26, 26, 46)));
        let selector_inner = selector_block.inner(chunks[2]);
        f.render_widget(selector_block, chunks[2]);

        let selector_lines = build_selector_lines(state, tree, completed_at, now, selector_inner);
        f.render_widget(Paragraph::new(selector_lines), selector_inner);

        // ── Help bar ──────────────────────────────────────────────────
        let total = state.messages.len();
        let scroll_info = if total > 0 {
            if state.scroll_offset == 0 {
                "(latest)".to_string()
            } else {
                format!("({}\u{2191})", state.scroll_offset)
            }
        } else {
            "(no events)".to_string()
        };
        let help_text = format!(
            " \u{2191}\u{2193} navigate  Enter switch/exit  t fold  Esc back  wheel scroll timeline  {}",
            scroll_info
        );
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
        SubagentStatus::WaitingForChildren => ("WAITING", Color::Rgb(137, 180, 250)),
        SubagentStatus::Finalizing => ("FINALIZING", Color::Rgb(166, 227, 161)),
        SubagentStatus::Cancelling => ("CANCELLING", Color::Rgb(243, 139, 168)),
    }
}

/// Compute the start index for displaying `available` events from `len` total,
/// where `scroll_offset` is lines-from-bottom (0 = newest, higher = older).
/// Clamps so the window never runs past the oldest event.
fn timeline_start_index(len: usize, available: usize, scroll_offset: usize) -> usize {
    let max_start = len.saturating_sub(available);
    max_start.saturating_sub(scroll_offset.min(max_start))
}

fn build_conversation_lines(
    state: &FocusViewState,
    inner: Rect,
    spinner_frame: u8,
) -> Vec<Line<'static>> {
    let mut lines: Vec<Line> = Vec::new();

    if state.messages.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            "Waiting for subagent…",
            Style::default().fg(Color::Rgb(108, 112, 134)),
        )]));
        return lines;
    }

    let ui_messages = chat_messages_to_ui_messages(&state.messages);
    let total = ui_messages.len();

    for (idx, ui_msg) in ui_messages.iter().enumerate() {
        let show_tool_expand_hint = idx == total - 1;

        let mut msg = ui_msg.clone();

        // Apply fold override from collapsed_tool_ids
        if msg.role == MessageRole::Tool {
            let tool_id = msg
                .tool_metadata
                .as_ref()
                .and_then(|m| m.get("tool_call_id"))
                .and_then(|v| v.as_str());
            if let Some(tid) = tool_id {
                // Not in set = collapsed (default); in set = expanded
                msg.tool_collapsed = !state.collapsed_tool_ids.contains(tid);
            }
        }

        lines.extend(message_to_lines(
            &msg,
            inner.width,
            spinner_frame,
            show_tool_expand_hint,
        ));
    }

    // Scroll: use existing timeline_start_index for lines-from-bottom
    let available = inner.height as usize;
    let total_lines = lines.len();
    let start = timeline_start_index(total_lines, available, state.scroll_offset);
    if start > 0 {
        lines = lines.into_iter().skip(start).collect();
    }

    lines
}

fn build_selector_lines(
    state: &FocusViewState,
    tree: &SubagentTree,
    completed_at: &HashMap<String, Instant>,
    now: Instant,
    inner: Rect,
) -> Vec<Line<'static>> {
    let visible = visible_node_ids(tree, completed_at, now, &state.node_id);
    // Unified list: ["main", ...visible]. selector_index 0 = main, i+1 = visible[i].
    let total_len = 1 + visible.len();
    let available = inner.height as usize;
    let avail = available.min(total_len);

    // Sliding window: keep selector_index visible. Pure function of
    // selector_index + list length + avail; not stored in state.
    let mut scroll_start = 0usize;
    if state.selector_index < scroll_start {
        scroll_start = state.selector_index;
    }
    if state.selector_index >= scroll_start + avail && avail > 0 {
        scroll_start = state.selector_index.saturating_sub(avail) + 1;
    }
    let max_start = total_len.saturating_sub(avail);
    if scroll_start > max_start {
        scroll_start = max_start;
    }

    let mut lines: Vec<Line<'static>> = Vec::new();
    let cursor_color = Color::Rgb(249, 226, 175);

    // "main" entry at absolute index 0 — only render when inside the window.
    if avail > 0 && scroll_start == 0 {
        let is_main_selected = state.selector_index == 0;
        let selector = if is_main_selected { "▶ " } else { "  " };
        lines.push(Line::from(vec![
            Span::styled(selector, Style::default().fg(cursor_color)),
            Span::styled(
                "main",
                Style::default()
                    .fg(Color::Rgb(180, 180, 200))
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    }

    // Subagent entries: absolute index = i + 1 (visible[i]).
    for (i, node_id) in visible.iter().enumerate() {
        let abs_index = i + 1;
        if abs_index < scroll_start || abs_index >= scroll_start + avail {
            continue;
        }
        let is_current = node_id == &state.node_id;
        let is_selected = abs_index == state.selector_index;
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
        let current_marker = if is_current { " ●" } else { "" };

        // Dim completed-but-not-yet-removed nodes; highlight current/selected.
        let is_completed = node
            .map(|n| {
                matches!(
                    n.progress.status,
                    SubagentStatus::Completed | SubagentStatus::Failed | SubagentStatus::Cancelled
                )
            })
            .unwrap_or(false);
        let label_color = if is_current {
            Color::Rgb(249, 226, 175)
        } else if is_selected {
            Color::Rgb(137, 180, 250)
        } else if is_completed {
            Color::Rgb(90, 90, 110) // dim gray for completed
        } else {
            Color::Rgb(180, 180, 200)
        };

        let max_w = inner.width.saturating_sub(8) as usize;
        let display = truncate(&label, max_w);

        lines.push(Line::from(vec![
            Span::styled(selector, Style::default().fg(cursor_color)),
            Span::styled(format!("{} ", icon), Style::default().fg(icon_color)),
            Span::styled(
                display,
                Style::default()
                    .fg(label_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(current_marker, Style::default().fg(cursor_color)),
        ]));
    }

    lines
}

fn selector_status_icon(status: &SubagentStatus) -> (&'static str, Color) {
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
    use crate::agent::progress::{SubagentEvent, SubagentEventType, SubagentProgress};
    use crate::tui::components::subagent_tree::SubagentNode;

    #[test]
    fn test_lifecycle_status_display_helpers() {
        assert_eq!(
            status_display(&SubagentStatus::WaitingForChildren),
            ("WAITING", Color::Rgb(137, 180, 250))
        );
        assert_eq!(
            status_display(&SubagentStatus::Finalizing),
            ("FINALIZING", Color::Rgb(166, 227, 161))
        );
        assert_eq!(
            status_display(&SubagentStatus::Cancelling),
            ("CANCELLING", Color::Rgb(243, 139, 168))
        );

        assert_eq!(
            selector_status_icon(&SubagentStatus::WaitingForChildren),
            ("◌", Color::Rgb(137, 180, 250))
        );
        assert_eq!(
            selector_status_icon(&SubagentStatus::Finalizing),
            ("◆", Color::Rgb(166, 227, 161))
        );
        assert_eq!(
            selector_status_icon(&SubagentStatus::Cancelling),
            ("◐", Color::Rgb(243, 139, 168))
        );
    }

    #[test]
    fn test_timeline_start_index_newest() {
        // 10 events, 5 visible, scroll=0 → start at 5 (show events[5..10], newest 5)
        assert_eq!(timeline_start_index(10, 5, 0), 5);
    }

    #[test]
    fn test_timeline_start_index_scrolled_to_older() {
        // scroll=3 (lines from bottom) → start at 2 (show events[2..7])
        assert_eq!(timeline_start_index(10, 5, 3), 2);
    }

    #[test]
    fn test_timeline_start_index_oldest() {
        // scroll=5 = max_start → start at 0 (oldest 5)
        assert_eq!(timeline_start_index(10, 5, 5), 0);
    }

    #[test]
    fn test_timeline_start_index_clamps_overscroll() {
        // scroll beyond max_start clamps to oldest (start=0)
        assert_eq!(timeline_start_index(10, 5, 100), 0);
    }

    #[test]
    fn test_timeline_start_index_fewer_events_than_viewport() {
        // len < available → start at 0 (show all)
        assert_eq!(timeline_start_index(3, 5, 0), 0);
    }

    #[test]
    fn test_timeline_start_index_empty() {
        assert_eq!(timeline_start_index(0, 5, 0), 0);
    }

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
                messages: vec![],
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
        // messages come from SubagentProgress.messages, not from events
        assert!(state.messages.is_empty());
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
        // n1 is the only real node (pos 0) → selector_index 1 (main is index 0)
        assert_eq!(state.selector_index, 1);
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
        // Simulate user interaction: scrolled and moved the selector cursor
        state.auto_scroll = false;
        state.scroll_offset = 3;
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
            ..Default::default()
        });
        tree.nodes.insert("n1".to_string(), node);
        tree.root_id = Some("n1".to_string());

        let state = FocusViewState::build("n1", &tree).unwrap();
        assert_eq!(state.status, SubagentStatus::Failed);
        assert_eq!(state.error_message.as_deref(), Some("timed out after 30s"));
    }

    #[test]
    fn test_rebuild_preserves_collapsed_tool_ids() {
        let mut tree = SubagentTree::default();
        tree.nodes.insert("n1".to_string(), make_node("n1", vec![]));
        tree.root_id = Some("n1".to_string());

        let mut state = FocusViewState::build("n1", &tree).unwrap();
        state.collapsed_tool_ids.insert("tc-xyz".to_string());

        // Rebuild — collapse state preserved
        state.rebuild(&tree);
        assert!(state.collapsed_tool_ids.contains("tc-xyz"));
    }

    // ── toggle_fold_all / toggle_fold_latest tests ─────────────────
    use crate::api::{ToolCall, ToolCallFunction};

    fn make_messages_with_two_tool_calls() -> Vec<ChatMessage> {
        let tc1 = ToolCall {
            id: "1".to_string(),
            r#type: "function".to_string(),
            function: ToolCallFunction {
                name: "grep".to_string(),
                arguments: "{}".to_string(),
            },
        };
        let tc2 = ToolCall {
            id: "2".to_string(),
            r#type: "function".to_string(),
            function: ToolCallFunction {
                name: "ls".to_string(),
                arguments: "{}".to_string(),
            },
        };
        vec![
            ChatMessage {
                role: "assistant".to_string(),
                content: None,
                reasoning_content: None,
                tool_calls: Some(vec![tc1, tc2]),
                tool_call_id: None,
            },
            ChatMessage::tool("1", "r1"),
            ChatMessage::tool("2", "r2"),
        ]
    }

    fn build_state_for_fold_tests() -> FocusViewState {
        let mut tree = SubagentTree::default();
        tree.nodes.insert("n1".to_string(), make_node("n1", vec![]));
        tree.root_id = Some("n1".to_string());
        let mut state = FocusViewState::build("n1", &tree).unwrap();
        state.messages = make_messages_with_two_tool_calls();
        state
    }

    #[test]
    fn test_toggle_fold_all_expands_then_collapses() {
        let mut state = build_state_for_fold_tests();

        // Empty set = all collapsed. toggle_fold_all expands all (Ctrl+E / 't').
        assert!(state.collapsed_tool_ids.is_empty());
        state.toggle_fold_all();
        assert_eq!(state.collapsed_tool_ids.len(), 2);
        assert!(state.collapsed_tool_ids.contains("1"));
        assert!(state.collapsed_tool_ids.contains("2"));

        // Non-empty = some expanded. toggle_fold_all collapses all.
        state.toggle_fold_all();
        assert!(state.collapsed_tool_ids.is_empty());
    }

    #[test]
    fn test_toggle_fold_latest_flips_only_last_tool_call() {
        let mut state = build_state_for_fold_tests();

        // toggle_fold_latest (Ctrl+O) flips only the last tool call ("2").
        assert!(state.collapsed_tool_ids.is_empty());
        state.toggle_fold_latest();
        assert_eq!(state.collapsed_tool_ids.len(), 1);
        assert!(state.collapsed_tool_ids.contains("2"));
        assert!(!state.collapsed_tool_ids.contains("1"));

        // Flipping again collapses "2" back; "1" never touched.
        state.toggle_fold_latest();
        assert!(state.collapsed_tool_ids.is_empty());
    }

    // ── chat_messages_to_ui_messages tests ──────────────────────────

    #[test]
    fn test_convert_user_message() {
        let messages = vec![ChatMessage::user("hello")];
        let result = chat_messages_to_ui_messages(&messages);
        assert_eq!(result.len(), 1);
        let ui = &result[0];
        assert!(matches!(ui.role, MessageRole::User));
        assert_eq!(ui.content, "hello");
        assert!(ui.tool_name.is_none());
        assert!(!ui.tool_collapsed);
        assert!(!ui.tool_running);
    }

    #[test]
    fn test_convert_assistant_text() {
        let messages = vec![ChatMessage::assistant("thinking")];
        let result = chat_messages_to_ui_messages(&messages);
        assert_eq!(result.len(), 1);
        let ui = &result[0];
        assert!(matches!(ui.role, MessageRole::Assistant));
        assert_eq!(ui.content, "thinking");
        assert!(ui.tool_name.is_none());
        assert!(!ui.tool_running);
    }

    #[test]
    fn test_convert_assistant_with_tool_calls_merged() {
        let tc = ToolCall {
            id: "1".to_string(),
            r#type: "function".to_string(),
            function: ToolCallFunction {
                name: "grep".to_string(),
                arguments: r#"{"pattern":"x"}"#.to_string(),
            },
        };
        let assistant_msg = ChatMessage {
            role: "assistant".to_string(),
            content: None,
            reasoning_content: None,
            tool_calls: Some(vec![tc]),
            tool_call_id: None,
        };
        let tool_result_msg = ChatMessage::tool("1", "Found 3 matches");
        let messages = vec![assistant_msg, tool_result_msg];
        let result = chat_messages_to_ui_messages(&messages);
        assert_eq!(
            result.len(),
            1,
            "should produce exactly 1 merged Tool UIMessage"
        );
        let ui = &result[0];
        assert!(matches!(ui.role, MessageRole::Tool));
        assert_eq!(ui.tool_name.as_deref(), Some("grep"));
        assert_eq!(ui.content, "Found 3 matches");
        assert!(!ui.tool_running);
        assert!(ui.tool_collapsed);
        assert!(ui.tool_args.is_some());
        let args = ui.tool_args.as_ref().unwrap();
        assert_eq!(args["pattern"], "x");
    }

    #[test]
    fn test_convert_tool_call_without_result() {
        let tc = ToolCall {
            id: "1".to_string(),
            r#type: "function".to_string(),
            function: ToolCallFunction {
                name: "grep".to_string(),
                arguments: r#"{"pattern":"x"}"#.to_string(),
            },
        };
        let assistant_msg = ChatMessage {
            role: "assistant".to_string(),
            content: None,
            reasoning_content: None,
            tool_calls: Some(vec![tc]),
            tool_call_id: None,
        };
        // No matching tool result message.
        let messages = vec![assistant_msg];
        let result = chat_messages_to_ui_messages(&messages);
        assert_eq!(result.len(), 1, "should produce 1 running Tool UIMessage");
        let ui = &result[0];
        assert!(matches!(ui.role, MessageRole::Tool));
        assert_eq!(ui.tool_name.as_deref(), Some("grep"));
        assert!(ui.tool_running);
        assert!(!ui.tool_collapsed);
        assert_eq!(ui.content, "");
        assert!(ui.diff_data.is_none());
        // tool_metadata should contain tool_call_id for fold tracking.
        let meta = ui.tool_metadata.as_ref().unwrap();
        assert_eq!(meta["tool_call_id"], "1");
    }

    #[test]
    fn test_convert_orphan_tool_result() {
        let tool_msg = ChatMessage::tool("orphan", "result content");
        let messages = vec![tool_msg];
        let result = chat_messages_to_ui_messages(&messages);
        assert_eq!(result.len(), 1);
        let ui = &result[0];
        assert!(matches!(ui.role, MessageRole::Tool));
        assert_eq!(ui.tool_name.as_deref(), Some("orphan"));
        assert_eq!(ui.content, "result content");
        assert!(ui.tool_collapsed);
        assert!(!ui.tool_running);
    }

    #[test]
    fn test_skip_system_message() {
        let messages = vec![ChatMessage::system("prompt")];
        let result = chat_messages_to_ui_messages(&messages);
        assert_eq!(result.len(), 0, "system messages should be skipped");
    }

    fn make_real_tree() -> SubagentTree {
        // root "a" has events (so it's NOT a grouping node) + children b, c
        let mut tree = SubagentTree::default();
        let mut node_a = make_node("a", vec![make_event("a-event")]);
        node_a.children = vec!["b".to_string(), "c".to_string()];
        tree.nodes.insert("a".to_string(), node_a);
        tree.nodes.insert("b".to_string(), make_node("b", vec![]));
        tree.nodes.insert("c".to_string(), make_node("c", vec![]));
        tree.root_id = Some("a".to_string());
        tree
    }

    #[test]
    fn test_build_selector_index_aligns_with_current_node() {
        let tree = make_real_tree();
        // real_node_list = [a, b, c]; opening "c" (pos 2) → selector_index 3
        let state_c = FocusViewState::build("c", &tree).unwrap();
        assert_eq!(state_c.selector_index, 3);
        // opening "a" (pos 0) → selector_index 1 (main is index 0)
        let state_a = FocusViewState::build("a", &tree).unwrap();
        assert_eq!(state_a.selector_index, 1);
    }

    #[test]
    fn test_visible_node_ids_filters_completed_after_delay() {
        use std::time::{Duration, Instant};
        let tree = make_real_tree();
        let now = Instant::now();
        let mut completed_at = HashMap::new();
        // b completed 20s ago — past the 10s removal delay
        completed_at.insert("b".to_string(), now - Duration::from_secs(20));
        // current node = a → b removed
        let visible = visible_node_ids(&tree, &completed_at, now, "a");
        assert!(visible.contains(&"a".to_string()));
        assert!(!visible.contains(&"b".to_string()));
        assert!(visible.contains(&"c".to_string()));
        // current node is exempt even if completed+past delay
        let visible_cur = visible_node_ids(&tree, &completed_at, now, "b");
        assert!(visible_cur.contains(&"b".to_string()));
    }

    #[test]
    fn test_build_selector_lines_no_overflow() {
        use std::time::Instant;
        // 10 real subagents, available height 4 → lines must not exceed 4
        let mut tree = SubagentTree::default();
        let mut root = make_node("root", vec![make_event("r")]);
        root.children = (0..10).map(|i| format!("s{}", i)).collect();
        tree.nodes.insert("root".to_string(), root);
        for i in 0..10 {
            tree.nodes
                .insert(format!("s{}", i), make_node(&format!("s{}", i), vec![]));
        }
        tree.root_id = Some("root".to_string());
        let state = FocusViewState::build("s0", &tree).unwrap();
        let inner = Rect::new(0, 0, 80, 4);
        let now = Instant::now();
        let completed_at = HashMap::new();
        let lines = build_selector_lines(&state, &tree, &completed_at, now, inner);
        assert!(
            lines.len() <= 4,
            "lines ({}) must not exceed available height (4)",
            lines.len()
        );
    }

    #[test]
    fn test_build_selector_lines_keeps_cursor_visible() {
        use std::time::Instant;
        // 10 real subagents, available height 4, cursor on the LAST one.
        // The sliding window must keep the cursor (▶) visible.
        let mut tree = SubagentTree::default();
        let mut root = make_node("root", vec![make_event("r")]);
        root.children = (0..10).map(|i| format!("s{}", i)).collect();
        tree.nodes.insert("root".to_string(), root);
        for i in 0..10 {
            tree.nodes
                .insert(format!("s{}", i), make_node(&format!("s{}", i), vec![]));
        }
        tree.root_id = Some("root".to_string());
        // real_node_list = [root, s0..s9]; opening "s9" (pos 10) → selector_index 11
        let mut state = FocusViewState::build("s9", &tree).unwrap();
        // Force cursor onto the last entry (main + 11 real = 12 entries, index 11)
        state.selector_index = 11;
        let inner = Rect::new(0, 0, 80, 4);
        let now = Instant::now();
        let completed_at = HashMap::new();
        let lines = build_selector_lines(&state, &tree, &completed_at, now, inner);
        // The cursor marker ▶ must appear in the rendered window (cursor visible)
        let cursor_visible = lines.iter().any(|line| {
            line.spans
                .iter()
                .any(|span| span.content.as_ref().starts_with("▶"))
        });
        assert!(
            cursor_visible,
            "cursor (▶) must be visible when selector_index is at the end"
        );
    }
}
