//! Context usage progress bar — renders a Unicode progress bar showing
//! how much of the context window has been consumed.

use ratatui::style::Color;
use ratatui::text::Span;

const BAR_WIDTH: usize = 8;

/// Build a series of spans representing a context-usage indicator.
///
/// The bar is 8 characters wide (▓ = filled, ░ = empty), followed by a space
/// and the percentage. Color changes with usage ratio:
/// - Green  (< 50%)
/// - Yellow (50–80%)
/// - Red    (> 80%)
pub fn spans(used: usize, max: usize) -> Vec<Span<'static>> {
    let ratio = if max == 0 {
        0.0
    } else {
        used as f64 / max as f64
    };
    let clamped = ratio.clamp(0.0, 1.0);
    #[allow(clippy::cast_possible_truncation)] // clamped ∈ 0.0..=1.0, result ≤ 100
    let pct = (clamped * 100.0).round() as usize;
    #[allow(clippy::cast_possible_truncation)] // clamped ∈ 0.0..=1.0, result ≤ BAR_WIDTH
    let filled = (clamped * BAR_WIDTH as f64).round() as usize;
    let empty = BAR_WIDTH - filled;

    let color = color_for_ratio(clamped);

    let bar: String = "▓".repeat(filled) + "░".repeat(empty).as_str();
    let label = format!(" {} {}%", bar, pct);

    vec![Span::styled(
        label,
        ratatui::style::Style::default().fg(color),
    )]
}

fn color_for_ratio(ratio: f64) -> Color {
    if ratio >= 0.8 {
        Color::Red
    } else if ratio >= 0.5 {
        Color::Yellow
    } else {
        Color::Green
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_color_green_below_50() {
        assert_eq!(color_for_ratio(0.0), Color::Green);
        assert_eq!(color_for_ratio(0.49), Color::Green);
    }

    #[test]
    fn test_color_yellow_50_to_80() {
        assert_eq!(color_for_ratio(0.5), Color::Yellow);
        assert_eq!(color_for_ratio(0.79), Color::Yellow);
    }

    #[test]
    fn test_color_red_above_80() {
        assert_eq!(color_for_ratio(0.8), Color::Red);
        assert_eq!(color_for_ratio(1.0), Color::Red);
    }

    #[test]
    fn test_spans_zero_usage() {
        let result = spans(0, 200_000);
        assert_eq!(result.len(), 1);
        // Should contain 0% and 8 empty blocks
        let content = &result[0].content;
        assert!(content.contains("0%"));
        assert!(content.contains("░"));
    }

    #[test]
    fn test_spans_full_usage() {
        let result = spans(200_000, 200_000);
        let content = &result[0].content;
        assert!(content.contains("100%"));
        assert_eq!(content.matches("▓").count(), 8);
    }

    #[test]
    fn test_spans_max_zero() {
        let result = spans(500, 0);
        let content = &result[0].content;
        assert!(content.contains("0%"));
    }
}
