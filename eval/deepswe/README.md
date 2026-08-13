# DeepSWE Evaluation for wgenty-code

Evaluation harness for running [wgenty-code](https://github.com/zhentingWu-wzt/wgenty-code)
against the [DeepSWE benchmark](https://github.com/datacurve-ai/deep-swe) — 113 original,
long-horizon software engineering tasks.

## Quick Start

### 1. Build the Linux binary

wgenty-code runs inside DeepSWE's Docker containers (x86_64 Linux). Build a
statically-linked binary with [cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild):

```bash
# Install Zig + cargo-zigbuild (one-time)
brew install zig
cargo install cargo-zigbuild
rustup target add x86_64-unknown-linux-musl

# Build
cargo zigbuild --release --target x86_64-unknown-linux-musl --features bundled-sqlite
cp target/x86_64-unknown-linux-musl/release/wgenty-code wgenty-code-linux-amd64
```

### 2. Install Pier + clone DeepSWE

```bash
uv tool install datacurve-pier
git clone https://github.com/datacurve-ai/deep-swe ~/project/deep-swe
```

### 3. Run

```bash
export DEEPSEEK_API_KEY=sk-xxx
./eval/deepswe/run_eval.sh ~/project/deep-swe 1      # single task (smoke test)
./eval/deepswe/run_eval.sh ~/project/deep-swe 10     # 10-task subset
./eval/deepswe/run_eval.sh ~/project/deep-swe 0      # all 113 tasks
```

## How It Works

```
Pier (harbor fork) ── builds task container (mars-base + repo@base_commit)
  └─ wgenty-code query --prompt-file task.md --yolo --max-rounds 500
       └─ Agent explores code, implements changes, runs tests, commits
  └─ git diff base HEAD → model.patch
  └─ Verifier applies model.patch, runs hidden tests (base + new)
       └─ reward = 1 if ALL new tests pass AND no regressions, else 0
```

## Configuration

Edit `job.yaml` to change:

| Setting | Default | Description |
|---------|---------|-------------|
| `model_name` | `deepseek-v4-flash` | LLM model (must be available via your API endpoint) |
| `max_rounds` | `500` | Max agent loop iterations |
| `max_tokens` | `16384` (in agent driver) | Max tokens per LLM response |
| `n_concurrent_trials` | `1` | Parallel tasks (increase for batch runs) |

## Files

| File | Purpose |
|------|---------|
| `wgenty_code_agent.py` | Pier agent driver (`BaseInstalledAgent`) — mounts binary, writes settings, runs agent, generates model.patch |
| `job.yaml` | Pier job config (template with `__WGENTY_BINARY__` / `__DEEPSWE_TASKS__` placeholders) |
| `run_eval.sh` | One-command runner — resolves paths, injects API keys, invokes Pier |

## wgenty-code CLI Flags (for SWE eval)

These flags were added to `query` subcommand for evaluation:

| Flag | Purpose |
|------|---------|
| `--prompt-file <path>` | Read task instruction from file (avoids shell ARG_MAX) |
| `--yolo` | Autonomous mode: sandbox disabled, no approval gating, guardian bypass |
| `--max-rounds <N>` | Override agent loop max rounds |

## First Successful Run (2025-08-09)

**Task**: `meriyah-explicit-resource-declarations` — add `using` / `await using` declaration support to the Meriyah JavaScript parser.

**Model**: deepseek-v4-flash, max_rounds=500, max_tokens=16384

**Results**:

| Metric | Value |
|--------|-------|
| Code changes | 6 files, +293 lines (54KB patch) |
| f2p (new feature tests) | **46/49 (93.9%)** |
| p2p (regression tests) | **51469/51469 (100%)** |
| Total tests | 51515/51518 passed (99.994%) |
| Reward | 0 (binary; needs 49/49 f2p) |
| Partial | 0.9999 |
| Runtime | ~40 min (QEMU x86_64 emulation) |

**3 failing tests** (all in `test/parser/declarations/using.ts`):
1. Array destructuring rejection (edge case)
2. Object destructuring rejection (edge case)
3. For-of parsing edge case

**Agent workflow**: researched ECMAScript spec → ran V8 tests → implemented 266 lines in parser.ts → updated snapshots → ran vitest → fixed ESLint → iterated.

## Key Lessons

1. **Binary architecture**: DeepSWE task containers are x86_64. On Apple Silicon, use `cargo-zigbuild --target x86_64-unknown-linux-musl` for a static binary (no dynamic linker needed).
2. **max_tokens**: Default 4096 truncates LLM responses mid-tool-call, causing the agent loop to end prematurely. Set to 16384+.
3. **max_rounds**: Complex SWE tasks need 200-500 rounds. 100 rounds was insufficient — the agent spent all rounds on research without implementing.
4. **Pier `VerifierConfig`**: The `[[verifier.collect]]` field in task.toml is silently dropped by pydantic. The agent driver must generate `model.patch` itself via `git diff`.
