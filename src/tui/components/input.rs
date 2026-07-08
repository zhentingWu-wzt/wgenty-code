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
    /// Previously observed space boundary position in the first line.
    /// `None` = no slash command; `Some(0)` = slash but no space yet; `Some(n)` = space at column n.
    last_boundary: Option<usize>,
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
                .title(" Input (Enter 提交 · Shift+Enter/Ctrl+J 换行) "),
        );

        textarea.set_style(Style::default().fg(Color::White));
        textarea.set_placeholder_style(Style::default().fg(Color::Rgb(100, 100, 110)));
        textarea.set_placeholder_text("在这里输入你的消息...");

        Self {
            textarea,
            last_boundary: None,
        }
    }

    /// Update slash-command text styling after content changes.
    /// Must be called after any text mutation (key input, paste, completion
    /// insert, `take_text`) so that the textarea is correctly styled *before*
    /// the next render — this keeps `render()` pure and prevents visual
    /// glitches (e.g. input box disappearing mid-typing when subagent status
    /// bar updates trigger frequent re-renders).
    pub fn update_style(&mut self) {
        let first_line = self
            .textarea
            .lines()
            .first()
            .map(|l| l.to_string())
            .unwrap_or_default();

        let is_slash = first_line.trim_start().starts_with('/');
        let space_pos = first_line.find(' ');

        // Current boundary state
        let current_boundary: Option<usize> = if is_slash {
            Some(space_pos.unwrap_or(0))
        } else {
            None
        };

        // Re-style only when the boundary actually moves
        if current_boundary != self.last_boundary {
            self.last_boundary = current_boundary;

            let text = self
                .textarea
                .lines()
                .iter()
                .map(|l| l.to_string())
                .collect::<Vec<_>>()
                .join("\n");

            if is_slash && !text.is_empty() {
                self.textarea.set_style(Style::default().fg(ACCENT));
                self.textarea.select_all();
                self.textarea.cut();
                if let Some(pos) = space_pos {
                    self.textarea.insert_str(&first_line[..pos]);
                    self.textarea.set_style(Style::default().fg(Color::White));
                    self.textarea.insert_str(&first_line[pos..]);
                } else {
                    self.textarea.insert_str(&first_line);
                }
                let rest_start = first_line.len();
                if text.len() > rest_start {
                    self.textarea.set_style(Style::default().fg(Color::White));
                    self.textarea.insert_str(&text[rest_start..]);
                }
            } else if is_slash {
                // Empty — just accent
                self.textarea.set_style(Style::default().fg(ACCENT));
            } else {
                // Not a slash command — re-style as white
                self.textarea.set_style(Style::default().fg(Color::White));
                self.textarea.select_all();
                self.textarea.cut();
                if !text.is_empty() {
                    self.textarea.insert_str(&text);
                }
            }
        } else if is_slash {
            // Boundary unchanged, just sync style for next typed char
            if space_pos.is_some() {
                self.textarea.set_style(Style::default().fg(Color::White));
            } else {
                self.textarea.set_style(Style::default().fg(ACCENT));
            }
        } else {
            self.textarea.set_style(Style::default().fg(Color::White));
        }
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect, border_color: Option<Color>) {
        // Update border color to reflect current agent mode.
        let border_fg = border_color.unwrap_or(ACCENT);
        self.textarea.set_block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_fg))
                .border_type(BorderType::Rounded)
                .title(" Input (Enter 提交 · Shift+Enter/Ctrl+J 换行) "),
        );

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
        self.last_boundary = None;
        // Sync style for next input (white, no slash command)
        self.textarea.set_style(Style::default().fg(Color::White));
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
