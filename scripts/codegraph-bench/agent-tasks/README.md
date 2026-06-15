# Agent 代码导航任务集

标准化的代码导航任务集合，用于测量 Agent 在代码导航场景中的 codegraph 工具使用率。

## 文件格式

每个任务一个 YAML 文件，结构：

```yaml
task_id: nav-001
category: definition_lookup
prompt: "Tool trait 定义在哪个文件，长什么样？"
expected_answer_anchor:
  file: src/tools/mod.rs
  contains: "trait Tool"
```

## 6 类任务

1. **definition_lookup** — 定义查找（"X 类型/trait 定义在哪"）
2. **reference_lookup** — 引用查找（"谁调用了 X"）
3. **call_chain** — 调用链（"X 从哪调用到 Y"）
4. **impl_enumeration** — 实现枚举（"谁实现了 X trait"）
5. **module_structure** — 模块结构（"X 目录/模块有什么"）
6. **cross_module_path** — 跨模块路径（"事件怎么从 X 流到 Y"）

## 使用方法

bench-agent.sh 逐条读取，通过 wgenty-code repl 回放任务，从 session JSON 中提取工具调用序列。
