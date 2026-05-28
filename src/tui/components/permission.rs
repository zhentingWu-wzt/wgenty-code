use crate::tui::theme;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

/// Permission popup state.
pub struct PermissionState {
    pub visible: bool,
    pub reason: String,
    pub rule: String,
}

impl PermissionState {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            visible: false,
            reason: String::new(),
            rule: String::new(),
        }
    }

    pub fn show(&mut self, reason: String, rule: String) {
        self.visible = true;
        self.reason = reason;
        self.rule = rule;
    }

    pub fn dismiss(&mut self) -> (String, String) {
        self.visible = false;
        (
            std::mem::take(&mut self.reason),
            std::mem::take(&mut self.rule),
        )
    }
}

/// Render the permission popup centered on screen.
pub fn render(
    f: &mut Frame,
    state: &PermissionState,
    centered_rect_fn: impl Fn(u16, u16, Rect) -> Rect,
) {
    if !state.visible {
        return;
    }

    let area = f.area();
    let popup_area = centered_rect_fn(60, 25, area);

    // Clear the background under the popup
    f.render_widget(Clear, popup_area);

    let text = vec![
        Line::from(Span::styled(
            " ⚠ Permission Required",
            Style::default().fg(theme::WARNING),
        )),
        Line::from(""),
        Line::from(Span::raw(format!(" {}", state.reason))),
        Line::from(""),
        Line::from(Span::styled(
            " [y] Allow once    [a] Always allow    [n] Deny",
            Style::default().fg(theme::DIM),
        )),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::WARNING))
        .title(" Permission ");

    let para = Paragraph::new(Text::from(text))
        .block(block)
        .wrap(Wrap { trim: true });

    f.render_widget(para, popup_area);
}
