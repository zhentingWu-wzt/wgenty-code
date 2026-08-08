use crate::tui::app::{PermissionResponder, PermissionResponse};
use crate::tui::client::DaemonClient;
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
    /// Server-side mode: request_id for POST /resolve-permission (None in
    /// client-side mode where the oneshot responder is used instead).
    pub server_request_id: Option<String>,
    pub client: Option<DaemonClient>,
}

impl PermissionState {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            visible: false,
            reason: String::new(),
            rule: String::new(),
            responder: None,
            server_request_id: None,
            client: None,
        }
    }

    pub fn show(&mut self, reason: String, rule: String, responder: PermissionResponder) {
        self.visible = true;
        self.reason = reason;
        self.rule = rule;
        self.responder = Some(responder);
        self.server_request_id = None;
        self.client = None;
    }

    /// Server-side variant: no oneshot responder; the decision is sent via
    /// POST /resolve-permission using the stored request_id + client.
    pub fn show_server(
        &mut self,
        reason: String,
        rule: String,
        request_id: String,
        client: DaemonClient,
    ) {
        self.visible = true;
        self.reason = reason;
        self.rule = rule;
        self.responder = None;
        self.server_request_id = Some(request_id);
        self.client = Some(client);
    }

    pub fn dismiss(&mut self) -> (String, String) {
        self.visible = false;
        self.responder = None;
        self.server_request_id = None;
        self.client = None;
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
            let _ =
                r.0.expect("responder sender exists when permission panel active"); // note: actual send happens in caller
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
        if let Some(req_id) = self.server_request_id.take() {
            let approved = !matches!(response, PermissionResponse::Deny);
            let always = matches!(response, PermissionResponse::AlwaysAllow);
            let rule = self.rule.clone();
            if let Some(client) = self.client.take() {
                tokio::spawn(async move {
                    let _ = client
                        .resolve_subagent_permission(&req_id, approved, always, Some(&rule))
                        .await;
                });
            }
        } else if let Some(responder) = self.responder.take() {
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
