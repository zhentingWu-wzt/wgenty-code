# wgenty-code web (MVP)

A browser-based thin client for the wgenty-code daemon. Architecturally a
**parallel sibling of `src/tui`** — both are thin frontends over the same HTTP
daemon API. No Rust/daemon changes are required.

## What works

- **Three-segment workbench layout**:
  - **AppTopbar** — top bar: theme toggle and left/right sidebar switches
  - **LeftSidebar** — session tree (project → worktree → session), width
    draggable
  - **SessionTabBar** — each open session is a tab (reorderable)
  - **RightRail** — activity bar + five panels: **Sessions** / **Skills** /
    **Memory** / **Checkpoints** / **Tasks**
  - **StatusBar** — bottom bar: connection · run state · pending approvals ·
    model
- **Theme**: light / dark / system, switched from the topbar and persisted in
  localStorage
- **Slash commands**: `/model` opens a modal; `/sessions`, `/memory`, `/undo`
  open the corresponding right-rail panel
- Streaming chat with real-time token rendering
- **Server-side runs**: the daemon owns the agent loop — the client POSTs
  `/sessions/:id/run` and mirrors the SSE session-event stream into the UI
  (`src/agent/sessionRunner.ts` is the send entry point)
- **Stop button** to abort a running turn (`POST /sessions/:id/cancel`)
- **Root-tool permission approval** (the in-band `permission_required` flow) via
  a modal — Allow once / Always allow / Deny
- **Markdown rendering** of assistant output (GFM + syntax-highlighted code)
- **Diff preview** for `file_edit` / `apply_patch` tool results
- Collapsible **reasoning** blocks

## Out of scope (phase 2)

- Subagent async permission queue (`pending-permissions` polling)
- Context compaction, stuck-detector, parallel `task` batches
- Mobile-responsive layout, production daemon-hosted build
- Guardian client-side short-circuit (we rely on the daemon's signal instead)

## How the architecture maps to the Rust codebase

| Web file               | Mirrors                                                        |
| ---------------------- | -------------------------------------------------------------- |
| `src/api/sseParser.ts` | `src/agent/core.rs` (`StreamProcessor`)                        |
| `src/api/client.ts`    | `src/tui/client.rs` (`DaemonClient`)                           |
| `src/api/types.ts`     | `src/daemon/models.rs` + `src/api/types.rs`                    |
| `src/agent/loop.ts`    | `src/agent/runtime/loop_.rs` (`run_agent_loop_inner`, slimmed) |

The key fact: turns now run **server-side** — `POST /sessions/:id/run` spawns
the agent loop on the daemon (LLM calls + tool execution + persistence), and
the client observes it via `GET /sessions/:id/events` (SSE). Closing the
browser no longer kills a turn.

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

| Command             | What it does                           |
| ------------------- | -------------------------------------- |
| `npm run dev`       | Vite dev server with the daemon proxy  |
| `npm run build`     | `tsc` type-check + production build    |
| `npm run typecheck` | type-check only                        |
| `npm test`          | run the full vitest suite               |

## Verify it works

1. **Read-only turn:** send _"summarize the README at ../README.md"_. You should
   see streamed tokens and (if the model uses a read tool) a green tool card —
   no approval needed.
2. **Permission turn:** send _"create /tmp/wgenty-test/hello.txt with hi
   inside"_. You should see a `file_write` tool card, a permission modal, and
   after approving, the tool executes and the agent confirms.
