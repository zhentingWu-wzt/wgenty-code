use crate::tui::app::{PermissionResponder, PermissionResponse};
use crate::tui::traits::Component;
use crossterm::event::KeyCode;
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
    /// Pending oneshot sender for permission response.
    pub responder: Option<PermissionResponder>,
}

impl PermissionState {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            visible: false,
            reason: String::new(),
            rule: String::new(),
            responder: None,
        }
    }

    pub fn show(&mut self, reason: String, rule: String, responder: PermissionResponder) {
        self.visible = true;
        self.reason = reason;
        self.rule = rule;
        self.responder = Some(responder);
    }

    pub fn dismiss(&mut self) -> (String, String) {
        self.visible = false;
        self.responder = None;
        (
            std::mem::take(&mut self.reason),
            std::mem::take(&mut self.rule),
        )
    }

    pub fn height_needed(&self) -> u16 {
        5
    }

    /// Take the permission decision if one was made via key press.
    /// Returns (reason, decision_label, PermissionResponse).
    pub fn take_decision(&mut self) -> Option<(String, String, PermissionResponse)> {
        let (reason, _rule) = self.dismiss();
        self.responder.take().map(|r| {
            let _ = r.0.unwrap(); // note: actual send happens in caller
            (
                reason,
                "Allowed once".to_string(),
                PermissionResponse::AllowOnce,
            )
        })
    }
}

impl Component for PermissionState {
    fn handle_key(&mut self, key: &crossterm::event::KeyEvent) -> bool {
        if !self.visible {
            return false;
        }
        match key.code {
            KeyCode::Char('y') => self.respond(PermissionResponse::AllowOnce),
            KeyCode::Char('a') => self.respond(PermissionResponse::AlwaysAllow),
            KeyCode::Char('n') | KeyCode::Esc => self.respond(PermissionResponse::Deny),
            _ => false,
        }
    }
}

impl PermissionState {
    fn respond(&mut self, response: PermissionResponse) -> bool {
        self.visible = false;
        if let Some(responder) = self.responder.take() {
            let _ = responder.0.map(|tx| tx.send(response));
        }
        true
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
