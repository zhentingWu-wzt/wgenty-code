use crate::agent::progress::SubagentStatus;
use crate::tui::app::{MessageRole, UIMessage};
use crate::tui::components::diff;
use crate::tui::components::subagent_tree::SubagentTree;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

const USER_COLOR: Color = Color::Rgb(255, 140, 66);
const ASSISTANT_COLOR: Color = Color::Rgb(147, 112, 219);
const TEXT_COLOR: Color = Color::Rgb(220, 220, 230);
const DIM_COLOR: Color = Color::Rgb(150, 150, 165);
const TURN_SEP_COLOR: Color = Color::Rgb(110, 110, 125);
const SEP_COLOR: Color = Color::Rgb(85, 85, 100);

/// Braille spinner animation frames (10 frames)
const SPINNER_CHARS: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Status indicator icon.
fn status_icon(status: &SubagentStatus) -> &'static str {
    match status {
        SubagentStatus::Pending => "⏳",
        SubagentStatus::Running => "🔄",
        SubagentStatus::Completed => "✅",
        SubagentStatus::Failed => "❌",
        SubagentStatus::Cancelled => "🚫",
    }
}

/// Render subagent tree as lines. Shared between inline card and panel.
pub fn render_subagent_card(
    lines: &mut Vec<Line>,
    tree: &SubagentTree,
    _width: u16,
    is_executing: bool,
    spinner_frame: u8,
) {
    if tree.nodes.is_empty() {
        return;
    }
    let done = tree.count_by_status(SubagentStatus::Completed);
    let total = tree.nodes.len();
    let indent = 4u16;

    let spinner = if is_executing {
        SPINNER_CHARS[(spinner_frame as usize) % SPINNER_CHARS.len()]
    } else {
        ' '
    };

    // Header
    lines.push(Line::from(vec![
        Span::styled(
            "  🌳 Subagent Tree",
            Style::default()
                .fg(Color::Rgb(203, 166, 247))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {} {}/{} done", spinner, done, total),
            Style::default().fg(DIM_COLOR),
        ),
    ]));

    if is_executing {
        render_tree_nodes(lines, tree, tree.root_id.as_deref(), 0, indent);
    } else {
        let failed = tree.count_by_status(SubagentStatus::Failed);
        let icon = if failed > 0 { "⚠️" } else { "✅" };
        lines.push(Line::from(vec![Span::styled(
            format!("    {} task · {}/{} done", icon, done, total),
            Style::default().fg(DIM_COLOR),
        )]));
    }
}

/// Recursively render tree nodes.
fn render_tree_nodes(
    lines: &mut Vec<Line>,
    tree: &SubagentTree,
    node_id: Option<&str>,
    depth: u16,
    base_indent: u16,
) {
    let Some(nid) = node_id else { return };
    let Some(node) = tree.nodes.get(nid) else {
        return;
    };
    let indent = base_indent + depth * 2;
    let prefix = if depth == 0 { "┌─" } else { "├─" };
    let indent_str = " ".repeat(indent as usize);
    let icon = status_icon(&node.progress.status);

    let color = match node.progress.status {
        SubagentStatus::Running => Color::Rgb(249, 226, 175),
        SubagentStatus::Completed => Color::Rgb(166, 227, 161),
        SubagentStatus::Failed | SubagentStatus::Cancelled => Color::Rgb(243, 139, 168),
        SubagentStatus::Pending => Color::Rgb(108, 112, 134),
    };

    // ── Build status line with timing ────────────────────────────────────
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
            // Token count
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

    let detail = if status_detail.is_empty() {
        format!(" {}", node.progress.label)
    } else {
        format!(" {} — {}", node.progress.label, status_detail)
    };

    lines.push(Line::from(vec![
        Span::styled(
            format!("{}{} ", indent_str, prefix),
            Style::default().fg(DIM_COLOR),
        ),
        Span::styled(icon, Style::default().fg(color)),
        Span::styled(format!(" {}", detail), Style::default().fg(color)),
    ]));

    // ── Running node: show current tool with params ──────────────────────
    if node.progress.status == SubagentStatus::Running {
        let tool_indent = " ".repeat((indent + 4) as usize);
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
                format!("{}└─ 🛠 {}", tool_indent, tool_label),
                Style::default().fg(Color::Rgb(137, 180, 250)),
            )]));
        }

        // ── Text snapshot: model's "thinking" ────────────────────────────
        let snapshot_indent = " ".repeat((indent + 4) as usize);
        if let Some(ref snapshot) = node.progress.text_snapshot {
            let preview: String = snapshot.chars().take(100).collect();
            let display = if snapshot.len() > 100 {
                format!("{}…", preview)
            } else {
                preview
            };
            lines.push(Line::from(vec![Span::styled(
                format!("{}   💬 {}", snapshot_indent, display),
                Style::default().fg(DIM_COLOR),
            )]));
        } else {
            lines.push(Line::from(vec![Span::styled(
                format!("{}   💭 thinking…", snapshot_indent),
                Style::default().fg(DIM_COLOR),
            )]));
        }

        // ── Recent action log (collapsed: last 3 Action events) ────────────
        let recent: Vec<_> = node
            .progress
            .action_log
            .iter()
            .filter(|e| {
                matches!(
                    e.event_type,
                    crate::agent::progress::SubagentEventType::Action { .. }
                )
            })
            .rev()
            .take(3)
            .collect();
        if !recent.is_empty() {
            for event in recent.iter().rev() {
                if let crate::agent::progress::SubagentEventType::Action {
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
                        format!("{}   ▸ {}", snapshot_indent, action_str),
                        Style::default().fg(Color::Rgb(108, 112, 134)),
                    )]));
                }
            }
        }
    }

    for child_id in &node.children {
        render_tree_nodes(lines, tree, Some(child_id), depth + 1, base_indent);
    }
}

/// Return animated ellipsis dots based on frame
fn running_suffix(frame: u8) -> &'static str {
    match (frame / 2) % 4 {
        0 => "running",
        1 => "running.",
        2 => "running..",
        _ => "running...",
    }
}

/// Render the chat message list with turn-based grouping.
/// Messages are grouped into turns: each user message starts a new turn,
/// followed by the assistant's streaming response and any tool calls.
/// A gray separator line is drawn between turns.
pub fn render(
    f: &mut Frame,
    area: Rect,
    committed_messages: &[UIMessage],
    streaming_content: &str,
    streaming_active: bool,
    scroll_offset: u16,
    user_scrolled: bool,
    spinner_frame: u8,
    subagent_tree: Option<&SubagentTree>,
    subagent_is_executing: bool,
) {
    let mut lines: Vec<Line> = Vec::new();

    let mut prev_role: Option<MessageRole> = None;
    for msg in committed_messages.iter() {
        if msg.role == MessageRole::User {
            add_turn_separator(&mut lines, area.width);
        } else if matches!(prev_role, Some(MessageRole::Tool)) && msg.role == MessageRole::Assistant
        {
            add_inline_separator(&mut lines, area.width);
        }
        lines.extend(message_to_lines(msg, area.width, spinner_frame));
        if msg.role == MessageRole::Tool {
            let tool_name = msg.tool_name.as_deref().unwrap_or("");
            if tool_name == "task" || tool_name == "delegate" {
                if let Some(tree) = subagent_tree {
                    if !tree.nodes.is_empty() {
                        render_subagent_card(
                            &mut lines,
                            tree,
                            area.width,
                            subagent_is_executing,
                            spinner_frame,
                        );
                    }
                }
            }
        }
        if msg.role == MessageRole::User {
            add_inline_separator(&mut lines, area.width);
        }
        prev_role = Some(msg.role.clone());
    }

    if let Some(last) = committed_messages.last() {
        if last.role != MessageRole::User {
            add_turn_separator(&mut lines, area.width);
        }
    }

    // Streaming assistant content: continues the current turn without a separator
    if streaming_active && !streaming_content.is_empty() {
        let wrap_w = area.width.saturating_sub(4) as usize;
        for line in streaming_content.lines() {
            push_wrapped(&mut lines, line, "  ", TEXT_COLOR, TEXT_COLOR, wrap_w + 2);
        }
    }

    let total_lines = lines.len() as u16;
    let viewport = area.height;

    // Auto-scroll: always show newest content (bottom).
    // Manual scroll: `scroll_offset` counts lines scrolled UP from bottom:
    //   0 = at bottom (newest), max = at top (oldest).
    // This convention ensures that transitioning from auto→manual never
    // jumps — at auto-scroll bottom, scroll_offset starts at 0, which
    // renders as bottom, not top.
    let max_scroll = total_lines.saturating_sub(viewport);
    let actual_scroll = if user_scrolled {
        max_scroll.saturating_sub(scroll_offset.min(max_scroll))
    } else {
        max_scroll
    };

    let para = Paragraph::new(Text::from(lines)).scroll((actual_scroll, 0));
    f.render_widget(para, area);
}

/// Draw a full-width separator line between turns.
fn add_turn_separator(lines: &mut Vec<Line<'static>>, width: u16) {
    let w = width.saturating_sub(2) as usize;
    if w > 0 {
        let bar = "\u{2500}".repeat(w);
        lines.push(Line::from(Span::styled(
            format!("  {}", bar),
            Style::default().fg(TURN_SEP_COLOR),
        )));
    }
}

/// Draw a subtle dotted separator between user input and assistant response within a turn.
fn add_inline_separator(lines: &mut Vec<Line<'static>>, width: u16) {
    let w = width.saturating_sub(2) as usize;
    if w == 0 {
        return;
    }
    let count = w / 3;
    if count > 0 {
        let bar = " \u{2500}".repeat(count);
        lines.push(Line::from(Span::styled(
            format!("  {}", bar),
            Style::default().fg(SEP_COLOR),
        )));
    }
}

fn message_to_lines(msg: &UIMessage, width: u16, spinner_frame: u8) -> Vec<Line<'static>> {
    let max_w = width.saturating_sub(4) as usize;

    match msg.role {
        MessageRole::User => {
            let mut lines = Vec::new();
            lines.push(Line::from(Span::styled(
                "\u{203a} You",
                Style::default().fg(USER_COLOR).add_modifier(Modifier::BOLD),
            )));
            for line in msg.content.lines() {
                push_wrapped(
                    &mut lines,
                    line,
                    "  ",
                    Color::White,
                    Color::White,
                    max_w + 2,
                );
            }
            lines.push(Line::raw(""));
            lines
        }
        MessageRole::Assistant => {
            let mut lines = Vec::new();
            if msg.content.is_empty() {
                lines.push(Line::from(Span::styled(
                    "   ",
                    Style::default().fg(ASSISTANT_COLOR),
                )));
            } else if msg.content_collapsed {
                render_collapsed(&mut lines, &msg.content, "  ", ASSISTANT_COLOR, max_w + 2);
            } else {
                for line in msg.content.lines() {
                    push_wrapped(&mut lines, line, "  ", TEXT_COLOR, TEXT_COLOR, max_w + 2);
                }
            }
            lines.push(Line::raw(""));
            lines
        }
        MessageRole::Tool => {
            if msg.content.is_empty() && !msg.tool_collapsed {
                // Empty expanded tool result — nothing to show
                Vec::new()
            } else if msg.tool_collapsed || msg.content.is_empty() {
                // ToolResult: codex-style tree display
                let name = msg.tool_name.as_deref().unwrap_or("Tool").to_string();
                let verb = tool_verb(&name).to_string();
                let detail = msg
                    .tool_args
                    .as_ref()
                    .map(|a| tool_label(&name, a))
                    .filter(|s| !s.is_empty())
                    .map(|s| format!(": {}", s))
                    .unwrap_or_default();
                let mut lines: Vec<Line<'static>> = Vec::new();

                // Header: • {verb} [mode] {detail} — or spinner {verb} [mode] {detail} while running
                let is_running = msg.content.is_empty();
                let execution_mode = msg
                    .tool_metadata
                    .as_ref()
                    .and_then(|m| m.get("execution_mode"))
                    .and_then(|v| v.as_str());

                // Mode tag: [RLM], [BG], or nothing for simple
                let mode_tag = match execution_mode {
                    Some("rlm") => " [RLM]",
                    Some("background") | Some("bg") => " [BG]",
                    _ => "",
                };

                let (prefix, verb_style) = if is_running {
                    let spinner = SPINNER_CHARS[spinner_frame as usize % SPINNER_CHARS.len()];
                    (
                        format!("{} ", spinner),
                        Style::default()
                            .fg(Color::Rgb(200, 200, 100))
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    (
                        "\u{2022} ".to_string(),
                        Style::default().fg(TEXT_COLOR).add_modifier(Modifier::BOLD),
                    )
                };
                let verb_with_mode = format!("{}{}", verb, mode_tag);
                lines.push(Line::from(vec![
                    Span::styled(prefix, Style::default().fg(DIM_COLOR)),
                    Span::styled(verb_with_mode.clone(), verb_style),
                    Span::styled(format!(" {}", detail), Style::default().fg(DIM_COLOR)),
                    if is_running {
                        Span::styled(
                            format!(" {}", running_suffix(spinner_frame)),
                            Style::default().fg(Color::Rgb(180, 180, 100)),
                        )
                    } else {
                        Span::styled("", Style::default())
                    },
                ]));

                // Routing reason for task/delegate tools
                if !is_running {
                    if let Some(reason) = msg
                        .tool_metadata
                        .as_ref()
                        .and_then(|m| m.get("routing_reason"))
                        .and_then(|v| v.as_str())
                    {
                        lines.push(Line::from(vec![Span::styled(
                            format!("   ⓘ {}", reason),
                            Style::default().fg(Color::Rgb(80, 80, 100)),
                        )]));
                    }
                }

                // If diff data is available, render it inline after the header
                if let Some(ref diff) = msg.diff_data {
                    let diff_lines = diff::diff_to_lines(
                        &diff.file_path,
                        &diff.old_content,
                        &diff.new_content,
                        width,
                    );
                    lines.extend(diff_lines);
                    return lines;
                }

                // Body lines: indented
                let content_lines: Vec<&str> = msg.content.lines().collect();
                let total = content_lines.len();
                let show: Vec<&str> = if msg.tool_collapsed {
                    Vec::new()
                } else {
                    content_lines
                        .iter()
                        .take(MAX_TOOL_EXPANDED_LINES)
                        .copied()
                        .collect()
                };
                let wrap_width = width.saturating_sub(4) as usize;
                for line in &show {
                    if line.is_empty() {
                        lines.push(Line::from(Span::styled(
                            "  ",
                            Style::default().fg(DIM_COLOR),
                        )));
                    } else {
                        push_wrapped(&mut lines, line, "  ", DIM_COLOR, DIM_COLOR, wrap_width + 2);
                    }
                }
                if total > show.len() {
                    lines.push(Line::from(vec![Span::styled(
                        format!(
                            "  {} +{} lines (Enter to expand)",
                            '\u{2026}',
                            total - show.len()
                        ),
                        Style::default().fg(DIM_COLOR),
                    )]));
                }
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
        MessageRole::System => msg
            .content
            .lines()
            .map(|line| {
                Line::from(Span::styled(
                    line.to_string(),
                    Style::default().fg(DIM_COLOR),
                ))
            })
            .collect(),
    }
}

/// Max lines to show in expanded tool result before truncating.
const MAX_TOOL_EXPANDED_LINES: usize = 100;

/// Map tool name to a human-readable action verb.
fn tool_verb(name: &str) -> &str {
    match name {
        "exec_command" | "execute_command" => "Ran",
        "file_read" | "read_file" => "Read",
        "file_write" => "Wrote",
        "file_edit" => "Edited",
        "apply_patch" => "Patched",
        "grep" | "search" => "Searched",
        "glob_search" | "glob" | "list_files" => "Listed",
        "web_search" => "Searched web",
        "web_fetch" => "Fetched",
        "view" => "Viewed",
        "task" => "Subagent",
        "delegate" => "Delegated",
        "TodoWrite" => "Planned",
        "compact" => "Compacted",
        _ => "Used",
    }
}

/// Extract a human-readable label from tool args (e.g., command string, file path).
fn tool_label(name: &str, args: &serde_json::Value) -> String {
    match name {
        "exec_command" | "execute_command" => args
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        "file_read" | "read_file" | "file_write" | "file_edit" | "apply_patch" | "view" => args
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        "grep" | "search" => args
            .get("pattern")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        "glob_search" | "glob" | "list_files" => args
            .get("path")
            .or_else(|| args.get("pattern"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        "web_search" => args
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        "web_fetch" => args
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        "task" => {
            let desc = args
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let sub_type = args
                .get("subagent_type")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if sub_type.is_empty() {
                desc.to_string()
            } else {
                format!("[{}] {}", sub_type, desc)
            }
        }
        _ => String::new(),
    }
}

// Diff rendering is handled by `crate::tui::components::diff`.
// Use `diff::diff_to_lines()` for inline chat diff, or
// `diff::render()` for standalone diff views.

/// Render a collapsed paragraph: first 3 lines + "... (N lines total, collapsed)" indicator.
fn render_collapsed(
    lines_buf: &mut Vec<Line<'static>>,
    content: &str,
    prefix: &str,
    prefix_color: Color,
    max_w: usize,
) {
    let total_lines = content.lines().count();
    for line in content.lines().take(3) {
        push_wrapped(lines_buf, line, prefix, prefix_color, DIM_COLOR, max_w);
    }
    let indicator = format!("   ... ({} lines total, collapsed)", total_lines);
    lines_buf.push(Line::from(Span::styled(
        indicator,
        Style::default().fg(DIM_COLOR),
    )));
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
