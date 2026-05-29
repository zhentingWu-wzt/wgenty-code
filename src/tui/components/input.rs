use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, BorderType, Borders};
use ratatui::Frame;
use tui_textarea::TextArea;

/// Input box wrapping tui-textarea for CJK/IME-compatible text input.
pub struct InputBox {
    pub textarea: TextArea<'static>,
}

impl InputBox {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        let mut textarea = TextArea::default();

        textarea.set_block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(147, 112, 219)))
                .border_type(BorderType::Rounded)
                .title(" Input (Enter 提交 · Shift+Enter 换行) "),
        );

        textarea.set_style(Style::default().fg(Color::White).bg(Color::Rgb(40, 40, 45)));
        textarea.set_placeholder_style(
            Style::default()
                .fg(Color::Rgb(100, 100, 110))
                .bg(Color::Rgb(40, 40, 45)),
        );
        textarea.set_placeholder_text("在这里输入你的消息...");

        Self { textarea }
    }

    pub fn render(&self, f: &mut Frame, area: Rect) {
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
