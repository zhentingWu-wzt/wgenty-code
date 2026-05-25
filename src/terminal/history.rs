//! Chat history rendering component for the TUI REPL

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget},
};

/// A single entry in the chat history
pub enum HistoryEntry {
    /// User message
    User(String),
    /// Assistant message (complete)
    Assistant(String),
    /// Assistant message being streamed (content updates in place)
    AssistantStreaming(String),
    /// Tool call result
    ToolCall { name: String, success: bool },
    /// System message
    System(String),
    /// Welcome banner with model name
    Welcome { model: String },
}

/// Chat history renderer
pub struct ChatHistory {
    entries: Vec<HistoryEntry>,
    /// Vertical scroll offset from the bottom (0 = at bottom showing latest)
    scroll_offset: u16,
    /// Whether user is scrolled up (not at bottom)
    user_scrolled: bool,
}

impl ChatHistory {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            scroll_offset: 0,
            user_scrolled: false,
        }
    }

    pub fn add(&mut self, entry: HistoryEntry) {
        self.entries.push(entry);
        self.user_scrolled = false;
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.scroll_offset = 0;
        self.user_scrolled = false;
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Scroll up (view older content) by the given number of lines
    pub fn scroll_up(&mut self, amount: u16) {
        self.scroll_offset = self.scroll_offset.saturating_add(amount);
        self.user_scrolled = true;
    }

    /// Scroll down (view newer content) — decreases offset from bottom
    pub fn scroll_down(&mut self, amount: u16) {
        self.scroll_offset = self.scroll_offset.saturating_sub(amount);
        if self.scroll_offset == 0 {
            self.user_scrolled = false;
        }
    }

    /// Whether the user has scrolled up from the bottom
    pub fn is_user_scrolled(&self) -> bool {
        self.user_scrolled
    }

    /// Scroll to the very top (view oldest content)
    pub fn scroll_to_top(&mut self) {
        self.scroll_offset = u16::MAX;
        self.user_scrolled = true;
    }

    /// Scroll to the bottom (reset user scroll)
    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = 0;
        self.user_scrolled = false;
    }

    /// Get the total number of rendered lines for a given width (for scroll calculations)
    /// If width is 0, use a default width estimate
    pub fn build_lines_count(&self, width: u16) -> u16 {
        let w = if width == 0 { 80 } else { width };
        self.build_lines(w).len() as u16
    }

    /// Update the content of the last AssistantStreaming entry
    pub fn update_last_streaming(&mut self, content: &str) {
        if let Some(entry) = self.entries.last_mut() {
            if let HistoryEntry::AssistantStreaming(ref mut s) = entry {
                *s = content.to_string();
            }
        }
    }

    /// Convert the last AssistantStreaming entry to a regular Assistant entry
    pub fn finalize_last_streaming(&mut self) {
        if let Some(entry) = self.entries.last_mut() {
            match entry {
                HistoryEntry::AssistantStreaming(s) => {
                    let content = std::mem::take(s);
                    *entry = HistoryEntry::Assistant(content);
                }
                _ => {}
            }
        }
    }

    fn build_lines(&self, width: u16) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        let max_content_width = width.saturating_sub(4) as usize;

        for entry in &self.entries {
            match entry {
                HistoryEntry::User(text) => {
                    lines.push(Self::border_top_line(
                        " You ",
                        Color::Rgb(255, 180, 100),
                        max_content_width,
                    ));
                    for line in text.lines() {
                        Self::push_wrapped_line(
                            &mut lines,
                            line,
                            "│ ",
                            Color::Rgb(255, 140, 66),
                            Color::White,
                            max_content_width,
                        );
                    }
                    lines.push(Self::border_bottom_line(
                        Color::Rgb(255, 140, 66),
                        max_content_width,
                    ));
                    lines.push(Line::raw(""));
                }
                HistoryEntry::Assistant(text) | HistoryEntry::AssistantStreaming(text) => {
                    let is_streaming = matches!(entry, HistoryEntry::AssistantStreaming(_));
                    let label = if is_streaming {
                        " Wgenty ▌"
                    } else {
                        " Wgenty "
                    };
                    lines.push(Self::border_top_line(
                        label,
                        Color::Rgb(200, 150, 255),
                        max_content_width,
                    ));
                    for line in text.lines() {
                        Self::push_wrapped_line(
                            &mut lines,
                            line,
                            "│ ",
                            Color::Rgb(147, 112, 219),
                            Color::Rgb(220, 220, 230),
                            max_content_width,
                        );
                    }
                    lines.push(Self::border_bottom_line(
                        Color::Rgb(147, 112, 219),
                        max_content_width,
                    ));
                    lines.push(Line::raw(""));
                }
                HistoryEntry::ToolCall { name, success } => {
                    let icon = if *success { "✓" } else { "✗" };
                    let icon_color = if *success {
                        Color::Green
                    } else {
                        Color::Red
                    };
                    lines.push(Line::from(vec![
                        Span::styled("  🔧 ", Style::default()),
                        Span::styled(
                            format!("Tool: {}", name),
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!(" {}", icon),
                            Style::default().fg(icon_color),
                        ),
                    ]));
                }
                HistoryEntry::System(text) => {
                    lines.push(Line::from(vec![Span::styled(
                        format!("  ℹ {}", text),
                        Style::default().fg(Color::Yellow),
                    )]));
                }
                HistoryEntry::Welcome { model } => {
                    let logo_lines = [
                        "  ▄   ▄   ▄▄▄   ▄▄▄▄▄  ▄   ▄  ▄▄▄▄▄  ▄   ▄",
                        "  █   █   ███   █████  █   █  █████  █   █",
                        "  █   █  █   █  █      ██  █    █    █   █",
                        "  █ █ █  █      ███    █ █ █    █     ███ ",
                        "  █ █ █  █  ██  █      █  ██    █      █  ",
                        "   █ █    ████  █████  █   █    █      █  ",
                    ];
                    let gradient = [
                        Color::Rgb(220, 180, 255),
                        Color::Rgb(200, 160, 240),
                        Color::Rgb(170, 130, 220),
                        Color::Rgb(140, 100, 195),
                        Color::Rgb(115, 80, 170),
                        Color::Rgb(100, 60, 150),
                    ];
                    for (i, line) in logo_lines.iter().enumerate() {
                        lines.push(Line::from(Span::styled(
                            line.to_string(),
                            Style::default().fg(gradient[i]).add_modifier(Modifier::BOLD),
                        )));
                    }
                    lines.push(Line::raw(""));
                    lines.push(Line::from(vec![
                        Span::styled("        🟣 ", Style::default()),
                        Span::styled(
                            "Wgenty Code",
                            Style::default()
                                .fg(Color::Rgb(200, 150, 255))
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            " · Rust Edition",
                            Style::default()
                                .fg(Color::Rgb(255, 140, 66))
                                .add_modifier(Modifier::BOLD),
                        ),
                    ]));
                    lines.push(Line::from(Span::styled(
                        "           高性能 AI 编码助手",
                        Style::default().fg(Color::Rgb(147, 112, 219)),
                    )));
                    lines.push(Line::raw(""));
                    lines.push(Line::from(vec![
                        Span::styled("        Model: ", Style::default().fg(Color::Rgb(100, 100, 100))),
                        Span::styled(
                            model.clone(),
                            Style::default().fg(Color::Rgb(220, 200, 255)),
                        ),
                    ]));
                    lines.push(Line::raw(""));
                    // Feature bar
                    let divider_color = Color::Rgb(80, 60, 100);
                    lines.push(Line::from(Span::styled(
                        "   ".to_string() + &"─".repeat(max_content_width.min(70)),
                        Style::default().fg(divider_color),
                    )));
                    lines.push(Line::from(vec![
                        Span::styled("     ⚡ ", Style::default()),
                        Span::styled("启动 ", Style::default().fg(Color::White)),
                        Span::styled("2.5x ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                        Span::styled("  💾 ", Style::default()),
                        Span::styled("内存 ", Style::default().fg(Color::White)),
                        Span::styled("-60% ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                        Span::styled("  🚀 ", Style::default()),
                        Span::styled("响应 ", Style::default().fg(Color::White)),
                        Span::styled("+40%", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                    ]));
                    lines.push(Line::from(Span::styled(
                        "   ".to_string() + &"─".repeat(max_content_width.min(70)),
                        Style::default().fg(divider_color),
                    )));
                    lines.push(Line::raw(""));
                    lines.push(Line::from(Span::styled(
                        "     输入 help 查看命令 · 输入 exit 退出",
                        Style::default()
                            .fg(Color::Rgb(100, 100, 100))
                            .add_modifier(Modifier::ITALIC),
                    )));
                    lines.push(Line::raw(""));
                }
            }
        }

        lines
    }

    fn border_top_line(label: &str, label_color: Color, width: usize) -> Line<'static> {
        let label_len = label.chars().count();
        let dashes = width.saturating_sub(label_len);
        Line::from(vec![
            Span::styled("╭", Style::default().fg(label_color)),
            Span::styled(
                label.to_string(),
                Style::default().fg(label_color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "─".repeat(dashes),
                Style::default().fg(label_color),
            ),
            Span::styled("╮", Style::default().fg(label_color)),
        ])
    }

    fn border_bottom_line(color: Color, width: usize) -> Line<'static> {
        Line::from(vec![
            Span::styled("╰", Style::default().fg(color)),
            Span::styled("─".repeat(width), Style::default().fg(color)),
            Span::styled("╯", Style::default().fg(color)),
        ])
    }

    fn push_wrapped_line(
        lines: &mut Vec<Line<'static>>,
        text: &str,
        prefix: &str,
        prefix_color: Color,
        text_color: Color,
        max_width: usize,
    ) {
        let prefix_len = prefix.chars().count();
        let content_width = max_width.saturating_sub(prefix_len);

        if content_width == 0 || text.is_empty() {
            lines.push(Line::from(vec![
                Span::styled(prefix.to_string(), Style::default().fg(prefix_color)),
                Span::styled(String::new(), Style::default().fg(text_color)),
            ]));
            return;
        }

        let mut remaining = text;
        let mut first = true;
        while !remaining.is_empty() {
            let chunk_end = Self::find_wrap_point(remaining, content_width);
            let chunk = &remaining[..chunk_end];
            remaining = &remaining[chunk_end..];

            let p = if first {
                first = false;
                prefix
            } else {
                "│ "
            };

            lines.push(Line::from(vec![
                Span::styled(p.to_string(), Style::default().fg(prefix_color)),
                Span::styled(chunk.to_string(), Style::default().fg(text_color)),
            ]));
        }
    }

    /// Find a good wrap point for the text within the given width
    fn find_wrap_point(text: &str, max_width: usize) -> usize {
        let mut width = 0;
        for (i, ch) in text.char_indices() {
            let ch_width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
            if width + ch_width > max_width {
                return i;
            }
            width += ch_width;
        }
        text.len()
    }
}

impl Widget for &ChatHistory {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let lines = self.build_lines(area.width);
        let total_lines = lines.len() as u16;
        let visible_height = area.height;
        let max_scroll_from_top = total_lines.saturating_sub(visible_height);

        // scroll_offset is distance from bottom; convert to Paragraph scroll (from top)
        let scroll_from_top = if self.user_scrolled {
            // offset from bottom = max_scroll_from_top - scroll_from_top
            let from_top = max_scroll_from_top.saturating_sub(self.scroll_offset);
            from_top.min(max_scroll_from_top)
        } else {
            // Auto-scroll to bottom
            max_scroll_from_top
        };

        let paragraph = Paragraph::new(lines)
            .scroll((scroll_from_top, 0))
            .block(
                Block::default()
                    .borders(Borders::NONE)
                    .style(Style::default().bg(Color::Rgb(30, 30, 35))),
            );

        paragraph.render(area, buf);
    }
}

impl Default for ChatHistory {
    fn default() -> Self {
        Self::new()
    }
}
