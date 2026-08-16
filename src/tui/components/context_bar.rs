//! Context usage progress bar — renders a Unicode progress bar showing
//! how much of the context window has been consumed.

use ratatui::style::{Color, Style};
use ratatui::text::Span;

use crate::tui::theme;

const BAR_WIDTH: usize = 8;

/// Build a series of spans representing a context-usage indicator.
///
/// The bar is 8 characters wide (█ = filled, ░ = empty), followed by a space
/// and the percentage. Filled cells take the threshold color while empty
/// cells stay dim gray, so the fill level is readable at any usage (a
/// uniformly colored bar is indistinguishable at low ratios):
/// - Green  (< 50%)
/// - Yellow (50–80%)
/// - Red    (≥ 80%)
pub fn spans(used: usize, max: usize) -> Vec<Span<'static>> {
    let ratio = if max == 0 {
        0.0
    } else {
        used as f64 / max as f64
    };
    let clamped = ratio.clamp(0.0, 1.0);
    #[allow(clippy::cast_possible_truncation)] // clamped ∈ 0.0..=1.0, result ≤ 100
    let pct = (clamped * 100.0).round() as usize;
    // Any non-zero usage shows at least one filled cell (2% must not render
    // as an empty bar), never more than BAR_WIDTH.
    #[allow(clippy::cast_possible_truncation)] // clamped ∈ 0.0..=1.0, ceil ≤ BAR_WIDTH
    let filled = if clamped == 0.0 {
        0
    } else {
        ((clamped * BAR_WIDTH as f64).ceil() as usize).min(BAR_WIDTH)
    };
    let empty = BAR_WIDTH - filled;

    let color = color_for_ratio(clamped);

    vec![
        Span::styled("█".repeat(filled), Style::default().fg(color)),
        Span::styled("░".repeat(empty), Style::default().fg(theme::DIM)),
        Span::styled(format!(" {pct}%"), Style::default().fg(color)),
    ]
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

    fn text(spans: &[Span<'static>]) -> String {
        spans.iter().map(|s| s.content.to_string()).collect()
    }

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
        assert_eq!(result.len(), 3);
        let content = text(&result);
        assert!(content.contains("0%"));
        assert_eq!(content.matches("░").count(), BAR_WIDTH);
        assert_eq!(content.matches("█").count(), 0);
    }

    #[test]
    fn test_spans_low_usage_shows_one_filled_cell() {
        // 2% must not render as an all-empty bar: ceil → 1 filled cell.
        let result = spans(4_000, 200_000);
        let content = text(&result);
        assert_eq!(content.matches("█").count(), 1);
        assert_eq!(content.matches("░").count(), BAR_WIDTH - 1);
        assert!(content.contains("2%"));
    }

    #[test]
    fn test_spans_full_usage() {
        let result = spans(200_000, 200_000);
        let content = text(&result);
        assert!(content.contains("100%"));
        assert_eq!(content.matches("█").count(), BAR_WIDTH);
        assert_eq!(content.matches("░").count(), 0);
    }

    #[test]
    fn test_spans_max_zero() {
        let result = spans(500, 0);
        assert!(text(&result).contains("0%"));
    }

    #[test]
    fn test_empty_cells_are_dim_independent_of_threshold() {
        // 60% (yellow threshold): filled cells yellow, empty cells still DIM.
        let result = spans(120_000, 200_000);
        assert_eq!(result[0].style.fg, Some(Color::Yellow));
        assert_eq!(result[1].style.fg, Some(theme::DIM));
        assert_eq!(result[2].style.fg, Some(Color::Yellow));
    }
}
