# API Reference

Wgenty Code provides multiple integration interfaces.

## CLI

```bash
wgenty-code <subcommand> [options]
```

Full CLI reference: `wgenty-code --help`

### Subcommands

| Command | Description |
|:--------|:------------|
| `repl` | Interactive TUI session |
| `query` | One-shot query |
| `config` | Configuration management |
| `mcp` | MCP server management |
| `plugin` | Plugin management |
| `memory` | Memory and session management |
| `skills` | Skills management |
| `sandbox` | Sandbox control |
| `agent` | Run agent subcommand |
| `init` | Initialize project |
| `daemon` | Start HTTP daemon |

## Daemon HTTP API

Start the daemon:

```bash
wgenty-code daemon --port 8371
```

All endpoints except `GET /api/v1/health` require `Authorization: Bearer <token>`.
The token is generated at startup and written to `~/.wgenty-code/daemon.token`
(mode `0600`).

### Session event stream (resume / replay)

`GET /api/v1/sessions/:id/events` — SSE stream of one session's run events.
Each event is a JSON envelope `{seq, session_id, run_id, kind, data}`; `seq`
is monotonic within a session (across runs), starting at 1.

- **Without `after`** (default): live-only — events published before the
  subscription are not delivered.
- **With `after=<seq>`**: the server replays buffered events with
  `seq > after` in order, then attaches live; live events already replayed
  are dropped at the seam (dedup by `seq`). `after >= latest` attaches live
  directly.
- **`sync_lost`**: if `after` fell out of the replay buffer window (evicted)
  or the buffer is empty (e.g. after a daemon restart — the buffer is
  in-memory only), the connection receives a control event
  `{seq: 0, kind: "sync_lost", data: {reason: "evicted", latest_seq}}`.
  If a live subscriber falls behind the broadcast window, that connection
  alone receives `{reason: "lagged", ...}`; other subscribers are unaffected.
  The stream stays open in both cases.
- **Recovery convention**: on `sync_lost` the client must do a full
  `GET /api/v1/sessions/:id`, realign on the latest persisted state, then
  re-subscribe (without `after`, or with the newest known seq).

Buffer capacity: `daemon.event_buffer_capacity` (default 1024). Keep-alive
comment every 15s.

### Global event stream

`GET /api/v1/events` — daemon-wide (cross-project) SSE stream, live-only.
Envelope `{seq, kind, data}`; `seq` is monotonic across the daemon process
and is **not** resumable after a restart. Arrival order is not guaranteed to
equal `seq` order with concurrent publishers — clients must sort/dedup by
`seq`. Kinds: `todos_changed`, `background_result`, `mode_changed`,
`model_changed`, `task_group_result`.

There is no replay: on (re)connect or after lag, realign via the plain GET
endpoints (`GET /api/v1/todos`, `GET /api/v1/background/results`, ...).
Background results are retained (capacity 256) and `GET
/api/v1/background/results` returns a snapshot without draining, so every
client can query results produced while it was offline.

### Session versioning

`PUT /api/v1/sessions/:id` accepts an optional `expected_version` (optimistic
concurrency guard):

- omitted → legacy last-write-wins;
- matching → write succeeds and `version` advances by 1;
- stale → `409` with `{"error": "version conflict", "current_version": N}`;
  the client re-reads and retries with the fresh version.

The check-and-write is serialized server-side: two concurrent PUTs carrying
the same `expected_version` always produce exactly one success and one 409.

Server-side run saves also advance `version`, so a client holding a pre-run
version always conflicts instead of silently overwriting run output.
New sessions start at `version: 0`; legacy sessions without the field
deserialize as 0.

### 409 semantics

| Endpoint | Condition | 409 body |
|:---------|:----------|:---------|
| `POST /api/v1/interactions/:id/resolve` | interaction already resolved | `{"resolved": false, "answer": <first answer>}` |
| `POST /api/v1/tools/resolve-permission` | permission already resolved | `{"success": false, "resolved": true, "approved": <first decision>}` |
| `PUT /api/v1/sessions/:id` | stale `expected_version` | `{"error": "version conflict", "current_version": N}` |
| `PUT /api/v1/sessions/:id` | a server-side run is active | `{"error": "run active"}` |
| `POST /api/v1/sessions/:id/run` | a run is already active | plain text message |

Duplicate resolutions never overwrite the first decision and trigger no
second side effect; unknown interaction/permission ids return 404.

Permission modes and approved session rules are scoped per session
(project root): entries set for one session are not visible via
`GET /api/v1/permission-mode?session_id=` of another session. Server-side
callers must always pass `session_id` (omitting it is a 400).

### Daemon discovery file

`~/.wgenty-code/daemon.json` lets UI processes reuse an already-running
daemon instead of spawning a duplicate:

```json
{
  "port": 8371,
  "token": "<api token>",
  "pid": 12345,
  "started_at": "2026-08-07T00:00:00Z",
  "heartbeat_at": "2026-08-07T00:00:30Z"
}
```

- Written atomically (temp file + rename, mode `0600`) at daemon startup;
  `heartbeat_at` is refreshed every **30s**; the file is removed on clean
  shutdown. The token also stays in `daemon.token` for existing readers.
- A UI process connects to the discovered daemon only when **all** hold:
  the file exists and parses, `token` matches `~/.wgenty-code/daemon.token`,
  and `heartbeat_at` is at most **120s** old. Any failure means the file is
  stale (or belongs to another daemon instance) and the UI falls back to
  spawning its own daemon — a stale file never causes a misconnection.
  `pid` liveness is advisory only; the heartbeat is authoritative.

## Configuration File

Path: `~/.wgenty-code/settings.json` (JSON, auto-generated)

Key sections: `models`, `agent`, `prompt`, `plugins`, `storage`, `integrations`.

See [WGENTY.md](../WGENTY.md#配置) for the full configuration reference.

## Environment Variables

| Variable | Priority | Description |
|:---------|:---------|:------------|
| `ANTHROPIC_API_KEY` | Highest | Anthropic API key |
| `DASHSCOPE_API_KEY` | — | DashScope API key |
| `DEEPSEEK_API_KEY` | — | DeepSeek API key |
| `API_BASE_URL` | — | Override API endpoint |
| `RUST_LOG` | — | Log level (e.g., `wgenty_code=debug`) |
