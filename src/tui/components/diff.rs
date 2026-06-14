//! Rich Diff Rendering Component — unified diff with hunk headers, context,
//! line numbers, word-level highlighting, and stats.
//!
//! Produces output that looks like `git diff` (unified format):
//! ```text
//!  ▸ src/main.rs                                       +5 -3
//! @@ -10,7 +11,8 @@ fn main() {
//!       let x = 1;
//!   -   let y = old_value;
//!   +   let y = new_configured_value;
//!       println!("{}", x);
//!   }
//! ```
//!
//! Changed words within delete/insert lines are rendered with a brighter style.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use similar::{ChangeTag, TextDiff};

// ── Colors ──────────────────────────────────────────────────────────────────

const ADD_COLOR: Color = Color::Rgb(80, 200, 120);
const ADD_WORD_COLOR: Color = Color::Rgb(40, 255, 100);
const DEL_COLOR: Color = Color::Rgb(240, 100, 100);
const DEL_WORD_COLOR: Color = Color::Rgb(255, 70, 70);
const CTX_COLOR: Color = Color::Rgb(100, 100, 110);
const HUNK_COLOR: Color = Color::Rgb(60, 180, 180);
const HEADER_COLOR: Color = Color::Rgb(180, 180, 200);

const CONTEXT: usize = 3;
const MAX_STANDALONE: usize = 50;
const MAX_INLINE: usize = 25;

// ── Types ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineTag {
    Context,
    Delete,
    Insert,
}

#[derive(Debug, Clone)]
struct Segment {
    changed: bool,
    text: String,
}

struct DiffLine {
    tag: LineTag,
    old_no: Option<usize>,
    new_no: Option<usize>,
    text: String,
    segments: Vec<Segment>,
}

struct Hunk {
    old_start: usize,
    old_count: usize,
    new_start: usize,
    new_count: usize,
    lines: Vec<DiffLine>,
}

struct UnifiedDiff {
    file_path: String,
    hunks: Vec<Hunk>,
    additions: usize,
    deletions: usize,
}

// ── Diff Generation ─────────────────────────────────────────────────────────

fn generate_diff(old: &str, new: &str, file_path: &str) -> UnifiedDiff {
    let diff = TextDiff::from_lines(old, new);
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut total_add = 0usize;
    let mut total_del = 0usize;

    for group in diff.grouped_ops(CONTEXT) {
        if group.is_empty() {
            continue;
        }

        let first = &group[0];
        let last = &group[group.len() - 1];
        let old_start = first.old_range().start + 1;
        let new_start = first.new_range().start + 1;
        let old_count = last.old_range().end.saturating_sub(first.old_range().start);
        let new_count = last.new_range().end.saturating_sub(first.new_range().start);

        let mut hunk_lines: Vec<DiffLine> = Vec::new();
        let mut old_line = old_start;
        let mut new_line = new_start;

        for op in &group {
            for change in diff.iter_changes(op) {
                let tag = match change.tag() {
                    ChangeTag::Equal => LineTag::Context,
                    ChangeTag::Delete => LineTag::Delete,
                    ChangeTag::Insert => LineTag::Insert,
                };

                for line_text in change.value().lines() {
                    let (o_no, n_no) = match tag {
                        LineTag::Context => (Some(old_line), Some(new_line)),
                        LineTag::Delete => {
                            total_del += 1;
                            (Some(old_line), None)
                        }
                        LineTag::Insert => {
                            total_add += 1;
                            (None, Some(new_line))
                        }
                    };

                    hunk_lines.push(DiffLine {
                        tag,
                        old_no: o_no,
                        new_no: n_no,
                        text: line_text.to_string(),
                        segments: Vec::new(),
                    });

                    if tag != LineTag::Insert {
                        old_line += 1;
                    }
                    if tag != LineTag::Delete {
                        new_line += 1;
                    }
                }
            }
        }

        compute_word_diffs(&mut hunk_lines);

        hunks.push(Hunk {
            old_start,
            old_count,
            new_start,
            new_count,
            lines: hunk_lines,
        });
    }

    UnifiedDiff {
        file_path: file_path.to_string(),
        hunks,
        additions: total_add,
        deletions: total_del,
    }
}

/// Word-level highlighting for paired delete/insert lines.
fn compute_word_diffs(lines: &mut [DiffLine]) {
    let mut i = 0;
    while i < lines.len() {
        // Skip past context lines to find the next change group.
        while i < lines.len() && lines[i].tag == LineTag::Context {
            i += 1;
        }

        // Collect consecutive delete lines.
        let del_start = i;
        while i < lines.len() && lines[i].tag == LineTag::Delete {
            i += 1;
        }
        let del_end = i;

        // Collect consecutive insert lines.
        let ins_start = i;
        while i < lines.len() && lines[i].tag == LineTag::Insert {
            i += 1;
        }
        let ins_end = i;

        // If no delete/insert pair found, we're done with this group.
        if del_start == del_end || ins_start == ins_end {
            continue;
        }

        let n = del_end - del_start;
        for j in 0..n {
            let di = del_start + j;
            let ii = ins_start + j;
            if ii >= ins_end {
                break;
            }
            let dt = &lines[di].text;
            let it = &lines[ii].text;
            if dt == it {
                continue;
            }

            let wd = TextDiff::from_words(dt, it);
            let ds = segments_for_side(&wd, LineTag::Delete);
            let is = segments_for_side(&wd, LineTag::Insert);

            if !ds.is_empty() {
                lines[di].segments = ds;
            }
            if !is.is_empty() {
                lines[ii].segments = is;
            }
        }
    }
}

fn segments_for_side<'a>(wd: &TextDiff<'a, 'a, 'a, str>, side: LineTag) -> Vec<Segment> {
    let mut out = Vec::new();
    for change in wd.iter_all_changes() {
        let t = change.tag();
        let v = change.value();
        match (t, side) {
            (ChangeTag::Delete, LineTag::Delete) => out.push(Segment {
                changed: true,
                text: v.to_string(),
            }),
            (ChangeTag::Insert, LineTag::Insert) => out.push(Segment {
                changed: true,
                text: v.to_string(),
            }),
            (ChangeTag::Equal, _) => out.push(Segment {
                changed: false,
                text: v.to_string(),
            }),
            (_, _) => {}
        }
    }
    out
}

// ── Rendering ───────────────────────────────────────────────────────────────

fn render_line(line: &DiffLine, gutter_w: usize, width: u16, skip_gutter: bool) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let max_w = if skip_gutter {
        width as usize
    } else {
        width.saturating_sub(gutter_w as u16) as usize
    };

    if !skip_gutter {
        let marker = match line.tag {
            LineTag::Context => " ",
            LineTag::Delete => "-",
            LineTag::Insert => "+",
        };
        // gutter_w = 2*d + 3; each number column gets d chars
        let nw = (gutter_w.saturating_sub(3)) / 2;
        let o = match line.old_no {
            Some(n) => format!("{:>w$}", n, w = nw),
            None => " ".repeat(nw),
        };
        let n = match line.new_no {
            Some(n) => format!("{:>w$}", n, w = nw),
            None => " ".repeat(nw),
        };
        spans.push(Span::styled(
            format!("{o} {marker} {n}"),
            Style::default().fg(CTX_COLOR),
        ));
    }

    let (prefix, base, changed) = match line.tag {
        LineTag::Context => (
            "  ",
            Style::default().fg(CTX_COLOR),
            Style::default().fg(CTX_COLOR),
        ),
        LineTag::Delete => (
            "- ",
            Style::default().fg(DEL_COLOR),
            Style::default().fg(DEL_WORD_COLOR),
        ),
        LineTag::Insert => (
            "+ ",
            Style::default().fg(ADD_COLOR),
            Style::default().fg(ADD_WORD_COLOR),
        ),
    };

    if line.segments.is_empty() {
        spans.push(Span::styled(format!("{prefix}{}", line.text), base));
    } else {
        spans.push(Span::styled(prefix, base));
        for s in &line.segments {
            let st = if s.changed { changed } else { base };
            spans.push(Span::styled(s.text.clone(), st));
        }
    }

    let full: String = spans.iter().map(|s| s.content.as_ref()).collect();
    if full.len() > max_w && max_w > 4 {
        let t: String = full.chars().take(max_w.saturating_sub(3)).collect();
        return Line::from(Span::styled(format!("{t}..."), base));
    }
    Line::from(spans)
}

fn gutter_width(hunks: &[Hunk]) -> usize {
    let m = hunks
        .iter()
        .flat_map(|h| h.lines.iter())
        .flat_map(|l| [l.old_no, l.new_no])
        .flatten()
        .max()
        .unwrap_or(1);
    let d = m.to_string().len();
    d + 1 + d + 2
}

fn hunk_header(hunk: &Hunk) -> Line<'static> {
    Line::from(Span::styled(
        format!(
            "@@ -{},{} +{},{} @@",
            hunk.old_start, hunk.old_count, hunk.new_start, hunk.new_count
        ),
        Style::default().fg(HUNK_COLOR),
    ))
}

fn stats_line(path: &str, add: usize, del: usize) -> Line<'static> {
    let mut s = vec![Span::styled(
        format!("  \u{25B8} {path}"),
        Style::default().fg(HEADER_COLOR),
    )];
    if add > 0 || del > 0 {
        s.push(Span::styled(
            format!("  +{add}"),
            Style::default().fg(ADD_COLOR),
        ));
        s.push(Span::styled(
            format!(" -{del}"),
            Style::default().fg(DEL_COLOR),
        ));
    }
    Line::from(s)
}

fn render_unified(
    diff: &UnifiedDiff,
    width: u16,
    max_lines: usize,
    compact: bool,
) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    let gw = if compact {
        0
    } else {
        gutter_width(&diff.hunks)
    };

    out.push(stats_line(&diff.file_path, diff.additions, diff.deletions));
    let mut shown = 1usize;

    for hunk in &diff.hunks {
        if shown >= max_lines {
            out.push(Line::from(Span::styled(
                "  ... (truncated)",
                Style::default().fg(CTX_COLOR),
            )));
            break;
        }
        out.push(hunk_header(hunk));
        shown += 1;

        for line in &hunk.lines {
            if shown >= max_lines {
                let rem = hunk.lines.len().saturating_sub(shown.saturating_sub(1));
                out.push(Line::from(Span::styled(
                    format!("  ... ({rem} more lines)"),
                    Style::default().fg(CTX_COLOR),
                )));
                break;
            }
            out.push(render_line(line, gw, width, compact));
            shown += 1;
        }
    }

    if diff.hunks.is_empty() {
        out.push(Line::from(Span::styled(
            "  (no changes detected)",
            Style::default().fg(CTX_COLOR),
        )));
    }

    out
}

// ── Public API ──────────────────────────────────────────────────────────────

/// Render a rich unified diff as a ratatui Paragraph in the given area.
pub fn render(f: &mut Frame, area: Rect, file_path: &str, old: &str, new: &str) -> u16 {
    let diff = generate_diff(old, new, file_path);
    let lines = render_unified(&diff, area.width, MAX_STANDALONE, false);
    let n = lines.len() as u16;
    f.render_widget(Paragraph::new(ratatui::text::Text::from(lines)), area);
    n
}

/// Convert diff data into ratatui Lines for inline rendering in chat.
pub fn diff_to_lines(file_path: &str, old: &str, new: &str, width: u16) -> Vec<Line<'static>> {
    let diff = generate_diff(old, new, file_path);
    let mut lines = render_unified(&diff, width, MAX_INLINE, true);
    lines.push(Line::raw(""));
    lines
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty() {
        let d = generate_diff("a\nb\n", "a\nb\n", "f");
        assert!(d.hunks.is_empty());
    }

    #[test]
    fn simple() {
        let d = generate_diff("a\nb\nc\n", "a\nX\nc\n", "f");
        assert_eq!(d.additions, 1);
        assert_eq!(d.deletions, 1);
        assert_eq!(d.hunks.len(), 1);
    }

    #[test]
    fn add_only() {
        let d = generate_diff("a\nb\n", "a\nx\nb\n", "f");
        assert_eq!(d.additions, 1);
        assert_eq!(d.deletions, 0);
    }

    #[test]
    fn del_only() {
        let d = generate_diff("a\nx\nb\n", "a\nb\n", "f");
        assert_eq!(d.deletions, 1);
        assert_eq!(d.additions, 0);
    }

    #[test]
    fn word_parts() {
        let d = generate_diff("let x = old;\n", "let x = new;\n", "f");
        let h = &d.hunks[0];
        assert!(!h.lines[0].segments.is_empty());
        assert!(h.lines[0].segments.iter().any(|s| s.changed));
    }

    #[test]
    fn multi_hunk() {
        // Two changes 8+ lines apart (> 2*CONTEXT) → separate hunks
        let d = generate_diff(
            "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\nn\n",
            "a\nb\nX\nd\ne\nf\ng\nh\ni\nj\nk\nY\nm\nn\n",
            "f",
        );
        assert_eq!(d.hunks.len(), 2);
    }

    #[test]
    fn render_output() {
        let d = generate_diff(
            "fn f() {\n  let x = 1;\n}\n",
            "fn f() {\n  let x = 2;\n}\n",
            "s.rs",
        );
        let ls = render_unified(&d, 80, 50, true);
        assert!(ls.len() >= 5);
    }

    #[test]
    fn hunk_fmt() {
        let d = generate_diff("a\nb\nc\n", "a\nX\nc\n", "f");
        let h = hunk_header(&d.hunks[0]);
        let t: String = h.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(t.starts_with("@@"));
        assert!(t.ends_with("@@"));
    }
}
