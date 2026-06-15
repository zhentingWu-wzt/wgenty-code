# Results Directory

运行产物目录。每次 `run-all.sh` 在 `<timestamp>/` 子目录下生成：
- env.json — 环境指纹
- perf.json — 性能基线
- coverage.json — 覆盖率基线
- agent.json — Agent 使用率基线
- transcript-analysis.json — 历史 session 分析

此目录内容被 .gitignore，仅保留此 README。
