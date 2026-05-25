//! TUI input box component based on tui-textarea

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    prelude::*,
    widgets::{Block, BorderType, Borders},
};
use tui_textarea::TextArea;

/// Result of processing a key event in the input box
pub enum InputResult {
    /// User submitted the input (pressed Enter without Shift)
    Submitted(String),
    /// Input continues (normal editing)
    Continue,
}

/// Bordered input box for the TUI REPL
pub struct InputBox {
    textarea: TextArea<'static>,
}

impl InputBox {
    pub fn new() -> Self {
        let mut textarea = TextArea::default();

        textarea.set_block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(147, 112, 219)))
                .border_type(BorderType::Rounded)
                .title(" Input (Enter 提交 · Shift+Enter 换行) "),
        );

        textarea.set_style(
            Style::default()
                .fg(Color::White)
                .bg(Color::Rgb(40, 40, 45)),
        );

        textarea.set_placeholder_style(
            Style::default()
                .fg(Color::Rgb(100, 100, 110))
                .bg(Color::Rgb(40, 40, 45)),
        );
        textarea.set_placeholder_text("在这里输入你的消息...");

        Self { textarea }
    }

    /// Process a key event. Returns Submitted if Enter (no Shift) is pressed.
    pub fn input(&mut self, key: KeyEvent) -> InputResult {
        if key.code == KeyCode::Enter && !key.modifiers.contains(KeyModifiers::SHIFT) {
            let text = self.textarea.lines().join("\n");
            self.textarea.select_all();
            self.textarea.cut();
            return InputResult::Submitted(text);
        }
        self.textarea.input(key);
        InputResult::Continue
    }

    /// Insert text from IME commit
    pub fn insert_ime_text(&mut self, text: &str) {
        self.textarea.insert_str(text);
    }

    /// Get the current input text without clearing
    pub fn get_text(&self) -> String {
        self.textarea.lines().join("\n")
    }

    /// Clear the input
    pub fn clear(&mut self) {
        self.textarea.select_all();
        self.textarea.cut();
    }

    /// Render the input box into the given area
    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        ratatui::widgets::Widget::render(&self.textarea, area, buf);
    }
}

impl Default for InputBox {
    fn default() -> Self {
        Self::new()
    }
}
