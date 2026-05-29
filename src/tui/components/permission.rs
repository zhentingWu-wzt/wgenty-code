use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders};
use ratatui::Frame;

const WARN_COLOR: Color = Color::Rgb(255, 200, 50);
const DIM_COLOR: Color = Color::Rgb(120, 120, 130);

/// Permission approval state.
/// Rendered inline between chat and status bar — same style as question panel.
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

    pub fn height_needed(&self) -> u16 {
        // border(2) + reason(1) + blank(1) + hint(1) = 5
        5
    }
}

/// Render the permission panel inline in the layout.
pub fn render(f: &mut Frame, area: Rect, state: &PermissionState) {
    if !state.visible {
        return;
    }

    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(Span::styled(
        format!(" {}", state.reason),
        Style::default().fg(Color::White),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        " [y] Allow once    [a] Always allow    [n] Deny",
        Style::default().fg(DIM_COLOR),
    )));

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(WARN_COLOR))
        .title(" Permission Required ");

    let para = ratatui::widgets::Paragraph::new(ratatui::text::Text::from(lines)).block(block);
    f.render_widget(para, area);
}
