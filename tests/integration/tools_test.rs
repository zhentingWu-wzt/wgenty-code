//! Tests for Tools Module

use std::fs;
use wgenty_code::tools::ToolRegistry;

#[tokio::test]
async fn test_tool_registry_creation() {
    let registry = ToolRegistry::new();
    let tools = registry.list();

    // Should have 9 tools now (6 original + 3 new)
    assert!(tools.len() >= 6);
}

#[tokio::test]
async fn test_file_read_tool() {
    let registry = ToolRegistry::new();
    let tool = registry
        .get("file_read")
        .expect("file_read tool should exist");

    assert_eq!(tool.name(), "file_read");
    assert!(!tool.description().is_empty());
}

#[tokio::test]
async fn test_git_operations_tool() {
    let registry = ToolRegistry::new();
    let tool = registry
        .get("git_operations")
        .expect("git_operations tool should exist");

    assert_eq!(tool.name(), "git_operations");
    assert!(tool.description().contains("Git"));
}

#[tokio::test]
async fn test_task_management_tool() {
    let registry = ToolRegistry::new();
    let tool = registry
        .get("task_management")
        .expect("task_management tool should exist");

    assert_eq!(tool.name(), "task_management");
    assert!(tool.description().contains("task"));
}

#[tokio::test]
async fn test_note_edit_tool() {
    let registry = ToolRegistry::new();
    let tool = registry
        .get("note_edit")
        .expect("note_edit tool should exist");

    assert_eq!(tool.name(), "note_edit");
    assert!(tool.description().contains("note"));
}

#[tokio::test]
async fn test_task_create_and_list() {
    use serde_json::json;

    let registry = ToolRegistry::new();

    // Create a task
    let create_result = registry
        .execute(
            "task_management",
            json!({
                "operation": "create",
                "subject": "Test Task",
                "description": "This is a test task"
            }),
        )
        .await;

    assert!(create_result.is_ok());

    // List tasks
    let list_result = registry
        .execute(
            "task_management",
            json!({
                "operation": "list"
            }),
        )
        .await;

    assert!(list_result.is_ok());
}

#[tokio::test]
async fn test_note_create_and_search() {
    use serde_json::json;

    let registry = ToolRegistry::new();

    // Create a note
    let create_result = registry
        .execute(
            "note_edit",
            json!({
                "operation": "create",
                "title": "Test Note",
                "content": "This is a test note content",
                "tags": ["test", "example"]
            }),
        )
        .await;

    assert!(create_result.is_ok());

    // Search notes
    let search_result = registry
        .execute(
            "note_edit",
            json!({
                "operation": "search",
                "search_query": "test"
            }),
        )
        .await;

    assert!(search_result.is_ok());
}

#[tokio::test]
async fn test_exec_command_session_lifecycle() {
    use serde_json::json;

    let registry = ToolRegistry::new();

    let start_result = registry
        .execute(
            "exec_command",
            json!({
                "command": "printf 'hello'",
                "yield_time_ms": 50,
                "max_output_chars": 200
            }),
        )
        .await
        .expect("exec_command should succeed");

    assert_eq!(start_result.output_type, "text");
    assert!(start_result.content.contains("hello"));

    let session_id = start_result.metadata["session_id"]
        .as_u64()
        .expect("session_id should be present");

    let followup_result = registry
        .execute(
            "write_stdin",
            json!({
                "session_id": session_id,
                "chars": "",
                "yield_time_ms": 10,
                "max_output_chars": 200
            }),
        )
        .await
        .expect("write_stdin should succeed");

    assert_eq!(followup_result.metadata["session_id"], json!(session_id));
}

#[tokio::test]
async fn test_exec_command_interactive_io() {
    use serde_json::json;

    let registry = ToolRegistry::new();

    let start_result = registry
        .execute(
            "exec_command",
            json!({
                "command": "read line; printf '%s' \"$line\"",
                "yield_time_ms": 10,
                "max_output_chars": 200
            }),
        )
        .await
        .expect("exec_command should succeed");

    let session_id = start_result.metadata["session_id"]
        .as_u64()
        .expect("session_id should be present");

    let io_result = registry
        .execute(
            "write_stdin",
            json!({
                "session_id": session_id,
                "chars": "world\n",
                "yield_time_ms": 50,
                "max_output_chars": 200
            }),
        )
        .await
        .expect("write_stdin should succeed");

    assert!(io_result.content.contains("world"));
}

#[tokio::test]
async fn test_kill_session_tool() {
    use serde_json::json;

    let registry = ToolRegistry::new();

    let start_result = registry
        .execute(
            "exec_command",
            json!({
                "command": "sleep 5",
                "yield_time_ms": 10,
                "max_output_chars": 100
            }),
        )
        .await
        .expect("exec_command should succeed");

    let session_id = start_result.metadata["session_id"]
        .as_u64()
        .expect("session_id should be present");

    let kill_result = registry
        .execute(
            "kill_session",
            json!({
                "session_id": session_id
            }),
        )
        .await
        .expect("kill_session should succeed");

    assert!(kill_result.content.contains("Killed session"));
}

#[tokio::test]
async fn test_apply_patch_add_update_delete() {
    use serde_json::json;
    use tempfile::tempdir;

    let dir = tempdir().expect("tempdir should be created");
    let file_path = dir.path().join("sample.txt");
    std::fs::write(&file_path, "alpha\nbeta\n").expect("seed file should be written");

    let registry = ToolRegistry::new();

    let add_result = registry
        .execute(
            "apply_patch",
            json!({
                "patch": "*** Begin Patch\n*** Add File: added.txt\n+hello\n+world\n*** End Patch",
                "workdir": dir.path()
            }),
        )
        .await
        .expect("add patch should succeed");
    assert!(add_result.content.contains("Applied patch"));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("added.txt")).expect("added file should exist"),
        "hello\nworld"
    );

    registry
        .execute(
            "apply_patch",
            json!({
                "patch": "*** Begin Patch\n*** Update File: sample.txt\n@@\n alpha\n-beta\n+gamma\n*** End Patch",
                "workdir": dir.path()
            }),
        )
        .await
        .expect("update patch should succeed");
    assert_eq!(
        std::fs::read_to_string(&file_path).expect("updated file should exist"),
        "alpha\ngamma\n"
    );

    registry
        .execute(
            "apply_patch",
            json!({
                "patch": "*** Begin Patch\n*** Delete File: added.txt\n*** End Patch",
                "workdir": dir.path()
            }),
        )
        .await
        .expect("delete patch should succeed");
    assert!(!dir.path().join("added.txt").exists());
}

#[tokio::test]
async fn test_apply_patch_context_mismatch_fails() {
    use serde_json::json;
    use tempfile::tempdir;

    let dir = tempdir().expect("tempdir should be created");
    let file_path = dir.path().join("sample.txt");
    std::fs::write(&file_path, "alpha\nbeta\n").expect("seed file should be written");

    let registry = ToolRegistry::new();

    let result = registry
        .execute(
            "apply_patch",
            json!({
                "patch": "*** Begin Patch\n*** Update File: sample.txt\n@@\n alpha\n-missing\n+gamma\n*** End Patch",
                "workdir": dir.path()
            }),
        )
        .await;

    assert!(result.is_err(), "patch should fail when context is missing");
}

#[tokio::test]
async fn test_file_read_range_and_metadata() {
    use serde_json::json;
    use tempfile::tempdir;

    let dir = tempdir().expect("tempdir should be created");
    let file_path = dir.path().join("sample.txt");
    std::fs::write(&file_path, "one\ntwo\nthree\nfour\n").expect("seed file should be written");

    let registry = ToolRegistry::new();
    let result = registry
        .execute(
            "file_read",
            json!({
                "path": file_path,
                "start_line": 2,
                "end_line": 3,
                "max_chars": 1000
            }),
        )
        .await
        .expect("file_read should succeed");

    assert!(result.content.contains("2\ttwo"));
    assert!(result.content.contains("3\tthree"));
    assert_eq!(result.metadata["total_lines"], json!(4));
}

#[tokio::test]
async fn test_grep_and_search_compatibility() {
    use serde_json::json;
    use tempfile::tempdir;

    let dir = tempdir().expect("tempdir should be created");
    let rust_file = dir.path().join("lib.rs");
    let txt_file = dir.path().join("notes.txt");
    std::fs::write(&rust_file, "fn main() {}\nlet value = 1;\n").expect("rust file should exist");
    std::fs::write(&txt_file, "fn fake()\n").expect("txt file should exist");

    let registry = ToolRegistry::new();

    let grep_result = registry
        .execute(
            "grep",
            json!({
                "path": dir.path(),
                "pattern": "fn",
                "include": ["*.rs"],
                "max_results": 10
            }),
        )
        .await
        .expect("grep should succeed");

    assert!(grep_result.content.contains("lib.rs:1"));
    assert!(!grep_result.content.contains("notes.txt"));

    let search_result = registry
        .execute(
            "search",
            json!({
                "path": dir.path(),
                "pattern": "fn",
                "file_pattern": "*.rs",
                "max_results": 10
            }),
        )
        .await
        .expect("search should succeed");

    assert!(search_result.content.contains("lib.rs:1"));
    assert!(!search_result.content.contains("notes.txt"));
}

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

#[tokio::test]
async fn test_skill_tool_not_configured_error() {
    let registry = ToolRegistry::new();
    let result = registry
        .execute("skill", serde_json::json!({"skill": "comet"}))
        .await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.code.as_deref(), Some("skill_registry_unconfigured"));
}

#[tokio::test]
async fn test_skill_tool_loads_external_skill_body() {
    use std::sync::Arc;
    use wgenty_code::knowledge::{
        ExternalSkillRegistry, ExternalSkillRoot, ExternalSkillSource, LoadedSkillContext,
    };
    use wgenty_code::tools::meta::SkillTool;
    use wgenty_code::tools::Tool;

    let repo = tempfile::tempdir().expect("tempdir should be created");
    let root = repo.path().join(".wgenty-code/skills");
    let skill_dir = root.join("comet");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: comet\ndescription: Comet workflow\n---\n# Comet\nInstructions.",
    )
    .unwrap();

    let registry = ExternalSkillRegistry::discover(vec![ExternalSkillRoot::new(
        root.clone(),
        ExternalSkillSource::ProjectWgentyCode { root: root.clone() },
    )])
    .unwrap();

    let tool = SkillTool::with_registry(Arc::new(registry), LoadedSkillContext::default());
    let output = tool
        .execute(serde_json::json!({"skill": "comet", "args": "hello"}))
        .await
        .expect("skill should load");

    assert_eq!(output.output_type, "markdown");
    assert!(output.content.contains("Base directory for this skill:"));
    assert!(output.content.contains("# Comet"));
    assert!(output.content.contains("ARGUMENTS: hello"));
}

#[tokio::test]
async fn test_skill_tool_missing_skill_suggests_similar_name() {
    use std::sync::Arc;
    use wgenty_code::knowledge::{
        ExternalSkillRegistry, ExternalSkillRoot, ExternalSkillSource, LoadedSkillContext,
    };
    use wgenty_code::tools::meta::SkillTool;
    use wgenty_code::tools::Tool;

    let repo = tempfile::tempdir().expect("tempdir should be created");
    let root = repo.path().join(".wgenty-code/skills");
    fs::create_dir_all(root.join("comet")).unwrap();
    fs::write(
        root.join("comet/SKILL.md"),
        "---\nname: comet\ndescription: Comet\n---\n# Comet",
    )
    .unwrap();

    let registry = ExternalSkillRegistry::discover(vec![ExternalSkillRoot::new(
        root.clone(),
        ExternalSkillSource::ProjectWgentyCode { root: root.clone() },
    )])
    .unwrap();
    let tool = SkillTool::with_registry(Arc::new(registry), LoadedSkillContext::default());

    let error = tool
        .execute(serde_json::json!({"skill": "comte"}))
        .await
        .expect_err("missing skill should error");

    assert_eq!(error.code.as_deref(), Some("skill_not_found"));
    assert!(error.message.contains("comet"));
}

#[tokio::test]
async fn test_skill_tool_depth_exceeded() {
    use std::sync::Arc;
    use wgenty_code::knowledge::{
        ExternalSkillRegistry, ExternalSkillRoot, ExternalSkillSource, LoadedSkillContext,
    };
    use wgenty_code::tools::meta::SkillTool;
    use wgenty_code::tools::Tool;

    let repo = tempfile::tempdir().expect("tempdir should be created");
    let root = repo.path().join(".wgenty-code/skills");
    fs::create_dir_all(root.join("comet")).unwrap();
    fs::write(
        root.join("comet/SKILL.md"),
        "---\nname: comet\ndescription: C\n---\n# C",
    )
    .unwrap();

    let registry = ExternalSkillRegistry::discover(vec![ExternalSkillRoot::new(
        root.clone(),
        ExternalSkillSource::ProjectWgentyCode { root: root.clone() },
    )])
    .unwrap();
    let tool = SkillTool::with_registry(Arc::new(registry), LoadedSkillContext::default());

    let error = tool
        .execute(serde_json::json!({"skill": "comet", "depth": 9}))
        .await
        .expect_err("depth exceeded should error");

    assert_eq!(error.code.as_deref(), Some("skill_depth_exceeded"));
}

#[tokio::test]
async fn test_skill_tool_set_registry() {
    use std::sync::Arc;
    use wgenty_code::knowledge::{ExternalSkillRegistry, ExternalSkillRoot, ExternalSkillSource};
    use wgenty_code::tools::meta::SkillTool;
    use wgenty_code::tools::Tool;

    let repo = tempfile::tempdir().expect("tempdir should be created");
    let root = repo.path().join(".wgenty-code/skills");
    fs::create_dir_all(root.join("testskill")).unwrap();
    fs::write(
        root.join("testskill/SKILL.md"),
        "---\nname: testskill\ndescription: Test\n---\n# Test Content",
    )
    .unwrap();

    let external_registry = ExternalSkillRegistry::discover(vec![ExternalSkillRoot::new(
        root.clone(),
        ExternalSkillSource::ProjectWgentyCode { root: root.clone() },
    )])
    .unwrap();

    // Start with no registry -- should return not-configured
    let mut tool = SkillTool::new();
    let err = tool
        .execute(serde_json::json!({"skill": "testskill"}))
        .await
        .expect_err("should fail without registry");
    assert_eq!(err.code.as_deref(), Some("skill_registry_unconfigured"));

    // Wire registry via set_registry
    tool.set_registry(Arc::new(external_registry));
    let result = tool
        .execute(serde_json::json!({"skill": "testskill"}))
        .await
        .expect("should work after set_registry");
    assert_eq!(result.output_type, "markdown");
    assert!(result.content.contains("# Test Content"));
}

#[tokio::test]
async fn test_skill_tool_registry_wired_through_tool_registry() {
    use std::sync::Arc;
    use wgenty_code::knowledge::{ExternalSkillRegistry, ExternalSkillRoot, ExternalSkillSource};

    let repo = tempfile::tempdir().expect("tempdir should be created");
    let root = repo.path().join(".wgenty-code/skills");
    let skill_dir = root.join("comet");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: comet\ndescription: Comet workflow\n---\n# Comet\nInstructions.",
    )
    .unwrap();

    let external_registry = ExternalSkillRegistry::discover(vec![ExternalSkillRoot::new(
        root.clone(),
        ExternalSkillSource::ProjectWgentyCode { root: root.clone() },
    )])
    .unwrap();

    let mut registry = ToolRegistry::new();
    registry.wire_skill_registry(Arc::new(external_registry));

    let result = registry
        .execute("skill", serde_json::json!({"skill": "comet"}))
        .await
        .expect("skill should load after wiring through ToolRegistry");
    assert_eq!(result.output_type, "markdown");
    assert!(result.content.contains("# Comet"));
}

#[tokio::test]
async fn test_skill_tool_policy_denies_skill_load() {
    use std::sync::Arc;
    use wgenty_code::knowledge::{
        ExternalSkillRegistry, ExternalSkillRoot, ExternalSkillSource, LoadedSkillContext,
        PolicyDecision, SkillLoadEvent, SkillPolicy,
    };
    use wgenty_code::tools::meta::SkillTool;
    use wgenty_code::tools::Tool;

    struct DenyAllPolicy;
    impl SkillPolicy for DenyAllPolicy {
        fn before_skill_load(&self, _event: &SkillLoadEvent) -> PolicyDecision {
            PolicyDecision::Deny {
                message: "Denied for test purposes".to_string(),
            }
        }
    }

    let repo = tempfile::tempdir().expect("tempdir should be created");
    let root = repo.path().join(".wgenty-code/skills");
    fs::create_dir_all(root.join("testskill")).unwrap();
    fs::write(
        root.join("testskill/SKILL.md"),
        "---\nname: testskill\ndescription: Test\n---\n# Test",
    )
    .unwrap();

    let external_registry = ExternalSkillRegistry::discover(vec![ExternalSkillRoot::new(
        root.clone(),
        ExternalSkillSource::ProjectWgentyCode { root: root.clone() },
    )])
    .unwrap();

    let mut tool =
        SkillTool::with_registry(Arc::new(external_registry), LoadedSkillContext::default());
    tool.set_policy(Arc::new(DenyAllPolicy));

    let error = tool
        .execute(serde_json::json!({"skill": "testskill"}))
        .await
        .expect_err("policy should deny skill load");

    assert_eq!(error.code.as_deref(), Some("skill_policy_denied"));
    assert!(error.message.contains("Denied"));
}

#[tokio::test]
async fn test_skill_tool_policy_allows_skill_load() {
    use std::sync::Arc;
    use wgenty_code::knowledge::{
        ExternalSkillRegistry, ExternalSkillRoot, ExternalSkillSource, LoadedSkillContext,
        PolicyDecision, SkillLoadEvent, SkillPolicy,
    };
    use wgenty_code::tools::meta::SkillTool;
    use wgenty_code::tools::Tool;

    struct AllowAllPolicy;
    impl SkillPolicy for AllowAllPolicy {
        fn before_skill_load(&self, _event: &SkillLoadEvent) -> PolicyDecision {
            PolicyDecision::Allow
        }
    }

    let repo = tempfile::tempdir().expect("tempdir should be created");
    let root = repo.path().join(".wgenty-code/skills");
    fs::create_dir_all(root.join("testskill")).unwrap();
    fs::write(
        root.join("testskill/SKILL.md"),
        "---\nname: testskill\ndescription: Test\n---\n# Test\nContent.",
    )
    .unwrap();

    let external_registry = ExternalSkillRegistry::discover(vec![ExternalSkillRoot::new(
        root.clone(),
        ExternalSkillSource::ProjectWgentyCode { root: root.clone() },
    )])
    .unwrap();

    let mut tool =
        SkillTool::with_registry(Arc::new(external_registry), LoadedSkillContext::default());
    tool.set_policy(Arc::new(AllowAllPolicy));

    let result = tool
        .execute(serde_json::json!({"skill": "testskill"}))
        .await
        .expect("policy should allow skill load");
    assert_eq!(result.output_type, "markdown");
    assert!(result.content.contains("Content"));
}

#[tokio::test]
async fn test_glob_tool() {
    use serde_json::json;
    use tempfile::tempdir;

    let dir = tempdir().expect("tempdir should be created");
    std::fs::create_dir_all(dir.path().join("src")).expect("src dir should be created");
    std::fs::write(dir.path().join("src/main.rs"), "fn main() {}").expect("main.rs should exist");
    std::fs::write(dir.path().join("README.md"), "# readme").expect("README should exist");

    let registry = ToolRegistry::new();
    let result = registry
        .execute(
            "glob",
            json!({
                "path": dir.path(),
                "pattern": "*main.rs",
                "max_results": 10
            }),
        )
        .await
        .expect("glob should succeed");

    assert!(result.content.contains("main.rs"));
    assert!(!result.content.contains("README.md"));
}
