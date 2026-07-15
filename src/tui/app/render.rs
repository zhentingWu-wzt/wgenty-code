//! Rendering methods for the TUI application.

use super::App;
use crate::mcp::codegraph::CodegraphInstallState;
use crate::tui::components;
use crate::tui::theme;
use crate::tui::util::centered_rect;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

impl App {
    pub(super) fn render(&mut self, f: &mut Frame) {
        let area = f.area();

        // Full-screen focus view takes over the entire terminal.
        if let Some(ref focus) = self.subagent_focus {
            components::subagent_focus_view::FocusView::render(
                f,
                area,
                focus,
                &self.subagent_tree,
                &self.completed_at,
                std::time::Instant::now(),
                self.spinner_frame,
            );
            return;
        }

        // Layout: chat | [panel] | status | [subagent_status_bar] | pending | input
        let has_question = self.question_state.visible;
        let has_permission = self.permission_state.visible;
        let has_plan = self.plan_panel_state.visible;
        let show_panel = has_question || has_permission || has_plan;
        let panel_height = if has_question {
            self.question_state.height_needed()
        } else if has_permission {
            self.permission_state.height_needed()
        } else if has_plan {
            self.plan_panel_state.height_needed()
        } else {
            0
        };
        let pending_height = self.pending_count().min(5).try_into().unwrap_or(5);
        let has_pending = pending_height > 0;
        // Status bar height must account for the Block's TOP border (1 row):
        // active_count lines of content + 1 border line. Without the +1 the
        // border consumes the only allocated row and 1 active subagent renders
        // 0 visible lines (the last item is always clipped). See spec
        // subagent-status-display: "minimum height needed to display all active
        // subagents (capped at 5 lines)".
        let active_count = self.subagent_tree.active_count();
        let has_status_bar = active_count > 0;
        let status_bar_height = status_bar_layout_height(active_count);
        let constraints: Vec<Constraint> = if show_panel {
            vec![
                Constraint::Min(3),
                Constraint::Length(panel_height),
                Constraint::Length(1),
                Constraint::Length(if has_status_bar { status_bar_height } else { 0 }),
                Constraint::Length(if has_pending { pending_height } else { 0 }),
                Constraint::Length(
                    (self.input_box.textarea.lines().len() + 3)
                        .clamp(6, 16)
                        .try_into()
                        .unwrap_or(16),
                ),
            ]
        } else {
            vec![
                Constraint::Min(3),
                Constraint::Length(1),
                Constraint::Length(if has_status_bar { status_bar_height } else { 0 }),
                Constraint::Length(if has_pending { pending_height } else { 0 }),
                Constraint::Length(
                    (self.input_box.textarea.lines().len() + 3)
                        .clamp(6, 16)
                        .try_into()
                        .unwrap_or(16),
                ),
            ]
        };
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(area);
        let chat_idx = 0;
        let panel_idx = if show_panel { 1 } else { 0 };
        let status_idx = if show_panel { 2 } else { 1 };
        let status_bar_idx = if show_panel { 3 } else { 2 };
        let pending_idx = if show_panel { 4 } else { 3 };
        let input_idx = if show_panel { 5 } else { 4 };
        let main_area = if self.task_panel.visible {
            let split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
                .split(layout[chat_idx]);
            components::task_panel::render(f, split[1], &self.task_panel);
            split[0]
        } else {
            layout[chat_idx]
        };
        // The welcome banner shows until the first message is committed to
        // the chat. Any message — including System messages produced by
        // slash commands like /help or /plan — should suppress the banner
        // so the user can see the output.
        let has_real_turn = !self.committed_messages.is_empty();
        if !has_real_turn && !self.streaming_active {
            let model_name = {
                let s = self.settings_lock.read().expect("lock poisoned: settings");
                crate::config::ApiConfig::default().get_model_id(&s.models.main.name)
            };
            components::welcome::render(f, main_area, &model_name);
        } else {
            self.render_chat(f, main_area);
        }
        // Inline question / permission panel
        if self.question_state.visible {
            components::question::render(f, layout[panel_idx], &self.question_state);
        } else if self.permission_state.visible {
            components::permission::render(f, layout[panel_idx], &self.permission_state);
        } else if self.plan_panel_state.visible {
            components::plan_panel::render(f, &self.plan_panel_state, layout[panel_idx]);
        }
        self.render_status(f, layout[status_idx]);
        if has_status_bar {
            components::subagent_status_bar::render(
                f,
                layout[status_bar_idx],
                &self.subagent_tree,
                self.subagent_status_bar_selected,
                self.subagent_status_bar_focused,
            );
        }
        if has_pending {
            self.render_pending_inputs(f, layout[pending_idx]);
        }
        self.render_input(f, layout[input_idx]);
        // Completion panel is an overlay above the input and must render after the input box.
        if let Some(ref completion) = self.completion_state {
            if completion.visible && !completion.matches.is_empty() {
                components::completion_panel::CompletionPanel::render(
                    f,
                    layout[input_idx],
                    completion,
                );
            }
        }
        // Session is still a popup overlay
        components::session::render(f, &self.session_state, centered_rect);
    }

    fn render_chat(&self, f: &mut Frame, area: Rect) {
        components::chat::render(
            f,
            area,
            &self.committed_messages,
            &self.streaming_content,
            self.streaming_active,
            self.scroll_offset,
            self.user_scrolled,
            self.spinner_frame,
        );
    }

    fn render_status(&self, f: &mut Frame, area: Rect) {
        let elapsed = self.turn_started_at.map(|t| t.elapsed().as_secs());
        let input_tokens = self.token_counter.turn_input_tokens();
        let output_tokens = self.token_counter.turn_output_tokens();
        let mode = self.mode.label();
        components::status::render(
            f,
            area,
            &self.phase,
            self.spinner_frame,
            elapsed,
            input_tokens,
            output_tokens,
            mode,
            Some(&self.subagent_tree),
        );
    }

    fn render_input(&mut self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(area);
        self.render_mode_label(f, chunks[1]);
        self.input_box.render(f, chunks[0], Some(self.mode.color()));
    }

    /// Render the agent mode label at the top-left of the input area.
    fn render_mode_label(&self, f: &mut Frame, area: Rect) {
        let color = self.mode.color();
        let label = format!(" {} ", self.mode.label());

        let mode_span = Span::styled(label, Style::default().fg(color));

        let mut spans: Vec<Span<'_>> = vec![mode_span];

        // Shift+Tab mode switch hint (always visible, dim style)
        spans.push(Span::styled(
            " Shift+Tab 切换模式",
            Style::default().fg(theme::DIM),
        ));

        // Only show context bar + CodeGraph status when the terminal is wide enough
        if area.width >= 40 {
            let used = self.token_counter.last_prompt_tokens();
            let max = self
                .settings_lock
                .read()
                .expect("lock poisoned: settings")
                .models
                .context_window;
            spans.push(Span::raw(" "));
            spans.extend(components::context_bar::spans(used, max));
        }

        // CodeGraph MCP connection status indicator (right-aligned area)
        if area.width >= 60 {
            spans.push(Span::raw(" "));
            spans.push(codegraph_status_span(&self.codegraph_status));
        }

        let line = Line::from(spans);
        let paragraph = Paragraph::new(line).alignment(ratatui::layout::Alignment::Left);
        f.render_widget(paragraph, area);
    }

    /// Display queued user inputs waiting to be processed.
    fn render_pending_inputs(&self, f: &mut Frame, area: Rect) {
        let pending_count = self.pending_inputs.len();
        if pending_count == 0 {
            return;
        }
        let max_show = (area.height as usize).min(pending_count);
        if max_show == 0 {
            return;
        }
        let mut lines: Vec<String> = Vec::new();
        for (i, input) in self.pending_inputs.iter().enumerate().take(max_show) {
            let first_line = input.display_text.lines().next().unwrap_or("");
            let trunc = truncate_preview(first_line, 60, 57);
            lines.push(format!("  {}. {}", i + 1, trunc));
        }
        let more = if pending_count > max_show {
            format!(" ... and {} more", pending_count - max_show)
        } else {
            String::new()
        };
        let text = format!(
            "⏳ Queued ({}){}:\n{}",
            pending_count,
            more,
            lines.join("\n")
        );
        f.render_widget(
            Paragraph::new(Span::styled(text, Style::default().fg(theme::DIM))),
            area,
        );
    }
}

/// Layout height (terminal rows) for the subagent status bar.
///
/// The status bar renders N active subagents as N content lines inside a
/// `Block` with `Borders::TOP`, so the block's inner area is `height - 1`.
/// To display all N items the allocated height must be `N + 1` (items + the
/// top border). Visible items are capped at 5; returns 0 when nothing is
/// active so the bar is hidden entirely.
fn status_bar_layout_height(active_count: usize) -> u16 {
    if active_count == 0 {
        return 0;
    }
    // +1 border, +1 for "main" placeholder row, then capped subagent rows.
    (active_count.min(5) + 2) as u16
}

/// Truncate `text` for a one-line preview.
///
/// When the char count exceeds `max`, the text is cut to the first `keep`
/// chars and an ellipsis (`...`) is appended. The cut always lands on a UTF-8
/// char boundary: naive byte slicing (`&text[..keep]`) panics when the index
/// falls inside a multi-byte sequence (e.g. CJK characters).
fn truncate_preview(text: &str, max: usize, keep: usize) -> String {
    if text.chars().count() > max {
        let end = text
            .char_indices()
            .nth(keep)
            .map(|(i, _)| i)
            .unwrap_or(text.len());
        format!("{}...", &text[..end])
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_bar_height_zero_when_no_active() {
        assert_eq!(status_bar_layout_height(0), 0);
    }

    #[test]
    fn test_status_bar_height_one_active_fits_border_plus_main_plus_item() {
        // 1 active subagent: main + 1 item + 1 border = 3 rows
        assert_eq!(status_bar_layout_height(1), 3);
    }

    #[test]
    fn test_status_bar_height_three_active_shows_all() {
        // 3 active: main + 3 items + 1 border = 5 rows — all visible
        assert_eq!(status_bar_layout_height(3), 5);
    }

    #[test]
    fn test_status_bar_height_five_active_at_cap() {
        // 5 active: main + 5 items + 1 border = 7 rows — all visible at the cap
        assert_eq!(status_bar_layout_height(5), 7);
    }

    #[test]
    fn test_status_bar_height_six_active_clipped_to_cap() {
        // 6 active: main + 5 capped items + 1 border = 7 rows (6th clipped)
        assert_eq!(status_bar_layout_height(6), 7);
    }

    #[test]
    fn test_status_bar_height_many_active_still_capped() {
        assert_eq!(status_bar_layout_height(50), 7);
    }

    #[test]
    fn truncate_preview_keeps_short_text_intact() {
        assert_eq!(truncate_preview("hello", 60, 57), "hello");
    }

    #[test]
    fn truncate_preview_ascii_appends_ellipsis() {
        let long = "a".repeat(61);
        let out = truncate_preview(&long, 60, 57);
        assert_eq!(out, format!("{}...", "a".repeat(57)));
    }

    #[test]
    fn truncate_preview_multibyte_does_not_panic_on_char_boundary() {
        // Regression: the old byte-based `&text[..57]` panicked with
        // "end byte index 57 is not a char boundary; it is inside '不'".
        // Construct input whose char count exceeds 60 and whose byte index 57
        // lands inside a 3-byte CJK character: 2 ASCII bytes + CJK chars, the
        // 19th CJK char (0-indexed 18) occupies bytes 56..59.
        let mut input = String::from("ab");
        input.push_str(&"不".repeat(59)); // 2 + 59 = 61 chars > 60
        let out = truncate_preview(&input, 60, 57);
        // Cut on a char boundary -> keeps 57 chars, no panic.
        assert!(out.ends_with("..."));
        assert_eq!(out.trim_end_matches("...").chars().count(), 57);
    }
}

/// Build a styled span for the CodeGraph availability indicator.
fn codegraph_status_span(status: &CodegraphInstallState) -> Span<'static> {
    let (icon, color, label) = match status {
        CodegraphInstallState::Ready => ("●", theme::SUCCESS, "CG"),
        CodegraphInstallState::NotInstalled => ("⚠", theme::WARNING, "CG"),
        CodegraphInstallState::NotInitialized => ("⚠", theme::WARNING, "CG"),
        CodegraphInstallState::Dismissed => ("○", theme::DIM, "CG"),
    };
    Span::styled(
        format!("{} {}", icon, label),
        ratatui::style::Style::default().fg(color),
    )
}

#[cfg(test)]
mod codegraph_span_tests {
    use super::*;
    use crate::mcp::codegraph::CodegraphInstallState;

    #[test]
    fn span_ready_shows_filled_dot() {
        let s = codegraph_status_span(&CodegraphInstallState::Ready);
        assert!(s.content.contains("●"));
    }

    #[test]
    fn span_not_installed_shows_warning() {
        let s = codegraph_status_span(&CodegraphInstallState::NotInstalled);
        assert!(s.content.contains("⚠"));
    }

    #[test]
    fn span_not_initialized_shows_warning() {
        let s = codegraph_status_span(&CodegraphInstallState::NotInitialized);
        assert!(s.content.contains("⚠"));
    }

    #[test]
    fn span_dismissed_shows_dim_circle() {
        let s = codegraph_status_span(&CodegraphInstallState::Dismissed);
        assert!(s.content.contains("○"));
    }
}
