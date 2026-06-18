# Debug Gate 协议

规范路径：`comet/reference/debug-gate.md`

本协议由 build、hotfix、tweak 等会直接修改代码的 comet 子 skill 共享。当运行程序、测试、构建或手动验证时出现崩溃、异常行为、测试失败或构建失败，必须进入 Debug Gate。

## 核心规则

- 立即使用 Skill 工具加载 Superpowers `systematic-debugging` 技能
- 在完成根因调查前，不得提出或实施源码修复

## 四阶段流程

1. 先复现并定位根因，读取完整错误、检查近期变更、追踪数据流
2. 若根因指向源码 bug，先补充能复现该崩溃/异常的最小失败测试，再修改源码
3. 修复后运行该失败测试、相关测试和项目构建/验证命令，确认全部通过
4. 将测试、源码修复和 tasks.md 勾选保留在当前 change 内；不得通过另起一个“写测试用例”的 change 来替代当前 change 的验证闭环