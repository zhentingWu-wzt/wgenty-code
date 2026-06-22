use wgenty_code::knowledge::comet_slash_agent_prompt;

#[test]
fn comet_root_slash_prompt_forces_openspec_discovery() {
    let prompt = comet_slash_agent_prompt("comet", "实现 tab 分类").expect("/comet should wrap");

    assert!(prompt.contains("/comet 实现 tab 分类"));
    assert!(prompt.contains("load_skill"));
    assert!(prompt.contains("comet"));
    assert!(prompt.contains("openspec list --json"));
    assert!(prompt.contains("MUST NOT continue"));
}

#[test]
fn comet_phase_slash_prompt_loads_specific_skill_and_checks_openspec() {
    let prompt =
        comet_slash_agent_prompt("comet-build", "继续执行").expect("/comet-build should wrap");

    assert!(prompt.contains("/comet-build 继续执行"));
    assert!(prompt.contains("comet-build"));
    assert!(prompt.contains("openspec list --json"));
    assert!(prompt.contains("openspec/changes"));
}

#[test]
fn non_comet_slash_prompt_is_not_wrapped() {
    assert!(comet_slash_agent_prompt("review", "").is_none());
    assert!(comet_slash_agent_prompt("openspec-new-change", "").is_none());
}
