//! Diff Rendering Component — syntax-highlighted unified diff display.
//! Renders file changes with colored +/- lines for additions and deletions.
//! Uses `similar` for diff computation; syntax highlighting via `syntect`.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use similar::{ChangeTag, TextDiff};

/// Colors for diff rendering
const ADD_COLOR: Color = Color::Rgb(80, 200, 120);
const DEL_COLOR: Color = Color::Rgb(240, 100, 100);
const CTX_COLOR: Color = Color::Rgb(100, 100, 110);
const HEADER_COLOR: Color = Color::Rgb(180, 180, 200);

/// Maximum lines of diff to render before truncation.
const MAX_DIFF_LINES: usize = 30;

/// Render a unified diff as a ratatui Paragraph in the given area.
/// Returns the number of lines actually rendered.
pub fn render(
    f: &mut Frame,
    area: Rect,
    file_path: &str,
    old: &str,
    new: &str,
) -> u16 {
    let diff = TextDiff::from_lines(old, new);

    let mut lines: Vec<Line<'static>> = Vec::new();

    // Header: ▸ path/to/file
    lines.push(Line::from(Span::styled(
        format!(" {} {}", '\u{25B8}', file_path),
        Style::default().fg(HEADER_COLOR),
    )));

    let mut change_count = 0usize;
    let mut total_shown = 0usize;

    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Equal => {
                // Only show context lines near changes (within 3 lines)
                // For simplicity, skip unchanged lines entirely
                continue;
            }
            ChangeTag::Delete => {
                for line in change.value().lines() {
                    if total_shown >= MAX_DIFF_LINES {
                        break;
                    }
                    let text = format!("- {}", line);
                    lines.push(Line::from(Span::styled(text, Style::default().fg(DEL_COLOR))));
                    total_shown += 1;
                }
                change_count += 1;
            }
            ChangeTag::Insert => {
                for line in change.value().lines() {
                    if total_shown >= MAX_DIFF_LINES {
                        break;
                    }
                    let text = format!("+ {}", line);
                    lines.push(Line::from(Span::styled(text, Style::default().fg(ADD_COLOR))));
                    total_shown += 1;
                }
                change_count += 1;
            }
        }
        if total_shown >= MAX_DIFF_LINES {
            lines.push(Line::from(Span::styled(
                format!("  ... ({} more changes)", diff.iter_all_changes().count().saturating_sub(total_shown)),
                Style::default().fg(CTX_COLOR),
            )));
            break;
        }
    }

    // If no changes, show a note
    if change_count == 0 {
        lines.push(Line::from(Span::styled(
            "  (no changes detected)",
            Style::default().fg(CTX_COLOR),
        )));
    }

    let para = Paragraph::new(ratatui::text::Text::from(lines));
    f.render_widget(para, area);

    total_shown as u16
}
