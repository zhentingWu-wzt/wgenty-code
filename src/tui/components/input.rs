use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, BorderType, Borders};
use ratatui::Frame;
use tui_textarea::TextArea;

/// Accent color used for command highlighting.
const ACCENT: Color = Color::Rgb(147, 112, 219);

/// Input box wrapping tui-textarea for CJK/IME-compatible text input.
pub struct InputBox {
    pub textarea: TextArea<'static>,
    last_style_was_accent: bool,
}

impl InputBox {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        let mut textarea = TextArea::default();

        textarea.set_block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(ACCENT))
                .border_type(BorderType::Rounded)
                .title(" Input (Enter 提交 · Shift+Enter 换行) "),
        );

        textarea.set_style(Style::default().fg(Color::White));
        textarea.set_placeholder_style(Style::default().fg(Color::Rgb(100, 100, 110)));
        textarea.set_placeholder_text("在这里输入你的消息...");

        Self {
            textarea,
            last_style_was_accent: false,
        }
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect) {
        let is_slash = self
            .textarea
            .lines()
            .first()
            .map(|l| l.trim_start().starts_with('/'))
            .unwrap_or(false);

        // tui-textarea's set_style only affects future input, not existing text.
        // When slash state changes, re-insert all text with the correct style.
        if is_slash != self.last_style_was_accent {
            self.last_style_was_accent = is_slash;
            let target_style = if is_slash {
                Style::default().fg(ACCENT)
            } else {
                Style::default().fg(Color::White)
            };

            // Save text, re-insert with target style
            let text = self
                .textarea
                .lines()
                .iter()
                .map(|l| l.to_string())
                .collect::<Vec<_>>()
                .join("\n");
            self.textarea.set_style(target_style);
            self.textarea.select_all();
            self.textarea.cut();
            if !text.is_empty() {
                self.textarea.insert_str(&text);
            }
        } else {
            // Keep style in sync for newly typed characters
            let style = if is_slash {
                Style::default().fg(ACCENT)
            } else {
                Style::default().fg(Color::White)
            };
            self.textarea.set_style(style);
        }

        f.render_widget(&self.textarea, area);
    }

    /// Extract text and reset for next input.
    pub fn take_text(&mut self) -> String {
        let text = self
            .textarea
            .lines()
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        self.textarea.select_all();
        self.textarea.cut();
        self.last_style_was_accent = false;
        self.textarea.set_style(Style::default().fg(Color::White));
        text
    }

    /// Insert a single character (used for Shift+Enter → newline).
    pub fn insert_char(&mut self, c: char) {
        self.textarea.insert_char(c);
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
