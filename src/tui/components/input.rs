//! Input box — TextArea-backed editing with soft-wrap display and slash styling.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use ratatui::Frame;
use tui_textarea::TextArea;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Accent color used for command / bang prefix highlighting.
const ACCENT: Color = Color::Rgb(147, 112, 219);
const FG: Color = Color::White;
const PLACEHOLDER_FG: Color = Color::Rgb(100, 100, 110);

/// Minimum / maximum outer height for the input block (including borders).
const MIN_OUTER_HEIGHT: u16 = 3;
const MAX_OUTER_HEIGHT: u16 = 12;

/// Input box wrapping tui-textarea for CJK/IME-compatible text input.
///
/// Editing state lives in [`TextArea`]; rendering is custom so we can soft-wrap
/// long lines and keep the `/cmd` (or `!cmd`) token accent-colored after a space.
pub struct InputBox {
    pub textarea: TextArea<'static>,
}

impl InputBox {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        let mut textarea = TextArea::default();

        // Block is applied only as a style source for the hidden widget path;
        // actual chrome is drawn in `render`.
        textarea.set_block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(ACCENT))
                .border_type(BorderType::Rounded)
                .title(" Input (Enter 提交 · Shift+Enter/Ctrl+J 换行) "),
        );
        textarea.set_style(Style::default().fg(FG));
        textarea.set_placeholder_style(Style::default().fg(PLACEHOLDER_FG));
        textarea.set_placeholder_text("在这里输入你的消息...");
        // Hide the widget cursor — we draw our own after soft-wrap mapping.
        textarea.set_cursor_style(Style::default());
        textarea.set_cursor_line_style(Style::default());

        Self { textarea }
    }

    /// Kept for call-site compatibility. Styling is applied at render time.
    pub fn update_style(&mut self) {
        // no-op: command highlighting is computed in `render` from plain text
    }

    /// Outer height (including border) needed for the current buffer at `total_width`.
    pub fn preferred_height(&self, total_width: u16) -> u16 {
        let inner_width = total_width.saturating_sub(2).max(1);
        let visual_rows = self.visual_row_count(inner_width as usize).max(1);
        let with_border = visual_rows
            .saturating_add(2)
            .clamp(MIN_OUTER_HEIGHT as usize, MAX_OUTER_HEIGHT as usize);
        u16::try_from(with_border).unwrap_or(MAX_OUTER_HEIGHT)
    }

    fn visual_row_count(&self, inner_width: usize) -> usize {
        let lines = self.textarea.lines();
        if lines.is_empty() {
            return 1;
        }
        let mut rows = 0usize;
        for line in lines {
            rows = rows.saturating_add(wrap_line_ranges(line, inner_width).len().max(1));
        }
        rows.max(1)
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect, border_color: Option<Color>) {
        let border_fg = border_color.unwrap_or(ACCENT);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_fg))
            .border_type(BorderType::Rounded)
            .title(" Input (Enter 提交 · Shift+Enter/Ctrl+J 换行) ");

        let inner = block.inner(area);
        f.render_widget(block, area);

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let wrap_width = inner.width as usize;

        let buffer_lines: Vec<String> = self
            .textarea
            .lines()
            .iter()
            .map(|l| l.to_string())
            .collect();
        let is_empty = buffer_lines.is_empty() || buffer_lines.iter().all(|l| l.trim().is_empty());

        let (cursor_row, cursor_col) = self.textarea.cursor();

        // Build styled, soft-wrapped lines for the whole buffer.
        let mut display_lines: Vec<Line<'static>> = Vec::new();
        let mut cursor_screen: Option<(u16, u16)> = None;
        let mut visual_row: usize = 0;

        if is_empty {
            display_lines.push(Line::from(Span::styled(
                "在这里输入你的消息...",
                Style::default().fg(PLACEHOLDER_FG),
            )));
            cursor_screen = Some((inner.x, inner.y));
        } else {
            for (line_idx, raw) in buffer_lines.iter().enumerate() {
                let segments = wrap_line_ranges(raw, wrap_width);
                let cmd = command_prefix_len(raw);
                let cursor_byte = if line_idx == cursor_row {
                    Some(char_index_to_byte(raw, cursor_col))
                } else {
                    None
                };

                for (seg_i, seg) in segments.iter().enumerate() {
                    let spans = style_segment(raw, seg.start, seg.end, cmd);
                    display_lines.push(Line::from(spans));

                    if let Some(cb) = cursor_byte {
                        let on_seg = if seg_i + 1 == segments.len() {
                            cb >= seg.start && cb <= seg.end
                        } else {
                            cb >= seg.start && cb < seg.end
                        };
                        if on_seg {
                            let within = &raw[seg.start..cb.min(seg.end)];
                            let col =
                                UnicodeWidthStr::width(within).min(wrap_width.saturating_sub(1));
                            cursor_screen = Some((
                                inner.x.saturating_add(col as u16),
                                inner.y.saturating_add(visual_row as u16),
                            ));
                        }
                    }

                    visual_row = visual_row.saturating_add(1);
                }
            }
            if cursor_screen.is_none() {
                // Fallback: end of content.
                let last_row = display_lines.len().saturating_sub(1);
                cursor_screen = Some((inner.x, inner.y.saturating_add(last_row as u16)));
            }
        }

        // Scroll so the cursor row stays inside the visible inner height.
        let total = display_lines.len();
        let view_h = inner.height as usize;
        let cursor_vrow = cursor_screen
            .map(|(_, y)| y.saturating_sub(inner.y) as usize)
            .unwrap_or(0);
        let scroll_top = if total > view_h {
            let max_top = total - view_h;
            cursor_vrow
                .saturating_sub(view_h.saturating_sub(1))
                .min(max_top)
        } else {
            0
        };

        let visible: Vec<Line<'static>> = display_lines
            .into_iter()
            .skip(scroll_top)
            .take(view_h)
            .collect();

        f.render_widget(Paragraph::new(visible), inner);

        if let Some((cx, cy)) = cursor_screen {
            let adj_y = cy.saturating_sub(u16::try_from(scroll_top).unwrap_or(0));
            if adj_y >= inner.y && adj_y < inner.y.saturating_add(inner.height) {
                f.set_cursor_position((
                    cx.min(inner.x.saturating_add(inner.width.saturating_sub(1))),
                    adj_y,
                ));
            }
        }
    }

    /// Extract text and reset for next input.
    pub fn take_text(&mut self) -> String {
        let text = self.text();
        self.textarea.select_all();
        self.textarea.cut();
        self.textarea.set_style(Style::default().fg(FG));
        text
    }

    /// Get current text without resetting.
    pub fn text(&self) -> String {
        self.textarea
            .lines()
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Check if input is empty (no content).
    pub fn is_empty(&self) -> bool {
        self.textarea.lines().is_empty()
            || self
                .textarea
                .lines()
                .iter()
                .all(|l| l.to_string().trim().is_empty())
    }
}

/// Length in **bytes** of a leading `/cmd` or `!cmd` token (no trailing space).
fn command_prefix_len(line: &str) -> Option<usize> {
    let trimmed_start = line.trim_start();
    let lead_ws = line.len() - trimmed_start.len();
    let mut chars = trimmed_start.chars();
    let sigil = chars.next()?;
    if sigil != '/' && sigil != '!' {
        return None;
    }
    let rest = chars.as_str();
    let token_body_len = rest.find(char::is_whitespace).unwrap_or(rest.len());
    // sigil (1 byte for / or !) + body; both sigils are ASCII.
    Some(lead_ws + 1 + token_body_len)
}

fn style_segment(full: &str, start: usize, end: usize, cmd: Option<usize>) -> Vec<Span<'static>> {
    let slice = &full[start..end];
    if slice.is_empty() {
        return vec![Span::raw("")];
    }
    let Some(cmd_end) = cmd else {
        return vec![Span::styled(slice.to_string(), Style::default().fg(FG))];
    };

    if end <= cmd_end {
        // Entirely within command token.
        return vec![Span::styled(slice.to_string(), Style::default().fg(ACCENT))];
    }
    if start >= cmd_end {
        return vec![Span::styled(slice.to_string(), Style::default().fg(FG))];
    }
    // Straddles the boundary.
    let mid = cmd_end - start;
    let (a, b) = slice.split_at(mid);
    vec![
        Span::styled(a.to_string(), Style::default().fg(ACCENT)),
        Span::styled(b.to_string(), Style::default().fg(FG)),
    ]
}

fn char_index_to_byte(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(i, _)| i)
        .unwrap_or(s.len())
}

/// Half-open byte range into the source line for one visual row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WrapSeg {
    start: usize,
    end: usize,
}

/// Soft-wrap one logical line by Unicode display width.
///
/// Ranges are contiguous and cover the full line (no dropped bytes), so cursor
/// mapping stays exact. Breaks after the last whitespace when possible.
fn wrap_line_ranges(line: &str, width: usize) -> Vec<WrapSeg> {
    if line.is_empty() {
        return vec![WrapSeg { start: 0, end: 0 }];
    }
    if width == 0 {
        return vec![WrapSeg {
            start: 0,
            end: line.len(),
        }];
    }
    if UnicodeWidthStr::width(line) <= width {
        return vec![WrapSeg {
            start: 0,
            end: line.len(),
        }];
    }

    let mut out = Vec::new();
    let mut start = 0usize;
    let mut x = 0usize;
    let mut last_break = start; // byte index *after* last whitespace
    let mut i = 0usize;

    while i < line.len() {
        let ch = line[i..].chars().next().expect("valid utf-8 at i");
        let ch_len = ch.len_utf8();
        let ch_w = UnicodeWidthChar::width(ch).unwrap_or(0);

        if ch_w > 0 && x + ch_w > width && i > start {
            let end = if last_break > start { last_break } else { i };
            out.push(WrapSeg { start, end });
            start = end;
            i = start;
            x = 0;
            last_break = start;
            continue;
        }

        i += ch_len;
        x = x.saturating_add(ch_w);
        if ch.is_whitespace() {
            last_break = i; // break *after* this whitespace
        }
    }

    if start < line.len() || out.is_empty() {
        out.push(WrapSeg {
            start,
            end: line.len(),
        });
    }
    out
}

#[cfg(test)]
fn wrap_line_segments(line: &str, width: usize) -> Vec<String> {
    wrap_line_ranges(line, width)
        .into_iter()
        .map(|s| line[s.start..s.end].to_string())
        .collect()
}

/// Text inserted when confirming a completion match for the given trigger prefix.
pub fn completion_insert_text(prefix: char, match_text: &str) -> String {
    match prefix {
        '@' => format!("@{} ", match_text),
        '!' => format!("!{} ", match_text),
        _ => format!("/{} ", match_text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_insert_keeps_at_prefix_for_skills() {
        assert_eq!(completion_insert_text('@', "comet"), "@comet ");
        assert_eq!(completion_insert_text('/', "help"), "/help ");
    }

    #[test]
    fn command_prefix_len_stops_at_space() {
        assert_eq!(command_prefix_len("/help me"), Some(5));
        assert_eq!(command_prefix_len("/help"), Some(5));
        assert_eq!(command_prefix_len("!ls -la"), Some(3));
        assert_eq!(command_prefix_len("hello"), None);
        assert_eq!(command_prefix_len("  /plan now"), Some(7));
    }

    #[test]
    fn wrap_line_splits_long_ascii() {
        let line = "hello world from wrap";
        let ranges = wrap_line_ranges(line, 10);
        assert!(ranges.len() >= 2, "ranges={ranges:?}");
        // Contiguous cover of the full line.
        assert_eq!(ranges.first().map(|r| r.start), Some(0));
        assert_eq!(ranges.last().map(|r| r.end), Some(line.len()));
        for w in ranges.windows(2) {
            assert_eq!(w[0].end, w[1].start);
        }
        let parts = wrap_line_segments(line, 10);
        assert!(
            parts
                .iter()
                .all(|p| UnicodeWidthStr::width(p.as_str()) <= 10),
            "parts={parts:?}"
        );
        assert_eq!(parts.concat(), line);
    }

    #[test]
    fn wrap_line_short_unchanged() {
        assert_eq!(wrap_line_segments("hi", 10), vec!["hi".to_string()]);
    }

    #[test]
    fn style_segment_splits_on_command_boundary() {
        let spans = style_segment("/help me", 0, 8, Some(5));
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].content, "/help");
        assert_eq!(spans[1].content, " me");
    }

    #[test]
    fn preferred_height_grows_with_wrapped_content() {
        let mut box_ = InputBox::new();
        box_.textarea.insert_str("word ".repeat(40));
        let h = box_.preferred_height(40);
        assert!(h > MIN_OUTER_HEIGHT);
        assert!(h <= MAX_OUTER_HEIGHT);
    }
}
