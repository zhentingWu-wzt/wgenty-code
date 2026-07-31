# wgenty-code web (MVP)

A browser-based thin client for the wgenty-code daemon. Architecturally a
**parallel sibling of `src/tui`** — both are thin frontends over the same HTTP
daemon API. No Rust/daemon changes are required.

## What works

- Streaming chat with real-time token rendering
- Client-side agent loop: the model can call tools across multiple rounds, and
  the browser executes them via `/api/v1/tools/execute` and feeds results back
- **Stop button** to abort a running turn mid-stream (AbortController)
- **Root-tool permission approval** (the in-band `permission_required` flow) via
  a modal — Allow once / Always allow / Deny
- **Markdown rendering** of assistant output (GFM + syntax-highlighted code)
- **Diff preview** for `file_edit` / `apply_patch` tool results
- Collapsible **reasoning** blocks
- Collapsible **sidebar** with panels:
  - **Sessions** — list / search / open / save / delete (`/sessions*`)
  - **Todos** — live todo list (`/todos`)
  - **Tasks** — ready/blocked progress + task graph (`/tasks*`)
  - **Model** — profile picker (`/models`, `/model/switch`)
  - **Memory** — status + filtered list + prune (`/memory*`, Tier 2 backend)
  - **Config** — read-only overview + config (assembled client-side)

## Out of scope (phase 2)

- Subagent async permission queue (`pending-permissions` polling)
- Context compaction, stuck-detector, parallel `task` batches
- Mobile-responsive layout, production daemon-hosted build
- Guardian client-side short-circuit (we rely on the daemon's signal instead)

## How the architecture maps to the Rust codebase

| Web file | Mirrors |
| --- | --- |
| `src/api/sseParser.ts` | `src/agent/core.rs` (`StreamProcessor`) |
| `src/api/client.ts` | `src/tui/client.rs` (`DaemonClient`) |
| `src/api/types.ts` | `src/daemon/models.rs` + `src/api/types.rs` |
| `src/agent/loop.ts` | `src/agent/runtime/loop_.rs` (`run_agent_loop_inner`, slimmed) |

The key fact: the daemon's `/api/v1/chat/stream` is a **pure passthrough
proxy** — it forwards the upstream LLM's SSE but does not execute tools. The
client must drive the stream → tool → re-stream loop itself, exactly like the
TUI does.

## Run it

### 1. Start the daemon

```bash
cargo run --features daemon -- daemon --port 8371
```

The daemon writes a per-start bearer token to `~/.wgenty-code/daemon.token`.

### 2. Start the web dev server

```bash
cd web
npm install
npm run dev
```

Open http://localhost:5173. The Vite dev server proxies `/api/*` to the daemon
and **injects the bearer token server-side** — the browser never sees the
secret. Port `5173` is already in the daemon's CORS allow-list.

If you used a custom `--port`, set it before starting vite:

```bash
DAEMON_PORT=9000 npm run dev
```

## Scripts

| Command | What it does |
| --- | --- |
| `npm run dev` | Vite dev server with the daemon proxy |
| `npm run build` | `tsc` type-check + production build |
| `npm run typecheck` | type-check only |
| `npm test` | run the SSE parser unit tests (vitest) |

## Verify it works

1. **Read-only turn:** send *"summarize the README at ../README.md"*. You should
   see streamed tokens and (if the model uses a read tool) a green tool card —
   no approval needed.
2. **Permission turn:** send *"create /tmp/wgenty-test/hello.txt with hi
   inside"*. You should see a `file_write` tool card, a permission modal, and
   after approving, the tool executes and the agent confirms.
