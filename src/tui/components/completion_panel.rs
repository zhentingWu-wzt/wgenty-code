//! CompletionPanel — inline completion suggestion list above the input box.

use crate::tui::app::types::CompletionState;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

pub struct CompletionPanel;

impl CompletionPanel {
    pub fn render(f: &mut Frame, area: Rect, state: &CompletionState) {
        if !state.visible || state.matches.is_empty() {
            return;
        }

        // Max 8 visible items
        let max_visible = 8.min(state.matches.len());
        let panel_height = max_visible as u16 + 2; // border top/bottom

        let panel_area = Rect {
            x: area.x,
            y: area.y.saturating_sub(panel_height),
            width: area.width.min(60),
            height: panel_height,
        };

        let border_color = Color::Rgb(255, 140, 66); // orange to match @ prefix

        let block = Block::default()
            .title(format!(" {} ", if state.prefix == '@' { "Skills" } else { "Commands" }))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .style(Style::default().bg(Color::Rgb(26, 26, 46)));

        let inner = block.inner(panel_area);

        let mut lines: Vec<Line> = Vec::new();
        let visible_matches = &state.matches[..max_visible];

        for (i, m) in visible_matches.iter().enumerate() {
            let is_selected = i == state.selected_index;
            let style = if is_selected {
                Style::default()
                    .fg(Color::Rgb(203, 166, 247))
                    .add_modifier(Modifier::BOLD)
                    .bg(Color::Rgb(40, 40, 70))
            } else {
                Style::default().fg(Color::Rgb(205, 205, 220))
            };

            let mut parts = vec![
                Span::styled(
                    format!(" {}", m.text),
                    style,
                ),
                Span::styled(
                    format!("  {}", m.description),
                    style.add_modifier(Modifier::DIM),
                ),
            ];
            if let Some(ref hint) = m.args_hint {
                parts.push(Span::styled(
                    format!(" {}", hint),
                    style.add_modifier(Modifier::DIM),
                ));
            }
            lines.push(Line::from(parts));
        }

        // Bottom hint
        let hint_style = Style::default().fg(Color::Rgb(108, 112, 134));
        let hint = Line::from(vec![
            Span::styled(" \u{2191}\u{2195} nav  Tab cycle  Enter select  Esc close ", hint_style),
        ]);

        // Render block and content
        f.render_widget(block, panel_area);
        let content_area = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: inner.height.saturating_sub(1),
        };
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), content_area);
        f.render_widget(Paragraph::new(hint), Rect {
            x: inner.x,
            y: inner.y + inner.height.saturating_sub(1),
            width: inner.width,
            height: 1,
        });
    }
}
