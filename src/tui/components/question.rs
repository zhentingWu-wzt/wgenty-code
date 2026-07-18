use crate::tui::app::{QuestionOption, QuestionResponder};
use crate::tui::traits::Component;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders};
use ratatui::Frame;
use textwrap::Options as WrapOptions;

/// Question panel state (for ask_user_question tool).
/// Rendered inline between chat and status bar - NOT as a floating popup.
pub struct QuestionState {
    pub visible: bool,
    pub question: String,
    pub options: Vec<QuestionOption>,
    pub multi_select: bool,
    pub selected: Vec<usize>,
    /// Current highlighted cursor position.
    /// `options.len()` means the "Other" custom-input line is highlighted.
    pub cursor: usize,
    /// Custom text typed into the "Other" option.
    pub other_value: String,
    /// Pending oneshot sender for question response.
    pub responder: Option<QuestionResponder>,
    /// Set to true by handle_key when the user confirms submission (Enter / number-key auto-select).
    /// The event loop reads this flag to decide whether to call take_response().
    pub just_submitted: bool,
}

const ACCENT_COLOR: Color = Color::Rgb(255, 200, 100);
const BORDER_COLOR: Color = Color::Rgb(100, 200, 255);
const DIM_COLOR: Color = Color::Rgb(120, 120, 130);

/// Background context shown at the top of the panel. Explains *why* the agent
/// is asking, so the user understands the situation before reading the question.
const BACKGROUND_HINT: &str =
    "💡 The agent paused and needs your input before it can continue - pick an option below.";

/// Left indent (in display columns) for option description sub-lines, so they
/// line up under the option label rather than the marker column.
const DESCRIPTION_INDENT: usize = 6;

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
            just_submitted: false,
        }
    }

    pub fn show(
        &mut self,
        question: String,
        options: Vec<QuestionOption>,
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
        self.just_submitted = false;
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
                .filter_map(|&i| self.options.get(i).map(|o| o.label.clone()))
                .collect()
        } else {
            self.options
                .get(self.cursor)
                .map(|o| o.label.clone())
                .into_iter()
                .collect()
        };
        // Send response via oneshot channel
        if let Some(responder) = self.responder.take() {
            let _ = responder.0.map(|tx| tx.send(answers.clone()));
        }
        answers
    }

    /// Take pending response if any (after handle_key triggered a submission).
    /// Returns None if there is no pending responder, or if multi-select has
    /// no selections (Enter pressed with nothing checked), or if the Other
    /// text input field is empty.
    pub fn take_response(&mut self) -> Option<Vec<String>> {
        // Guard: no responder means already taken (e.g. Esc cancelled).
        self.responder.as_ref()?;
        // In multi-select mode, require at least one selection to submit.
        if self.multi_select && self.selected.is_empty() {
            return None;
        }
        // Build answers BEFORE consuming the responder, so we can bail out
        // early without dropping the oneshot sender (e.g. empty Other value).
        let answers = if self.cursor_on_other() {
            if self.other_value.is_empty() {
                return None;
            }
            vec![std::mem::take(&mut self.other_value)]
        } else if self.multi_select {
            self.selected
                .iter()
                .filter_map(|&i| self.options.get(i).map(|o| o.label.clone()))
                .collect()
        } else {
            self.options
                .get(self.cursor)
                .map(|o| o.label.clone())
                .into_iter()
                .collect()
        };
        // Now it is safe to consume the responder and submit.
        let responder = self.responder.take()?;
        let _ = responder.0.map(|tx| tx.send(answers.clone()));
        self.visible = false;
        Some(answers)
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
                true // Signal auto-submit for single-select
            }
        } else {
            false
        }
    }

    /// Height needed to render this panel given the available `width`.
    ///
    /// Wrapping is width-dependent, so the caller (which knows the terminal
    /// width) must pass it in. The estimate mirrors [`render`] so the
    /// pre-allocated panel area fits the wrapped content without clipping.
    pub fn height_needed(&self, width: u16) -> u16 {
        // inner content width = total width minus left/right borders (2 cols).
        let inner_w = (width as usize).saturating_sub(2).max(1);
        // Background and question lines are prefixed with a single leading
        // space, so reserve 1 column for that prefix.
        let text_w = inner_w.saturating_sub(1).max(1);
        let desc_w = inner_w.saturating_sub(DESCRIPTION_INDENT).max(1);

        let bg_lines = wrap_lines(BACKGROUND_HINT, text_w).len();
        let question_lines = wrap_lines(&self.question, text_w).len();
        let mut option_lines = 0usize;
        for opt in &self.options {
            // Option label is short by schema (1-5 words); keep it on one line.
            option_lines += 1;
            if !opt.description.is_empty() {
                option_lines += wrap_lines(&opt.description, desc_w).len();
            }
        }

        // 2 borders + bg + blank + question + blank + hint + blank + options + other(1)
        let total = 2 + bg_lines + 1 + question_lines + 1 + 1 + 1 + option_lines + 1;
        total.try_into().unwrap_or(u16::MAX)
    }
}

impl Component for QuestionState {
    fn handle_key(&mut self, key: &KeyEvent) -> bool {
        if !self.visible {
            return false;
        }

        // Every key press resets the submission flag; only explicit submission
        // keys (Enter, number in single-select) set it to true.
        self.just_submitted = false;

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
                    self.just_submitted = true;
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
            // Navigation / selection mode
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
                    // For multi-select, Enter only submits when at least one item is checked.
                    // Otherwise the key is consumed but no submission occurs.
                    if !self.multi_select || !self.selected.is_empty() {
                        self.just_submitted = true;
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
                    let n = c.to_digit(10).expect("ascii digit verified by guard") as usize;
                    self.just_submitted = self.select_number(n);
                    self.just_submitted
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

    // inner content width = area width minus left/right borders (2 cols).
    let inner_w = (area.width as usize).saturating_sub(2).max(1);
    let text_w = inner_w.saturating_sub(1).max(1);
    let desc_w = inner_w.saturating_sub(DESCRIPTION_INDENT).max(1);
    let indent = " ".repeat(DESCRIPTION_INDENT);

    let mut lines: Vec<Line> = Vec::new();

    // Background / context: explain why the panel is showing before the
    // user reads the question and options.
    for chunk in wrap_lines(BACKGROUND_HINT, text_w) {
        lines.push(Line::from(Span::styled(
            format!(" {}", chunk),
            Style::default().fg(Color::Cyan),
        )));
    }
    lines.push(Line::raw(""));

    // Question text (wrapped)
    for chunk in wrap_lines(&state.question, text_w) {
        lines.push(Line::from(Span::styled(
            format!(" {}", chunk),
            Style::default().fg(Color::White),
        )));
    }
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

    // Numbered options: a label line followed by the wrapped description
    // (explanation) rendered as dimmed indented sub-lines.
    for (i, opt) in state.options.iter().enumerate() {
        lines.push(option_line(i, &opt.label, state));
        if !opt.description.is_empty() {
            for chunk in wrap_lines(&opt.description, desc_w) {
                lines.push(Line::from(Span::styled(
                    format!("{}{}", indent, chunk),
                    Style::default().fg(DIM_COLOR),
                )));
            }
        }
    }

    // "Other" option - inline text input when highlighted
    let other_idx = state.options.len();
    let is_other_active = state.cursor_on_other();
    let cursor_char_other = if is_other_active { "❯" } else { " " };
    let color_other = if is_other_active {
        ACCENT_COLOR
    } else {
        DIM_COLOR
    };

    let mut other_spans = vec![Span::styled(
        format!(" {} ○ {}. Other - ", cursor_char_other, other_idx + 1),
        Style::default().fg(color_other),
    )];

    if is_other_active {
        other_spans.push(Span::styled(
            if state.other_value.is_empty() {
                "type custom answer...".to_string()
            } else {
                state.other_value.clone()
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

/// Wrap `text` to at most `width` display columns using textwrap.
///
/// textwrap's default features include `unicode-width`, so CJK / wide
/// characters are measured correctly (width 2 per cell). Returns one owned
/// string per wrapped line; empty input yields a single empty line.
fn wrap_lines(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }
    let options = WrapOptions::new(width).break_words(true);
    textwrap::wrap(text, &options)
        .into_iter()
        .map(|cow| cow.into_owned())
        .collect()
}
