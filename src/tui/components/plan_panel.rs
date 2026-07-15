use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PlanItem {
    pub step: String,
    pub status: PlanStatus,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum PlanStatus {
    #[serde(rename = "pending")]
    Pending,
    #[serde(rename = "in_progress")]
    InProgress,
    #[serde(rename = "completed")]
    Completed,
}

impl PlanStatus {
    pub fn parse_status(s: &str) -> Self {
        match s.to_lowercase().trim() {
            "in_progress" | "in-progress" | "inprogress" => PlanStatus::InProgress,
            "completed" | "complete" | "done" => PlanStatus::Completed,
            _ => PlanStatus::Pending,
        }
    }

    pub fn symbol(&self) -> &'static str {
        match self {
            PlanStatus::Pending => "\u{25CB}",
            PlanStatus::InProgress => "\u{25D0}",
            PlanStatus::Completed => "\u{2713}",
        }
    }

    pub fn color(&self) -> Color {
        match self {
            PlanStatus::Pending => Color::Rgb(140, 140, 150),
            PlanStatus::InProgress => Color::Rgb(100, 200, 255),
            PlanStatus::Completed => Color::Rgb(80, 200, 120),
        }
    }
}

fn normalize_plan_index(index: usize, len: usize) -> Option<usize> {
    if index < len {
        Some(index)
    } else if index > 0 && index - 1 < len {
        Some(index - 1)
    } else {
        None
    }
}

#[derive(Debug, Clone)]
pub struct PlanPanelState {
    pub items: Vec<PlanItem>,
    pub visible: bool,
}

impl Default for PlanPanelState {
    fn default() -> Self {
        Self::new()
    }
}

impl PlanPanelState {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            visible: false,
        }
    }

    pub fn update(&mut self, items: Vec<PlanItem>) {
        self.items = items;
        self.visible = !self.items.is_empty();
    }

    pub fn apply_update_value(&mut self, value: &serde_json::Value) -> bool {
        let Some(plan_array) = value.get("plan").and_then(|plan| plan.as_array()) else {
            tracing::warn!(payload = %value, "Ignoring update_plan payload without plan array");
            return false;
        };

        let mut next_items = self.items.clone();
        let mut changed = false;
        let replace_entire_plan = plan_array.iter().any(|item| {
            item.get("step")
                .or_else(|| item.get("content"))
                .or_else(|| item.get("description"))
                .and_then(|step| step.as_str())
                .is_some()
        });

        if replace_entire_plan {
            let items: Vec<PlanItem> = plan_array
                .iter()
                .filter_map(|item| {
                    let step = item
                        .get("step")
                        .or_else(|| item.get("content"))
                        .or_else(|| item.get("description"))
                        .and_then(|step| step.as_str())?
                        .to_string();
                    let status = item
                        .get("status")
                        .and_then(|status| status.as_str())
                        .map(PlanStatus::parse_status)
                        .unwrap_or(PlanStatus::Pending);
                    Some(PlanItem { step, status })
                })
                .collect();

            if items.len() != plan_array.len() {
                tracing::warn!(payload = %value, "Some update_plan items were missing a step");
            }

            self.update(items);
            return true;
        }

        for item in plan_array {
            let status = item
                .get("status")
                .and_then(|status| status.as_str())
                .map(PlanStatus::parse_status);
            let index = item
                .get("index")
                .or_else(|| item.get("step_index"))
                .and_then(|index| index.as_u64())
                .and_then(|index| usize::try_from(index).ok());

            let Some(status) = status else {
                tracing::warn!(payload = %item, "Ignoring update_plan item without status");
                continue;
            };

            let Some(index) = index else {
                tracing::warn!(payload = %item, "Ignoring status-only update_plan item without index");
                continue;
            };

            let Some(normalized_index) = normalize_plan_index(index, next_items.len()) else {
                tracing::warn!(
                    index,
                    len = next_items.len(),
                    "Ignoring update_plan item with out-of-range index"
                );
                continue;
            };

            let Some(plan_item) = next_items.get_mut(normalized_index) else {
                continue;
            };

            plan_item.status = status;
            changed = true;
        }

        if changed {
            self.update(next_items);
        }
        changed
    }

    /// Height needed for the plan panel given the full terminal height.
    ///
    /// Grows with the number of steps, but reserves room for chat/status/input
    /// so a long plan cannot crush the rest of the UI. When there are more
    /// items than fit, [`render`] shows a scrolled window focused on the
    /// active step.
    pub fn height_needed(&self, terminal_height: u16) -> u16 {
        if !self.visible || self.items.is_empty() {
            return 0;
        }

        // Reserve roughly: chat(min 3) + status(1) + input(~6) + slack(2).
        let reserved = 12u16;
        let max_panel = terminal_height.saturating_sub(reserved).max(3);
        let desired = (self.items.len() as u16).saturating_add(2); // + borders
        desired.clamp(3, max_panel)
    }

    /// Index of the step the viewport should keep visible.
    /// Prefers the first `in_progress` item, then the first `pending`, else 0.
    pub fn focus_index(&self) -> usize {
        self.items
            .iter()
            .position(|item| item.status == PlanStatus::InProgress)
            .or_else(|| {
                self.items
                    .iter()
                    .position(|item| item.status == PlanStatus::Pending)
            })
            .unwrap_or(0)
    }

    /// Inclusive-exclusive window of items that fit in `content_rows`.
    pub fn visible_window(&self, content_rows: usize) -> (usize, usize) {
        let len = self.items.len();
        if content_rows == 0 || len == 0 {
            return (0, 0);
        }
        if len <= content_rows {
            return (0, len);
        }

        let focus = self.focus_index();
        // Bias the window so the active step sits near the upper third,
        // keeping nearby context above and more upcoming steps below.
        let mut start = focus.saturating_sub(content_rows / 3);
        if start + content_rows > len {
            start = len - content_rows;
        }
        (start, start + content_rows)
    }

    pub fn complete_active_items(&mut self) {
        if !self.visible {
            return;
        }

        for item in &mut self.items {
            if item.status == PlanStatus::InProgress {
                item.status = PlanStatus::Completed;
            }
        }
    }

    pub fn dismiss(&mut self) {
        self.visible = false;
        self.items.clear();
    }
}

const HEADER_COLOR: Color = Color::Rgb(147, 112, 219);

pub fn render(f: &mut Frame, state: &PlanPanelState, area: Rect) {
    if !state.visible || state.items.is_empty() {
        return;
    }

    // Inner content rows available after top/bottom borders.
    let content_rows = area.height.saturating_sub(2) as usize;
    let (start, end) = state.visible_window(content_rows);
    let total = state.items.len();
    let scrolled = start > 0 || end < total;

    let mut lines: Vec<Line<'static>> = Vec::new();
    for (i, item) in state.items.iter().enumerate().take(end).skip(start) {
        let symbol = item.status.symbol();
        let color = item.status.color();
        let step_text = format!("  {}  {}. {}", symbol, i + 1, item.step);
        lines.push(Line::from(Span::styled(
            step_text,
            Style::default().fg(color),
        )));
    }

    let title = if scrolled {
        format!(" Plan ({}/{}) ", end.min(total), total)
    } else {
        " Plan ".to_string()
    };

    let para = Paragraph::new(Text::from(lines)).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(HEADER_COLOR))
            .title(title),
    );
    f.render_widget(para, area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_update_value_replaces_full_plan() {
        let mut state = PlanPanelState::new();

        assert!(state.apply_update_value(&serde_json::json!({
            "plan": [
                {"step": "Inspect", "status": "completed"},
                {"step": "Fix", "status": "in_progress"}
            ]
        })));

        assert!(state.visible);
        assert_eq!(state.items.len(), 2);
        assert_eq!(state.items[0].status, PlanStatus::Completed);
        assert_eq!(state.items[1].status, PlanStatus::InProgress);
    }

    #[test]
    fn apply_update_value_accepts_step_aliases() {
        let mut state = PlanPanelState::new();

        assert!(state.apply_update_value(&serde_json::json!({
            "plan": [
                {"content": "Inspect", "status": "done"},
                {"description": "Fix", "status": "in-progress"}
            ]
        })));

        assert_eq!(state.items[0].step, "Inspect");
        assert_eq!(state.items[1].step, "Fix");
        assert_eq!(state.items[0].status, PlanStatus::Completed);
        assert_eq!(state.items[1].status, PlanStatus::InProgress);
    }

    #[test]
    fn apply_update_value_supports_status_only_index_updates() {
        let mut state = PlanPanelState::new();
        state.update(vec![
            PlanItem {
                step: "Inspect".to_string(),
                status: PlanStatus::InProgress,
            },
            PlanItem {
                step: "Fix".to_string(),
                status: PlanStatus::Pending,
            },
        ]);

        assert!(state.apply_update_value(&serde_json::json!({
            "plan": [
                {"index": 0, "status": "completed"},
                {"index": 1, "status": "in_progress"}
            ]
        })));

        assert_eq!(state.items[0].status, PlanStatus::Completed);
        assert_eq!(state.items[1].status, PlanStatus::InProgress);
    }

    #[test]
    fn apply_update_value_accepts_one_based_out_of_range_index() {
        let mut state = PlanPanelState::new();
        state.update(vec![PlanItem {
            step: "Inspect".to_string(),
            status: PlanStatus::Pending,
        }]);

        assert!(state.apply_update_value(&serde_json::json!({
            "plan": [
                {"step_index": 1, "status": "completed"}
            ]
        })));

        assert_eq!(state.items[0].status, PlanStatus::Completed);
    }

    #[test]
    fn apply_update_value_rejects_invalid_payload() {
        let mut state = PlanPanelState::new();

        assert!(!state.apply_update_value(&serde_json::json!({
            "items": []
        })));
        assert!(!state.visible);
    }

    fn sample_items(n: usize, in_progress_at: Option<usize>) -> Vec<PlanItem> {
        (0..n)
            .map(|i| {
                let status = match in_progress_at {
                    Some(idx) if idx == i => PlanStatus::InProgress,
                    Some(idx) if i < idx => PlanStatus::Completed,
                    _ => PlanStatus::Pending,
                };
                PlanItem {
                    step: format!("Step {i}"),
                    status,
                }
            })
            .collect()
    }

    #[test]
    fn height_needed_grows_with_items_and_terminal() {
        let mut state = PlanPanelState::new();
        state.update(sample_items(12, Some(3)));

        // Tall terminal: show all 12 items + 2 borders.
        assert_eq!(state.height_needed(40), 14);
        // Short terminal: capped by reserved space (40 - 12 = 28 would fit, but
        // with height 20 max panel is 8).
        assert_eq!(state.height_needed(20), 8);
        // Very short terminal still keeps a usable minimum.
        assert_eq!(state.height_needed(10), 3);
    }

    #[test]
    fn visible_window_focuses_in_progress() {
        let mut state = PlanPanelState::new();
        state.update(sample_items(10, Some(7)));

        // 4 rows available, focus at index 7 -> start near 7 - 4/3 = 6
        let (start, end) = state.visible_window(4);
        assert_eq!((start, end), (6, 10));
        assert!(start <= 7 && 7 < end);
    }

    #[test]
    fn visible_window_shows_all_when_fits() {
        let mut state = PlanPanelState::new();
        state.update(sample_items(3, Some(1)));
        assert_eq!(state.visible_window(6), (0, 3));
    }

    #[test]
    fn focus_index_prefers_in_progress_then_pending() {
        let mut state = PlanPanelState::new();
        state.update(sample_items(4, None));
        assert_eq!(state.focus_index(), 0); // all pending

        state.items[0].status = PlanStatus::Completed;
        state.items[2].status = PlanStatus::InProgress;
        assert_eq!(state.focus_index(), 2);

        state.items[2].status = PlanStatus::Completed;
        // first pending is index 1
        assert_eq!(state.focus_index(), 1);
    }
}
