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

    pub fn height_needed(&self) -> u16 {
        if !self.visible || self.items.is_empty() {
            return 0;
        }

        (self.items.len() as u16 + 2).clamp(3, 8)
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

    let mut lines: Vec<Line<'static>> = Vec::new();

    for (i, item) in state.items.iter().enumerate() {
        let symbol = item.status.symbol();
        let color = item.status.color();
        let step_text = format!("  {}  {}. {}", symbol, i + 1, item.step);
        lines.push(Line::from(Span::styled(
            step_text,
            Style::default().fg(color),
        )));
    }

    let para = Paragraph::new(Text::from(lines)).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(HEADER_COLOR))
            .title(" Plan "),
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
}
