//! Plan Panel — inline panel for reviewing and approving agent-generated plans.
//! Rendered above the status bar when in Plan Mode.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

const HEADER_COLOR: Color = Color::Rgb(100, 200, 255);
const TEXT_COLOR: Color = Color::Rgb(200, 200, 220);
const HINT_COLOR: Color = Color::Rgb(120, 120, 140);
const ACCEPT_COLOR: Color = Color::Rgb(80, 200, 120);
const REJECT_COLOR: Color = Color::Rgb(240, 100, 100);

pub struct PlanPanelState {
    pub visible: bool,
    pub plan_text: String,
}

impl PlanPanelState {
    pub fn new() -> Self {
        Self {
            visible: false,
            plan_text: String::new(),
        }
    }

    pub fn show(&mut self, plan: String) {
        self.plan_text = plan;
        self.visible = true;
    }

    pub fn dismiss(&mut self) {
        self.visible = false;
        self.plan_text.clear();
    }
}

impl Default for PlanPanelState {
    fn default() -> Self {
        Self::new()
    }
}

pub fn render(f: &mut Frame, area: Rect, state: &PlanPanelState) {
    let block = Block::default()
        .borders(Borders::TOP | Borders::LEFT | Borders::RIGHT)
        .border_style(Style::default().fg(HEADER_COLOR))
        .title(Span::styled(" Plan Review ", Style::default().fg(HEADER_COLOR)));

    let mut lines: Vec<Line> = Vec::new();

    // Plan content
    for line in state.plan_text.lines() {
        lines.push(Line::from(Span::styled(
            format!("  {}", line),
            Style::default().fg(TEXT_COLOR),
        )));
    }

    // Blank line then hints
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled(" [y] ", Style::default().fg(ACCEPT_COLOR).add_modifier(Modifier::BOLD)),
        Span::styled("Approve  ", Style::default().fg(HINT_COLOR)),
        Span::styled("[n] ", Style::default().fg(REJECT_COLOR).add_modifier(Modifier::BOLD)),
        Span::styled("Reject", Style::default().fg(HINT_COLOR)),
    ]));

    let inner = Paragraph::new(Text::from(lines)).block(block);
    f.render_widget(inner, area);
}

pub fn height_needed(plan_text: &str) -> u16 {
    let lines = plan_text.lines().count() as u16;
    (lines + 4).min(20).max(5)
}
