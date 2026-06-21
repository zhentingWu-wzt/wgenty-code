# Migration Guide

Guide for migrating from the TypeScript version of Wgenty Code (Claude Code) to the Rust rewrite.

## Overview

The Rust rewrite is a drop-in replacement for most workflows, with significantly better performance and zero runtime dependencies.

## Key Differences

### Installation

| TypeScript | Rust |
|:-----------|:-----|
| `npm install -g @anthropic-ai/claude-code` | `cargo build --release` or download binary |
| Requires Node.js 18+ runtime | Single self-contained binary |

### CLI Commands

Most commands are identical. The binary name is `wgenty-code`.

| Operation | TypeScript | Rust |
|:----------|:-----------|:-----|
| REPL mode | `claude` | `wgenty-code repl` |
| One-shot query | `claude -p "..."` | `wgenty-code query -p "..."` |
| Config management | `claude config ...` | `wgenty-code config ...` |

### Configuration

- **TypeScript**: `~/.claude.json` or `~/.config/claude/`
- **Rust**: `~/.wgenty-code/settings.json` (JSON format, auto-generated on first run)

### Configuration Keys

TypeScript config keys have been restructured into a nested hierarchy:

| TypeScript | Rust |
|:-----------|:-----|
| `model` | `models.main.name` |
| `maxSubagentDepth` | `agent.subagent.max_depth` |
| `maxConcurrentSubagents` | `agent.subagent.max_concurrent` |
| `planMode` | `agent.plan_mode` |
| `tokenBudgetK` | `agent.token_budget.main_k` |
| `guardian.enabled` | `integrations.guardian.enabled` |

Set values with: `wgenty-code config set <dotted.key> <value>`

### Environment Variables

| Variable | Purpose |
|:---------|:--------|
| `ANTHROPIC_API_KEY` | Anthropic API key |
| `DASHSCOPE_API_KEY` | DashScope API key |
| `DEEPSEEK_API_KEY` | DeepSeek API key |
| `API_BASE_URL` | Custom API endpoint |

## Unsupported Features

> This section will be updated as features are ported. See the [GitHub Issues](https://github.com/zhentingWu-wzt/wgenty-code/issues) for the latest status.
