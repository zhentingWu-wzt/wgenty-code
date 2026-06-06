use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PlanItem {
    pub step: String,
    pub status: PlanStatus,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum PlanStatus {
    #[serde(rename = "pending")]
    Pending,
    #[serde(rename = "in_progress")]
    InProgress,
    #[serde(rename = "completed")]
    Completed,
}

impl PlanStatus {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().trim() {
            "in_progress" | "in-progress" | "inprogress" => PlanStatus::InProgress,
            "completed" | "complete" | "done" => PlanStatus::Completed,
            _ => PlanStatus::Pending,
        }
    }

    pub fn symbol(&self) -> &'static str {
        match self {
            PlanStatus::Pending => "\u{25CB}",
            PlanStatus::InProgress => "\u{25D0}",
            PlanStatus::Completed => "\u{2713}",
        }
    }

    pub fn color(&self) -> Color {
        match self {
            PlanStatus::Pending => Color::Rgb(140, 140, 150),
            PlanStatus::InProgress => Color::Rgb(100, 200, 255),
            PlanStatus::Completed => Color::Rgb(80, 200, 120),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlanPanelState {
    pub items: Vec<PlanItem>,
    pub visible: bool,
}

impl Default for PlanPanelState {
    fn default() -> Self {
        Self::new()
    }
}

impl PlanPanelState {
    pub fn new() -> Self {
        Self { items: Vec::new(), visible: false }
    }

    pub fn update(&mut self, items: Vec<PlanItem>) {
        self.items = items;
        self.visible = !self.items.is_empty();
    }

    pub fn dismiss(&mut self) {
        self.visible = false;
        self.items.clear();
    }
}

const HEADER_COLOR: Color = Color::Rgb(147, 112, 219);

pub fn render(f: &mut Frame, state: &PlanPanelState, area: Rect) {
    if !state.visible || state.items.is_empty() { return; }

    let mut lines: Vec<Line<'static>> = Vec::new();

    for (i, item) in state.items.iter().enumerate() {
        let symbol = item.status.symbol();
        let color = item.status.color();
        let step_text = format!("  {}  {}. {}", symbol, i + 1, item.step);
        lines.push(Line::from(Span::styled(step_text, Style::default().fg(color))));
    }

    let para = Paragraph::new(Text::from(lines))
        .block(Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(HEADER_COLOR))
            .title(" Plan "));
    f.render_widget(para, area);
}
