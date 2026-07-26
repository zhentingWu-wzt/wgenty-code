use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};
use std::collections::BTreeSet;

use crate::prompts::LayerSource;
use crate::tui::app::types::TurnContext;
use crate::tui::theme;
use crate::tui::traits::Component;

// ── InspectorTab ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InspectorTab {
    Layers,
    Memories,
    Messages,
    Hooks,
    Tokens,
}

impl InspectorTab {
    pub fn label(&self) -> &'static str {
        match self {
            InspectorTab::Layers => "L",
            InspectorTab::Memories => "M",
            InspectorTab::Messages => "Msg",
            InspectorTab::Hooks => "H",
            InspectorTab::Tokens => "T",
        }
    }

    pub fn full_label(&self) -> &'static str {
        match self {
            InspectorTab::Layers => "Layers",
            InspectorTab::Memories => "Memories",
            InspectorTab::Messages => "Messages",
            InspectorTab::Hooks => "Hooks",
            InspectorTab::Tokens => "Tokens",
        }
    }

    pub fn all() -> &'static [InspectorTab] {
        &[
            InspectorTab::Layers,
            InspectorTab::Memories,
            InspectorTab::Messages,
            InspectorTab::Hooks,
            InspectorTab::Tokens,
        ]
    }

    pub fn next(&self) -> Self {
        let all = Self::all();
        let idx = all.iter().position(|t| t == self).unwrap_or(0);
        all[(idx + 1) % all.len()]
    }

    pub fn prev(&self) -> Self {
        let all = Self::all();
        let idx = all.iter().position(|t| t == self).unwrap_or(0);
        all[(idx + all.len() - 1) % all.len()]
    }
}

// ── InspectorComponent ─────────────────────────────────────────────────────────

pub struct InspectorComponent {
    pub visible: bool,
    pub was_visible_before_focus: bool,
    pub active_tab: InspectorTab,
    pub selected_turn: usize,
    pub expanded_layers: BTreeSet<usize>,
    pub scroll_offset: u16,
    /// Cached snapshot of the currently selected turn for rendering.
    pub current_context: Option<TurnContext>,
    /// Total number of turns for footer display.
    pub total_turns: usize,
}

impl Default for InspectorComponent {
    fn default() -> Self {
        Self::new()
    }
}

impl InspectorComponent {
    pub fn new() -> Self {
        Self {
            visible: false,
            was_visible_before_focus: false,
            active_tab: InspectorTab::Layers,
            selected_turn: 0,
            expanded_layers: BTreeSet::new(),
            scroll_offset: 0,
            current_context: None,
            total_turns: 0,
        }
    }

    /// Refresh the cached snapshot from the live turn-context ring buffer.
    pub fn sync(&mut self, turn_contexts: &[TurnContext]) {
        self.total_turns = turn_contexts.len();
        // Clamp selected_turn to valid range
        if !turn_contexts.is_empty() {
            if self.selected_turn >= turn_contexts.len() {
                self.selected_turn = turn_contexts.len() - 1;
            }
            self.current_context = turn_contexts.get(self.selected_turn).cloned();
        } else {
            self.selected_turn = 0;
            self.current_context = None;
        }
    }

    /// Convenience accessor for the cached turn.
    fn ctx(&self) -> Option<&TurnContext> {
        self.current_context.as_ref()
    }
}

// ── Component impl ─────────────────────────────────────────────────────────────

impl Component for InspectorComponent {
    fn render(&self, f: &mut Frame, area: Rect) {
        if !self.visible {
            return;
        }
        if area.width < 40 || area.height < 5 {
            return;
        }

        let block = Block::default()
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::Gray));
        let inner = block.inner(area);
        f.render_widget(block, area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2), // header
                Constraint::Min(1),    // body
                Constraint::Length(1), // footer
            ])
            .split(inner);

        self.render_header(f, chunks[0]);
        self.render_body(f, chunks[1]);
        self.render_footer(f, chunks[2]);
    }

    fn handle_key(&mut self, key: &KeyEvent) -> bool {
        if !self.visible {
            return false;
        }

        match key.code {
            KeyCode::Tab => {
                self.active_tab = self.active_tab.next();
                self.scroll_offset = 0;
                return true;
            }
            KeyCode::BackTab => {
                self.active_tab = self.active_tab.prev();
                self.scroll_offset = 0;
                return true;
            }
            KeyCode::Up => {
                // Navigate to previous turn
                if self.selected_turn > 0 {
                    self.selected_turn -= 1;
                    self.scroll_offset = 0;
                }
                return true;
            }
            KeyCode::Down => {
                // Navigate to next turn
                if self.selected_turn + 1 < self.total_turns {
                    self.selected_turn += 1;
                    self.scroll_offset = 0;
                }
                return true;
            }
            KeyCode::Enter => {
                if self.active_tab == InspectorTab::Layers {
                    // Toggle expand/collapse for the layer at the current scroll position
                    if let Some(ctx) = self.ctx() {
                        let idx = self.scroll_offset as usize;
                        if idx < ctx.layers.len() {
                            if self.expanded_layers.contains(&idx) {
                                self.expanded_layers.remove(&idx);
                            } else {
                                self.expanded_layers.insert(idx);
                            }
                        }
                    }
                    return true;
                }
                return false;
            }
            KeyCode::Char('l') | KeyCode::Char('L') => {
                if self.active_tab == InspectorTab::Layers {
                    // Toggle expand all / collapse all
                    if let Some(ctx) = self.ctx() {
                        if self.expanded_layers.len() == ctx.layers.len() && !ctx.layers.is_empty()
                        {
                            self.expanded_layers.clear();
                        } else {
                            self.expanded_layers = (0..ctx.layers.len()).collect();
                        }
                    }
                    return true;
                }
                return false;
            }
            KeyCode::Char('j') | KeyCode::Char('J') => {
                // Scroll down
                self.scroll_offset = self.scroll_offset.saturating_add(1);
                return true;
            }
            KeyCode::Char('k') | KeyCode::Char('K') => {
                // Scroll up
                self.scroll_offset = self.scroll_offset.saturating_sub(1);
                return true;
            }
            KeyCode::PageDown => {
                self.scroll_offset = self.scroll_offset.saturating_add(10);
                return true;
            }
            KeyCode::PageUp => {
                self.scroll_offset = self.scroll_offset.saturating_sub(10);
                return true;
            }
            _ => {}
        }

        false
    }
}

// ── Header ─────────────────────────────────────────────────────────────────────

impl InspectorComponent {
    fn render_header(&self, f: &mut Frame, area: Rect) {
        let title = Span::styled(
            "Inspector [F2]",
            Style::default()
                .fg(theme::INSPECTOR_BORDER)
                .add_modifier(Modifier::BOLD),
        );

        let tab_spans: Vec<Span> = InspectorTab::all()
            .iter()
            .enumerate()
            .flat_map(|(i, tab)| {
                let mut parts: Vec<Span> = Vec::new();
                if i > 0 {
                    parts.push(Span::raw(" "));
                }
                let is_active = *tab == self.active_tab;
                let style = if is_active {
                    Style::default()
                        .fg(theme::INSPECTOR_TAB_ACTIVE)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme::INSPECTOR_TAB_DIM)
                };
                parts.push(Span::styled(tab.label(), style));
                parts.push(Span::raw(" "));
                parts
            })
            .collect();

        let mut spans: Vec<Span> = vec![title, Span::raw("  ")];
        spans.extend(tab_spans);

        let line = Line::from(spans);
        let p = Paragraph::new(line).block(Block::default());
        f.render_widget(p, area);
    }
}

// ── Body dispatcher ────────────────────────────────────────────────────────────

impl InspectorComponent {
    fn render_body(&self, f: &mut Frame, area: Rect) {
        // Inner padding: 1 column on the left
        let inner = Rect {
            x: area.x + 1,
            y: area.y,
            width: area.width.saturating_sub(1),
            height: area.height,
        };

        match self.active_tab {
            InspectorTab::Layers => self.render_layers_tab(f, inner),
            InspectorTab::Memories => self.render_memories_tab(f, inner),
            InspectorTab::Messages => self.render_messages_tab(f, inner),
            InspectorTab::Hooks => self.render_hooks_tab(f, inner),
            InspectorTab::Tokens => self.render_tokens_tab(f, inner),
        }
    }
}

// ── Layers tab ─────────────────────────────────────────────────────────────────

impl InspectorComponent {
    fn render_layers_tab(&self, f: &mut Frame, area: Rect) {
        let ctx = match self.ctx() {
            Some(c) => c,
            None => {
                let p = Paragraph::new("(no turn data)").style(Style::default().fg(theme::DIM));
                f.render_widget(p, area);
                return;
            }
        };

        if ctx.layers.is_empty() {
            let p = Paragraph::new("(no layers)").style(Style::default().fg(theme::DIM));
            f.render_widget(p, area);
            return;
        }

        // Build lines: for each layer, show header + optionally content
        let mut lines: Vec<Line> = Vec::new();
        let visible_start = self.scroll_offset as usize;

        for (i, layer) in ctx.layers.iter().enumerate() {
            // Skip layers above the scroll offset
            if i < visible_start {
                continue;
            }

            // --- Layer header line ---
            let source_style = source_color(&layer.source);
            let source_text = layer.source.to_string();
            let expanded = self.expanded_layers.contains(&i);
            let toggle = if expanded { "▼" } else { "▶" };

            let header = Line::from(vec![
                Span::styled(format!("{} ", toggle), Style::default().fg(theme::DIM)),
                Span::styled(
                    format!("Layer {}: ", i + 1),
                    Style::default().fg(theme::INFO),
                ),
                Span::styled(&layer.label, Style::default().fg(Color::White)),
                Span::raw("  ["),
                Span::styled(source_text, source_style),
                Span::raw("]"),
                Span::styled(
                    format!("  {} chars", layer.char_count),
                    Style::default().fg(theme::DIM),
                ),
            ]);
            lines.push(header);

            // --- Layer content (if expanded) ---
            if expanded {
                // Insert a blank spacer
                lines.push(Line::raw(""));
                for content_line in layer.content.lines() {
                    lines.push(Line::from(Span::styled(
                        content_line,
                        Style::default().fg(Color::Rgb(200, 200, 200)),
                    )));
                }
                // Blank line after content
                lines.push(Line::raw(""));
                // Separator
                lines.push(Line::from(Span::styled(
                    "─".repeat(area.width as usize),
                    Style::default().fg(theme::DIM),
                )));
            }

            // Stop if we've produced enough lines to fill the area
            if lines.len() >= area.height as usize {
                break;
            }
        }

        let text = Text::from(lines);
        let p = Paragraph::new(text).wrap(Wrap { trim: false });
        f.render_widget(p, area);
    }
}

// ── Memories tab ───────────────────────────────────────────────────────────────

impl InspectorComponent {
    fn render_memories_tab(&self, f: &mut Frame, area: Rect) {
        let ctx = match self.ctx() {
            Some(c) => c,
            None => {
                let p = Paragraph::new("(no turn data)").style(Style::default().fg(theme::DIM));
                f.render_widget(p, area);
                return;
            }
        };

        if ctx.memories.is_empty() {
            let p =
                Paragraph::new("(no memories this turn)").style(Style::default().fg(theme::DIM));
            f.render_widget(p, area);
            return;
        }

        let mut lines: Vec<Line> = Vec::new();
        let visible_start = self.scroll_offset as usize;

        for (i, mem) in ctx.memories.iter().enumerate() {
            if i < visible_start {
                continue;
            }

            // Scope badge
            let scope_style = match mem.scope.as_str() {
                "project" => Style::default()
                    .fg(theme::WARNING)
                    .add_modifier(Modifier::BOLD),
                "global" => Style::default()
                    .fg(theme::INFO)
                    .add_modifier(Modifier::BOLD),
                _ => Style::default().fg(theme::DIM),
            };

            // Importance bar (importance is 0.0..1.0)
            let importance = mem.importance.clamp(0.0, 1.0);
            let filled = (importance * 10.0).round() as usize;
            let bar: String = (0..10)
                .map(|j| if j < filled { '█' } else { '░' })
                .collect();
            let bar_style = if importance >= 0.7 {
                theme::INSPECTOR_IMPORTANCE_HIGH
            } else if importance >= 0.4 {
                theme::INSPECTOR_IMPORTANCE_MID
            } else {
                theme::INSPECTOR_IMPORTANCE_LOW
            };

            // Content preview (truncate to one line)
            let preview = mem
                .content_preview
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(area.width as usize - 40)
                .collect::<String>();

            // Convert bar to owned Span to avoid borrow-after-drop
            let bar_span = Span::styled(bar, Style::default().fg(bar_style));

            lines.push(Line::from(vec![
                Span::styled(format!("[{}]", mem.scope), scope_style),
                Span::raw(" "),
                bar_span,
                Span::raw(" "),
                Span::styled(
                    format!("{:.0}%", importance * 100.0),
                    Style::default().fg(bar_style),
                ),
                Span::raw("  "),
                Span::styled(preview, Style::default().fg(Color::Rgb(200, 200, 200))),
            ]));

            if lines.len() >= area.height as usize {
                break;
            }
        }

        let text = Text::from(lines);
        let p = Paragraph::new(text).wrap(Wrap { trim: false });
        f.render_widget(p, area);
    }
}

// ── Messages tab ───────────────────────────────────────────────────────────────

impl InspectorComponent {
    fn render_messages_tab(&self, f: &mut Frame, area: Rect) {
        let ctx = match self.ctx() {
            Some(c) => c,
            None => {
                let p = Paragraph::new("(no turn data)").style(Style::default().fg(theme::DIM));
                f.render_widget(p, area);
                return;
            }
        };

        if ctx.full_messages.is_empty() {
            let p = Paragraph::new("(no messages)").style(Style::default().fg(theme::DIM));
            f.render_widget(p, area);
            return;
        }

        let mut lines: Vec<Line> = Vec::new();

        for msg in &ctx.full_messages {
            // Role header
            let role_style = role_color(&msg.role);
            let role_display = msg.role.to_uppercase();
            let header = format!("─── {} ───", role_display);
            lines.push(Line::from(Span::styled(
                header,
                Style::default().fg(role_style).add_modifier(Modifier::BOLD),
            )));

            // Content (word-wrapped)
            let body = msg.content.as_deref().unwrap_or("(no content)");
            for content_line in body.lines() {
                lines.push(Line::from(Span::styled(
                    content_line,
                    Style::default().fg(Color::Rgb(210, 210, 210)),
                )));
            }

            // Reasoning content if present
            if let Some(ref reasoning) = msg.reasoning_content {
                if !reasoning.is_empty() {
                    lines.push(Line::from(Span::styled(
                        "─── REASONING ───",
                        Style::default()
                            .fg(theme::INSPECTOR_TAB_DIM)
                            .add_modifier(Modifier::ITALIC),
                    )));
                    for rc_line in reasoning.lines() {
                        lines.push(Line::from(Span::styled(
                            rc_line,
                            Style::default().fg(Color::Rgb(160, 160, 180)),
                        )));
                    }
                }
            }

            // Tool calls
            if let Some(ref tool_calls) = msg.tool_calls {
                for tc in tool_calls {
                    lines.push(Line::from(Span::styled(
                        format!("  🔧 {}", tc.function.name),
                        Style::default().fg(theme::ROLE_TOOL),
                    )));
                }
            }

            // Separator between messages
            lines.push(Line::raw(""));
        }

        // Apply scroll offset viewport clipping
        let start = self.scroll_offset as usize;
        let visible: Vec<Line> = lines
            .into_iter()
            .skip(start)
            .take(area.height as usize)
            .collect();

        let text = Text::from(visible);
        let p = Paragraph::new(text).wrap(Wrap { trim: false });
        f.render_widget(p, area);
    }
}

// ── Hooks tab ──────────────────────────────────────────────────────────────────

impl InspectorComponent {
    fn render_hooks_tab(&self, f: &mut Frame, area: Rect) {
        let ctx = match self.ctx() {
            Some(c) => c,
            None => {
                let p = Paragraph::new("(no turn data)").style(Style::default().fg(theme::DIM));
                f.render_widget(p, area);
                return;
            }
        };

        let reminder = match &ctx.reminder {
            Some(r) => r,
            None => {
                let p = Paragraph::new("(No hook injection this turn)")
                    .style(Style::default().fg(theme::DIM));
                f.render_widget(p, area);
                return;
            }
        };

        let mut lines: Vec<Line> = Vec::new();

        // to_model
        lines.push(Line::from(Span::styled(
            "─── to_model ───",
            Style::default()
                .fg(theme::ROLE_SYSTEM)
                .add_modifier(Modifier::BOLD),
        )));
        for line in reminder.to_model.lines() {
            lines.push(Line::from(Span::styled(
                line,
                Style::default().fg(Color::Rgb(210, 210, 210)),
            )));
        }

        lines.push(Line::raw(""));

        // to_transcript
        lines.push(Line::from(Span::styled(
            "─── to_transcript ───",
            Style::default()
                .fg(theme::ROLE_ASSISTANT)
                .add_modifier(Modifier::BOLD),
        )));
        let transcript = reminder.to_transcript.as_deref().unwrap_or("(none)");
        for line in transcript.lines() {
            lines.push(Line::from(Span::styled(
                line,
                Style::default().fg(Color::Rgb(210, 210, 210)),
            )));
        }

        // Apply scroll offset
        let start = self.scroll_offset as usize;
        let visible: Vec<Line> = lines
            .into_iter()
            .skip(start)
            .take(area.height as usize)
            .collect();

        let text = Text::from(visible);
        let p = Paragraph::new(text).wrap(Wrap { trim: false });
        f.render_widget(p, area);
    }
}

// ── Tokens tab ─────────────────────────────────────────────────────────────────

impl InspectorComponent {
    fn render_tokens_tab(&self, f: &mut Frame, area: Rect) {
        let ctx = match self.ctx() {
            Some(c) => c,
            None => {
                let p = Paragraph::new("(no turn data)").style(Style::default().fg(theme::DIM));
                f.render_widget(p, area);
                return;
            }
        };

        if ctx.layers.is_empty() {
            let p = Paragraph::new("(no layers)").style(Style::default().fg(theme::DIM));
            f.render_widget(p, area);
            return;
        }

        let mut lines: Vec<Line> = Vec::new();

        // Header row
        lines.push(Line::from(vec![Span::styled(
            format!("{:<30} {:>8} {:>10}", "Layer", "Chars", "Est.Tokens"),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )]));
        lines.push(Line::raw(""));

        let mut total_chars: usize = 0;

        for layer in &ctx.layers {
            let label = if layer.label.len() > 28 {
                format!("{}…", &layer.label[..27])
            } else {
                layer.label.clone()
            };
            let chars = layer.char_count;
            let est_tokens = (chars as f64 / 4.0).ceil() as u64;
            total_chars += chars;

            let row = format!("{:<30} {:>8} {:>10}", label, chars, est_tokens);
            lines.push(Line::from(Span::styled(
                row,
                Style::default().fg(Color::Rgb(200, 200, 200)),
            )));
        }

        // Total row
        let total_est_tokens = (total_chars as f64 / 4.0).ceil() as u64;
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            "─".repeat(50),
            Style::default().fg(theme::DIM),
        )));
        lines.push(Line::from(Span::styled(
            format!(
                "{:<30} {:>8} {:>10}",
                "TOTAL", total_chars, total_est_tokens
            ),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )));

        // Apply scroll offset
        let start = self.scroll_offset as usize;
        let visible: Vec<Line> = lines
            .into_iter()
            .skip(start)
            .take(area.height as usize)
            .collect();

        let text = Text::from(visible);
        let p = Paragraph::new(text).wrap(Wrap { trim: false });
        f.render_widget(p, area);
    }
}

// ── Footer ─────────────────────────────────────────────────────────────────────

impl InspectorComponent {
    fn render_footer(&self, f: &mut Frame, area: Rect) {
        let total = self.total_turns;
        let current = if total > 0 { self.selected_turn + 1 } else { 0 };
        let footer_text = format!(" Turn {}/{} ↑↓ ", current, total);

        let span = Span::styled(
            footer_text,
            Style::default()
                .fg(theme::INSPECTOR_TAB_DIM)
                .add_modifier(Modifier::DIM),
        );

        let p = Paragraph::new(Line::from(span))
            .alignment(ratatui::layout::Alignment::Right)
            .block(Block::default());
        f.render_widget(p, area);
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────────

/// Return a style for the given LayerSource variant badge.
fn source_color(source: &LayerSource) -> Style {
    match source {
        LayerSource::Builtin => Style::default().fg(theme::INSPECTOR_SOURCE_BUILTIN),
        LayerSource::ConfigFile(_) | LayerSource::ProjectFile(_) => {
            Style::default().fg(theme::INSPECTOR_SOURCE_FILE)
        }
        LayerSource::MemoryRecall { .. } => Style::default().fg(theme::INSPECTOR_SOURCE_MEMORY),
        LayerSource::SkillInventory => Style::default().fg(theme::INSPECTOR_SOURCE_SKILL),
        LayerSource::ConfigSettings | LayerSource::HookInjection => {
            Style::default().fg(theme::INSPECTOR_SOURCE_CONFIG)
        }
        LayerSource::Unknown => Style::default().fg(theme::DIM),
    }
}

/// Return a color for a chat message role.
fn role_color(role: &str) -> Color {
    match role {
        "system" => theme::ROLE_SYSTEM,
        "user" => theme::ROLE_USER,
        "assistant" => theme::ROLE_ASSISTANT,
        "tool" => theme::ROLE_TOOL,
        _ => theme::DIM,
    }
}
