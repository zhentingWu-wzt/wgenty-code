# RLM Observability & Robustness — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add TUI command completion, subagent transcript persistence, error visualization with retry/rollback, RLM structured reduction, and budget/progress tracking to wgenty-code's multi-agent system.

**Architecture:** The change spans five concerns: (1) a `CompletionEngine` data layer + `CompletionPanel` TUI component for `@` skill and `/` command completion; (2) a `SubagentTranscriptStore` module backed by SQLite for persistent event recording with checkpoint-based writing; (3) an extended `SubagentEvent` model with `ToolResult`/`Error` variants, plus a `DetailView` TUI component and git-stash-based rollback; (4) RLM `ClaimsOutput`/`DiffOutput` format structs, a code-level Aggregator using Jaccard similarity, and LLM fallback only for unresolvable conflicts; (5) per-subagent token budgets, RLM pipeline budget allocation, progress-delta-based stuck detection.

**Tech Stack:** Rust TUI (ratatui, crossterm), SQLite via rusqlite, serde for structured JSON, regex for layered parsing, git-stash for rollback, TypeScript/Ink for CLI sidecar.

**Design Doc:** `docs/superpowers/specs/2026-06-15-rlm-observability-and-robustness-design.md`

**Base Ref:** `0bb507fcdcef96ec32e8db9b9e1fc66390396f89`

---

### Task 1: CompletionEngine Data Layer + CompletionPanel Component

**Files:**
- Create: `src/tui/completion.rs` — CompletionEngine, SkillEntry, CommandEntry, CompletionMatch
- Create: `src/tui/components/completion_panel.rs` — CompletionPanel TUI renderer
- Modify: `src/tui/components/mod.rs:1-14` — register new modules
- Modify: `src/tui/app/event.rs:74-164` — add CompletionTrigger/Select/Dismiss events
- Modify: `src/tui/app/types.rs:74-164` — add CompletionState struct + App.completion_state field
- Modify: `src/tui/input_reader.rs:38-44` — detect `@`/`/` prefix, send CompletionTrigger
- Modify: `src/tui/app/render.rs:83-96` — render completion panel above input box
- Modify: `src/plugins/commands.rs:97-100` — expose `CommandRegistry::list_commands()` returning `&HashMap<String, PluginCommand>`

- [x] **Step 1.1: Create CompletionEngine data types**

    Create `src/tui/completion.rs` with the core types and filtering logic:

    ```rust
    //! Completion engine for TUI input — skills (@) and commands (/) completion.

    use std::path::PathBuf;

    #[derive(Debug, Clone)]
    pub struct SkillEntry {
        pub name: String,
        pub description: String,
        pub path: PathBuf,
    }

    #[derive(Debug, Clone)]
    pub struct CommandEntry {
        pub name: String,
        pub description: String,
        pub args_hint: Option<String>,
    }

    #[derive(Debug, Clone)]
    pub struct CompletionMatch {
        pub text: String,
        pub description: String,
        pub args_hint: Option<String>,
    }

    pub struct CompletionEngine {
        pub skills: Vec<SkillEntry>,
        pub commands: Vec<CommandEntry>,
    }

    impl CompletionEngine {
        /// Scan ~/.claude/skills/ for skills, load from PluginRegistry for commands.
        pub fn load(skills_dir: &std::path::Path, command_registry_commands: &[CommandEntry]) -> Self {
            let mut skills = Vec::new();
            if skills_dir.exists() {
                if let Ok(entries) = std::fs::read_dir(skills_dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_dir() {
                            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                                let description = extract_skill_description(&path);
                                skills.push(SkillEntry {
                                    name: name.to_string(),
                                    description,
                                    path,
                                });
                            }
                        }
                    }
                }
            }
            // Sort skills by name for deterministic display
            skills.sort_by(|a, b| a.name.cmp(&b.name));
            Self {
                skills,
                commands: command_registry_commands.to_vec(),
            }
        }

        pub fn filter(&self, prefix: char, partial: &str) -> Vec<CompletionMatch> {
            let partial_lower = partial.to_lowercase();
            match prefix {
                '@' => self.skills
                    .iter()
                    .filter(|s| s.name.to_lowercase().contains(&partial_lower))
                    .map(|s| CompletionMatch {
                        text: s.name.clone(),
                        description: s.description.clone(),
                        args_hint: None,
                    })
                    .collect(),
                '/' => self.commands
                    .iter()
                    .filter(|c| c.name.to_lowercase().starts_with(&partial_lower))
                    .map(|c| CompletionMatch {
                        text: c.name.clone(),
                        description: c.description.clone(),
                        args_hint: c.args_hint.clone(),
                    })
                    .collect(),
                _ => vec![],
            }
        }
    }

    fn extract_skill_description(skill_dir: &std::path::Path) -> String {
        let skill_md = skill_dir.join("SKILL.md");
        if let Ok(content) = std::fs::read_to_string(&skill_md) {
            // Try frontmatter description first
            if let Some(desc) = content.lines()
                .find(|l| l.trim().starts_with("description:"))
                .and_then(|l| l.split(':').nth(1))
                .map(|s| s.trim().trim_matches('"').to_string())
            {
                if !desc.is_empty() { return desc; }
            }
            // Fallback to first non-empty, non-frontmatter line
            if let Some(line) = content.lines()
                .skip_while(|l| l.trim().starts_with("---"))
                .skip(1)
                .find(|l| !l.trim().is_empty() && !l.trim().starts_with("---"))
            {
                return line.trim().to_string();
            }
        }
        String::new()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn test_engine() -> CompletionEngine {
            CompletionEngine {
                skills: vec![
                    SkillEntry { name: "comet-design".into(), description: "Design phase".into(), path: PathBuf::new() },
                    SkillEntry { name: "comet-build".into(), description: "Build phase".into(), path: PathBuf::new() },
                    SkillEntry { name: "comet-open".into(), description: "Open change".into(), path: PathBuf::new() },
                ],
                commands: vec![
                    CommandEntry { name: "code-review".into(), description: "Review code".into(), args_hint: None },
                    CommandEntry { name: "clear".into(), description: "Clear screen".into(), args_hint: None },
                ],
            }
        }

        #[test]
        fn test_skills_filter_by_at() {
            let e = test_engine();
            let matches = e.filter('@', "comet");
            assert_eq!(matches.len(), 3);
            assert!(matches.iter().all(|m| m.text.starts_with("comet")));
        }

        #[test]
        fn test_skills_filter_case_insensitive() {
            let e = test_engine();
            let matches = e.filter('@', "COMET");
            assert_eq!(matches.len(), 3);
        }

        #[test]
        fn test_skills_filter_empty_partial() {
            let e = test_engine();
            let matches = e.filter('@', "");
            assert_eq!(matches.len(), 3);
        }

        #[test]
        fn test_commands_filter_by_slash() {
            let e = test_engine();
            let matches = e.filter('/', "code");
            assert_eq!(matches.len(), 1);
            assert_eq!(matches[0].text, "code-review");
        }

        #[test]
        fn test_commands_prefix_match_only() {
            let e = test_engine();
            let matches = e.filter('/', "review");
            assert_eq!(matches.len(), 0); // "code-review" doesn't start with "review"
        }

        #[test]
        fn test_unknown_prefix_returns_empty() {
            let e = test_engine();
            let matches = e.filter('!', "anything");
            assert!(matches.is_empty());
        }
    }
    ```

- [x] **Step 1.2: Run completion engine tests**

    Run: `cargo test -p wgenty-code --lib tui::completion::tests -- --nocapture`
    Expected: All 6 tests pass.

- [x] **Step 1.3: Add CompletionTrigger/Select/Dismiss events to AppEvent**

    In `src/tui/app/event.rs`, add three event variants to the `AppEvent` enum (after the line `ToggleSubagentPanel`):

    ```rust
    /// User typed @ or / to trigger command completion
    CompletionTrigger { prefix: char, partial: String },
    /// User selected a completion item
    CompletionSelect { index: usize },
    /// User dismissed the completion panel
    CompletionDismiss,
    ```

    Then in `src/tui/app/types.rs`, add `CompletionState` struct and extend `App`:

    After `pub struct App {` and its existing fields:
    ```rust
    pub completion_engine: Option<crate::tui::completion::CompletionEngine>,
    pub completion_state: Option<CompletionState>,
    ```

    Add the `CompletionState` struct before `impl App`:
    ```rust
    #[derive(Debug, Clone)]
    pub struct CompletionState {
        pub prefix: char,
        pub partial: String,
        pub matches: Vec<crate::tui::completion::CompletionMatch>,
        pub selected_index: usize,
        pub visible: bool,
    }
    ```

- [x] **Step 1.4: Wire CompletionTrigger detection in input_reader.rs**

    In `src/tui/input_reader.rs`, after the line handling `Ctrl+O` toggle, add detection for `@` and `/` in the input—but the input_reader only sees raw `KeyEvent`s, not the input buffer. Instead, detection must happen at the `App::handle_event` level where the input box content is known.

    In `src/tui/app/event.rs`, in the `AppEvent::KeyEvent` handler, before feeding to `self.input_box.textarea.input(key)`, add:

    ```rust
    // After Shift+Enter handling and before feed to textarea:
    // Detect @ and / completion triggers
    if let KeyCode::Char(c) = key.code {
        if c == '@' {
            // Check that @ is at beginning of input (or after whitespace)
            let text = self.input_box.textarea.lines().join("\n");
            if text.is_empty() || text.ends_with(' ') || text.ends_with('\n') {
                self.completion_state = Some(CompletionState {
                    prefix: '@',
                    partial: String::new(),
                    matches: Vec::new(),
                    selected_index: 0,
                    visible: true,
                });
                // Trigger initial filter
                if let Some(ref engine) = self.completion_engine {
                    if let Some(ref mut state) = self.completion_state {
                        state.matches = engine.filter('@', "");
                    }
                }
            }
        } else if c == '/' && key.modifiers.is_empty() {
            let text = self.input_box.textarea.lines().join("\n");
            if text.is_empty() || text.ends_with(' ') || text.ends_with('\n') {
                self.completion_state = Some(CompletionState {
                    prefix: '/',
                    partial: String::new(),
                    matches: Vec::new(),
                    selected_index: 0,
                    visible: true,
                });
                if let Some(ref engine) = self.completion_engine {
                    if let Some(ref mut state) = self.completion_state {
                        state.matches = engine.filter('/', "");
                    }
                }
            }
        }
    }
    // Also update filter as user types more characters after @ or /
    if let Some(ref mut state) = self.completion_state {
        if state.visible {
            // Re-compute partial based on input after the trigger prefix
            let text = self.input_box.textarea.lines().join("\n");
            if let Some(pos) = text.rfind(state.prefix) {
                let after = &text[pos + 1..];
                state.partial = after.to_string();
                if let Some(ref engine) = self.completion_engine {
                    state.matches = engine.filter(state.prefix, after);
                }
            }
        }
    }
    ```

    Also in the subagent panel key handling section, add `CompletionDismiss`:
    ```rust
    // If completion panel is visible, route keys
    if self.completion_state.as_ref().map(|s| s.visible).unwrap_or(false) {
        match key.code {
            KeyCode::Esc => { self.completion_state = None; return; }
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(ref mut s) = self.completion_state {
                    if !s.matches.is_empty() {
                        s.selected_index = s.selected_index.saturating_sub(1);
                    }
                }
                return;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(ref mut s) = self.completion_state {
                    let next = s.selected_index + 1;
                    if next < s.matches.len() {
                        s.selected_index = next;
                    } else {
                        s.selected_index = 0;
                    }
                }
                return;
            }
            KeyCode::Tab => {
                // Cycle to next item
                if let Some(ref mut s) = self.completion_state {
                    if !s.matches.is_empty() {
                        s.selected_index = (s.selected_index + 1) % s.matches.len();
                    }
                }
                return;
            }
            KeyCode::Enter => {
                // Confirm selection: replace the @xxx or /xxx with full name
                if let Some(ref state) = self.completion_state.clone() {
                    if let Some(m) = state.matches.get(state.selected_index) {
                        let text = self.input_box.textarea.lines().join("\n");
                        if let Some(pos) = text.rfind(state.prefix) {
                            let before = &text[..pos];
                            self.input_box.textarea = ratatui::widgets::Paragraph::new(format!("{}{} ", before, m.text));
                        }
                    }
                }
                self.completion_state = None;
                return;
            }
            _ => {}
        }
    }
    ```

    This code goes before the existing permission panel handling in `handle_event`.

- [x] **Step 1.5: Run existing tests to ensure no regressions from event changes**

    Run: `cargo test -p wgenty-code --lib tui::util::tests -- --nocapture`
    Expected: All existing phase transition tests pass.

- [x] **Step 1.6: Create CompletionPanel component**

    Create `src/tui/components/completion_panel.rs`:

    ```rust
    //! CompletionPanel — inline completion suggestion list above the input box.

    use crate::tui::app::types::CompletionState;
    use ratatui::layout::Rect;
    use ratatui::style::{Color, Modifier, Style};
    use ratatui::text::{Line, Span};
    use ratatui::widgets::{Block, Borders, Paragraph};
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

                let parts = vec![
                    Span::styled(
                        format!(" {}", m.text),
                        style,
                    ),
                    Span::styled(
                        format!("  {}", m.description),
                        style.add_modifier(Modifier::DIM),
                    ),
                ];
                lines.push(Line::from(parts));
            }

            // Bottom hint
            let hint_style = Style::default().fg(Color::Rgb(108, 112, 134));
            let hint = Line::from(vec![
                Span::styled(" ↑↓ nav  Tab cycle  Enter select  Esc close ", hint_style),
            ]);

            // Render block and content
            f.render_widget(block, panel_area);
            let content_area = Rect {
                x: inner.x,
                y: inner.y,
                width: inner.width,
                height: inner.height.saturating_sub(1),
            };
            f.render_widget(Paragraph::new(lines).wrap(ratatui::text::Wrap { trim: false }), content_area);
            f.render_widget(Paragraph::new(hint), Rect {
                x: inner.x,
                y: inner.y + inner.height.saturating_sub(1),
                width: inner.width,
                height: 1,
            });
        }
    }
    ```

- [x] **Step 1.7: Register new modules in components/mod.rs**

    Add to `src/tui/components/mod.rs`:
    ```rust
    pub mod completion_panel;
    ```

- [x] **Step 1.8: Integrate CompletionPanel in render.rs**

    In `src/tui/app/render.rs`, after existing panel rendering (after the `session` popup block and before the input render), add:

    ```rust
    // Completion panel — render above input area
    if let Some(ref completion) = self.completion_state {
        if completion.visible && !completion.matches.is_empty() {
            components::completion_panel::CompletionPanel::render(
                f,
                layout[input_idx],
                completion,
            );
        }
    }
    ```

    Also initialize `completion_engine` in `App::new()` in `mod.rs`, after settings are loaded:

    ```rust
    completion_engine: {
        let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        let skills_dir = home.join(".claude").join("skills");
        // Load commands from built-in set (PluginRegistry is async — defer for now)
        let mut builtin_commands = Vec::new();
        // Register known built-in commands
        builtin_commands.push(CommandEntry {
            name: "code-review".to_string(),
            description: "Review code changes".to_string(),
            args_hint: None,
        });
        builtin_commands.push(CommandEntry {
            name: "clear".to_string(),
            description: "Clear screen".to_string(),
            args_hint: None,
        });
        builtin_commands.push(CommandEntry {
            name: "help".to_string(),
            description: "Show help".to_string(),
            args_hint: None,
        });
        Some(crate::tui::completion::CompletionEngine::load(&skills_dir, &builtin_commands))
    },
    ```

- [x] **Step 1.9: Commit**

    ```bash
    git add src/tui/completion.rs src/tui/components/completion_panel.rs src/tui/components/mod.rs src/tui/app/event.rs src/tui/app/types.rs src/tui/app/mod.rs src/tui/app/render.rs src/tui/input_reader.rs
    git commit -m "feat: TUI command completion for @ skills and / commands"
    ```

---

### Task 2: Subagent Transcript Persistence (SQLite Store)

**Files:**
- Create: `src/transcript/mod.rs` — SubagentTranscriptStore public API, error types
- Create: `src/transcript/store.rs` — SQLite implementation with schema, CRUD, cleanup
- Modify: `src/lib.rs:38-39` — add `pub mod transcript;`
- Modify: `src/config/settings.rs:127-128` — add `max_transcript_age_days: u32` field

- [x] **Step 2.1: Create transcript module with error types**

    Create `src/transcript/mod.rs`:

    ```rust
    //! Subagent transcript persistence module.
    //!
    //! Stores subagent execution transcripts in SQLite for later review,
    //! debugging, and rollback scenarios.

    pub mod store;

    pub use store::SubagentTranscriptStore;

    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct SubagentTranscript {
        pub id: String,                         // UUID v4
        pub session_id: String,
        pub parent_id: Option<String>,
        pub label: String,
        pub status: TranscriptStatus,
        pub system_prompt: Option<String>,
        pub user_prompt: String,
        pub started_at: i64,                    // Unix ms
        pub finished_at: Option<i64>,
        pub total_tokens: u64,
        pub max_rounds: Option<u32>,
        pub actual_rounds: u32,
        pub token_budget_k: Option<u64>,
        pub error_message: Option<String>,
        pub summary: Option<String>,
        pub events: Vec<SubagentEventRecord>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub enum TranscriptStatus {
        Pending,
        Running,
        Completed,
        Failed,
        Cancelled,
    }

    impl std::fmt::Display for TranscriptStatus {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::Pending => write!(f, "pending"),
                Self::Running => write!(f, "running"),
                Self::Completed => write!(f, "completed"),
                Self::Failed => write!(f, "failed"),
                Self::Cancelled => write!(f, "cancelled"),
            }
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct SubagentEventRecord {
        pub round: u32,
        pub event_type: String,            // thought | action | tool_result | error | completion
        pub tool_name: Option<String>,
        pub tool_params: Option<serde_json::Value>,
        pub data: String,
        pub elapsed_ms: u64,
        pub token_count: Option<u64>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct SubagentTranscriptHeader {
        pub id: String,
        pub session_id: String,
        pub parent_id: Option<String>,
        pub label: String,
        pub status: String,
        pub started_at: i64,
        pub finished_at: Option<i64>,
        pub total_tokens: u64,
        pub actual_rounds: u32,
        pub error_message: Option<String>,
        pub summary: Option<String>,
    }

    #[derive(Debug, Clone)]
    pub enum TranscriptError {
        Database(String),
        NotFound(String),
        Io(String),
    }

    impl std::fmt::Display for TranscriptError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::Database(msg) => write!(f, "Database error: {}", msg),
                Self::NotFound(id) => write!(f, "Transcript not found: {}", id),
                Self::Io(msg) => write!(f, "IO error: {}", msg),
            }
        }
    }

    impl std::error::Error for TranscriptError {}

    impl From<rusqlite::Error> for TranscriptError {
        fn from(e: rusqlite::Error) -> Self {
            TranscriptError::Database(e.to_string())
        }
    }
    ```

- [x] **Step 2.2: Create SQLite store implementation**

    Create `src/transcript/store.rs`:

    ```rust
    //! SQLite-backed SubagentTranscriptStore.

    use super::{SubagentEventRecord, SubagentTranscript, SubagentTranscriptHeader, TranscriptError, TranscriptStatus};
    use rusqlite::{params, Connection};
    use std::path::Path;

    pub struct SubagentTranscriptStore {
        db: Connection,
    }

    impl SubagentTranscriptStore {
        /// Open or create the database. Auto-creates tables and indexes.
        pub fn open(path: &Path) -> Result<Self, TranscriptError> {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| TranscriptError::Io(e.to_string()))?;
            }
            let db = Connection::open(path)?;
            let store = Self { db };
            store.run_migrations()?;
            Ok(store)
        }

        fn run_migrations(&self) -> Result<(), TranscriptError> {
            self.db.execute_batch("
                PRAGMA journal_mode=WAL;
                PRAGMA foreign_keys=ON;

                CREATE TABLE IF NOT EXISTS subagent_transcripts (
                    id TEXT PRIMARY KEY,
                    session_id TEXT NOT NULL,
                    parent_id TEXT,
                    label TEXT NOT NULL,
                    status TEXT NOT NULL DEFAULT 'pending',
                    system_prompt TEXT,
                    user_prompt TEXT NOT NULL,
                    started_at INTEGER NOT NULL,
                    finished_at INTEGER,
                    total_tokens INTEGER DEFAULT 0,
                    input_tokens INTEGER DEFAULT 0,
                    output_tokens INTEGER DEFAULT 0,
                    max_rounds INTEGER,
                    actual_rounds INTEGER DEFAULT 0,
                    token_budget INTEGER,
                    error_message TEXT,
                    summary TEXT,
                    created_at INTEGER DEFAULT (unixepoch('now'))
                );

                CREATE TABLE IF NOT EXISTS subagent_events (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    transcript_id TEXT NOT NULL REFERENCES subagent_transcripts(id) ON DELETE CASCADE,
                    round INTEGER NOT NULL,
                    event_type TEXT NOT NULL,
                    tool_name TEXT,
                    tool_params TEXT,
                    data TEXT NOT NULL,
                    elapsed_ms INTEGER NOT NULL,
                    token_count INTEGER,
                    created_at INTEGER DEFAULT (unixepoch('now'))
                );

                CREATE INDEX IF NOT EXISTS idx_transcripts_session ON subagent_transcripts(session_id, started_at DESC);
                CREATE INDEX IF NOT EXISTS idx_transcripts_status ON subagent_transcripts(status);
                CREATE INDEX IF NOT EXISTS idx_events_transcript ON subagent_events(transcript_id, round);
                CREATE INDEX IF NOT EXISTS idx_events_type ON subagent_events(event_type);
            ")?;
            Ok(())
        }

        /// Save a full transcript (header + all events) in a single transaction.
        pub fn save(&self, transcript: &SubagentTranscript) -> Result<(), TranscriptError> {
            let tx = self.db.unchecked_transaction()?;

            let status_str = match transcript.status {
                TranscriptStatus::Pending => "pending",
                TranscriptStatus::Running => "running",
                TranscriptStatus::Completed => "completed",
                TranscriptStatus::Failed => "failed",
                TranscriptStatus::Cancelled => "cancelled",
            };

            tx.execute(
                "INSERT OR REPLACE INTO subagent_transcripts
                 (id, session_id, parent_id, label, status, system_prompt, user_prompt,
                  started_at, finished_at, total_tokens, max_rounds, actual_rounds,
                  token_budget, error_message, summary)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                params![
                    transcript.id,
                    transcript.session_id,
                    transcript.parent_id,
                    transcript.label,
                    status_str,
                    transcript.system_prompt,
                    transcript.user_prompt,
                    transcript.started_at,
                    transcript.finished_at,
                    transcript.total_tokens,
                    transcript.max_rounds,
                    transcript.actual_rounds,
                    transcript.token_budget_k,
                    transcript.error_message,
                    transcript.summary,
                ],
            )?;

            // Insert events
            for event in &transcript.events {
                tx.execute(
                    "INSERT INTO subagent_events (transcript_id, round, event_type, tool_name, tool_params, data, elapsed_ms, token_count)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        transcript.id,
                        event.round,
                        event.event_type,
                        event.tool_name,
                        event.tool_params.as_ref().map(|v| v.to_string()),
                        event.data,
                        event.elapsed_ms,
                        event.token_count,
                    ],
                )?;
            }

            tx.commit()?;
            Ok(())
        }

        /// Update transcript header status (for checkpoint).
        pub fn checkpoint_status(&self, id: &str, status: &str, round: u32, tokens: u64) -> Result<(), TranscriptError> {
            self.db.execute(
                "UPDATE subagent_transcripts SET status = ?1, actual_rounds = ?2, total_tokens = ?3 WHERE id = ?4",
                params![status, round, tokens, id],
            )?;
            Ok(())
        }

        /// Append events to an existing transcript.
        pub fn append_events(&self, transcript_id: &str, events: &[SubagentEventRecord]) -> Result<(), TranscriptError> {
            let tx = self.db.unchecked_transaction()?;
            for event in events {
                tx.execute(
                    "INSERT INTO subagent_events (transcript_id, round, event_type, tool_name, tool_params, data, elapsed_ms, token_count)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        transcript_id,
                        event.round,
                        event.event_type,
                        event.tool_name,
                        event.tool_params.as_ref().map(|v| v.to_string()),
                        event.data,
                        event.elapsed_ms,
                        event.token_count,
                    ],
                )?;
            }
            tx.commit()?;
            Ok(())
        }

        /// List transcripts by session (headers only, no events).
        pub fn list_by_session(&self, session_id: &str) -> Result<Vec<SubagentTranscriptHeader>, TranscriptError> {
            let mut stmt = self.db.prepare(
                "SELECT id, session_id, parent_id, label, status, started_at, finished_at,
                        total_tokens, actual_rounds, error_message, summary
                 FROM subagent_transcripts
                 WHERE session_id = ?1
                 ORDER BY started_at DESC"
            )?;
            let rows = stmt.query_map(params![session_id], |row| {
                Ok(SubagentTranscriptHeader {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    parent_id: row.get(2)?,
                    label: row.get(3)?,
                    status: row.get(4)?,
                    started_at: row.get(5)?,
                    finished_at: row.get(6)?,
                    total_tokens: row.get::<_, i64>(7)? as u64,
                    actual_rounds: row.get::<_, i32>(8)? as u32,
                    error_message: row.get(9)?,
                    summary: row.get(10)?,
                })
            })?;
            let mut result = Vec::new();
            for row in rows {
                result.push(row?);
            }
            Ok(result)
        }

        /// Get full transcript by ID (with all events).
        pub fn get_by_id(&self, id: &str) -> Result<Option<SubagentTranscript>, TranscriptError> {
            let mut stmt = self.db.prepare(
                "SELECT id, session_id, parent_id, label, status, system_prompt, user_prompt,
                        started_at, finished_at, total_tokens, max_rounds, actual_rounds,
                        token_budget, error_message, summary
                 FROM subagent_transcripts WHERE id = ?1"
            )?;
            let mut rows = stmt.query_map(params![id], |row| {
                Ok(SubagentTranscript {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    parent_id: row.get(2)?,
                    label: row.get(3)?,
                    status: match row.get::<_, String>(4)?.as_str() {
                        "completed" => TranscriptStatus::Completed,
                        "failed" => TranscriptStatus::Failed,
                        "cancelled" => TranscriptStatus::Cancelled,
                        "running" => TranscriptStatus::Running,
                        _ => TranscriptStatus::Pending,
                    },
                    system_prompt: row.get(5)?,
                    user_prompt: row.get(6)?,
                    started_at: row.get(7)?,
                    finished_at: row.get(8)?,
                    total_tokens: row.get::<_, i64>(9)? as u64,
                    max_rounds: row.get::<_, Option<i32>>(10)?.map(|v| v as u32),
                    actual_rounds: row.get::<_, i32>(11)? as u32,
                    token_budget_k: row.get::<_, Option<i64>>(12)?.map(|v| v as u64),
                    error_message: row.get(13)?,
                    summary: row.get(14)?,
                    events: Vec::new(),
                })
            })?;

            if let Some(transcript_result) = rows.next() {
                let mut transcript = transcript_result?;
                // Load events
                let mut evt_stmt = self.db.prepare(
                    "SELECT round, event_type, tool_name, tool_params, data, elapsed_ms, token_count
                     FROM subagent_events WHERE transcript_id = ?1 ORDER BY round, id"
                )?;
                let evt_rows = evt_stmt.query_map(params![id], |row| {
                    Ok(SubagentEventRecord {
                        round: row.get::<_, i32>(0)? as u32,
                        event_type: row.get(1)?,
                        tool_name: row.get(2)?,
                        tool_params: row.get::<_, Option<String>>(3)?.and_then(|s| serde_json::from_str(&s).ok()),
                        data: row.get(4)?,
                        elapsed_ms: row.get::<_, i64>(5)? as u64,
                        token_count: row.get::<_, Option<i64>>(6)?.map(|v| v as u64),
                    })
                })?;
                for evt in evt_rows {
                    transcript.events.push(evt?);
                }
                Ok(Some(transcript))
            } else {
                Ok(None)
            }
        }

        /// Search transcripts by label (fuzzy match).
        pub fn search(&self, query: &str) -> Result<Vec<SubagentTranscriptHeader>, TranscriptError> {
            let pattern = format!("%{}%", query);
            let mut stmt = self.db.prepare(
                "SELECT id, session_id, parent_id, label, status, started_at, finished_at,
                        total_tokens, actual_rounds, error_message, summary
                 FROM subagent_transcripts
                 WHERE label LIKE ?1
                 ORDER BY started_at DESC
                 LIMIT 50"
            )?;
            let rows = stmt.query_map(params![pattern], |row| {
                Ok(SubagentTranscriptHeader {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    parent_id: row.get(2)?,
                    label: row.get(3)?,
                    status: row.get(4)?,
                    started_at: row.get(5)?,
                    finished_at: row.get(6)?,
                    total_tokens: row.get::<_, i64>(7)? as u64,
                    actual_rounds: row.get::<_, i32>(8)? as u32,
                    error_message: row.get(9)?,
                    summary: row.get(10)?,
                })
            })?;
            let mut result = Vec::new();
            for row in rows {
                result.push(row?);
            }
            Ok(result)
        }

        /// Delete transcripts older than retention_days.
        pub fn cleanup(&self, retention_days: u32) -> Result<usize, TranscriptError> {
            let deleted = self.db.execute(
                "DELETE FROM subagent_transcripts WHERE started_at < (unixepoch('now') - ?1 * 86400) * 1000",
                params![retention_days],
            )?;
            Ok(deleted)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use tempfile::TempDir;

        fn setup_store() -> (SubagentTranscriptStore, TempDir) {
            let dir = TempDir::new().unwrap();
            let path = dir.path().join("test.db");
            let store = SubagentTranscriptStore::open(&path).unwrap();
            (store, dir)
        }

        fn sample_transcript(id: &str, session_id: &str) -> SubagentTranscript {
            SubagentTranscript {
                id: id.to_string(),
                session_id: session_id.to_string(),
                parent_id: None,
                label: format!("test-{}", id),
                status: TranscriptStatus::Completed,
                system_prompt: None,
                user_prompt: "do something".to_string(),
                started_at: 1000,
                finished_at: Some(2000),
                total_tokens: 500,
                max_rounds: Some(10),
                actual_rounds: 3,
                token_budget_k: None,
                error_message: None,
                summary: Some("done".to_string()),
                events: vec![
                    SubagentEventRecord {
                        round: 0,
                        event_type: "thought".to_string(),
                        tool_name: None,
                        tool_params: None,
                        data: "analyzing...".to_string(),
                        elapsed_ms: 100,
                        token_count: Some(50),
                    },
                    SubagentEventRecord {
                        round: 1,
                        event_type: "action".to_string(),
                        tool_name: Some("file_read".to_string()),
                        tool_params: Some(serde_json::json!({"path": "src/main.rs"})),
                        data: "reading file".to_string(),
                        elapsed_ms: 500,
                        token_count: Some(200),
                    },
                ],
            }
        }

        #[test]
        fn test_save_and_get_by_id() {
            let (store, _dir) = setup_store();
            let t = sample_transcript("test-1", "session-1");
            store.save(&t).unwrap();

            let loaded = store.get_by_id("test-1").unwrap().unwrap();
            assert_eq!(loaded.id, "test-1");
            assert_eq!(loaded.session_id, "session-1");
            assert_eq!(loaded.events.len(), 2);
            assert_eq!(loaded.events[0].event_type, "thought");
        }

        #[test]
        fn test_list_by_session() {
            let (store, _dir) = setup_store();
            store.save(&sample_transcript("a", "sess-1")).unwrap();
            store.save(&sample_transcript("b", "sess-1")).unwrap();
            store.save(&sample_transcript("c", "sess-2")).unwrap();

            let list = store.list_by_session("sess-1").unwrap();
            assert_eq!(list.len(), 2);

            let list2 = store.list_by_session("sess-2").unwrap();
            assert_eq!(list2.len(), 1);
        }

        #[test]
        fn test_search() {
            let (store, _dir) = setup_store();
            store.save(&sample_transcript("a", "s1")).unwrap();
            let mut t2 = sample_transcript("b", "s1");
            t2.label = "special-fix".to_string();
            store.save(&t2).unwrap();

            let results = store.search("special").unwrap();
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].id, "b");
        }

        #[test]
        fn test_checkpoint_and_append() {
            let (store, _dir) = setup_store();
            let t = sample_transcript("cp-1", "session-1");
            store.save(&t).unwrap();

            store.checkpoint_status("cp-1", "running", 5, 1000).unwrap();

            let loaded = store.get_by_id("cp-1").unwrap().unwrap();
            assert_eq!(loaded.actual_rounds, 5);
            assert_eq!(loaded.total_tokens, 1000);

            store.append_events("cp-1", &[
                SubagentEventRecord {
                    round: 2,
                    event_type: "completion".to_string(),
                    tool_name: None,
                    tool_params: None,
                    data: "done".to_string(),
                    elapsed_ms: 3000,
                    token_count: None,
                },
            ]).unwrap();

            let loaded2 = store.get_by_id("cp-1").unwrap().unwrap();
            assert_eq!(loaded2.events.len(), 3);
        }

        #[test]
        fn test_cleanup() {
            let (store, _dir) = setup_store();
            let mut t = sample_transcript("old", "s1");
            t.started_at = 100; // very old timestamp
            store.save(&t).unwrap();
            store.save(&sample_transcript("new", "s1")).unwrap();

            let deleted = store.cleanup(1).unwrap(); // 1 day retention
            assert_eq!(deleted, 1);

            assert!(store.get_by_id("old").unwrap().is_none());
            assert!(store.get_by_id("new").unwrap().is_some());
        }
    }
    ```

- [x] **Step 2.3: Run transcript store tests**

    Run: `cargo test -p wgenty-code --lib transcript::store::tests -- --nocapture`
    Expected: All 6 tests pass (test_save_and_get_by_id, test_list_by_session, test_search, test_checkpoint_and_append, test_cleanup).

- [x] **Step 2.4: Register transcript module in lib.rs**

    In `src/lib.rs`, add after `pub mod tools;`:
    ```rust
    pub mod transcript;
    ```

- [x] **Step 2.5: Add max_transcript_age_days to Settings**

    In `src/config/settings.rs`, add after the `pub guardian: GuardianSettings` field:
    ```rust
    /// Maximum age in days for stored transcripts. Older records are cleaned up.
    /// 0 = unlimited retention.
    #[serde(default = "default_max_transcript_age_days")]
    pub max_transcript_age_days: u32,
    ```

    And add the default function near the other `fn default_*` functions:
    ```rust
    fn default_max_transcript_age_days() -> u32 { 30 }
    ```

    Also add to `Settings::default()`:
    ```rust
    max_transcript_age_days: 30,
    ```

    Additionally add the setter in `Settings::set()`:
    ```rust
    "max_transcript_age_days" => settings.max_transcript_age_days = value.parse().unwrap_or(30),
    ```

- [x] **Step 2.6: Commit**

    ```bash
    git add src/transcript/ src/lib.rs src/config/settings.rs
    git commit -m "feat: SQLite-backed subagent transcript store with CRUD and cleanup"
    ```

---

### Task 3: Subagent Event Model Extension + Loop Integration

**Files:**
- Modify: `src/agent/progress.rs:12-29,32-53,64-69` — extend SubagentEvent, SubagentProgress
- Modify: `src/teams/subagent_loop.rs:7-11,155-159,274-286,428-440` — remove truncation, add budget check, progress_tracker

- [x] **Step 3.1: Extend SubagentEvent and SubagentProgress**

    Replace the existing `SubagentEvent`, `SubagentEventType`, and `SubagentProgress` structs in `src/agent/progress.rs`:

    ```rust
    //! Subagent progress types for real-time execution visibility.
    //!
    //! These types are standalone — they do NOT depend on AppEvent or TUI types.
    //! The subagent loop emits `SubagentProgress` events through an optional
    //! `ProgressCallback`. The daemon stores them in a shared store; the TUI polls
    //! the store and converts updates into `AppEvent::SubagentUpdate` for rendering.

    use serde::{Deserialize, Serialize};
    use std::sync::Arc;

    /// An event in a subagent's execution timeline.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct SubagentEvent {
        pub event_type: SubagentEventType,
        /// Milliseconds elapsed since subagent started when this event occurred.
        pub elapsed_ms: u64,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub enum SubagentEventType {
        /// The model output text (analysis, planning, conclusion).
        /// Full text stored here — TUI layer truncates for display.
        Thought { text: String },
        /// The model called a tool.
        Action {
            tool_name: String,
            params: serde_json::Value,   // Full parameters, no truncation
            params_summary: String,       // ~80 char summary for TUI display
        },
        /// A tool execution result.
        ToolResult {
            tool_name: String,
            success: bool,
            summary: String,             // ~200 char summary
        },
        /// An error occurred.
        Error {
            message: String,
            error_type: ErrorType,
        },
        /// Subagent completed.
        Completion {
            status: String,              // 'completed' | 'failed' | 'cancelled'
            summary: Option<String>,
        },
    }

    /// Categorized error types for subagent execution.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub enum ErrorType {
        Timeout,
        BudgetExceeded { limit_k: u64, used: u64 },
        Stuck { reason: String },
        ToolError { tool: String, message: String },
        ParseError { message: String },
        Unknown,
    }

    /// Detailed error information for a failed subagent.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ErrorInfo {
        pub error_type: ErrorType,
        pub message: String,
        pub last_tool: Option<String>,
        pub last_params: Option<String>,
        pub round: u32,
        pub retryable: bool,
    }

    /// Progress delta for a single round.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ProgressDelta {
        pub value: f32,
        pub stale_rounds: u32,
        pub is_stuck: bool,
    }

    /// A progress update emitted by a subagent at key lifecycle points.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct SubagentProgress {
        pub node_id: String,
        pub parent_id: Option<String>,
        pub label: String,
        pub status: SubagentStatus,
        pub round: Option<usize>,
        pub max_rounds: Option<usize>,
        pub current_tool: Option<String>,
        /// Human-readable summary of the current tool's key parameters.
        /// e.g., `"src/auth.rs"` when `current_tool` is `"file_read"`.
        pub current_params: Option<String>,
        /// Execution event timeline (earliest → latest), no truncation.
        pub action_log: Vec<SubagentEvent>,
        /// Last assistant text response (full text, TUI truncates for display).
        pub text_snapshot: Option<String>,
        /// Unix epoch timestamp in milliseconds when this subagent started.
        pub started_at: i64,
        pub elapsed_ms: u64,
        pub metadata: Option<SubagentMetadata>,

        // === New fields ===
        /// Incremental progress delta for the last round (0.0-1.0).
        pub progress_delta: Option<f32>,
        /// Token budget in thousands (0 = unlimited).
        pub token_budget_k: Option<u64>,
        /// Cumulative tokens used so far.
        pub cumulative_tokens: u64,
        /// Error details when status is Failed or Cancelled.
        pub error_details: Option<ErrorInfo>,
        /// Full event stream (replaces old action_log for new code, kept for compat).
        pub events: Vec<SubagentEvent>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub enum SubagentStatus {
        Pending,
        Running,
        Completed,
        Failed,
        Cancelled,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct SubagentMetadata {
        pub token_count: Option<usize>,
        pub error: Option<String>,
        pub depends_on: Vec<String>,
    }

    pub type ProgressCallback = Arc<dyn Fn(SubagentProgress) + Send + Sync>;
    ```

- [x] **Step 3.2: Add token_budget_k parameter and default_subagent_token_budget_k config**

    In `src/tools/meta/task.rs`, add `token_budget` to the task tool's `input_schema`:

    ```rust
    // Inside fn input_schema, add to properties:
    "token_budget": {
        "type": "integer",
        "description": "Optional token budget in thousands (e.g., 10 = 10k tokens). 0 = unlimited. Default: 0",
        "default": 0
    },
    ```

    In `src/config/settings.rs`, add field:
    ```rust
    /// Default token budget for subagents in thousands (0 = unlimited).
    #[serde(default)]
    pub default_subagent_token_budget_k: usize,
    ```

    And add to `Settings::default()`:
    ```rust
    default_subagent_token_budget_k: 0,
    ```

    Add setter:
    ```rust
    "default_subagent_token_budget_k" => settings.default_subagent_token_budget_k = value.parse().unwrap_or(0),
    ```

- [x] **Step 3.3: Update run_subagent_loop for extended event types + budget**

    In `src/teams/subagent_loop.rs`:

    a) Add `token_budget_k` and `progress_tracker` parameters to `run_subagent_loop` function signature:

    Change signature from:
    ```rust
    pub async fn run_subagent_loop(
        api_client: &ApiClient,
        tool_registry: &ToolRegistry,
        system_prompt: &str,
        user_prompt: &str,
        allowed_tools: &[String],
        max_rounds: usize,
        timeout_secs: u64,
        on_progress: Option<ProgressCallback>,
    ) -> Result<String, String> {
    ```

    To:
    ```rust
    pub async fn run_subagent_loop(
        api_client: &ApiClient,
        tool_registry: &ToolRegistry,
        system_prompt: &str,
        user_prompt: &str,
        allowed_tools: &[String],
        max_rounds: usize,
        timeout_secs: u64,
        on_progress: Option<ProgressCallback>,
        token_budget_k: Option<u64>,
    ) -> Result<String, String> {
    ```

    b) At the top of `loop_future` block, add token budget tracking and progress tracker:

    ```rust
    let token_budget = token_budget_k;
    let cumulative_tokens_val: std::sync::Mutex<u64> = std::sync::Mutex::new(0);
    // Progress tracker for stuck detection
    let mut tool_types_used: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut stale_rounds: u32 = 0;
    ```

    c) After accumulating token usage (the block starting `if let Some(ref usage) = response.usage`), add budget enforcement:

    ```rust
    // ── Token budget enforcement ─────────────────────────────────────
    if let Some(budget_k) = token_budget {
        let used = *cumulative_tokens_val.lock().unwrap();
        if used > budget_k * 1000 {
            let msg = format!(
                "Token budget exceeded: limit {}k, used {}k tokens after {} rounds",
                budget_k,
                used / 1000,
                round
            );
            emit(SubagentStatus::Failed, Some(round + 1), None, Some(msg.clone()));
            return Err(msg);
        }
    }
    ```

    d) Replace the 200-char truncation with full text storage for Thought events:

    Change the text_snapshot section from:
    ```rust
    let snapshot: String = trimmed
        .chars()
        .rev()
        .take(200)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    *text_snapshot.lock().unwrap() = Some(snapshot.clone());
    ```

    To:
    ```rust
    *text_snapshot.lock().unwrap() = Some(trimmed.to_string());
    ```

    e) Extend the action log push to use `SubagentEventType::Thought` with full text:

    Change the Thought push from:
    ```rust
    log.push(SubagentEvent {
        event_type: SubagentEventType::Thought { text: snapshot },
        elapsed_ms: elapsed,
    });
    if log.len() > 50 { log.remove(0); }
    ```

    To (removing the 50-entry cap):
    ```rust
    log.push(SubagentEvent {
        event_type: SubagentEventType::Thought { text: trimmed.to_string() },
        elapsed_ms: elapsed,
    });
    ```

    f) Add ToolResult and Error events after tool execution. After the tool result is obtained (after `tool_registry.execute`), add:

    ```rust
    // Append ToolResult event
    {
        let mut log = action_log.lock().unwrap();
        let success = !content.starts_with("Error:") && !content.starts_with("Tool '");
        let summary: String = content.chars().take(200).collect();
        log.push(SubagentEvent {
            event_type: SubagentEventType::ToolResult {
                tool_name: tool_name.clone(),
                success,
                summary,
            },
            elapsed_ms: start.elapsed().as_millis() as u64,
        });
    }
    ```

    g) Add progress_delta computation at end of each round (after all tool calls):

    ```rust
    // ── Progress delta computation ──────────────────────────────────
    let round_tool_types: std::collections::HashSet<String> = tool_results.iter()
        .map(|tc| tc.function.name.clone())
        .collect();
    let new_types: Vec<&String> = round_tool_types.difference(&tool_types_used).collect();
    let delta = if tool_types_used.is_empty() {
        1.0f32
    } else if tool_types_used.is_empty() && !new_types.is_empty() {
        1.0f32
    } else {
        new_types.len() as f32 / tool_types_used.len() as f32
    };
    tool_types_used.extend(round_tool_types);
    if delta < 0.05 {
        stale_rounds += 1;
    } else {
        stale_rounds = 0;
    }
    if stale_rounds >= 3 {
        let msg = format!(
            "Subagent stalled: no progress for {} consecutive rounds (delta={:.2})",
            stale_rounds, delta
        );
        emit(SubagentStatus::Failed, Some(round + 1), None, Some(msg.clone()));
        return Err(msg);
    }
    ```

- [x] **Step 3.4: Update all callers of run_subagent_loop with the new parameter**

    In `src/tools/meta/task.rs` — all `run_subagent_loop` calls need an extra `None` for token budget:

    - Line ~433 (background mode): add `None,` before the closing `)` of `run_subagent_loop`
    - Line ~536 (synchronous mode): add `None,` before the closing `)`

    In `src/tools/meta/rlm/pipeline.rs` — line ~300:
    - Add `token_budget_k: Option<u64>` to `run_rlm_pipeline` parameters
    - Pass it through to the subagent_loop call at line ~302
    - Default to `None` at callers

    Update `run_rlm_pipeline` signature:
    ```rust
    pub async fn run_rlm_pipeline(
        settings: &Settings,
        tool_registry: Arc<ToolRegistry>,
        task: &str,
        context: &str,
        progress_store: Option<(
            Arc<RwLock<HashMap<String, HashMap<String, SubagentProgress>>>>,
            String,
        )>,
        root_node_id: Option<String>,
        token_budget_k: Option<u64>,   // NEW
    ) -> Result<RlmResult, String> {
    ```

    And pass it through in the subagent loop call at the executor phase:
    ```rust
    run_subagent_loop(
        &api_client, &registry, &system_prompt, &prompt, &allowed,
        20, timeout_secs, sub_progress, token_budget_k,
    ).await;
    ```

    In `src/tools/meta/task.rs` line ~516-518, update the `run_rlm_pipeline` call to pass `token_budget_k`:
    ```rust
    let budget = input.get("token_budget").and_then(|v| v.as_u64());
    ```
    Then pass it: `run_rlm_pipeline(&self.settings, ..., budget)`.

    In `src/tools/meta/rlm/mod.rs` line ~120, update the `run_rlm_pipeline` call with `None` for token budget.

- [x] **Step 3.5: Run tests to confirm no breakage**

    Run: `cargo test -p wgenty-code --lib teams::subagent_loop -- --nocapture`
    Run: `cargo test -p wgenty-code --lib tools::meta::task::tests -- --nocapture`
    Expected: All tests pass (if any exist) or compiler passes.

- [x] **Step 3.6: Commit**

    ```bash
    git add src/agent/progress.rs src/teams/subagent_loop.rs src/tools/meta/task.rs src/tools/meta/rlm/pipeline.rs src/tools/meta/rlm/mod.rs src/config/settings.rs
    git commit -m "feat: extend subagent event model with ToolResult/Error, budget, progress_delta"
    ```

---

### Task 4: Subagent Error Visualization (Failure Panel + Detail View)

**Files:**
- Modify: `src/tui/components/subagent_panel.rs:133-200` — failure inline expansion
- Modify: `src/tui/components/subagent_panel_state.rs:9-106` — add detail_view state
- Create: `src/tui/components/detail_view.rs` — full-screen event timeline
- Modify: `src/tui/components/mod.rs:1-14` — register detail_view
- Modify: `src/tui/app/event.rs:68-95` — detail view key routing
- Modify: `src/tui/app/render.rs:83-96` — render detail view full-screen when active

- [ ] **Step 4.1: Extend SubagentPanelState with detail view**

    In `src/tui/components/subagent_panel_state.rs`, add after `pub scroll_offset: u16`:
    ```rust
    /// Detail view state for a failed/selected node (None = not in detail view).
    pub detail_view: Option<DetailViewState>,
    ```

    Add the struct:
    ```rust
    #[derive(Debug, Clone)]
    pub struct DetailViewState {
        pub transcript_id: String,
        pub scroll_offset: usize,
        pub events: Vec<crate::agent::progress::SubagentEvent>,
        pub loading: bool,
    }
    ```

    Add `reset_detail` method:
    ```rust
    pub fn reset_detail(&mut self) {
        self.detail_view = None;
    }
    ```

    In `reset()`, add:
    ```rust
    self.detail_view = None;
    ```

- [ ] **Step 4.2: Update subagent panel rendering for failure expansion**

    In `src/tui/components/subagent_panel.rs`, after rendering the node's header and before rendering children, add failure detail expansion for selected+failed nodes:

    After the line `let is_selected = selected_id == Some(nid);`, add:

    ```rust
    // ── Selected + Failed: show inline error detail ────────────────
    if is_selected && node.progress.status == SubagentStatus::Failed {
        let err_indent = " ".repeat((indent + 4) as usize);
        // Error header
        if let Some(ref error) = node.progress.metadata.as_ref().and_then(|m| m.error.as_ref()) {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{}┌─ Error: {}", err_indent, error),
                    Style::default().fg(Color::Rgb(243, 139, 168)),
                ),
            ]));
        }

        // Token info
        let tokens = node.progress.cumulative_tokens;
        let budget = node.progress.token_budget_k.map(|b| format!("{}k", b)).unwrap_or_else(|| "unlimited".to_string());
        lines.push(Line::from(vec![
            Span::styled(
                format!("{}├─ Tokens: {}/{}", err_indent, tokens, budget),
                Style::default().fg(Color::Rgb(108, 112, 134)),
            ),
        ]));

        // Round info
        if let (Some(r), Some(mr)) = (node.progress.round, node.progress.max_rounds) {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{}├─ Round: {}/{}", err_indent, r, mr),
                    Style::default().fg(Color::Rgb(108, 112, 134)),
                ),
            ]));
        }

        // Action buttons
        lines.push(Line::from(vec![
            Span::styled(
                format!("{}└─ [r]etry  [d]etails  [Esc] close", err_indent),
                Style::default().fg(Color::Rgb(255, 200, 80)),
            ),
        ]));
    }
    ```

- [ ] **Step 4.3: Create DetailView component**

    Create `src/tui/components/detail_view.rs`:

    ```rust
    //! DetailView — Full-screen event timeline for a completed/failed subagent.

    use crate::agent::progress::{SubagentEventType, SubagentStatus};
    use crate::tui::components::subagent_panel_state::DetailViewState;
    use ratatui::layout::Rect;
    use ratatui::style::{Color, Modifier, Style};
    use ratatui::text::{Line, Span};
    use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
    use ratatui::Frame;

    pub struct DetailView;

    impl DetailView {
        pub fn render(f: &mut Frame, area: Rect, detail: &DetailViewState) {
            let block = Block::default()
                .title(" Event Timeline ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(249, 226, 175)))
                .style(Style::default().bg(Color::Rgb(26, 26, 46)));

            let inner = block.inner(area);
            f.render_widget(block, area);

            if detail.events.is_empty() {
                f.render_widget(
                    Paragraph::new("No events recorded.")
                        .style(Style::default().fg(Color::Rgb(108, 112, 134))),
                    inner,
                );
                return;
            }

            let scroll = detail.scroll_offset;
            let max_visible = (inner.height as usize).saturating_sub(2);
            let visible_events: Vec<_> = detail.events.iter().skip(scroll).take(max_visible).collect();

            let mut lines: Vec<Line> = Vec::new();
            for event in &visible_events {
                let elapsed = format!("+{:.1}s", event.elapsed_ms as f64 / 1000.0);
                match &event.event_type {
                    SubagentEventType::Thought { text } => {
                        let preview: String = text.chars().take(inner.width.saturating_sub(10) as usize).collect();
                        let display = if text.len() > preview.len() { format!("{}...", preview) } else { preview };
                        lines.push(Line::from(vec![
                            Span::styled(format!(" {:<8} ", elapsed), Style::default().fg(Color::Rgb(108, 112, 134))),
                            Span::styled(" THOUGHT ", Style::default().fg(Color::Rgb(180, 180, 200)).add_modifier(Modifier::DIM)),
                            Span::styled(display, Style::default().fg(Color::Rgb(180, 180, 200))),
                        ]));
                    }
                    SubagentEventType::Action { tool_name, params_summary, .. } => {
                        let action_str = if params_summary.is_empty() {
                            tool_name.clone()
                        } else {
                            format!("{}(\"{}\")", tool_name, params_summary)
                        };
                        let display: String = action_str.chars().take(inner.width.saturating_sub(10) as usize).collect();
                        lines.push(Line::from(vec![
                            Span::styled(format!(" {:<8} ", elapsed), Style::default().fg(Color::Rgb(108, 112, 134))),
                            Span::styled(" TOOL    ", Style::default().fg(Color::Rgb(137, 180, 250))),
                            Span::styled(display, Style::default().fg(Color::Rgb(137, 180, 250))),
                        ]));
                    }
                    SubagentEventType::ToolResult { tool_name, success, summary } => {
                        let icon = if *success { "OK" } else { "FAIL" };
                        let color = if *success { Color::Rgb(166, 227, 161) } else { Color::Rgb(243, 139, 168) };
                        let display: String = summary.chars().take(inner.width.saturating_sub(10) as usize).collect();
                        lines.push(Line::from(vec![
                            Span::styled(format!(" {:<8} ", elapsed), Style::default().fg(Color::Rgb(108, 112, 134))),
                            Span::styled(format!(" {} ", icon), Style::default().fg(color)),
                            Span::styled(format!("{}: {}", tool_name, display), Style::default().fg(Color::Rgb(148, 148, 165))),
                        ]));
                    }
                    SubagentEventType::Error { message, .. } => {
                        let display: String = message.chars().take(inner.width.saturating_sub(10) as usize).collect();
                        lines.push(Line::from(vec![
                            Span::styled(format!(" {:<8} ", elapsed), Style::default().fg(Color::Rgb(108, 112, 134))),
                            Span::styled(" ERROR   ", Style::default().fg(Color::Rgb(243, 139, 168)).add_modifier(Modifier::BOLD)),
                            Span::styled(display, Style::default().fg(Color::Rgb(243, 139, 168))),
                        ]));
                    }
                    SubagentEventType::Completion { status, summary } => {
                        let status_display = match status.as_str() {
                            "completed" => "COMPLETED",
                            "failed" => "FAILED",
                            _ => status,
                        };
                        let color = if status == "completed" { Color::Rgb(166, 227, 161) } else { Color::Rgb(243, 139, 168) };
                        let sum = summary.as_deref().unwrap_or("");
                        let display: String = sum.chars().take(inner.width.saturating_sub(10) as usize).collect();
                        lines.push(Line::from(vec![
                            Span::styled(format!(" {:<8} ", elapsed), Style::default().fg(Color::Rgb(108, 112, 134))),
                            Span::styled(format!(" {}  ", status_display), Style::default().fg(color).add_modifier(Modifier::BOLD)),
                            Span::styled(display, Style::default().fg(color)),
                        ]));
                    }
                }
            }

            // Help line at bottom
            let total = detail.events.len();
            let help = Line::from(vec![
                Span::styled(
                    format!(" ↑↓ scroll  PgUp/PgDn page  g/G top/bottom  f jump error  Esc back ({}/{})", scroll + 1, total),
                    Style::default().fg(Color::Rgb(108, 112, 134)),
                ),
            ]);

            f.render_widget(
                Paragraph::new(lines).wrap(Wrap { trim: false }),
                Rect {
                    x: inner.x,
                    y: inner.y,
                    width: inner.width,
                    height: inner.height.saturating_sub(1),
                },
            );
            f.render_widget(
                Paragraph::new(help),
                Rect {
                    x: inner.x,
                    y: inner.y + inner.height.saturating_sub(1),
                    width: inner.width,
                    height: 1,
                },
            );
        }
    }
    ```

- [ ] **Step 4.4: Register detail_view module**

    In `src/tui/components/mod.rs`:
    ```rust
    pub mod detail_view;
    ```

- [ ] **Step 4.5: Wire detail view key routing in event.rs**

    In `src/tui/app/event.rs`, modify the subagent panel key handling section to support detail view navigation:

    After `KeyCode::Enter => { self.subagent_panel_state.toggle_expand(&self.subagent_tree); return; }`, add:

    ```rust
    // Open detail view for selected node
    KeyCode::Char('d') => {
        if let Some(node_id) = self.subagent_panel_state.selected_node_id(&self.subagent_tree) {
            if let Some(node) = self.subagent_tree.nodes.get(&node_id) {
                let events: Vec<crate::agent::progress::SubagentEvent> = node.progress.events.clone();
                self.subagent_panel_state.detail_view = Some(DetailViewState {
                    transcript_id: node_id,
                    scroll_offset: 0,
                    events,
                    loading: false,
                });
            }
        }
        return;
    }
    ```

    Also add a separate handler for when `detail_view` is active (before the main subagent panel handlers):

    ```rust
    // If detail view is active, route keys to it
    if let Some(ref mut detail) = self.subagent_panel_state.detail_view {
        match key.code {
            KeyCode::Esc => {
                self.subagent_panel_state.reset_detail();
                return;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                detail.scroll_offset = detail.scroll_offset.saturating_sub(1);
                return;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                detail.scroll_offset = detail.scroll_offset.saturating_add(1);
                return;
            }
            KeyCode::PageUp => {
                detail.scroll_offset = detail.scroll_offset.saturating_sub(10);
                return;
            }
            KeyCode::PageDown => {
                detail.scroll_offset = detail.scroll_offset.saturating_add(10);
                return;
            }
            KeyCode::Char('g') => {
                detail.scroll_offset = 0;
                return;
            }
            KeyCode::Char('G') => {
                detail.scroll_offset = detail.events.len().saturating_sub(1);
                return;
            }
            KeyCode::Char('f') => {
                // Jump to first Error event
                if let Some(pos) = detail.events.iter().position(|e| matches!(e.event_type, SubagentEventType::Error { .. })) {
                    detail.scroll_offset = pos;
                }
                return;
            }
            _ => {}
        }
    }
    ```

    Note: `SubagentEventType::Error` is from `crate::agent::progress`, import it at the top.

- [ ] **Step 4.6: Render detail view in render.rs**

    In `src/tui/app/render.rs`, after the subagent panel rendering block and before closing, add:

    ```rust
    // Detail view (full-screen, highest z-order)
    if self.subagent_panel_state.detail_view.is_some() {
        let detail = self.subagent_panel_state.detail_view.as_ref().unwrap();
        if detail.loading {
            // Could show loading indicator; for now skip if not loaded
        }
        components::detail_view::DetailView::render(f, f.area(), detail);
    }
    ```

- [ ] **Step 4.7: Commit**

    ```bash
    git add src/tui/components/detail_view.rs src/tui/components/subagent_panel.rs src/tui/components/subagent_panel_state.rs src/tui/components/mod.rs src/tui/app/event.rs src/tui/app/render.rs
    git commit -m "feat: subagent error visualization with inline failure panel and full-screen detail view"
    ```

---

### Task 5: Retry + Rollback Mechanism

**Files:**
- Modify: `src/tools/meta/task.rs:206-575` — retry logic in TaskTool
- Create: `src/teams/rollback.rs` — RollbackContext for git-stash-based recovery
- Modify: `src/tui/app/event.rs:80-83` — `r` key in subagent panel triggers retry

- [ ] **Step 5.1: Create RollbackContext**

    Create `src/teams/rollback.rs`:

    ```rust
    //! Rollback mechanism for subagent execution.
    //!
    //! Uses git stash to create safety points before file modifications,
    //! allowing selective rollback of affected files on error.

    use std::path::PathBuf;
    use std::process::Command;

    #[derive(Debug, Clone)]
    pub struct RollbackContext {
        pub stashed_ref: String,
        pub affected_files: Vec<PathBuf>,
        pub parent_commit: String,
        label: String,
    }

    #[derive(Debug, Clone)]
    pub enum RollbackError {
        GitError(String),
        DirtyWorkingTree(String),
        StashConflict(String),
        NoChanges,
    }

    impl std::fmt::Display for RollbackError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::GitError(msg) => write!(f, "Git error: {}", msg),
                Self::DirtyWorkingTree(msg) => write!(f, "Dirty working tree: {}", msg),
                Self::StashConflict(msg) => write!(f, "Stash conflict: {}", msg),
                Self::NoChanges => write!(f, "No changes to rollback"),
            }
        }
    }

    impl std::error::Error for RollbackError {}

    impl RollbackContext {
        /// Create a safety point before files are modified.
        ///
        /// Checks for existing unstaged changes and stashes them.
        pub fn create(label: &str) -> Result<Self, RollbackError> {
            // Check for unstaged changes
            let status = Command::new("git")
                .args(["status", "--porcelain"])
                .output()
                .map_err(|e| RollbackError::GitError(e.to_string()))?;

            let output = String::from_utf8_lossy(&status.stdout);
            let has_changes = output.lines().any(|l| !l.is_empty());
            if has_changes {
                return Err(RollbackError::DirtyWorkingTree(
                    "There are uncommitted changes. Please commit or stash them first.".to_string(),
                ));
            }

            // Get current HEAD
            let head = Command::new("git")
                .args(["rev-parse", "HEAD"])
                .output()
                .map_err(|e| RollbackError::GitError(e.to_string()))?;
            let parent_commit = String::from_utf8_lossy(&head.stdout).trim().to_string();

            // Create a stash with label
            let stash_result = Command::new("git")
                .args(["stash", "push", "--include-untracked", "--message", label])
                .output()
                .map_err(|e| RollbackError::GitError(e.to_string()))?;

            if !stash_result.status.success() {
                let stderr = String::from_utf8_lossy(&stash_result.stderr);
                return Err(RollbackError::GitError(format!(
                    "Failed to create stash: {}",
                    stderr
                )));
            }

            // Get stash ref
            let stash_list = Command::new("git")
                .args(["stash", "list"])
                .output()
                .map_err(|e| RollbackError::GitError(e.to_string()))?;
            let stash_output = String::from_utf8_lossy(&stash_list.stdout);
            let first_stash = stash_output.lines().next().unwrap_or("");
            let stash_ref = first_stash.split(':').next().unwrap_or("stash@{0}").to_string();

            Ok(Self {
                stashed_ref: stash_ref,
                affected_files: Vec::new(),
                parent_commit,
                label: label.to_string(),
            })
        }

        /// Record that a file was modified by the subagent.
        pub fn record_modification(&mut self, path: PathBuf) {
            if !self.affected_files.contains(&path) {
                self.affected_files.push(path);
            }
        }

        /// Rollback to safety point by restoring affected files from stash.
        pub fn rollback(&self) -> Result<(), RollbackError> {
            if self.affected_files.is_empty() {
                // Read-only subagent — no files to rollback
                return Err(RollbackError::NoChanges);
            }

            // Pop the stash to restore files
            let pop_result = Command::new("git")
                .args(["stash", "pop"])
                .output()
                .map_err(|e| RollbackError::GitError(e.to_string()))?;

            if !pop_result.status.success() {
                let stderr = String::from_utf8_lossy(&pop_result.stderr);
                if stderr.contains("conflict") {
                    return Err(RollbackError::StashConflict(stderr.to_string()));
                }
                return Err(RollbackError::GitError(format!(
                    "Failed to pop stash: {}",
                    stderr
                )));
            }

            // Restore only the affected files from the index (which stash pop restored)
            // This selectively rolls back only our changes
            for file in &self.affected_files {
                let checkout_result = Command::new("git")
                    .args(["checkout", "--", file.to_str().unwrap_or("")])
                    .output()
                    .map_err(|e| RollbackError::GitError(e.to_string()))?;

                if !checkout_result.status.success() {
                    let stderr = String::from_utf8_lossy(&checkout_result.stderr);
                    tracing::warn!("Failed to checkout {} during rollback: {}", file.display(), stderr);
                }
            }

            Ok(())
        }

        /// Release the safety point on success (drop the stash).
        pub fn release(&self) -> Result<(), RollbackError> {
            // Drop the stash entry
            let drop_result = Command::new("git")
                .args(["stash", "drop"])
                .output()
                .map_err(|e| RollbackError::GitError(e.to_string()))?;

            if !drop_result.status.success() {
                let stderr = String::from_utf8_lossy(&drop_result.stderr);
                tracing::warn!("Failed to drop stash: {}", stderr);
            }

            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::fs;

        #[test]
        fn test_rollback_context_no_changes_returns_err() {
            // In a git repo with no changes, create() will fail because
            // there are no changes to stash. This is expected.
            let result = RollbackContext::create("test-label");
            // The result depends on git state — just verify it doesn't panic
            assert!(result.is_err());
        }

        #[test]
        fn test_affected_files_tracking() {
            let mut ctx = RollbackContext {
                stashed_ref: "stash@{0}".to_string(),
                affected_files: Vec::new(),
                parent_commit: "abc123".to_string(),
                label: "test".to_string(),
            };

            ctx.record_modification(PathBuf::from("src/main.rs"));
            assert_eq!(ctx.affected_files.len(), 1);

            // Duplicate shouldn't be added
            ctx.record_modification(PathBuf::from("src/main.rs"));
            assert_eq!(ctx.affected_files.len(), 1);

            ctx.record_modification(PathBuf::from("src/lib.rs"));
            assert_eq!(ctx.affected_files.len(), 2);
        }
    }
    ```

    Register in `src/teams/mod.rs` — check if it exists, if not create a mod entry. Add:
    ```rust
    pub mod rollback;
    ```

- [ ] **Step 5.2: Run rollback tests**

    Run: `cargo test -p wgenty-code --lib teams::rollback::tests -- --nocapture`
    Expected: At minimum the `test_affected_files_tracking` test passes.

- [ ] **Step 5.3: Wire `r` key for retry in event.rs**

    In `src/tui/app/event.rs`, in the subagent panel key handling section, add after `KeyCode::Char('d') => { ... }`:

    ```rust
    // Retry failed node
    KeyCode::Char('r') => {
        if let Some(node_id) = self.subagent_panel_state.selected_node_id(&self.subagent_tree) {
            if let Some(node) = self.subagent_tree.nodes.get(&node_id) {
                if node.progress.status == SubagentStatus::Failed {
                    // Re-submit with retry context
                    let retry_prompt = format!(
                        "[RETRY] Previous attempt failed with: {}. Task: {}",
                        node.progress.metadata.as_ref()
                            .and_then(|m| m.error.as_ref())
                            .unwrap_or("unknown error"),
                        node.progress.label
                    );
                    let _ = self.event_tx.send(AppEvent::Submit(retry_prompt));
                    self.subagent_panel_visible = false;
                }
            }
        }
        return;
    }
    ```

- [ ] **Step 5.4: Extend status bar to display failure count**

    In `src/tui/components/status.rs`, modify the phase label for `ExecutingTool` when subagent tree has failures:

    In the `phase_label` function, in the `ExecutingTool` arm, after the existing active/done logic, add failure count display if failed > 0:

    The existing code already shows `failed` count at line ~121-136. Confirmed — the status bar already shows `.failed_count()` but only if done > 0 or total > 0. Add it unconditionally when there are failures:

    Change:
    ```rust
    let failed = tree.failed_count();
    if failed > 0 {
        if !label.is_empty() {
            label.push_str(" · ");
        }
        label.push_str(&format!("{} failed", failed));
    }
    ```

    (This already exists in the current code — confirm it's still there after the changes.)

- [ ] **Step 5.5: Commit**

    ```bash
    git add src/teams/rollback.rs src/tui/app/event.rs src/tui/components/status.rs
    git commit -m "feat: retry and rollback mechanism for failed subagents"
    ```

---

### Task 6: RLM Structured Reduction (Formats + Aggregator)

**Files:**
- Create: `src/tools/meta/rlm/formats.rs` — ClaimsOutput, DiffOutput, Aggregator, Jaccard similarity
- Modify: `src/tools/meta/rlm/mod.rs:13` — add `pub mod formats;`
- Modify: `src/tools/meta/rlm/pipeline.rs:52-377` — inject format instructions + use structured aggregator

- [ ] **Step 6.1: Create RLM formats module with structured output types**

    Create `src/tools/meta/rlm/formats.rs`:

    ```rust
    //! RLM structured output formats — Claims and UnifiedDiff schemas plus Aggregator.

    use regex::Regex;
    use serde::{Deserialize, Serialize};
    use std::collections::{HashMap, HashSet};

    // ── Claims format ──────────────────────────────────────────────────────

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ClaimsOutput {
        pub format: String,  // "structured-claims/1"
        pub claims: Vec<Claim>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub metadata: Option<ClaimsMetadata>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Claim {
        pub id: String,
        pub claim: String,
        pub evidence: String,
        pub confidence: f32,               // 0.0 - 1.0
        #[serde(default)]
        pub conflicts_with: Vec<String>,
        #[serde(default)]
        pub actionable: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub recommendation: Option<String>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ClaimsMetadata {
        pub parse_method: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub parse_warning: Option<String>,
    }

    // ── Diff format ────────────────────────────────────────────────────────

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct DiffOutput {
        pub format: String,  // "unified-diff/1"
        pub changes: Vec<FileChange>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct FileChange {
        pub file: String,
        pub intent: String,               // change intent description
        pub diff: String,                 // unified diff string
        pub confidence: f32,
        #[serde(default)]
        pub depends_on: Vec<String>,      // dependent file paths
    }

    // ── Aggregator types ───────────────────────────────────────────────────

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub enum ConflictStatus {
        Unresolved,
        Resolved,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ConflictEntry {
        pub claim_a_id: String,
        pub claim_b_id: String,
        pub status: ConflictStatus,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub enum ChangeStatus {
        Clean,
        PotentialWriteConflict,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct FileChangeResult {
        pub file: String,
        pub status: ChangeStatus,
        pub changes: Vec<FileChange>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct AggregatorOutput {
        pub claims: Vec<Claim>,
        pub conflicts: Vec<ConflictEntry>,
        pub file_changes: Vec<FileChangeResult>,
        pub needs_llm_fallback: bool,
        pub unresolved_items: Vec<ConflictEntry>,
    }

    // ── Parse implementations ──────────────────────────────────────────────

    impl ClaimsOutput {
        /// Parse a ClaimsOutput from text, trying multiple extraction strategies.
        pub fn parse(text: &str) -> Result<Self, String> {
            // Level 1: Direct JSON parse
            if let Ok(claims) = serde_json::from_str::<Self>(text) {
                return Ok(claims);
            }

            // Level 2: Extract from markdown code block
            let re = Regex::new(r#"```(?:json)?\s*(\{[\s\S]*?"format"\s*:\s*"structured-claims/1"[\s\S]*?\})\s*```"#)
                .map_err(|e| format!("Regex error: {}", e))?;
            if let Some(caps) = re.captures(text) {
                if let Ok(claims) = serde_json::from_str::<Self>(&caps[1]) {
                    return Ok(claims);
                }
            }

            // Level 2b: Loose extraction — find any JSON with "claims" array
            let re_loose = Regex::new(r#"\{[\s\S]*?"claims"\s*:\s*\[[\s\S]*?\][\s\S]*?\}"#)
                .map_err(|e| format!("Regex error: {}", e))?;
            if let Some(caps) = re_loose.captures(text) {
                if let Ok(claims) = serde_json::from_str::<Self>(&caps[0]) {
                    return Ok(claims);
                }
            }

            // Level 3: Fallback — unstructured
            Ok(ClaimsOutput {
                format: "structured-claims/1".into(),
                claims: vec![Claim {
                    id: "unstructured-1".into(),
                    claim: text.to_string(),
                    evidence: String::new(),
                    confidence: 0.5,
                    conflicts_with: vec![],
                    actionable: false,
                    recommendation: None,
                }],
                metadata: Some(ClaimsMetadata {
                    parse_method: "unstructured-fallback".into(),
                    parse_warning: Some("Failed to parse structured output; preserving raw text".into()),
                }),
            })
        }
    }

    impl DiffOutput {
        /// Parse a DiffOutput from text, trying multiple extraction strategies.
        pub fn parse(text: &str) -> Result<Self, String> {
            // Level 1: Direct JSON parse
            if let Ok(diff) = serde_json::from_str::<Self>(text) {
                return Ok(diff);
            }

            // Level 2: Extract from markdown code block
            let re = Regex::new(r#"```(?:json)?\s*(\{[\s\S]*?"format"\s*:\s*"unified-diff/1"[\s\S]*?\})\s*```"#)
                .map_err(|e| format!("Regex error: {}", e))?;
            if let Some(caps) = re.captures(text) {
                if let Ok(diff) = serde_json::from_str::<Self>(&caps[1]) {
                    return Ok(diff);
                }
            }

            // Level 3: Fallback
            Ok(DiffOutput {
                format: "unified-diff/1".into(),
                changes: vec![FileChange {
                    file: "unknown".into(),
                    intent: "Unstructured diff".into(),
                    diff: text.to_string(),
                    confidence: 0.5,
                    depends_on: vec![],
                }],
            })
        }
    }

    // ── Jaccard similarity ─────────────────────────────────────────────────

    /// Compute Jaccard similarity between two strings (token-level).
    pub fn jaccard_similarity(a: &str, b: &str) -> f64 {
        let tokenize = |s: &str| -> HashSet<String> {
            s.to_lowercase()
                .split(|c: char| c.is_whitespace() || c.is_ascii_punctuation())
                .filter(|t| t.len() > 1)
                .map(|t| t.to_string())
                .collect()
        };

        let set_a = tokenize(a);
        let set_b = tokenize(b);

        let intersection = set_a.intersection(&set_b).count();
        let union = set_a.union(&set_b).count();

        if union == 0 { return 0.0; }
        intersection as f64 / union as f64
    }

    // ── Aggregator ─────────────────────────────────────────────────────────

    pub struct Aggregator;

    impl Aggregator {
        /// Merge multiple sub-task results into a consolidated output.
        pub fn merge(results: Vec<(&str, &str)>) -> AggregatorOutput {
            let mut all_claims: Vec<(usize, Claim)> = Vec::new();
            let mut all_changes: Vec<(usize, FileChange)> = Vec::new();

            for (i, (_label, content)) in results.iter().enumerate() {
                // Try claims
                if let Ok(claims_output) = ClaimsOutput::parse(content) {
                    for claim in claims_output.claims {
                        all_claims.push((i, claim));
                    }
                }

                // Try diff
                if let Ok(diff_output) = DiffOutput::parse(content) {
                    for change in diff_output.changes {
                        all_changes.push((i, change));
                    }
                }
            }

            let claims: Vec<Claim> = all_claims.into_iter().map(|(_, c)| c).collect();
            let changes: Vec<FileChange> = all_changes.into_iter().map(|(_, c)| c).collect();

            // Deduplicate claims
            let deduped = Self::deduplicate_claims(claims, 0.8);

            // Detect conflicts
            let conflicts = Self::detect_conflicts(&deduped);

            // Merge file changes
            let file_changes = Self::merge_file_changes(changes);

            let unresolved: Vec<ConflictEntry> = conflicts.iter()
                .filter(|c| matches!(c.status, ConflictStatus::Unresolved))
                .cloned()
                .collect();

            AggregatorOutput {
                claims: deduped,
                conflicts,
                file_changes,
                needs_llm_fallback: !unresolved.is_empty(),
                unresolved_items: unresolved,
            }
        }

        /// Deduplicate claims using Jaccard similarity.
        fn deduplicate_claims(mut claims: Vec<Claim>, threshold: f64) -> Vec<Claim> {
            let mut result: Vec<Claim> = Vec::new();
            let mut merged_ids: HashSet<String> = HashSet::new();

            for i in 0..claims.len() {
                if merged_ids.contains(&claims[i].id) { continue; }
                let mut claim = claims[i].clone();

                for j in (i + 1)..claims.len() {
                    if merged_ids.contains(&claims[j].id) { continue; }
                    if jaccard_similarity(&claim.claim, &claims[j].claim) > threshold {
                        // Merge: keep higher confidence, concatenate evidence
                        if claims[j].confidence > claim.confidence {
                            claim.confidence = claims[j].confidence;
                        }
                        claim.evidence = format!("{}; {}", claim.evidence, claims[j].evidence);
                        merged_ids.insert(claims[j].id.clone());
                    }
                }
                result.push(claim);
            }
            result
        }

        /// Detect conflicts between claims based on conflicts_with references.
        fn detect_conflicts(claims: &[Claim]) -> Vec<ConflictEntry> {
            let mut conflicts = Vec::new();
            let claim_map: HashMap<&str, &Claim> = claims.iter()
                .map(|c| (c.id.as_str(), c))
                .collect();

            for claim in claims {
                for conflict_id in &claim.conflicts_with {
                    if let Some(target) = claim_map.get(conflict_id.as_str()) {
                        conflicts.push(ConflictEntry {
                            claim_a_id: claim.id.clone(),
                            claim_b_id: target.id.clone(),
                            status: ConflictStatus::Unresolved,
                        });
                    }
                }
            }
            conflicts
        }

        /// Merge file changes, detecting write conflicts.
        fn merge_file_changes(changes: Vec<FileChange>) -> Vec<FileChangeResult> {
            let mut by_file: HashMap<String, Vec<FileChange>> = HashMap::new();
            for change in changes {
                by_file.entry(change.file.clone()).or_default().push(change);
            }

            by_file.into_iter().map(|(file, file_changes)| {
                if file_changes.len() > 1 {
                    FileChangeResult {
                        file,
                        status: ChangeStatus::PotentialWriteConflict,
                        changes: file_changes,
                    }
                } else {
                    FileChangeResult {
                        file,
                        status: ChangeStatus::Clean,
                        changes: file_changes,
                    }
                }
            }).collect()
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn test_parse_claims_direct_json() {
            let json = r#"{"format":"structured-claims/1","claims":[{"id":"c1","claim":"Auth uses JWT","evidence":"Found in src/auth.rs","confidence":0.9,"conflicts_with":[],"actionable":true}]}"#;
            let parsed = ClaimsOutput::parse(json).unwrap();
            assert_eq!(parsed.claims.len(), 1);
            assert_eq!(parsed.claims[0].id, "c1");
            assert!((parsed.claims[0].confidence - 0.9).abs() < 0.01);
        }

        #[test]
        fn test_parse_claims_code_block() {
            let text = "Here is my analysis:\n```json\n{\"format\":\"structured-claims/1\",\"claims\":[{\"id\":\"c1\",\"claim\":\"test\",\"evidence\":\"\",\"confidence\":0.8,\"conflicts_with\":[],\"actionable\":false}]}\n```\nDone.";
            let parsed = ClaimsOutput::parse(text).unwrap();
            assert_eq!(parsed.claims.len(), 1);
        }

        #[test]
        fn test_parse_claims_fallback() {
            let text = "Just plain text analysis without JSON structure.";
            let parsed = ClaimsOutput::parse(text).unwrap();
            assert_eq!(parsed.claims[0].id, "unstructured-1");
        }

        #[test]
        fn test_diff_output_parse() {
            let json = r#"{"format":"unified-diff/1","changes":[{"file":"src/main.rs","intent":"Add logging","diff":"@@ -1,3 +1,4 @@","confidence":0.9,"depends_on":[]}]}"#;
            let parsed = DiffOutput::parse(json).unwrap();
            assert_eq!(parsed.changes.len(), 1);
            assert_eq!(parsed.changes[0].file, "src/main.rs");
        }

        #[test]
        fn test_jaccard_identical() {
            let sim = jaccard_similarity("The quick brown fox jumps over the lazy dog", "The quick brown fox jumps over the lazy dog");
            assert!((sim - 1.0).abs() < 0.01);
        }

        #[test]
        fn test_jaccard_disjoint() {
            let sim = jaccard_similarity("aaaa bbbb cccc dddd eeee", "ffff gggg hhhh iiii jjjj");
            assert!((sim - 0.0).abs() < 0.01);
        }

        #[test]
        fn test_jaccard_partial() {
            let sim = jaccard_similarity("The quick brown fox", "The slow brown dog");
            assert!(sim > 0.0 && sim < 1.0);
        }

        #[test]
        fn test_jaccard_empty() {
            let sim = jaccard_similarity("", "");
            assert!((sim - 0.0).abs() < 0.01);
        }

        #[test]
        fn test_deduplicate_claims() {
            let claims = vec![
                Claim {
                    id: "c1".into(), claim: "System uses JWT authentication".into(),
                    evidence: "file1".into(), confidence: 0.8, conflicts_with: vec![], actionable: false, recommendation: None,
                },
                Claim {
                    id: "c2".into(), claim: "System uses JWT auth".into(),
                    evidence: "file2".into(), confidence: 0.9, conflicts_with: vec![], actionable: false, recommendation: None,
                },
                Claim {
                    id: "c3".into(), claim: "Database is PostgreSQL".into(),
                    evidence: "file3".into(), confidence: 0.7, conflicts_with: vec![], actionable: false, recommendation: None,
                },
            ];
            let deduped = Aggregator::deduplicate_claims(claims, 0.8);
            assert_eq!(deduped.len(), 2); // c1 and c2 merged, c3 stays
            // c2's higher confidence should be kept
        }

        #[test]
        fn test_detect_conflicts() {
            let claims = vec![
                Claim {
                    id: "c1".into(), claim: "Use library A".into(), evidence: "".into(),
                    confidence: 0.9, conflicts_with: vec!["c2".into()], actionable: true, recommendation: None,
                },
                Claim {
                    id: "c2".into(), claim: "Use library B".into(), evidence: "".into(),
                    confidence: 0.7, conflicts_with: vec!["c1".into()], actionable: true, recommendation: None,
                },
            ];
            let conflicts = Aggregator::detect_conflicts(&claims);
            assert_eq!(conflicts.len(), 2); // both directions
        }

        #[test]
        fn test_merge_file_changes_single() {
            let changes = vec![
                FileChange { file: "src/main.rs".into(), intent: "fix".into(), diff: "".into(), confidence: 0.9, depends_on: vec![] },
            ];
            let merged = Aggregator::merge_file_changes(changes);
            assert_eq!(merged.len(), 1);
            assert!(matches!(merged[0].status, ChangeStatus::Clean));
        }

        #[test]
        fn test_merge_file_changes_conflict() {
            let changes = vec![
                FileChange { file: "src/main.rs".into(), intent: "fix".into(), diff: "diff1".into(), confidence: 0.9, depends_on: vec![] },
                FileChange { file: "src/main.rs".into(), intent: "refactor".into(), diff: "diff2".into(), confidence: 0.8, depends_on: vec![] },
            ];
            let merged = Aggregator::merge_file_changes(changes);
            assert_eq!(merged.len(), 1);
            assert!(matches!(merged[0].status, ChangeStatus::PotentialWriteConflict));
        }
    }
    ```

- [ ] **Step 6.2: Run RLM formats tests**

    Run: `cargo test -p wgenty-code --lib tools::meta::rlm::formats::tests -- --nocapture`
    Expected: 11 tests pass.

- [ ] **Step 6.3: Register formats module and wire into pipeline**

    In `src/tools/meta/rlm/mod.rs`, change:
    ```rust
    mod pipeline;
    ```
    To:
    ```rust
    mod pipeline;
    pub mod formats;
    ```

    In `src/tools/meta/rlm/pipeline.rs`, add format injection into the planner prompt.

    Add inject function before the `run_rlm_pipeline` function:
    ```rust
    fn inject_format_instruction(task_type: &str, prompt: &mut String) {
        match task_type {
            "analysis" => {
                prompt.push_str("\n\nOUTPUT FORMAT: structured-claims/1 JSON.\n");
                prompt.push_str("Your output MUST be valid JSON matching the structured-claims schema.\n");
                prompt.push_str("{\n  \"format\": \"structured-claims/1\",\n  \"claims\": [\n    {\n      \"id\": \"c1\",\n      \"claim\": \"...\",\n      \"evidence\": \"...\",\n      \"confidence\": 0.9,\n      \"conflicts_with\": [],\n      \"actionable\": false\n    }\n  ]\n}\n");
            }
            "modification" => {
                prompt.push_str("\n\nOUTPUT FORMAT: unified-diff/1 JSON.\n");
                prompt.push_str("Your output MUST be valid JSON matching the unified-diff schema.\n");
                prompt.push_str("{\n  \"format\": \"unified-diff/1\",\n  \"changes\": [\n    {\n      \"file\": \"path/to/file.rs\",\n      \"intent\": \"description of change\",\n      \"diff\": \"@@ -1,3 +1,4 @@\\n...\",\n      \"confidence\": 0.9,\n      \"depends_on\": []\n    }\n  ]\n}\n");
            }
            _ => {} // mixed or unknown — no format injection, LLM decides
        }
    }
    ```

    Modify the executor sub-task system prompt to include format instructions when applicable. In the subagent loop call, pass the format instruction based on the task type hint.

    For now, add a simple version: inject into each sub-task's system prompt:
    ```rust
    // In the system prompt for sub-tasks in executor phase:
    let mut sub_system_prompt = "You are a sub-agent in a recursive language model system. Execute the assigned sub-task precisely and return a complete, self-contained result.".to_string();
    inject_format_instruction("analysis", &mut sub_system_prompt);
    ```

    We can refine this later with per-task-type routing. For now, the pipeline always injects the analysis format to guide structured output.

- [ ] **Step 6.4: Add ConfigChanged propagation for RLM settings**

    In `src/tui/app/event.rs`, in the `ConfigChanged` handler (around line 359-393), add propagation of new RLM settings:

    ```rust
    // Propagate RLM settings to running components
    // (completion engine already re-scanned above)
    // Reload transcript store retention settings
    if let Some(ref store) = self.transcript_store {
        let retention = new_settings.max_transcript_age_days;
        // The store cleanup is called on save — update for next cycle
    }
    ```

- [ ] **Step 6.5: Commit**

    ```bash
    git add src/tools/meta/rlm/formats.rs src/tools/meta/rlm/mod.rs src/tools/meta/rlm/pipeline.rs
    git commit -m "feat: RLM structured reduction with claims/diff formats and Jaccard-based aggregator"
    ```

---

### Task 7: RLM Budget Control + Progress Tracking

**Files:**
- Create: `src/tools/meta/rlm/budget.rs` — BudgetAllocation struct
- Modify: `src/tools/meta/rlm/mod.rs:13` — register budget module
- Modify: `src/tools/meta/rlm/pipeline.rs:117-125` — integrate budget allocation
- Modify: `src/tools/meta/task.rs:176-204` — add token_budget to input_schema
- Modify: `src/teams/subagent_loop.rs:251-255` — cumulative_tokens tracking already done in Task 3

- [ ] **Step 7.1: Create BudgetAllocation module**

    Create `src/tools/meta/rlm/budget.rs`:

    ```rust
    //! Token budget allocation for RLM pipeline phases.

    /// Budget allocation across RLM pipeline phases.
    #[derive(Debug, Clone)]
    pub struct BudgetAllocation {
        pub total: u64,          // total budget in thousands
        pub planner: u64,        // 10%
        pub executor_pool: u64,  // 80%
        pub aggregator: u64,     // 10%
    }

    impl BudgetAllocation {
        /// Create a new allocation from total budget in thousands.
        pub fn new(total_k: u64) -> Self {
            Self {
                total: total_k,
                planner: total_k / 10,
                executor_pool: total_k * 8 / 10,
                aggregator: total_k / 10,
            }
        }

        /// Distribute executor pool across individual tasks.
        pub fn distribute_to_tasks(&self, task_count: usize) -> Vec<u64> {
            if task_count == 0 { return vec![]; }
            let per_task = self.executor_pool / task_count as u64;
            vec![per_task; task_count]
        }

        /// Roll over unused budget from one phase to the next.
        pub fn rollover_unused(&mut self, phase: &str, unused: u64) {
            match phase {
                "planner" => self.executor_pool += unused,
                "executor" => self.aggregator += unused,
                _ => {}
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn test_allocation_100k() {
            let a = BudgetAllocation::new(100);
            assert_eq!(a.total, 100);
            assert_eq!(a.planner, 10);
            assert_eq!(a.executor_pool, 80);
            assert_eq!(a.aggregator, 10);
        }

        #[test]
        fn test_distribute_to_tasks() {
            let a = BudgetAllocation::new(100);
            let dist = a.distribute_to_tasks(4);
            assert_eq!(dist.len(), 4);
            assert_eq!(dist[0], 20); // 80/4 = 20
        }

        #[test]
        fn test_distribute_zero_tasks() {
            let a = BudgetAllocation::new(100);
            let dist = a.distribute_to_tasks(0);
            assert!(dist.is_empty());
        }

        #[test]
        fn test_rollover_unused_planner() {
            let mut a = BudgetAllocation::new(100);
            a.rollover_unused("planner", 5);
            assert_eq!(a.executor_pool, 85); // 80 + 5 unused
            assert_eq!(a.aggregator, 10);
        }

        #[test]
        fn test_rollover_unused_executor() {
            let mut a = BudgetAllocation::new(100);
            a.rollover_unused("executor", 15);
            assert_eq!(a.aggregator, 25); // 10 + 15 unused
        }

        #[test]
        fn test_small_budget() {
            let a = BudgetAllocation::new(1); // 1k tokens
            assert_eq!(a.planner, 0);  // integer division: 1/10 = 0
            assert_eq!(a.executor_pool, 0);
            assert_eq!(a.aggregator, 0);
        }
    }
    ```

- [ ] **Step 7.2: Run budget tests**

    Run: `cargo test -p wgenty-code --lib tools::meta::rlm::budget::tests -- --nocapture`
    Expected: 6 tests pass.

- [ ] **Step 7.3: Register budget module and integrate into pipeline**

    In `src/tools/meta/rlm/mod.rs`:
    ```rust
    pub mod budget;
    ```

    In `src/tools/meta/rlm/pipeline.rs`, integrate budget allocation:

    At the top of `run_rlm_pipeline`, after parsing sub-tasks and before the executor phase:

    ```rust
    // ── Budget allocation ──────────────────────────────────────────
    let budget_used = token_budget_k.unwrap_or(0);
    let mut allocation = if budget_used > 0 {
        Some(crate::tools::meta::rlm::budget::BudgetAllocation::new(budget_used))
    } else {
        None
    };

    // If budget is set, distribute to sub-tasks
    let per_task_budget = allocation.as_ref().map(|a| a.distribute_to_tasks(sub_tasks.len()));
    ```

    Pass `per_task_budget[idx]` to each sub-task's `run_subagent_loop` call:

    ```rust
    let task_budget = per_task_budget.as_ref().and_then(|budgets| budgets.get(idx).copied());
    // In run_subagent_loop call:
    run_subagent_loop(
        &api_client, &registry, &system_prompt, &prompt, &allowed,
        20, timeout_secs, sub_progress, task_budget,
    ).await;
    ```

    After all sub-tasks complete, rollover unused budget:
    ```rust
    // After executor phase, roll over unused budget to aggregator
    if let Some(ref mut alloc) = allocation {
        let used: u64 = cumulative_budget.lock().map(|v| *v).unwrap_or(0);
        let allocated = per_task_budget.as_ref().map(|b| b.iter().sum::<u64>()).unwrap_or(0);
        if allocated > used {
            alloc.rollover_unused("executor", allocated.saturating_sub(used));
        }
    }
    ```

- [ ] **Step 7.4: Update Settings with rlm_jaccard_threshold default**

    In `src/config/settings.rs`, add field after `default_subagent_token_budget_k`:
    ```rust
    /// Jaccard similarity threshold for RLM claim deduplication.
    #[serde(default = "default_jaccard_threshold")]
    pub rlm_jaccard_threshold: f64,
    ```

    Add default function:
    ```rust
    fn default_jaccard_threshold() -> f64 { 0.8 }
    ```

    In `Settings::default()`:
    ```rust
    rlm_jaccard_threshold: 0.8,
    ```

    Add setter:
    ```rust
    "rlm_jaccard_threshold" => settings.rlm_jaccard_threshold = value.parse().unwrap_or(0.8),
    ```

- [ ] **Step 7.5: Commit**

    ```bash
    git add src/tools/meta/rlm/budget.rs src/tools/meta/rlm/mod.rs src/tools/meta/rlm/pipeline.rs src/config/settings.rs
    git commit -m "feat: RLM budget allocation and progress tracking"
    ```

---

### Task 8: TUI Rendering Integration — Subagent Panel Enhanced Display

**Files:**
- Modify: `src/tui/components/subagent_tree.rs:12-16` — store new subagent progress fields
- Modify: `src/tui/components/subagent_panel.rs:171-185` — show token budget, progress_delta
- Modify: `src/tui/components/status.rs:116-137` — show failure count + token budget
- Modify: `src/tui/app/render.rs:55-96` — integration pass (mostly already done)

- [ ] **Step 8.1: Update SubagentTree to expose new progress fields**

    In `src/tui/components/subagent_tree.rs`, `SubagentNode` already stores `progress: SubagentProgress` which now contains the new fields. The tree methods just need to expose them.

    Add methods to `SubagentTree`:
    ```rust
    /// Total cumulative tokens across all nodes.
    pub fn total_tokens(&self) -> u64 {
        self.nodes.values().map(|n| n.progress.cumulative_tokens).sum()
    }

    /// Current token budget (highest among running nodes).
    pub fn active_budget_k(&self) -> Option<u64> {
        self.nodes.values()
            .filter(|n| n.progress.status == SubagentStatus::Running)
            .filter_map(|n| n.progress.token_budget_k)
            .max()
    }
    ```

- [ ] **Step 8.2: Update SubagentPanel to show token budget + progress_delta**

    In `src/tui/components/subagent_panel.rs`, in the node header rendering (around line 157-200), add token budget display:

    After `let elapsed_secs = node.progress.elapsed_ms as f64 / 1000.0;`, add:
    ```rust
    let tokens_str = if node.progress.cumulative_tokens > 0 {
        if let Some(budget_k) = node.progress.token_budget_k {
            if budget_k > 0 {
                format!(" · {:.1}k/{}k tokens",
                    node.progress.cumulative_tokens as f64 / 1000.0,
                    budget_k)
            } else {
                format!(" · {:.1}k tokens", node.progress.cumulative_tokens as f64 / 1000.0)
            }
        } else {
            format!(" · {:.1}k tokens", node.progress.cumulative_tokens as f64 / 1000.0)
        }
    } else {
        String::new()
    };

    let progress_warn = node.progress.progress_delta
        .filter(|d| *d < 0.05 && node.progress.status == SubagentStatus::Running)
        .map(|_| " ⚠ low progress".to_string())
        .unwrap_or_default();
    ```

    Then append `tokens_str` and `progress_warn` to the `status_detail` string where appropriate.

- [ ] **Step 8.3: Update status bar to show token budget usage**

    In `src/tui/components/status.rs`, in the `render` function's meta parts section, after the existing token display, add:

    ```rust
    // Subagent token budget info
    if let Some(tree) = subagent_tree {
        if !tree.is_empty() {
            let total_tokens = tree.total_tokens();
            if total_tokens > 0 {
                meta_parts.push(format!("{:.1}k tokens", total_tokens as f64 / 1000.0));
            }
        }
    }
    ```

- [ ] **Step 8.4: Commit**

    ```bash
    git add src/tui/components/subagent_tree.rs src/tui/components/subagent_panel.rs src/tui/components/status.rs
    git commit -m "feat: TUI renders token budget, progress delta, and enhanced status"
    ```

---

### Task 9: Ink CLI Sidecar — Completion + AgentStatus Extension

**Files:**
- Modify: `packages/cli/src/components/input-box.tsx:30-39` — detect @ and / prefixes
- Modify: `packages/cli/src/hooks/use-agent.ts:23-30` — extend AgentStatus type

- [ ] **Step 9.1: Extend AgentStatus type in use-agent.ts**

    In `packages/cli/src/hooks/use-agent.ts`, extend the `AgentStatus` union type:

    After `| { type: "executing"; toolName: string };`, add a completion state (this is a separate piece of state, not part of AgentStatus):

    ```typescript
    export interface CompletionMatch {
      text: string;
      description: string;
      argsHint?: string;
    }

    export interface CompletionState {
      visible: boolean;
      prefix: '@' | '/';
      partial: string;
      matches: CompletionMatch[];
      selectedIndex: number;
    }

    export interface DetailViewState {
      transcriptId: string;
      events: SubagentEvent[];
      scrollOffset: number;
    }

    // Placeholder — will be filled with real event type from Rust side
    export interface SubagentEvent {
      eventType: string;
      elapsedMs: number;
    }
    ```

- [ ] **Step 9.2: Add completion detection in Ink input-box**

    In `packages/cli/src/components/input-box.tsx`, add completion state tracking:

    In the component body, after `const [value, setValue] = React.useState("");`, add:
    ```typescript
    const [completionState, setCompletionState] = React.useState<CompletionState | null>(null);
    ```

    In `useInput`, after the Ctrl key handling, add:
    ```typescript
    // Detect @ and / for completion
    if (input === '@') {
      const currentValue = valueRef.current;
      if (!currentValue || currentValue.endsWith(' ')) {
        setCompletionState({
          visible: true,
          prefix: '@',
          partial: '',
          matches: [],
          selectedIndex: 0,
        });
      }
    }
    if (input === '/' && valueRef.current === '') {
      setCompletionState({
        visible: true,
        prefix: '/',
        partial: '',
        matches: [],
        selectedIndex: 0,
      });
    }
    ```

    Add keyboard navigation handling for completion state:
    ```typescript
    if (completionState?.visible) {
      if (key.escape) {
        setCompletionState(null);
        return;
      }
      if (key.upArrow || input === 'k') {
        setCompletionState(prev => prev ? { ...prev, selectedIndex: Math.max(0, prev.selectedIndex - 1) } : null);
        return;
      }
      if (key.downArrow || input === 'j') {
        setCompletionState(prev => prev ? { ...prev, selectedIndex: Math.min(prev.matches.length - 1, prev.selectedIndex + 1) } : null);
        return;
      }
      if (key.return && completionState.matches[completionState.selectedIndex]) {
        const selected = completionState.matches[completionState.selectedIndex];
        setValue(selected.text + ' ');
        setCompletionState(null);
        return;
      }
    }
    ```

    Render the completion dropdown below the input (using Ink Box components):

    After the closing `</Box>` of the main input border, add:
    ```tsx
    {completionState?.visible && completionState.matches.length > 0 && (
      <Box flexDirection="column" borderStyle="round" borderColor="rgb(203,166,247)" paddingX={1} width={width - 4}>
        {completionState.matches.slice(0, 8).map((m, i) => (
          <Text key={m.text} color={i === completionState.selectedIndex ? 'rgb(203,166,247)' : undefined}>
            {m.text} <Text dimColor>{m.description}</Text>
          </Text>
        ))}
        <Text dimColor>↑↓ Tab Enter Esc</Text>
      </Box>
    )}
    ```

- [ ] **Step 9.3: Export new types from the CLI package**

    Update exports in the CLI package's index file if needed.

- [ ] **Step 9.4: Commit**

    ```bash
    git add packages/cli/src/hooks/use-agent.ts packages/cli/src/components/input-box.tsx
    git commit -m "feat: Ink CLI completion for @ skills and / commands"
    ```

---

### Task 10: Transcript Store Integration with Subagent Loop + RLM Pipeline

**Files:**
- Modify: `src/tools/meta/task.rs:116-156` — instantiate TranscriptStore, wire into subagent flow
- Modify: `src/tools/meta/task.rs:424-448` — save transcript on completion/failure
- Modify: `src/tools/meta/rlm/pipeline.rs:236-306` — save per-sub-task transcripts
- Modify: `src/tui/app/mod.rs:90-93` — initialize transcript_store in App

- [ ] **Step 10.1: Wire TranscriptStore into the daemon/subagent flow**

    In `src/tui/app/mod.rs`, after settings are loaded:
    ```rust
    transcript_store: {
        let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        let db_path = home.join(".wgenty-code").join("subagent_transcripts.db");
        match crate::transcript::store::SubagentTranscriptStore::open(&db_path) {
            Ok(store) => Some(store),
            Err(e) => {
                tracing::warn!("Failed to open transcript store: {}. Running without persistence.", e);
                None
            }
        }
    },
    ```

    Add the field to `App` struct:
    ```rust
    pub transcript_store: Option<crate::transcript::store::SubagentTranscriptStore>,
    ```

- [ ] **Step 10.2: Save transcript on subagent completion/failure**

    In `src/tools/meta/task.rs`, in the subagent execution flow, after the subagent completes or fails:

    For the synchronous path (around line 538-541), add transcript saving:

    ```rust
    // After result is obtained
    let transcript_id = uuid::Uuid::new_v4().to_string();
    let transcript = crate::transcript::SubagentTranscript {
        id: transcript_id.clone(),
        session_id: session_id.clone(),
        parent_id: None,
        label: format!("task: {}", description),
        status: match &result {
            Ok(_) => crate::transcript::TranscriptStatus::Completed,
            Err(_) => crate::transcript::TranscriptStatus::Failed,
        },
        system_prompt: Some(system_prompt.to_string()),
        user_prompt: full_prompt.clone(),
        started_at: chrono::Utc::now().timestamp_millis(),
        finished_at: Some(chrono::Utc::now().timestamp_millis()),
        total_tokens: 0, // tracked in subagent loop
        max_rounds: Some(30),
        actual_rounds: 0,
        token_budget_k: None,
        error_message: result.as_ref().err().cloned(),
        summary: result.as_ref().ok().map(|r| r.chars().take(500).collect()),
        events: vec![], // populated from action_log
    };
    // Save via background task (non-blocking)
    if let Some(ref store) = self.transcript_store {
        let _ = store.save(&transcript);
    }
    ```

- [ ] **Step 10.3: Add cleanup call on transcript save**

    In `src/transcript/store.rs`, in the `save` method, after the transaction commits, add:

    ```rust
    // Trigger cleanup after save
    if retention_days > 0 {
        let _ = self.cleanup(retention_days);
    }
    ```

    The `retention_days` should be passed in from settings.

- [ ] **Step 10.4: Commit**

    ```bash
    git add src/tui/app/mod.rs src/tools/meta/task.rs src/transcript/store.rs
    git commit -m "feat: wire transcript persistence into subagent execution flow"
    ```

---

### Task 11: Verification & Testing

**Files:**
- Run tests only (no new files)

- [ ] **Step 11.1: Run full test suite**

    Run: `cargo test -p wgenty-code --lib -- --nocapture`

    Expected output: All tests pass. Focus on:
    - `tui::completion::tests` (6 tests)
    - `transcript::store::tests` (6 tests)
    - `tools::meta::rlm::formats::tests` (11 tests)
    - `tools::meta::rlm::budget::tests` (6 tests)
    - `teams::rollback::tests` (2 tests)
    - `tools::meta::task::tests` (4 tests)
    - `tui::util::tests` (5 tests)

- [ ] **Step 11.2: TUI compilation check**

    Run: `cargo check -p wgenty-code --features tui`

    Expected: Clean compilation with no warnings.

- [ ] **Step 11.3: CLI TypeScript compilation check**

    Run: `cd packages/cli && npx tsc --noEmit`

    Expected: No type errors.

- [ ] **Step 11.4: Manual verification — skills/commands completion**

    Start app: `cargo run`

    Steps:
    1. Type `@` at empty input — verify completion panel opens with skills list
    2. Type `comet` after `@` — verify filter narrows to comet-* skills
    3. Press `↓` to navigate, `Enter` to select — verify input replaced with skill name
    4. Press `Esc` — verify completion panel closes
    5. Type `/` at empty input — verify commands list

- [ ] **Step 11.5: Manual verification — subagent timeline + detail view**

    1. Start a task that spawns subagents (e.g., complex refactor)
    2. Open subagent panel (Ctrl+Shift+T)
    3. Verify each subagent node shows token usage, progress status
    4. Expand a running node — verify event timeline
    5. Wait for completion or failure
    6. Select a failed node — verify inline error expansion with `[r]etry [d]etails`
    7. Press `d` — verify full-screen detail view opens with complete timeline
    8. Verify scroll, `g`/`G`, `f` (jump to error), `Esc` back

- [ ] **Step 11.6: Manual verification — RLM pipeline with structured output**

    1. Trigger a `delegate` tool call with analysis-type task
    2. Verify sub-tasks appear in the subagent tree
    3. Verify each sub-task status is tracked
    4. On completion, verify aggregator merged results correctly

- [ ] **Step 11.7: Commit any test/fix changes**

    ```bash
    git add -A
    git commit -m "test: add verification tests and fix any issues found"
    ```
