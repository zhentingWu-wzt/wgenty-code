use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// Render the ASCII art welcome banner.
pub fn render(f: &mut Frame, area: Rect) {
    let banner = [
        "  ╔══════════════════════════════════╗",
        "  ║      Claude Code Rust            ║",
        "  ║      ratatui frontend             ║",
        "  ╚══════════════════════════════════╝",
        "",
        "  Type a message and press Enter to start.",
        "  Ctrl+C to quit.",
    ];

    let lines: Vec<Line> = banner
        .iter()
        .map(|s| {
            Line::from(Span::styled(
                *s,
                Style::default().fg(Color::Rgb(200, 180, 255)),
            ))
        })
        .collect();

    let para = Paragraph::new(Text::from(lines));
    f.render_widget(para, area);
}
