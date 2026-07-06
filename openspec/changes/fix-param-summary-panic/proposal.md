## Why

运行时 panic：
```
thread 'tokio-rt-worker' (36834069) panicked at src/teams/subagent_loop.rs:81:38:
end byte index 80 is not a char boundary; it is inside '构' (bytes 79..82 of string)
```
当 subagent 调用的工具参数（如 task `description`）含多字节 UTF-8 字符且超过 80 字节时，`extract_params_summary` 截断参数摘要时按**字节下标**切片，命中字符中间字节，整个 tokio worker 线程 panic，subagent 循环崩溃。

## 根因分析

`src/teams/subagent_loop.rs`：
- 第 27 行 `const MAX_PARAMS_SUMMARY_LEN: usize = 80;`
- 第 80 行 `if s.len() > MAX_PARAMS_SUMMARY_LEN` — `s.len()` 是**字节**长度。
- 第 81 行 `format!("{}…", &s[..MAX_PARAMS_SUMMARY_LEN])` — `&s[..80]` 按**字节下标**切片。

当字符串第 80 字节落在某个多字节 UTF-8 字符内部（如 '构' 占字节 79..82，第 80 字节在 '构' 中间），`&s[..80]` 触发 panic：`end byte index 80 is not a char boundary`。

该 bug 自 commit `d5044a46`（2026-06-13）起存在，为**预存问题**，与当前活跃的 `fix-subagent-timeout-default` 变更无关（后者修改 `subagent_loop.rs` 的 `SubagentError` 部分，line 92+，未触及 `extract_params_summary`）。

## What Changes

将 `extract_params_summary` 第 81 行的字节下标切片改为字符边界安全截断：取 `≤ MAX_PARAMS_SUMMARY_LEN` 的最大字符边界处切片（`str::floor_char_boundary`，Rust 1.80+ 稳定）。

## 修复目标

- 含多字节字符的工具参数超过 80 字节时，`extract_params_summary` 不再 panic，而是截断到最近的字符边界并加省略号。
- 单元测试覆盖多字节截断场景（第 80 字节落在字符内部）。

## Impact

- **Code**: `src/teams/subagent_loop.rs`（`extract_params_summary` 截断逻辑 1 处 + 单元测试）。
- **Docs**: 无。
- **User-visible behavior**: subagent 不再因多字节参数摘要截断而崩溃。
- **Non-goals**: 不调整 `MAX_PARAMS_SUMMARY_LEN` 数值；不重构 `extract_params_summary` 其他逻辑。systematic-debugging 阶段会 grep 同类字节切片模式；若同一函数/同一根因的近邻位点存在相同 bug 则一并修，远端位列为 follow-up。
