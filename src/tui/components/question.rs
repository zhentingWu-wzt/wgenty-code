use crate::tui::theme;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem};
use ratatui::Frame;

/// Question popup state (for ask_user_question tool).
pub struct QuestionState {
    pub visible: bool,
    pub question: String,
    pub options: Vec<String>,
    pub multi_select: bool,
    pub selected: Vec<usize>,
}

impl QuestionState {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            visible: false,
            question: String::new(),
            options: Vec::new(),
            multi_select: false,
            selected: Vec::new(),
        }
    }

    pub fn show(&mut self, question: String, options: Vec<String>, multi_select: bool) {
        self.visible = true;
        self.question = question;
        self.selected = if options.is_empty() { vec![] } else { vec![0] };
        self.options = options;
        self.multi_select = multi_select;
    }

    pub fn dismiss(&mut self) -> Vec<String> {
        self.visible = false;
        self.selected
            .iter()
            .filter_map(|&i| self.options.get(i).cloned())
            .collect()
    }

    pub fn move_up(&mut self) {
        if let Some(first) = self.selected.first_mut() {
            *first = first.saturating_sub(1);
        }
    }

    pub fn move_down(&mut self) {
        if let Some(first) = self.selected.first_mut() {
            let max = self.options.len().saturating_sub(1);
            *first = (*first + 1).min(max);
        }
    }
}

/// Render the question popup centered on screen.
pub fn render(
    f: &mut Frame,
    state: &QuestionState,
    centered_rect_fn: impl Fn(u16, u16, Rect) -> Rect,
) {
    if !state.visible {
        return;
    }

    let area = f.area();
    let popup_area = centered_rect_fn(70, 30, area);
    f.render_widget(Clear, popup_area);

    let items: Vec<ListItem> = state
        .options
        .iter()
        .enumerate()
        .map(|(i, opt)| {
            let prefix = if state.selected.contains(&i) {
                "▶ "
            } else {
                "  "
            };
            ListItem::new(Span::raw(format!("{}{}", prefix, opt)))
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::ACCENT))
            .title(format!(" {} ", state.question)),
    );

    f.render_widget(list, popup_area);
}
