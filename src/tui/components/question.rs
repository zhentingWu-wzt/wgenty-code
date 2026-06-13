use crate::tui::app::QuestionResponder;
use crate::tui::traits::Component;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders};
use ratatui::Frame;

/// Question panel state (for ask_user_question tool).
/// Rendered inline between chat and status bar — NOT as a floating popup.
pub struct QuestionState {
    pub visible: bool,
    pub question: String,
    pub options: Vec<String>,
    pub multi_select: bool,
    pub selected: Vec<usize>,
    /// Current highlighted cursor position.
    /// `options.len()` means the "Other" custom-input line is highlighted.
    pub cursor: usize,
    /// Custom text typed into the "Other" option.
    pub other_value: String,
    /// Pending oneshot sender for question response.
    pub responder: Option<QuestionResponder>,
}

const ACCENT_COLOR: Color = Color::Rgb(255, 200, 100);
const BORDER_COLOR: Color = Color::Rgb(100, 200, 255);
const DIM_COLOR: Color = Color::Rgb(120, 120, 130);

impl QuestionState {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            visible: false,
            question: String::new(),
            options: Vec::new(),
            multi_select: false,
            selected: Vec::new(),
            cursor: 0,
            other_value: String::new(),
            responder: None,
        }
    }

    pub fn show(
        &mut self,
        question: String,
        options: Vec<String>,
        multi_select: bool,
        responder: QuestionResponder,
    ) {
        self.visible = true;
        self.question = question;
        self.options = options;
        self.multi_select = multi_select;
        self.selected = if multi_select { vec![] } else { vec![0] };
        self.cursor = 0;
        self.other_value.clear();
        self.responder = Some(responder);
    }

    pub fn cursor_on_other(&self) -> bool {
        self.cursor == self.options.len()
    }

    fn max_cursor(&self) -> usize {
        self.options.len() // last index = Other
    }

    /// Returns the selected labels and sends response. Clears visibility.
    pub fn dismiss(&mut self) -> Vec<String> {
        self.visible = false;
        let answers = if self.cursor_on_other() {
            vec![std::mem::take(&mut self.other_value)]
        } else if self.multi_select {
            self.selected
                .iter()
                .filter_map(|&i| self.options.get(i).cloned())
                .collect()
        } else {
            self.options.get(self.cursor).cloned().into_iter().collect()
        };
        // Send response via oneshot channel
        if let Some(responder) = self.responder.take() {
            let _ = responder.0.map(|tx| tx.send(answers.clone()));
        }
        answers
    }

    /// Take pending response if any (after handle_key triggered a submission).
    pub fn take_response(&mut self) -> Option<Vec<String>> {
        if let Some(responder) = self.responder.take() {
            let answers = if self.cursor_on_other() {
                vec![std::mem::take(&mut self.other_value)]
            } else if self.multi_select {
                self.selected
                    .iter()
                    .filter_map(|&i| self.options.get(i).cloned())
                    .collect()
            } else {
                self.options.get(self.cursor).cloned().into_iter().collect()
            };
            let _ = responder.0.map(|tx| tx.send(answers.clone()));
            self.visible = false;
            Some(answers)
        } else {
            None
        }
    }

    pub fn move_up(&mut self) {
        let max = self.max_cursor();
        self.cursor = if self.cursor > 0 {
            self.cursor - 1
        } else {
            max
        };
    }

    pub fn move_down(&mut self) {
        let max = self.max_cursor();
        self.cursor = if self.cursor < max {
            self.cursor + 1
        } else {
            0
        };
    }

    pub fn toggle_selection(&mut self) {
        if !self.multi_select || self.cursor_on_other() {
            return;
        }
        if self.selected.contains(&self.cursor) {
            self.selected.retain(|&s| s != self.cursor);
        } else {
            self.selected.push(self.cursor);
            self.selected.sort();
        }
    }

    /// Quick-select by number key (1-based). Returns true if submitted.
    pub fn select_number(&mut self, n: usize) -> bool {
        if n >= 1 && n <= self.options.len() {
            let idx = n - 1;
            if self.multi_select {
                if self.selected.contains(&idx) {
                    self.selected.retain(|&s| s != idx);
                } else {
                    self.selected.push(idx);
                    self.selected.sort();
                }
                false
            } else {
                self.cursor = idx;
                true // Auto-submit for single-select
            }
        } else {
            false
        }
    }

    /// Height needed to render this panel (estimated).
    pub fn height_needed(&self) -> u16 {
        // border(2) + question(2) + hint(1) + options(N) + Other(1) + padding
        (self.options.len() + 7) as u16
    }
}

impl Component for QuestionState {
    fn handle_key(&mut self, key: &KeyEvent) -> bool {
        if !self.visible {
            return false;
        }

        // Text input mode: cursor is on "Other" option
        if self.cursor_on_other() {
            match key.code {
                KeyCode::Char(c) => {
                    self.other_value.push(c);
                    true
                }
                KeyCode::Backspace => {
                    self.other_value.pop();
                    true
                }
                KeyCode::Enter => {
                    self.take_response();
                    true
                }
                KeyCode::Up => {
                    self.move_up();
                    true
                }
                KeyCode::Down => {
                    self.move_down();
                    true
                }
                KeyCode::Esc => {
                    self.visible = false;
                    self.responder = None;
                    true
                }
                _ => false,
            }
        } else {
            // Navigation mode
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    self.move_up();
                    true
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.move_down();
                    true
                }
                KeyCode::Enter => {
                    let can_submit = !self.multi_select || !self.selected.is_empty();
                    if can_submit {
                        self.take_response();
                    }
                    true
                }
                KeyCode::Char(' ') => {
                    self.toggle_selection();
                    true
                }
                KeyCode::Esc => {
                    self.visible = false;
                    self.responder = None;
                    true
                }
                KeyCode::Char(c) if c.is_ascii_digit() => {
                    let n = c.to_digit(10).unwrap() as usize;
                    if self.select_number(n) {
                        self.take_response();
                    }
                    true
                }
                _ => false,
            }
        }
    }
}

/// Render the question panel inline in the layout.
pub fn render(f: &mut Frame, area: Rect, state: &QuestionState) {
    if !state.visible {
        return;
    }

    let mut lines: Vec<Line> = Vec::new();

    // Question text
    lines.push(Line::from(Span::styled(
        format!(" {}", state.question),
        Style::default().fg(Color::White),
    )));
    lines.push(Line::raw(""));

    // Hint
    let hint = if state.multi_select {
        " [Space] toggle · [Enter] submit · [↑↓] navigate · [1-9] quick select"
    } else {
        " [↑↓] navigate · [Enter] select · [1-9] quick select · [Esc] cancel"
    };
    lines.push(Line::from(Span::styled(
        hint,
        Style::default().fg(DIM_COLOR),
    )));
    lines.push(Line::raw(""));

    // Numbered options
    for (i, opt) in state.options.iter().enumerate() {
        lines.push(option_line(i, opt, state));
    }

    // "Other" option — inline text input when highlighted
    let other_idx = state.options.len();
    let is_other_active = state.cursor_on_other();
    let cursor_char_other = if is_other_active { "❯" } else { " " };
    let color_other = if is_other_active {
        ACCENT_COLOR
    } else {
        DIM_COLOR
    };

    let mut other_spans = vec![Span::styled(
        format!(" {} ○ {}. Other — ", cursor_char_other, other_idx + 1),
        Style::default().fg(color_other),
    )];

    if is_other_active {
        other_spans.push(Span::styled(
            if state.other_value.is_empty() {
                "type custom answer..."
            } else {
                &state.other_value
            },
            Style::default().fg(Color::White),
        ));
        // Blinking cursor indicator
        other_spans.push(Span::styled("▌", Style::default().fg(ACCENT_COLOR)));
    } else {
        other_spans.push(Span::styled(
            "enter custom answer",
            Style::default().fg(DIM_COLOR),
        ));
    }

    lines.push(Line::from(other_spans));

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(BORDER_COLOR))
        .title(" Question ");

    let para = ratatui::widgets::Paragraph::new(ratatui::text::Text::from(lines)).block(block);

    f.render_widget(para, area);
}

fn option_line(idx: usize, label: &str, state: &QuestionState) -> Line<'static> {
    let is_cursor = idx == state.cursor;
    let is_selected = state.selected.contains(&idx);

    let cursor_char = if is_cursor { "❯" } else { " " };
    let marker = if state.multi_select {
        if is_selected {
            "◉"
        } else {
            "○"
        }
    } else if is_cursor {
        "●"
    } else {
        "○"
    };

    let color = if is_cursor { ACCENT_COLOR } else { DIM_COLOR };

    Line::from(vec![
        Span::styled(
            format!(" {} {} {}. ", cursor_char, marker, idx + 1),
            Style::default().fg(color),
        ),
        Span::styled(label.to_string(), Style::default().fg(Color::White)),
    ])
}
