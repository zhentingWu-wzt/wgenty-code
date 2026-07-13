# 上下文压缩恢复协议

规范路径：`comet/reference/context-recovery.md`

本协议由所有可能触发上下文压缩的 comet 子 skill 共享。当 agent 怀疑发生上下文压缩（之前对话被摘要、找不到之前讨论的内容）时，按本协议恢复。

## 恢复步骤

```bash
"$COMET_BASH" "$COMET_STATE" check <change-name> <phase> --recover
```

脚本输出结构化恢复上下文（phase、已完成字段、待完成字段、恢复动作）。按 **Recovery action** 决定下一步。

## build 阶段特殊恢复

若恢复脚本输出 `build_mode: subagent-driven-development`：

1. 使用 Skill 工具重新加载 Superpowers `subagent-driven-development` 技能
2. 重新阅读 `comet/reference/subagent-dispatch.md` 获取 Comet 专属扩展
3. 读取 `openspec/changes/<name>/.comet/subagent-progress.md`，恢复当前 task 或 final review、实现提交、RED/GREEN 证据、已通过审查、未解决反馈和审查-修复轮次
4. 禁止在主会话中直接执行 task
5. 按检查点记录的精确阶段恢复；检查点缺失或不匹配时才从第一个未勾选 task 的 implementer 派发开始
6. task 通过双审查并完成定向勾选验证后，立即继续下一个 task，不得总结或询问是否继续

## design 阶段特殊恢复

- 若用户尚未确认设计方案，回到 brainstorming 继续
- 若用户已确认，继续创建 Design Doc
- 恢复时重新加载 `brainstorm-summary.md` + handoff 上下文文件

## verify/archive 阶段恢复

- verify：脚本输出验证状态、分支状态和恢复动作
- archive：若 `archived: true` 且归档目录存在，归档已完成，无需再次执行
