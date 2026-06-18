---
change: external-skill-runtime
design-doc: docs/superpowers/specs/2026-06-18-external-skill-runtime-design.md
base-ref: 114424659d602468c4bfbfa981a344e5c8792ec6
---

# External Skill Runtime Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a wgenty-code-native external instruction skill runtime that can discover, list, load, route, and nest Claude Code-style markdown skills such as Comet without hardcoding those workflows.

**Architecture:** Extend the existing `knowledge` skill loader and `load_skill` tool path instead of creating a separate runtime. Add external skill definitions, registry resolution, policy hooks, loaded-skill context, a Claude-compatible `skill` runtime action, and slash-command fallback routing. Keep side effects behind existing tools and guardian checks.

**Tech Stack:** Rust, async_trait, serde/serde_json, existing wgenty-code `Tool` trait, existing `knowledge` module, existing plugin/cache conventions, Cargo tests.

---

## File Structure

Create these files:

- `src/knowledge/external.rs` — external instruction skill data structures, frontmatter parsing, source metadata, canonical-name derivation helpers.
- `src/knowledge/external_registry.rs` — discovery across `.wgenty-code` roots, plugin/cache roots, configured extra roots, priority resolution, diagnostics, suggestions.
- `src/knowledge/policy.rs` — policy event structs, `SkillPolicy` trait, `PolicyDecision`, `DefaultAllowPolicy`.
- `src/tools/meta/skill.rs` — Claude Code-compatible read-only `skill` runtime action backed by `ExternalSkillRegistry` and loaded-skill context.

Modify these files:

- `src/knowledge/mod.rs` — export new modules and public types.
- `src/knowledge/loader.rs` — keep existing API where possible; delegate or adapt to external registry for markdown instruction skills.
- `src/tools/meta/load_skill.rs` — preserve legacy `load_skill`, optionally back it with `ExternalSkillRegistry` or share formatting helpers.
- `src/tools/meta/mod.rs` — export `SkillTool`.
- `src/tools/mod.rs` — register and re-export `SkillTool`; wire it where `LoadSkillTool` is registered if registry construction supports a skill loader.
- `src/prompts/mod.rs` — ensure available external skills are represented as compact `SkillEntry` records.
- Slash command handling file discovered during implementation — add built-in-first, external-skill fallback. Search for slash command parsing before editing; if no central handler exists, add a small routing helper and tests without broad UI refactors.
- `tests/skills_test.rs` — add tests for external model, discovery, source priority, namespace mapping, policy.
- `tests/tools_test.rs` — add tests for `skill` tool registration and behavior.

Do not implement Comet-specific policy enforcement in this change.

## Task 1: External skill data model

**Files:**
- Create: `src/knowledge/external.rs`
- Modify: `src/knowledge/mod.rs`
- Test: `tests/skills_test.rs`

- [ ] **Step 1: Write failing tests for metadata parsing and canonical names**

Add tests to `tests/skills_test.rs`:

```rust
use std::path::PathBuf;
use wgenty_code::knowledge::{
    derive_canonical_skill_name, parse_external_skill_document, ExternalSkillSource,
};

#[test]
fn test_external_skill_frontmatter_name_and_description() {
    let body = r#"---
name: comet
description: Comet workflow
---
# Comet

Instructions here.
"#;

    let parsed = parse_external_skill_document(body).expect("frontmatter should parse");

    assert_eq!(parsed.name.as_deref(), Some("comet"));
    assert_eq!(parsed.description.as_deref(), Some("Comet workflow"));
    assert!(parsed.body.contains("# Comet"));
    assert!(parsed.raw_frontmatter.contains("name: comet"));
}

#[test]
fn test_external_skill_missing_name_falls_back_to_directory() {
    let canonical = derive_canonical_skill_name(
        None,
        &PathBuf::from(".wgenty-code/skills/comet/SKILL.md"),
        &PathBuf::from(".wgenty-code/skills"),
    )
    .expect("canonical name should derive from directory");

    assert_eq!(canonical, "comet");
}

#[test]
fn test_external_skill_portable_namespace_directory() {
    let canonical = derive_canonical_skill_name(
        None,
        &PathBuf::from(".wgenty-code/skills/superpowers/brainstorming/SKILL.md"),
        &PathBuf::from(".wgenty-code/skills"),
    )
    .expect("canonical name should derive from namespace directory");

    assert_eq!(canonical, "superpowers:brainstorming");
}

#[test]
fn test_external_skill_source_labels() {
    let source = ExternalSkillSource::ProjectWgentyCode {
        root: PathBuf::from("/repo/.wgenty-code/skills"),
    };

    assert_eq!(source.priority_rank(), 0);
    assert!(source.label().contains("project"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
cargo test --test skills_test external_skill -- --nocapture
```

Expected: FAIL because `ExternalSkillSource`, `parse_external_skill_document`, and `derive_canonical_skill_name` do not exist yet.

- [ ] **Step 3: Implement `src/knowledge/external.rs`**

Create `src/knowledge/external.rs`:

```rust
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExternalSkillSource {
    ProjectWgentyCode { root: PathBuf },
    UserWgentyCode { root: PathBuf },
    PluginCache {
        plugin_name: String,
        version: Option<String>,
        root: PathBuf,
    },
    Configured { label: String, root: PathBuf },
}

impl ExternalSkillSource {
    pub fn priority_rank(&self) -> u8 {
        match self {
            Self::ProjectWgentyCode { .. } => 0,
            Self::UserWgentyCode { .. } => 1,
            Self::PluginCache { .. } => 2,
            Self::Configured { .. } => 3,
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::ProjectWgentyCode { root } => format!("project:{}", root.display()),
            Self::UserWgentyCode { root } => format!("user:{}", root.display()),
            Self::PluginCache {
                plugin_name,
                version,
                root,
            } => format!(
                "plugin:{}@{}:{}",
                plugin_name,
                version.as_deref().unwrap_or("unknown"),
                root.display()
            ),
            Self::Configured { label, root } => format!("configured:{}:{}", label, root.display()),
        }
    }

    pub fn root(&self) -> &Path {
        match self {
            Self::ProjectWgentyCode { root }
            | Self::UserWgentyCode { root }
            | Self::PluginCache { root, .. }
            | Self::Configured { root, .. } => root,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SkillFrontmatter {
    pub name: Option<String>,
    pub description: Option<String>,
    pub raw: String,
    pub extra: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedExternalSkillDocument {
    pub name: Option<String>,
    pub description: Option<String>,
    pub raw_frontmatter: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShadowedSkillDefinition {
    pub canonical_name: String,
    pub source: ExternalSkillSource,
    pub source_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalSkillDefinition {
    pub canonical_name: String,
    pub display_name: String,
    pub description: String,
    pub body: String,
    pub frontmatter: SkillFrontmatter,
    pub source: ExternalSkillSource,
    pub source_path: PathBuf,
    pub base_dir: PathBuf,
    pub shadowed: Vec<ShadowedSkillDefinition>,
}

pub fn parse_external_skill_document(
    content: &str,
) -> Result<ParsedExternalSkillDocument, String> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return Ok(ParsedExternalSkillDocument {
            name: None,
            description: None,
            raw_frontmatter: String::new(),
            body: content.to_string(),
        });
    }

    let rest = &trimmed[3..];
    let end = rest
        .find("---")
        .ok_or_else(|| "frontmatter start marker has no closing marker".to_string())?;
    let raw_frontmatter = rest[..end].trim().to_string();
    let body = rest[end + 3..].trim_start().to_string();

    let mut name = None;
    let mut description = None;

    for line in raw_frontmatter.lines() {
        if let Some(value) = line.strip_prefix("name:") {
            name = Some(value.trim().trim_matches('"').to_string());
        } else if let Some(value) = line.strip_prefix("description:") {
            description = Some(value.trim().trim_matches('"').to_string());
        }
    }

    Ok(ParsedExternalSkillDocument {
        name,
        description,
        raw_frontmatter,
        body,
    })
}

pub fn derive_canonical_skill_name(
    frontmatter_name: Option<&str>,
    skill_file: &Path,
    skills_root: &Path,
) -> Result<String, String> {
    if let Some(name) = frontmatter_name {
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    let relative = skill_file
        .strip_prefix(skills_root)
        .map_err(|_| format!("{} is not under {}", skill_file.display(), skills_root.display()))?;
    let parent = relative
        .parent()
        .ok_or_else(|| format!("{} has no skill directory", relative.display()))?;
    let parts: Vec<String> = parent
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .filter(|part| !part.is_empty())
        .collect();

    match parts.as_slice() {
        [name] => Ok(name.clone()),
        [namespace, name] => Ok(format!("{}:{}", namespace, name)),
        _ => Err(format!(
            "unsupported skill path {}; expected skills/<name>/SKILL.md or skills/<namespace>/<name>/SKILL.md",
            relative.display()
        )),
    }
}
```

- [ ] **Step 4: Export new types from `src/knowledge/mod.rs`**

Add module and exports:

```rust
pub mod external;

pub use external::{
    derive_canonical_skill_name, parse_external_skill_document, ExternalSkillDefinition,
    ExternalSkillSource, ParsedExternalSkillDocument, ShadowedSkillDefinition, SkillFrontmatter,
};
```

- [ ] **Step 5: Run tests to verify Task 1 passes**

Run:

```bash
cargo test --test skills_test external_skill -- --nocapture
```

Expected: PASS.

- [ ] **Step 6: Commit Task 1**

```bash
git add src/knowledge/external.rs src/knowledge/mod.rs tests/skills_test.rs
git commit -m "feat(skills): add external skill data model"
```

## Task 2: External skill registry and discovery

**Files:**
- Create: `src/knowledge/external_registry.rs`
- Modify: `src/knowledge/mod.rs`
- Test: `tests/skills_test.rs`

- [ ] **Step 1: Write failing tests for discovery priority and shadowing**

Add tests to `tests/skills_test.rs`:

```rust
use std::fs;
use tempfile::TempDir;
use wgenty_code::knowledge::{ExternalSkillRegistry, ExternalSkillRoot, ExternalSkillSource};

fn write_skill(root: &std::path::Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

#[test]
fn test_external_registry_discovers_project_skill() {
    let repo = TempDir::new().unwrap();
    write_skill(
        repo.path(),
        ".wgenty-code/skills/comet/SKILL.md",
        "---\nname: comet\ndescription: Comet\n---\n# Comet",
    );

    let registry = ExternalSkillRegistry::discover(vec![ExternalSkillRoot::new(
        repo.path().join(".wgenty-code/skills"),
        ExternalSkillSource::ProjectWgentyCode {
            root: repo.path().join(".wgenty-code/skills"),
        },
    )])
    .expect("registry should discover skills");

    let skill = registry.resolve("comet").expect("comet should resolve");
    assert_eq!(skill.canonical_name, "comet");
    assert_eq!(skill.description, "Comet");
    assert!(skill.source_path.ends_with("SKILL.md"));
}

#[test]
fn test_external_registry_project_shadows_user_skill() {
    let repo = TempDir::new().unwrap();
    let user = TempDir::new().unwrap();

    write_skill(
        repo.path(),
        ".wgenty-code/skills/comet/SKILL.md",
        "---\nname: comet\ndescription: Project Comet\n---\n# Project",
    );
    write_skill(
        user.path(),
        ".wgenty-code/skills/comet/SKILL.md",
        "---\nname: comet\ndescription: User Comet\n---\n# User",
    );

    let registry = ExternalSkillRegistry::discover(vec![
        ExternalSkillRoot::new(
            repo.path().join(".wgenty-code/skills"),
            ExternalSkillSource::ProjectWgentyCode {
                root: repo.path().join(".wgenty-code/skills"),
            },
        ),
        ExternalSkillRoot::new(
            user.path().join(".wgenty-code/skills"),
            ExternalSkillSource::UserWgentyCode {
                root: user.path().join(".wgenty-code/skills"),
            },
        ),
    ])
    .expect("registry should discover skills");

    let skill = registry.resolve("comet").expect("comet should resolve");
    assert_eq!(skill.description, "Project Comet");
    assert_eq!(skill.shadowed.len(), 1);
    assert!(registry.diagnostics().join("\n").contains("shadowed"));
}

#[test]
fn test_external_registry_suggests_similar_names() {
    let repo = TempDir::new().unwrap();
    write_skill(
        repo.path(),
        ".wgenty-code/skills/comet/SKILL.md",
        "---\nname: comet\ndescription: Comet\n---\n# Comet",
    );

    let registry = ExternalSkillRegistry::discover(vec![ExternalSkillRoot::new(
        repo.path().join(".wgenty-code/skills"),
        ExternalSkillSource::ProjectWgentyCode {
            root: repo.path().join(".wgenty-code/skills"),
        },
    )])
    .expect("registry should discover skills");

    assert_eq!(registry.suggest("comte", 3), vec!["comet".to_string()]);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
cargo test --test skills_test external_registry -- --nocapture
```

Expected: FAIL because `ExternalSkillRegistry` and `ExternalSkillRoot` do not exist.

- [ ] **Step 3: Implement `src/knowledge/external_registry.rs`**

Create `src/knowledge/external_registry.rs`:

```rust
use super::external::{
    derive_canonical_skill_name, parse_external_skill_document, ExternalSkillDefinition,
    ExternalSkillSource, ShadowedSkillDefinition, SkillFrontmatter,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ExternalSkillRoot {
    pub skills_root: PathBuf,
    pub source: ExternalSkillSource,
}

impl ExternalSkillRoot {
    pub fn new(skills_root: PathBuf, source: ExternalSkillSource) -> Self {
        Self { skills_root, source }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ExternalSkillRegistry {
    skills: HashMap<String, ExternalSkillDefinition>,
    diagnostics: Vec<String>,
}

impl ExternalSkillRegistry {
    pub fn discover(roots: Vec<ExternalSkillRoot>) -> Result<Self, String> {
        let mut discovered = Vec::new();
        let mut diagnostics = Vec::new();

        for root in roots {
            if !root.skills_root.exists() {
                continue;
            }
            scan_root(&root.skills_root, &root, &mut discovered, &mut diagnostics)?;
        }

        discovered.sort_by_key(|skill| skill.source.priority_rank());

        let mut skills: HashMap<String, ExternalSkillDefinition> = HashMap::new();
        for skill in discovered {
            if let Some(existing) = skills.get_mut(&skill.canonical_name) {
                diagnostics.push(format!(
                    "skill '{}' from {} shadowed by {}",
                    skill.canonical_name,
                    skill.source_path.display(),
                    existing.source_path.display()
                ));
                existing.shadowed.push(ShadowedSkillDefinition {
                    canonical_name: skill.canonical_name.clone(),
                    source: skill.source.clone(),
                    source_path: skill.source_path.clone(),
                });
            } else {
                skills.insert(skill.canonical_name.clone(), skill);
            }
        }

        Ok(Self { skills, diagnostics })
    }

    pub fn resolve(&self, name: &str) -> Option<&ExternalSkillDefinition> {
        self.skills.get(name)
    }

    pub fn list(&self) -> Vec<&ExternalSkillDefinition> {
        let mut values: Vec<_> = self.skills.values().collect();
        values.sort_by(|a, b| a.canonical_name.cmp(&b.canonical_name));
        values
    }

    pub fn diagnostics(&self) -> &[String] {
        &self.diagnostics
    }

    pub fn suggest(&self, name: &str, limit: usize) -> Vec<String> {
        let mut candidates: Vec<(usize, String)> = self
            .skills
            .keys()
            .map(|candidate| (levenshtein(name, candidate), candidate.clone()))
            .collect();
        candidates.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        candidates
            .into_iter()
            .filter(|(distance, _)| *distance <= 3)
            .take(limit)
            .map(|(_, candidate)| candidate)
            .collect()
    }
}

fn scan_root(
    skills_root: &Path,
    root: &ExternalSkillRoot,
    discovered: &mut Vec<ExternalSkillDefinition>,
    diagnostics: &mut Vec<String>,
) -> Result<(), String> {
    let entries = std::fs::read_dir(skills_root)
        .map_err(|error| format!("read {} failed: {}", skills_root.display(), error))?;

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        collect_skill_files(&path, skills_root, root, discovered, diagnostics)?;
    }

    Ok(())
}

fn collect_skill_files(
    directory: &Path,
    skills_root: &Path,
    root: &ExternalSkillRoot,
    discovered: &mut Vec<ExternalSkillDefinition>,
    diagnostics: &mut Vec<String>,
) -> Result<(), String> {
    let skill_file = directory.join("SKILL.md");
    if skill_file.exists() {
        match load_skill_file(&skill_file, skills_root, root) {
            Ok(skill) => discovered.push(skill),
            Err(error) => diagnostics.push(error),
        }
        return Ok(());
    }

    for entry in std::fs::read_dir(directory)
        .map_err(|error| format!("read {} failed: {}", directory.display(), error))?
        .flatten()
    {
        let path = entry.path();
        if path.is_dir() && path.join("SKILL.md").exists() {
            match load_skill_file(&path.join("SKILL.md"), skills_root, root) {
                Ok(skill) => discovered.push(skill),
                Err(error) => diagnostics.push(error),
            }
        }
    }

    Ok(())
}

fn load_skill_file(
    skill_file: &Path,
    skills_root: &Path,
    root: &ExternalSkillRoot,
) -> Result<ExternalSkillDefinition, String> {
    let content = std::fs::read_to_string(skill_file)
        .map_err(|error| format!("read {} failed: {}", skill_file.display(), error))?;
    let parsed = parse_external_skill_document(&content)?;
    let canonical_name = derive_canonical_skill_name(parsed.name.as_deref(), skill_file, skills_root)?;
    let description = parsed.description.clone().unwrap_or_default();
    let base_dir = skill_file
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", skill_file.display()))?
        .to_path_buf();

    Ok(ExternalSkillDefinition {
        display_name: parsed.name.clone().unwrap_or_else(|| canonical_name.clone()),
        canonical_name,
        description,
        body: content,
        frontmatter: SkillFrontmatter {
            name: parsed.name,
            description: parsed.description,
            raw: parsed.raw_frontmatter,
            extra: HashMap::new(),
        },
        source: root.source.clone(),
        source_path: skill_file.to_path_buf(),
        base_dir,
        shadowed: Vec::new(),
    })
}

fn levenshtein(a: &str, b: &str) -> usize {
    let mut costs: Vec<usize> = (0..=b.chars().count()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut last = i;
        costs[0] = i + 1;
        for (j, cb) in b.chars().enumerate() {
            let old = costs[j + 1];
            costs[j + 1] = if ca == cb {
                last
            } else {
                1 + last.min(costs[j]).min(costs[j + 1])
            };
            last = old;
        }
    }
    costs[b.chars().count()]
}
```

- [ ] **Step 4: Export registry types**

Update `src/knowledge/mod.rs`:

```rust
pub mod external_registry;

pub use external_registry::{ExternalSkillRegistry, ExternalSkillRoot};
```

- [ ] **Step 5: Run registry tests**

Run:

```bash
cargo test --test skills_test external_registry -- --nocapture
```

Expected: PASS.

- [ ] **Step 6: Commit Task 2**

```bash
git add src/knowledge/external_registry.rs src/knowledge/mod.rs tests/skills_test.rs
git commit -m "feat(skills): discover external skill roots"
```

## Task 3: Policy hooks and loaded skill context

**Files:**
- Create: `src/knowledge/policy.rs`
- Modify: `src/knowledge/mod.rs`
- Test: `tests/skills_test.rs`

- [ ] **Step 1: Write failing tests for default policy and denial**

Add tests to `tests/skills_test.rs`:

```rust
use wgenty_code::knowledge::{
    DefaultAllowPolicy, LoadedSkillContext, LoadedSkillRecord, PolicyDecision, SkillLoadEvent,
    SkillPolicy,
};

#[test]
fn test_default_allow_policy_allows_skill_load() {
    let policy = DefaultAllowPolicy::default();
    let context = LoadedSkillContext::default();
    let event = SkillLoadEvent {
        skill_name: "comet".to_string(),
        args: Some("hello".to_string()),
        depth: 0,
        loaded_context: context,
    };

    assert!(matches!(policy.before_skill_load(&event), PolicyDecision::Allow));
}

#[test]
fn test_loaded_skill_context_prevents_duplicate_body_injection() {
    let mut context = LoadedSkillContext::default();
    let first = LoadedSkillRecord {
        name: "comet".to_string(),
        source_path: "one/SKILL.md".into(),
        base_dir: "one".into(),
        args: Some("a".to_string()),
        parent: None,
        depth: 0,
        turn_id: 1,
    };

    assert!(context.record_load(first.clone()));
    assert!(!context.record_load(first));
    assert_eq!(context.records().len(), 1);
}

#[test]
fn test_loaded_skill_context_depth_limit() {
    let context = LoadedSkillContext::default();
    assert!(context.depth_allowed(8));
    assert!(!context.depth_allowed(9));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
cargo test --test skills_test loaded_skill_context default_allow_policy -- --nocapture
```

Expected: FAIL because policy and context types do not exist.

- [ ] **Step 3: Implement `src/knowledge/policy.rs`**

Create `src/knowledge/policy.rs`:

```rust
use std::path::PathBuf;

pub const MAX_NESTED_SKILL_DEPTH: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow,
    Warn { message: String },
    Deny { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedSkillRecord {
    pub name: String,
    pub source_path: PathBuf,
    pub base_dir: PathBuf,
    pub args: Option<String>,
    pub parent: Option<String>,
    pub depth: usize,
    pub turn_id: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoadedSkillContext {
    records: Vec<LoadedSkillRecord>,
}

impl LoadedSkillContext {
    pub fn record_load(&mut self, record: LoadedSkillRecord) -> bool {
        if self.records.iter().any(|existing| {
            existing.name == record.name && existing.source_path == record.source_path
        }) {
            return false;
        }
        self.records.push(record);
        true
    }

    pub fn records(&self) -> &[LoadedSkillRecord] {
        &self.records
    }

    pub fn depth_allowed(&self, requested_depth: usize) -> bool {
        requested_depth <= MAX_NESTED_SKILL_DEPTH
    }
}

#[derive(Debug, Clone)]
pub struct SkillLoadEvent {
    pub skill_name: String,
    pub args: Option<String>,
    pub depth: usize,
    pub loaded_context: LoadedSkillContext,
}

#[derive(Debug, Clone)]
pub struct NestedSkillCallEvent {
    pub parent: Option<String>,
    pub child: String,
    pub depth: usize,
    pub loaded_context: LoadedSkillContext,
}

#[derive(Debug, Clone)]
pub struct ToolCallObservedEvent {
    pub tool_name: String,
    pub loaded_context: LoadedSkillContext,
}

pub trait SkillPolicy: Send + Sync {
    fn before_skill_load(&self, _event: &SkillLoadEvent) -> PolicyDecision {
        PolicyDecision::Allow
    }

    fn before_nested_skill_call(&self, _event: &NestedSkillCallEvent) -> PolicyDecision {
        PolicyDecision::Allow
    }

    fn before_tool_call_observed(&self, _event: &ToolCallObservedEvent) -> PolicyDecision {
        PolicyDecision::Allow
    }
}

#[derive(Debug, Default)]
pub struct DefaultAllowPolicy;

impl SkillPolicy for DefaultAllowPolicy {}
```

- [ ] **Step 4: Export policy types**

Update `src/knowledge/mod.rs`:

```rust
pub mod policy;

pub use policy::{
    DefaultAllowPolicy, LoadedSkillContext, LoadedSkillRecord, NestedSkillCallEvent,
    PolicyDecision, SkillLoadEvent, SkillPolicy, ToolCallObservedEvent, MAX_NESTED_SKILL_DEPTH,
};
```

- [ ] **Step 5: Run policy tests**

Run:

```bash
cargo test --test skills_test loaded_skill_context default_allow_policy -- --nocapture
```

Expected: PASS.

- [ ] **Step 6: Commit Task 3**

```bash
git add src/knowledge/policy.rs src/knowledge/mod.rs tests/skills_test.rs
git commit -m "feat(skills): add skill policy hooks"
```

## Task 4: Claude-compatible `skill` tool

**Files:**
- Create: `src/tools/meta/skill.rs`
- Modify: `src/tools/meta/mod.rs`
- Modify: `src/tools/mod.rs`
- Test: `tests/tools_test.rs`

- [ ] **Step 1: Write failing tests for tool registration and schema**

Add tests to `tests/tools_test.rs`:

```rust
#[tokio::test]
async fn test_skill_tool_registered() {
    let registry = ToolRegistry::new();
    let tool = registry.get("skill").expect("skill tool should exist");

    assert_eq!(tool.name(), "skill");
    assert!(tool.is_read_only());
    assert!(tool.description().contains("skill"));

    let schema = tool.input_schema();
    assert!(schema["properties"].get("skill").is_some());
    assert!(schema["properties"].get("args").is_some());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run:

```bash
cargo test --test tools_test test_skill_tool_registered -- --nocapture
```

Expected: FAIL because `skill` tool is not registered.

- [ ] **Step 3: Implement `src/tools/meta/skill.rs`**

Create a minimal read-only tool first. It can return a clear not-configured error until the registry is wired in Task 5:

```rust
use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;

pub struct SkillTool;

impl SkillTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for SkillTool {
    fn name(&self) -> &str {
        "skill"
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn description(&self) -> &str {
        "Load a Claude Code-compatible external skill by canonical name. Use for nested skill invocation."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "skill": {
                    "type": "string",
                    "description": "Canonical external skill name to load"
                },
                "args": {
                    "type": "string",
                    "description": "Optional raw arguments passed to the skill"
                }
            },
            "required": ["skill"]
        })
    }

    async fn execute(&self, _input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        Err(ToolError {
            message: "External skill registry is not configured".to_string(),
            code: Some("skill_registry_unconfigured".to_string()),
        })
    }
}
```

- [ ] **Step 4: Export and register the tool**

Update `src/tools/meta/mod.rs`:

```rust
pub mod skill;
pub use skill::SkillTool;
```

Update `src/tools/mod.rs` registry construction near meta tools:

```rust
registry.register(Box::new(meta::skill::SkillTool::new()));
```

Update re-export list:

```rust
SkillTool,
```

- [ ] **Step 5: Run tool registration test**

Run:

```bash
cargo test --test tools_test test_skill_tool_registered -- --nocapture
```

Expected: PASS.

- [ ] **Step 6: Commit Task 4**

```bash
git add src/tools/meta/skill.rs src/tools/meta/mod.rs src/tools/mod.rs tests/tools_test.rs
git commit -m "feat(tools): add skill runtime tool"
```

## Task 5: Wire registry-backed skill loading

**Files:**
- Modify: `src/tools/meta/skill.rs`
- Modify: `src/tools/meta/load_skill.rs`
- Modify: `src/tools/mod.rs`
- Test: `tests/tools_test.rs`

- [ ] **Step 1: Write failing tests for registry-backed skill tool execution**

Add tests to `tests/tools_test.rs`:

```rust
#[tokio::test]
async fn test_skill_tool_loads_external_skill_body() {
    use serde_json::json;
    use std::fs;
    use std::sync::Arc;
    use tempfile::TempDir;
    use wgenty_code::knowledge::{
        ExternalSkillRegistry, ExternalSkillRoot, ExternalSkillSource, LoadedSkillContext,
    };
    use wgenty_code::tools::meta::SkillTool;
    use wgenty_code::tools::Tool;

    let repo = TempDir::new().unwrap();
    let root = repo.path().join(".wgenty-code/skills");
    let skill_dir = root.join("comet");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: comet\ndescription: Comet\n---\n# Comet\nInstructions.",
    )
    .unwrap();

    let registry = ExternalSkillRegistry::discover(vec![ExternalSkillRoot::new(
        root.clone(),
        ExternalSkillSource::ProjectWgentyCode { root },
    )])
    .unwrap();

    let tool = SkillTool::with_registry(Arc::new(registry), LoadedSkillContext::default());
    let output = tool
        .execute(json!({"skill": "comet", "args": "hello"}))
        .await
        .expect("skill should load");

    assert_eq!(output.output_type, "markdown");
    assert!(output.content.contains("Base directory for this skill:"));
    assert!(output.content.contains("# Comet"));
    assert!(output.content.contains("ARGUMENTS: hello"));
}

#[tokio::test]
async fn test_skill_tool_missing_skill_suggests_similar_name() {
    use serde_json::json;
    use std::fs;
    use std::sync::Arc;
    use tempfile::TempDir;
    use wgenty_code::knowledge::{
        ExternalSkillRegistry, ExternalSkillRoot, ExternalSkillSource, LoadedSkillContext,
    };
    use wgenty_code::tools::meta::SkillTool;
    use wgenty_code::tools::Tool;

    let repo = TempDir::new().unwrap();
    let root = repo.path().join(".wgenty-code/skills");
    fs::create_dir_all(root.join("comet")).unwrap();
    fs::write(root.join("comet/SKILL.md"), "---\nname: comet\ndescription: Comet\n---\n# Comet").unwrap();

    let registry = ExternalSkillRegistry::discover(vec![ExternalSkillRoot::new(
        root.clone(),
        ExternalSkillSource::ProjectWgentyCode { root },
    )])
    .unwrap();
    let tool = SkillTool::with_registry(Arc::new(registry), LoadedSkillContext::default());

    let error = tool
        .execute(json!({"skill": "comte"}))
        .await
        .expect_err("missing skill should error");

    assert_eq!(error.code.as_deref(), Some("skill_not_found"));
    assert!(error.message.contains("comet"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
cargo test --test tools_test skill_tool_ -- --nocapture
```

Expected: FAIL because `SkillTool::with_registry` and registry-backed execution do not exist.

- [ ] **Step 3: Implement registry-backed `SkillTool`**

Update `src/tools/meta/skill.rs`:

```rust
use crate::knowledge::{ExternalSkillRegistry, LoadedSkillContext, LoadedSkillRecord};
use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;
use std::sync::{Arc, Mutex};

pub struct SkillTool {
    registry: Option<Arc<ExternalSkillRegistry>>,
    loaded_context: Arc<Mutex<LoadedSkillContext>>,
}

impl SkillTool {
    pub fn new() -> Self {
        Self {
            registry: None,
            loaded_context: Arc::new(Mutex::new(LoadedSkillContext::default())),
        }
    }

    pub fn with_registry(
        registry: Arc<ExternalSkillRegistry>,
        loaded_context: LoadedSkillContext,
    ) -> Self {
        Self {
            registry: Some(registry),
            loaded_context: Arc::new(Mutex::new(loaded_context)),
        }
    }
}

#[async_trait]
impl Tool for SkillTool {
    fn name(&self) -> &str {
        "skill"
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn description(&self) -> &str {
        "Load a Claude Code-compatible external skill by canonical name. Use for nested skill invocation."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "skill": {"type": "string", "description": "Canonical external skill name to load"},
                "args": {"type": "string", "description": "Optional raw arguments passed to the skill"},
                "depth": {"type": "integer", "description": "Nested skill depth; defaults to 0"}
            },
            "required": ["skill"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let registry = self.registry.as_ref().ok_or_else(|| ToolError {
            message: "External skill registry is not configured".to_string(),
            code: Some("skill_registry_unconfigured".to_string()),
        })?;

        let skill_name = input["skill"].as_str().ok_or_else(|| ToolError {
            message: "Missing required field: skill".to_string(),
            code: Some("invalid_input".to_string()),
        })?;
        let args = input["args"].as_str().map(|value| value.to_string());
        let depth = input["depth"].as_u64().unwrap_or(0) as usize;

        let mut context = self.loaded_context.lock().map_err(|_| ToolError {
            message: "Loaded skill context lock poisoned".to_string(),
            code: Some("skill_context_error".to_string()),
        })?;

        if !context.depth_allowed(depth) {
            return Err(ToolError {
                message: format!("Nested skill depth {} exceeds maximum depth 8", depth),
                code: Some("skill_depth_exceeded".to_string()),
            });
        }

        let skill = registry.resolve(skill_name).ok_or_else(|| {
            let suggestions = registry.suggest(skill_name, 3);
            let suffix = if suggestions.is_empty() {
                String::new()
            } else {
                format!(" Did you mean: {}?", suggestions.join(", "))
            };
            ToolError {
                message: format!("Skill '{}' not found.{}", skill_name, suffix),
                code: Some("skill_not_found".to_string()),
            }
        })?;

        let was_new = context.record_load(LoadedSkillRecord {
            name: skill.canonical_name.clone(),
            source_path: skill.source_path.clone(),
            base_dir: skill.base_dir.clone(),
            args: args.clone(),
            parent: None,
            depth,
            turn_id: 0,
        });

        let content = if was_new {
            format!(
                "Base directory for this skill: {}\n\n{}\n\nARGUMENTS: {}",
                skill.base_dir.display(),
                skill.body,
                args.as_deref().unwrap_or("")
            )
        } else {
            format!(
                "Skill '{}' is already loaded from {}. Invocation recorded.\n\nARGUMENTS: {}",
                skill.canonical_name,
                skill.source_path.display(),
                args.as_deref().unwrap_or("")
            )
        };

        Ok(ToolOutput {
            output_type: "markdown".to_string(),
            content,
            metadata: std::collections::HashMap::from([
                (
                    "skill_name".to_string(),
                    serde_json::Value::String(skill.canonical_name.clone()),
                ),
                (
                    "source_path".to_string(),
                    serde_json::Value::String(skill.source_path.display().to_string()),
                ),
            ]),
        })
    }
}
```

- [ ] **Step 4: Preserve `load_skill` compatibility**

Do not remove `LoadSkillTool`. If changing it to use `ExternalSkillRegistry`, keep its current behavior:

- empty `name` returns list
- non-empty `name` returns markdown body
- read-only remains true

If no registry is available in `ToolRegistry::new()`, keep `load_skill` unchanged for now and only add registry-backed behavior in call sites that can supply a registry.

- [ ] **Step 5: Run skill tool tests**

Run:

```bash
cargo test --test tools_test skill_tool_ -- --nocapture
```

Expected: PASS.

- [ ] **Step 6: Commit Task 5**

```bash
git add src/tools/meta/skill.rs src/tools/meta/load_skill.rs src/tools/mod.rs tests/tools_test.rs
git commit -m "feat(skills): load external skills with skill tool"
```

## Task 6: Prompt inventory and slash routing

**Files:**
- Modify: `src/prompts/mod.rs`
- Modify: slash command handling file discovered by search before implementation
- Test: add or extend the nearest prompt/routing test file

- [ ] **Step 1: Locate slash command routing**

Run:

```bash
grep -R "strip_prefix(\"/\"\|slash\|command" -n src tests
```

Expected: identify the smallest existing handler for user slash commands. If none exists, create a pure helper in the module that receives `(input, builtins, external_registry)` and returns a routing enum.

- [ ] **Step 2: Write failing tests for routing helper**

Add tests near the routing helper:

```rust
#[test]
fn test_slash_routing_external_skill_fallback() {
    let result = route_slash_command_for_test("/comet hello", &["help", "clear"], &["comet"]);
    assert_eq!(result.command_name(), "comet");
    assert_eq!(result.args(), "hello");
    assert!(result.is_external_skill());
}

#[test]
fn test_slash_routing_builtin_wins() {
    let result = route_slash_command_for_test("/help", &["help"], &["help"]);
    assert!(result.is_builtin());
}
```

- [ ] **Step 3: Implement built-in-first external fallback**

Implement the smallest routing helper needed by the existing loop:

```rust
pub enum SlashRoute {
    BuiltIn { command: String, args: String },
    ExternalSkill { skill: String, args: String },
    Unknown { command: String, suggestions: Vec<String> },
    NotSlash,
}
```

Rules:

- only route inputs that start with `/`
- split once on whitespace
- command excludes leading `/`
- args preserves the raw tail
- built-ins win over external skills
- unknown returns suggestions from `ExternalSkillRegistry::suggest`

- [ ] **Step 4: Wire prompt inventory**

Where prompt context is built, convert `ExternalSkillRegistry::list()` into `Vec<SkillEntry>`:

```rust
SkillEntry {
    name: skill.canonical_name.clone(),
    description: skill.description.clone(),
}
```

Keep `prompts/mod.rs` behavior compact: names and descriptions only.

- [ ] **Step 5: Run routing/prompt tests**

Run the nearest tests identified in Step 1. Also run:

```bash
cargo test --test skills_test external_registry -- --nocapture
cargo test --test tools_test test_skill_tool_registered -- --nocapture
```

Expected: PASS.

- [ ] **Step 6: Commit Task 6**

```bash
git add src/prompts/mod.rs <slash-routing-file> <routing-test-file>
git commit -m "feat(skills): route slash commands to external skills"
```

## Task 7: Plugin cache discovery

**Files:**
- Modify: `src/knowledge/external_registry.rs`
- Potentially modify: `src/plugins/registry.rs` or the existing plugin cache integration point discovered during implementation
- Test: `tests/skills_test.rs`

- [ ] **Step 1: Write failing plugin cache fixture test**

Add to `tests/skills_test.rs`:

```rust
#[test]
fn test_external_registry_discovers_plugin_cache_skill() {
    use std::fs;
    use tempfile::TempDir;
    use wgenty_code::knowledge::{ExternalSkillRegistry, ExternalSkillRoot, ExternalSkillSource};

    let cache = TempDir::new().unwrap();
    let plugin_root = cache.path().join("anthropic/superpowers/5.1.0");
    let skills_root = plugin_root.join("skills");
    fs::create_dir_all(skills_root.join("brainstorming")).unwrap();
    fs::write(
        plugin_root.join("package.json"),
        r#"{"name":"@anthropic/superpowers","version":"5.1.0","main":"index.js"}"#,
    )
    .unwrap();
    fs::write(
        skills_root.join("brainstorming/SKILL.md"),
        "---\nname: superpowers:brainstorming\ndescription: Brainstorming\n---\n# Brainstorming",
    )
    .unwrap();

    let registry = ExternalSkillRegistry::discover(vec![ExternalSkillRoot::new(
        skills_root.clone(),
        ExternalSkillSource::PluginCache {
            plugin_name: "superpowers".to_string(),
            version: Some("5.1.0".to_string()),
            root: skills_root,
        },
    )])
    .unwrap();

    let skill = registry
        .resolve("superpowers:brainstorming")
        .expect("plugin cache skill should resolve");
    assert_eq!(skill.description, "Brainstorming");
    assert!(skill.source.label().contains("plugin:superpowers@5.1.0"));
}
```

- [ ] **Step 2: Run plugin cache test**

Run:

```bash
cargo test --test skills_test plugin_cache_skill -- --nocapture
```

Expected: FAIL if plugin cache source handling is incomplete, PASS if Task 2 already covers it generically. If it passes, still keep the fixture as regression coverage.

- [ ] **Step 3: Add helper for enabled plugin roots if needed**

If implementation needs a helper, add a method such as:

```rust
ExternalSkillRoot::plugin_cache(plugin_name, version, plugin_root.join("skills"))
```

Do not execute plugin commands or load plugin modules for instruction skill discovery.

- [ ] **Step 4: Run plugin cache test again**

Run:

```bash
cargo test --test skills_test plugin_cache_skill -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Commit Task 7**

```bash
git add src/knowledge/external_registry.rs tests/skills_test.rs
git commit -m "feat(skills): discover plugin cache skills"
```

## Task 8: Full verification and OpenSpec task sync

**Files:**
- Modify: `openspec/changes/external-skill-runtime/tasks.md`
- Maybe modify docs if implementation exposed user-facing behavior

- [ ] **Step 1: Run focused tests**

Run:

```bash
cargo test --test skills_test external_skill -- --nocapture
cargo test --test skills_test external_registry -- --nocapture
cargo test --test skills_test loaded_skill_context default_allow_policy -- --nocapture
cargo test --test tools_test skill_tool_ -- --nocapture
```

Expected: all PASS.

- [ ] **Step 2: Run broader tests**

Run:

```bash
cargo test --all
```

Expected: PASS.

- [ ] **Step 3: Run format and clippy**

Run:

```bash
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
```

Expected: PASS.

- [ ] **Step 4: Manually verify minimal external skill flow**

Create a temporary fixture outside committed source or use a test fixture:

```text
.wgenty-code/skills/comet/SKILL.md
.wgenty-code/skills/comet-open/SKILL.md
```

Verify through tests or local runtime that:

```text
/comet test
```

loads `comet`, preserves `ARGUMENTS: test`, and nested `skill({ skill: "comet-open" })` loads the child skill.

Expected: skill body includes base directory, markdown body, and arguments.

- [ ] **Step 5: Update OpenSpec task checkboxes after verified implementation**

Only after implementation and tests pass, update `openspec/changes/external-skill-runtime/tasks.md` checkboxes corresponding to completed work.

- [ ] **Step 6: Commit verification/task sync**

```bash
git add openspec/changes/external-skill-runtime/tasks.md
# Include any docs updated during verification.
git commit -m "chore: mark external skill runtime tasks complete"
```

## Self-Review Checklist

- Spec coverage: discovery, metadata parsing, conflict resolution, available listing, slash routing, nested Skill action, loaded context, policy hooks, plugin cache discovery, and depth limit are covered by tasks.
- Placeholder scan: no `TBD`, `TODO`, or vague implementation-only placeholders are intentionally left in this plan.
- Type consistency: plan consistently uses `ExternalSkillDefinition`, `ExternalSkillSource`, `ExternalSkillRegistry`, `ExternalSkillRoot`, `LoadedSkillContext`, `SkillTool`, and `PolicyDecision`.
- TDD path: each implementation task starts with failing tests and includes exact commands and expected outcomes.
