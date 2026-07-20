//! CompletionPanel — inline completion suggestion list above the input box.

use crate::tui::app::types::CompletionState;
use crate::tui::theme::ACCENT;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;
use std::ops::Range;

const MAX_VISIBLE_SKILL_ITEMS: usize = 8;
const MAX_VISIBLE_COMMAND_ITEMS: usize = 10;
const MAX_SKILL_PANEL_WIDTH: u16 = 60;
const MAX_COMMAND_PANEL_WIDTH: u16 = 88;
const HINT_ROWS: u16 = 1;
const TAB_ROWS: u16 = 1;

pub struct CompletionPanel;

impl CompletionPanel {
    pub fn render(f: &mut Frame, area: Rect, state: &CompletionState) {
        if !state.visible {
            return;
        }

        let visible_matches = state.visible_matches();
        if visible_matches.is_empty() {
            return;
        }

        let max_visible_items = if state.prefix == '/' {
            MAX_VISIBLE_COMMAND_ITEMS
        } else {
            MAX_VISIBLE_SKILL_ITEMS
        };
        let max_panel_width = if state.prefix == '/' {
            MAX_COMMAND_PANEL_WIDTH
        } else {
            MAX_SKILL_PANEL_WIDTH
        };
        let has_tabs = state.prefix == '/' && !state.tabs.is_empty();
        let tab_rows = if has_tabs { TAB_ROWS } else { 0 };
        let visible_item_count = max_visible_items.min(visible_matches.len());
        let panel_height =
            u16::try_from(visible_item_count).unwrap_or(u16::MAX) + tab_rows + HINT_ROWS + 2; // border top/bottom

        let panel_area = Rect {
            x: area.x,
            y: area.y.saturating_sub(panel_height),
            width: area.width.min(max_panel_width),
            height: panel_height,
        };

        let border_color = ACCENT;

        let selected_index = state.selected_index.min(visible_matches.len() - 1);
        let block_title = format!(
            " {} {}/{} ",
            if state.prefix == '@' {
                "Skills"
            } else {
                "Commands"
            },
            selected_index + 1,
            visible_matches.len()
        );
        let block = Block::default()
            .title(block_title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .style(Style::default().bg(Color::Rgb(26, 26, 46)));

        let inner = block.inner(panel_area);

        let mut lines: Vec<Line> = Vec::new();
        if has_tabs {
            lines.push(tab_line(state));
        }

        let visible_range = visible_item_range(
            state.selected_index,
            visible_matches.len(),
            visible_item_count,
        );

        for (i, m) in visible_matches[visible_range.clone()].iter().enumerate() {
            let item_index = visible_range.start + i;
            let is_selected = item_index == selected_index;
            let style = if is_selected {
                Style::default()
                    .fg(Color::Rgb(203, 166, 247))
                    .add_modifier(Modifier::BOLD)
                    .bg(Color::Rgb(40, 40, 70))
            } else {
                Style::default().fg(Color::Rgb(205, 205, 220))
            };
            let category_style = Style::default()
                .fg(Color::Rgb(108, 112, 134))
                .add_modifier(Modifier::DIM);

            let marker = if is_selected { "›" } else { " " };
            let name_style = if is_selected {
                Style::default()
                    .fg(Color::Rgb(203, 166, 247))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Rgb(220, 220, 235))
            };
            // Fixed-ish columns: marker+name, category badge, description, args.
            let name_col = format!("{} {}", marker, truncate_pad(&m.text, 18));
            let cat_col = format!(" {}", truncate_pad(&format!("[{}]", m.category), 10));
            let row_bg = if is_selected {
                Color::Rgb(40, 40, 70)
            } else {
                Color::Rgb(26, 26, 46)
            };
            let mut parts = vec![
                Span::styled(name_col, name_style.bg(row_bg)),
                Span::styled(cat_col, category_style.bg(row_bg)),
                Span::styled(
                    format!(" {}", m.description),
                    style.add_modifier(Modifier::DIM).bg(row_bg),
                ),
            ];
            if let Some(ref hint) = m.args_hint {
                parts.push(Span::styled(
                    format!("  {}", hint),
                    Style::default()
                        .fg(Color::Rgb(137, 180, 250))
                        .add_modifier(Modifier::DIM),
                ));
            }
            lines.push(Line::from(parts));
        }

        // Bottom hint
        let hint_style = Style::default().fg(Color::Rgb(108, 112, 134));
        let hint_text = if has_tabs {
            " \u{2191}\u{2195} nav  \u{2190}\u{2192} tabs  Tab cycle  Enter select  Esc close "
        } else {
            " \u{2191}\u{2195} nav  Tab cycle  Enter select  Esc close "
        };
        let hint = Line::from(vec![Span::styled(hint_text, hint_style)]);

        // Render block and content
        f.render_widget(block, panel_area);
        let content_area = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: inner.height.saturating_sub(HINT_ROWS),
        };
        f.render_widget(Paragraph::new(lines), content_area);
        f.render_widget(
            Paragraph::new(hint),
            Rect {
                x: inner.x,
                y: inner.y + inner.height.saturating_sub(HINT_ROWS),
                width: inner.width,
                height: HINT_ROWS,
            },
        );
    }
}

fn tab_line(state: &CompletionState) -> Line<'static> {
    let active_tab = state.active_tab_label();
    let mut parts = vec![Span::styled(
        " \u{25c0} ",
        Style::default().fg(Color::Rgb(108, 112, 134)),
    )];

    for tab in &state.tabs {
        let is_active = active_tab == Some(tab.as_str());
        let style = if is_active {
            Style::default()
                .fg(Color::Rgb(203, 166, 247))
                .add_modifier(Modifier::BOLD)
                .bg(Color::Rgb(40, 40, 70))
        } else {
            Style::default().fg(Color::Rgb(108, 112, 134))
        };
        let label = if is_active {
            format!("[{}] ", tab)
        } else {
            format!(" {}  ", tab)
        };
        parts.push(Span::styled(label, style));
    }

    parts.push(Span::styled(
        "\u{25b6}",
        Style::default().fg(Color::Rgb(108, 112, 134)),
    ));

    Line::from(parts)
}

fn truncate_pad(s: &str, width: usize) -> String {
    let mut out = String::new();
    let mut w = 0usize;
    for ch in s.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if w + cw > width {
            break;
        }
        out.push(ch);
        w += cw;
    }
    while w < width {
        out.push(' ');
        w += 1;
    }
    out
}

fn visible_item_range(
    selected_index: usize,
    total_items: usize,
    max_visible: usize,
) -> Range<usize> {
    if total_items <= max_visible {
        return 0..total_items;
    }

    let selected_index = selected_index.min(total_items - 1);
    let start = if selected_index < max_visible {
        0
    } else {
        selected_index + 1 - max_visible
    };
    let end = (start + max_visible).min(total_items);

    start..end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_item_range_shows_initial_items() {
        assert_eq!(visible_item_range(0, 12, MAX_VISIBLE_SKILL_ITEMS), 0..8);
        assert_eq!(visible_item_range(7, 12, MAX_VISIBLE_SKILL_ITEMS), 0..8);
    }

    #[test]
    fn visible_item_range_scrolls_to_selected_item() {
        assert_eq!(visible_item_range(8, 12, MAX_VISIBLE_SKILL_ITEMS), 1..9);
        assert_eq!(visible_item_range(11, 12, MAX_VISIBLE_SKILL_ITEMS), 4..12);
    }

    #[test]
    fn visible_item_range_handles_short_and_empty_lists() {
        assert_eq!(visible_item_range(0, 6, MAX_VISIBLE_SKILL_ITEMS), 0..6);
        assert_eq!(visible_item_range(0, 0, MAX_VISIBLE_SKILL_ITEMS), 0..0);
    }

    #[test]
    fn visible_item_range_clamps_out_of_bounds_selection() {
        assert_eq!(visible_item_range(99, 12, MAX_VISIBLE_SKILL_ITEMS), 4..12);
    }
}
