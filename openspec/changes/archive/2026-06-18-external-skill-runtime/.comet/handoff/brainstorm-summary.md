# Brainstorm Summary

- Change: external-skill-runtime
- Date: 2026-06-18

## 确认的技术方案

采用方案 A：扩展现有 `SkillLoader` / `LoadSkillTool`，在 `knowledge` 模块中演进出 Claude Code 近似兼容的外部 instruction skill runtime。该 runtime 不硬编码 Comet，而是让 `/comet`、`/comet-open`、`superpowers:brainstorming` 等 markdown skills 作为外部 runtime assets 被发现、列出、按需加载和嵌套调用。

架构上保留 Rust executable skills 与 external instruction skills 的边界：现有 `Skill` trait/`SkillExecutor` 继续服务内置可执行技能；新增 external skill 数据模型、registry、source priority、shadowed diagnostics、loaded-skill context 和 policy hook。Prompt 继续使用两层注入：available skills listing 只包含名称和描述，完整 `SKILL.md` 只在 slash command 或 `skill`/`Skill` runtime action 被调用时加载。

Slash routing 采用 built-in command first、external skill fallback。外部 skill 加载时保留 raw args 为 `ARGUMENTS`，并提供 skill base directory。Nested skill 调用通过新增 Claude Code 兼容的 `skill`/`Skill` runtime action 完成，同时保留 `load_skill` 作为 legacy/internal 路径。Nested skill 最大深度设为 8。

## 关键取舍与风险

- 取舍：选择演进现有 `knowledge::SkillLoader`，避免新建平行 runtime。风险是需要清晰区分 executable skills 与 instruction skills；通过独立 `ExternalSkillDefinition` 模型缓解。
- 取舍：第一版使用 wgenty-code 自有 skill roots，而非默认 `.claude`/`.codex`。优先级为 `repo/.wgenty-code/skills` → `~/.wgenty-code/skills` → enabled plugin cache → configured extra roots。`.claude`/`.codex` 可作为 configured extra roots 兼容。
- 取舍：支持 portable namespace 目录，例如 `.wgenty-code/skills/superpowers/brainstorming/SKILL.md` 映射为 `superpowers:brainstorming`，避免 Windows 路径 `:` 问题。
- 风险：模型驱动 workflow 可能漂移。缓解：保留完整 skill 原文、准确 available listing、loaded-skill tracking，并新增 policy hook 接口为后续 CometPhasePolicy 下沉做准备。
- 风险：重复 skill 来源造成混淆。缓解：确定性 source priority，保留 shadowed entries 并提供诊断输出。
- 风险：prompt 膨胀。缓解：默认只注入 compact listing，full body 按需加载。

## 测试策略

- `knowledge::external` 单元测试：frontmatter name/description parsing、missing name fallback、raw body preservation、base_dir/source_path、portable namespace 映射。
- `knowledge::external_registry` 单元测试：source priority、shadowed diagnostics、invalid/unreadable skill handling。
- `knowledge::policy` 单元测试：DefaultAllowPolicy、Deny decision、hook event payload。
- `tools::meta::skill` 或升级后的 `load_skill` 测试：`skill({ skill: "comet" })`、namespaced skill、missing skill suggestions、depth > 8 error、duplicate load idempotence。
- Slash routing 测试：`/comet abc` 走 external skill fallback，built-in command 优先，unknown command 产生 suggestions。
- Plugin cache fixture 测试：CC-format plugin cache 下的 `skills/*/SKILL.md` 被发现，并带有 PluginCache source metadata。
- 回归验证：现有 built-in skills tests、prompt skills_inventory 行为、format/clippy/test suite。

## Spec Patch

已回写 `openspec/changes/external-skill-runtime/specs/external-skill-runtime/spec.md`：

- discovery scenarios 从 `.claude`/`.codex` 调整为 `.wgenty-code` roots。
- 新增 configured extra root discovery scenario。
- 新增 portable namespace directory scenario。
- 明确 nested runtime action 为 Claude Code 兼容的 `skill`/`Skill`。
- 新增 nested skill depth limit scenario：深度超过 8 时拒绝加载并给出可操作错误。
