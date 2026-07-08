use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// Render the wgenty welcome banner with gradient ASCII art logo.
pub fn render(f: &mut Frame, area: Rect, model_name: &str) {
    let logo_lines = [
        "  ▄   ▄   ▄▄▄   ▄▄▄▄▄  ▄   ▄  ▄▄▄▄▄  ▄   ▄",
        "  █   █   ███   █████  █   █  █████  █   █",
        "  █   █  █   █  █      ██  █    █    █   █",
        "  █ █ █  █      ███    █ █ █    █     ███ ",
        "  █ █ █  █  ██  █      █  ██    █      █  ",
        "   █ █    ████  █████  █   █    █      █  ",
    ];

    let gradient = [
        Color::Rgb(220, 180, 255),
        Color::Rgb(200, 160, 240),
        Color::Rgb(170, 130, 220),
        Color::Rgb(140, 100, 195),
        Color::Rgb(115, 80, 170),
        Color::Rgb(100, 60, 150),
    ];

    let mut lines: Vec<Line> = Vec::new();

    // Gradient ASCII logo
    for (i, logo_line) in logo_lines.iter().enumerate() {
        lines.push(Line::from(Span::styled(
            *logo_line,
            Style::default()
                .fg(gradient[i])
                .add_modifier(ratatui::style::Modifier::BOLD),
        )));
    }

    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::raw("        "),
        Span::styled(
            "Wgenty Code",
            Style::default()
                .fg(Color::Rgb(200, 150, 255))
                .add_modifier(ratatui::style::Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            "· Rust Edition",
            Style::default()
                .fg(Color::Rgb(255, 140, 66))
                .add_modifier(ratatui::style::Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(Span::styled(
        "           高性能 AI 编码助手",
        Style::default().fg(Color::Rgb(147, 112, 219)),
    )));
    lines.push(Line::from(Span::styled(
        format!("           Model: {}", model_name),
        Style::default().fg(Color::Rgb(140, 140, 160)),
    )));
    lines.push(Line::raw(""));

    // Comet workflow feature highlight
    lines.push(Line::from(Span::styled(
        "Comet spec-driven workflow · open → design → build → verify → archive",
        Style::default().fg(Color::Rgb(160, 140, 200)),
    )));
    lines.push(Line::raw(""));

    // Comet workflow onboarding
    lines.push(Line::from(Span::styled(
        "Type /comet to start a spec-driven workflow, or just begin typing.",
        Style::default().fg(Color::Rgb(150, 140, 185)),
    )));
    lines.push(Line::from(Span::styled(
        "/comet-tweak · small change   /comet-hotfix · urgent fix   /help · commands",
        Style::default().fg(Color::Rgb(150, 140, 185)),
    )));
    lines.push(Line::raw(""));

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(16), Constraint::Min(0)])
        .split(area);

    let para = Paragraph::new(Text::from(lines)).alignment(Alignment::Center);
    f.render_widget(para, layout[0]);
}
