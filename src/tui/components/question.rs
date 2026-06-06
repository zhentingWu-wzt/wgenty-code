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
        }
    }

    pub fn show(&mut self, question: String, options: Vec<String>, multi_select: bool) {
        self.visible = true;
        self.question = question;
        self.options = options;
        self.multi_select = multi_select;
        self.selected = if multi_select { vec![] } else { vec![0] };
        self.cursor = 0;
        self.other_value.clear();
    }

    pub fn cursor_on_other(&self) -> bool {
        self.cursor == self.options.len()
    }

    fn max_cursor(&self) -> usize {
        self.options.len() // last index = Other
    }

    /// Returns the selected labels. Clears visibility.
    pub fn dismiss(&mut self) -> Vec<String> {
        self.visible = false;
        if self.cursor_on_other() {
            vec![std::mem::take(&mut self.other_value)]
        } else if self.multi_select {
            self.selected
                .iter()
                .filter_map(|&i| self.options.get(i).cloned())
                .collect()
        } else {
            self.options
                .get(self.cursor)
                .cloned()
                .into_iter()
                .collect()
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
    lines.push(Line::from(Span::styled(hint, Style::default().fg(DIM_COLOR))));
    lines.push(Line::raw(""));

    // Numbered options
    for (i, opt) in state.options.iter().enumerate() {
        lines.push(option_line(
            i,
            opt,
            state,
        ));
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
        if is_selected { "◉" } else { "○" }
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
