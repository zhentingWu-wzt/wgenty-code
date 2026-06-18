//! Tests for Skills Framework

use std::sync::Arc;
use wgenty_code::knowledge::{
    BuiltinSkills, SkillCategory, SkillContext, SkillExecutor, SkillRegistry,
};

#[test]
fn test_skill_registry_creation() {
    let registry = SkillRegistry::new();
    assert!(registry.list_names().is_empty());
}

#[test]
fn test_skill_registration() {
    let mut registry = SkillRegistry::new();

    // Register built-in skills
    for (skill, categories) in BuiltinSkills::all() {
        registry.register(skill, categories);
    }

    // Should have 5 skills now
    assert_eq!(registry.list_names().len(), 5);

    // Check specific skills exist
    assert!(registry.has("commit"));
    assert!(registry.has("review"));
    assert!(registry.has("test"));
    assert!(registry.has("document"));
    assert!(registry.has("build"));
}

#[test]
fn test_skill_search() {
    let mut registry = SkillRegistry::new();

    // Register built-in skills
    for (skill, categories) in BuiltinSkills::all() {
        registry.register(skill, categories);
    }

    // Search for "commit"
    let results = registry.search("commit");
    assert!(!results.is_empty());
}

#[test]
fn test_skill_categories() {
    let mut registry = SkillRegistry::new();

    // Register built-in skills
    for (skill, categories) in BuiltinSkills::all() {
        registry.register(skill, categories);
    }

    // Check Git category
    let git_skills = registry.list_by_category(SkillCategory::Git);
    assert!(!git_skills.is_empty());

    // Check Utility category (should have multiple)
    let utility_skills = registry.list_by_category(SkillCategory::Utility);
    assert_eq!(utility_skills.len(), 5);
}

#[tokio::test]
async fn test_skill_executor() {
    let mut registry = SkillRegistry::new();

    // Register built-in skills
    for (skill, categories) in BuiltinSkills::all() {
        registry.register(skill, categories);
    }

    let registry_arc = Arc::new(registry);
    let executor = SkillExecutor::new(registry_arc);

    // List skills
    let skills = executor.list_skills();
    assert_eq!(skills.len(), 5);

    // Execute commit skill
    let context = SkillContext {
        cwd: ".".to_string(),
        env: std::collections::HashMap::new(),
        tool_registry: None,
        data: std::collections::HashMap::new(),
    };

    let result = executor.execute("commit", "", context).await;
    assert!(result.is_ok());
}

#[test]
fn test_skill_help() {
    let mut registry = SkillRegistry::new();

    // Register built-in skills
    for (skill, categories) in BuiltinSkills::all() {
        registry.register(skill, categories);
    }

    let registry_arc = Arc::new(registry);
    let executor = SkillExecutor::new(registry_arc);

    // Get help for commit skill
    let help = executor.get_help("commit");
    assert!(help.is_ok());
    let help_text = help.unwrap();
    assert!(help_text.contains("Skill: commit"));
    assert!(help_text.contains("Examples:"));
}

#[test]
fn test_skill_parameter_parsing() {
    let mut registry = SkillRegistry::new();

    // Register built-in skills
    for (skill, categories) in BuiltinSkills::all() {
        registry.register(skill, categories);
    }

    let registry_arc = Arc::new(registry);
    let executor = SkillExecutor::new(registry_arc);

    // Parse input with flags
    let params = executor.parse_input("--message=\"test message\" --verbose");
    assert_eq!(
        params.named_params.get("message"),
        Some(&"test message".to_string())
    );
    assert!(params.flags.contains_key("verbose"));

    // Parse input with positional args
    let params2 = executor.parse_input("file1.rs file2.rs");
    assert_eq!(params2.args.len(), 2);
    assert_eq!(params2.args[0], "file1.rs");
    assert_eq!(params2.args[1], "file2.rs");
}

use std::path::PathBuf;
use wgenty_code::knowledge::{
    derive_canonical_skill_name, parse_external_skill_document, ExternalSkillError,
    ExternalSkillSource,
};

#[test]
fn test_external_skill_error_display_unclosed() {
    let err = ExternalSkillError::UnclosedFrontmatter;
    assert_eq!(err.to_string(), "frontmatter has no closing marker");
}

#[test]
fn test_external_skill_error_display_path_not_under_root() {
    let err = ExternalSkillError::PathNotUnderRoot(
        PathBuf::from("/other/skill.md"),
        PathBuf::from("/skills"),
    );
    let msg = err.to_string();
    assert!(msg.contains("is not under"));
}

#[test]
fn test_external_skill_error_display_unsupported_path() {
    let err = ExternalSkillError::UnsupportedPath(PathBuf::from(
        "skills/a/b/c/SKILL.md",
    ));
    let msg = err.to_string();
    assert!(msg.contains("unsupported skill path"));
}

#[test]
fn test_parse_external_skill_returns_external_skill_error() {
    let result: Result<_, ExternalSkillError> = parse_external_skill_document("---\nname: comet");
    assert!(result.is_err());
}

#[test]
fn test_skill_frontmatter_raw_frontmatter_field() {
    let extra = std::collections::HashMap::new();
    let fm = wgenty_code::knowledge::SkillFrontmatter {
        name: Some("test".to_string()),
        description: None,
        raw_frontmatter: "name: test".to_string(),
        extra: extra.clone(),
    };
    assert_eq!(fm.raw_frontmatter, "name: test");
}

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
fn test_external_skill_frontmatter_no_closing_marker() {
    let body = "---\nname: comet";
    let result = parse_external_skill_document(body);
    assert!(result.is_err());
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

use std::fs;
use tempfile::TempDir;

fn write_skill(root: &std::path::Path, relative: &str, content: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

use wgenty_code::knowledge::{
    route_slash_command, ExternalSkillRegistry, ExternalSkillRoot, SlashRoute,
};

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

    write_skill(repo.path(), ".wgenty-code/skills/comet/SKILL.md",
        "---\nname: comet\ndescription: Project Comet\n---\n# Project");
    write_skill(user.path(), ".wgenty-code/skills/comet/SKILL.md",
        "---\nname: comet\ndescription: User Comet\n---\n# User");

    let registry = ExternalSkillRegistry::discover(vec![
        ExternalSkillRoot::new(repo.path().join(".wgenty-code/skills"),
            ExternalSkillSource::ProjectWgentyCode { root: repo.path().join(".wgenty-code/skills") }),
        ExternalSkillRoot::new(user.path().join(".wgenty-code/skills"),
            ExternalSkillSource::UserWgentyCode { root: user.path().join(".wgenty-code/skills") }),
    ]).expect("registry should discover skills");

    let skill = registry.resolve("comet").expect("comet should resolve");
    assert_eq!(skill.description, "Project Comet");
    assert_eq!(skill.shadowed.len(), 1);
    assert!(registry.diagnostics().join("\n").contains("shadowed"));
}

#[test]
fn test_external_skill_error_io_error() {
    let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
    let err = ExternalSkillError::IoError(io_err);
    let msg = err.to_string();
    assert!(msg.contains("I/O error"));
}

#[test]
fn test_external_skill_error_no_parent_directory() {
    let err = ExternalSkillError::NoParentDirectory(PathBuf::from("/"));
    let msg = err.to_string();
    assert!(msg.contains("no parent directory"));
}

#[test]
fn test_external_registry_suggests_similar_names() {
    let repo = TempDir::new().unwrap();
    write_skill(repo.path(), ".wgenty-code/skills/comet/SKILL.md",
        "---\nname: comet\ndescription: Comet\n---\n# Comet");

    let registry = ExternalSkillRegistry::discover(vec![ExternalSkillRoot::new(
        repo.path().join(".wgenty-code/skills"),
        ExternalSkillSource::ProjectWgentyCode { root: repo.path().join(".wgenty-code/skills") },
    )]).expect("registry should discover skills");

    assert_eq!(registry.suggest("comte", 3), vec!["comet".to_string()]);
}

use wgenty_code::knowledge::{
    DefaultAllowPolicy, LoadedSkillContext, LoadedSkillRecord, PolicyDecision, SkillLoadEvent,
    SkillPolicy, MAX_NESTED_SKILL_DEPTH,
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
    assert!(context.depth_allowed(MAX_NESTED_SKILL_DEPTH));
    assert!(!context.depth_allowed(MAX_NESTED_SKILL_DEPTH + 1));
}

// ── Slash route tests ──────────────────────────────────────────────────────

#[test]
fn test_slash_route_builtin_wins() {
    let result = route_slash_command("/help me", &["help", "clear"], None);
    assert!(matches!(result, SlashRoute::BuiltIn { command, .. } if command == "help"));
}

#[test]
fn test_slash_route_not_slash() {
    let result = route_slash_command("hello world", &[], None);
    assert_eq!(result, SlashRoute::NotSlash);
}

#[test]
fn test_slash_route_unknown_command() {
    let result = route_slash_command("/unknown_cmd", &["help"], None);
    assert!(matches!(result, SlashRoute::Unknown { ref command, .. } if command == "unknown_cmd"));
}

#[test]
fn test_slash_route_builtin_without_args() {
    let result = route_slash_command("/clear", &["help", "clear"], None);
    assert!(matches!(
        result,
        SlashRoute::BuiltIn { ref command, ref args } if command == "clear" && args.is_empty()
    ));
}

#[test]
fn test_slash_route_external_skill_resolved_with_registry() {
    let repo = tempfile::TempDir::new().unwrap();
    let root = repo.path().join(".wgenty-code/skills");
    std::fs::create_dir_all(root.join("comet")).unwrap();
    std::fs::write(
        root.join("comet/SKILL.md"),
        "---\nname: comet\ndescription: Comet\n---\n# Comet",
    )
    .unwrap();

    let registry = ExternalSkillRegistry::discover(vec![ExternalSkillRoot::new(
        root.clone(),
        ExternalSkillSource::ProjectWgentyCode { root },
    )])
    .expect("registry should discover skills");

    let result = route_slash_command("/comet hello", &["help"], Some(&registry));
    assert!(matches!(
        result,
        SlashRoute::ExternalSkill { ref skill, ref args } if skill == "comet" && args == "hello"
    ));
}

#[test]
fn test_slash_route_unknown_with_suggestions() {
    let repo = tempfile::TempDir::new().unwrap();
    let root = repo.path().join(".wgenty-code/skills");
    std::fs::create_dir_all(root.join("comet")).unwrap();
    std::fs::write(
        root.join("comet/SKILL.md"),
        "---\nname: comet\ndescription: Comet\n---\n# Comet",
    )
    .unwrap();

    let registry = ExternalSkillRegistry::discover(vec![ExternalSkillRoot::new(
        root.clone(),
        ExternalSkillSource::ProjectWgentyCode { root },
    )])
    .expect("registry should discover skills");

    let result = route_slash_command("/comte hello", &["help"], Some(&registry));
    assert!(matches!(
        result,
        SlashRoute::Unknown { ref command, ref suggestions, .. } if command == "comte" && !suggestions.is_empty()
    ));
}

#[test]
fn test_slash_route_no_registry_fallback_to_unknown() {
    let result = route_slash_command("/comet hello", &["help"], None);
    assert!(matches!(result, SlashRoute::Unknown { ref command, .. } if command == "comet"));
}

#[test]
fn test_external_registry_discovers_plugin_cache_skill() {
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
