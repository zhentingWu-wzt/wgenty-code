# Design D8: Daemon-Hosted Web UI (Single-Binary Distribution)

- **Status**: Proposed
- **Date**: 2026-08-17
- **Scope**: `src/daemon/{mod.rs,routes.rs}`, `src/cli/mod.rs`, `Cargo.toml`, `web/` build wiring
- **Depends on**: D4 (ws push channel), daemon discovery (`~/.wgenty-code/daemon.json`)

## Problem

The web frontend today only runs in dev mode: `npm run dev` proxies `/api/v1`
and injects auth via `/__daemon-info`. There is no way to serve the built UI
from a release binary — `wgenty-code daemon` exposes JSON API + WS only, and
`web/dist` has no consumer outside the Tauri bundle.

Users who want the web UI without a JS toolchain currently have no path; the
binary cannot act as the whole product.

## Goals

1. `wgenty-code daemon` (one binary, zero external files) serves the full web
   UI at `http://127.0.0.1:8371/`.
2. Frontend changes require no Rust recompile in dev (external-dir mode).
3. Zero frontend changes: same-origin API calls, same `/__daemon-info`
   discovery the dev proxy provides.
4. Feature-gated embed so default builds stay lean; opt-in single-file
   distribution costs ~0.6 MB (brotli-compressed).

## Non-Goals

- Changing the dev workflow (`npm run dev` + vite proxy stays canonical).
- Desktop (Tauri) packaging — it keeps its own shell + sidecar layout.
- Multi-user / remote-host deployments; daemon binds 127.0.0.1 only.

## Architecture

Two serving modes, one router:

```
wgenty-code daemon --serve-web            → embed feature: assets in binary
wgenty-code daemon --serve-web <dir>      → external dir (default debug dev)
```

Both mount AFTER `/api/v1` and the WS route — API paths always win. The
fallback layer serves `web/dist` with SPA semantics.

### Route layers (existing router, extended)

```
GET /api/v1/*         JSON API (unchanged)
GET /api/v1/ws        WS push (unchanged)
GET /__daemon-info    NEW: { port, token } — same-origin discovery
GET /*                NEW: ServeDir(web-dist) → fallback index.html (SPA)
```

### `/__daemon-info` contract

Same shape the vite plugin serves today (`web/vite.config.ts`):
`{ "port": 8371, "token": "<hex>" }`. The frontend's
`resolveDaemonDirect()` already fetches this path and falls back to
same-origin relative calls — no frontend change needed.

Security: daemon binds 127.0.0.1 only. The endpoint is same-origin-readable
by any local page, but cross-origin pages cannot read the response (no CORS
headers on this route). This matches the dev-proxy threat model exactly.

### Embed strategy (rust-embed)

```toml
[features]
default = []
web-ui = ["dep:rust-embed"]

[dependencies]
rust-embed = { version = "8", optional = true, features = ["compression"] }
```

```rust
#[cfg(feature = "web-ui")]
#[derive(rust_embed::RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/../web/dist"]
struct WebDist;
```

rust-embed default behavior gives us goal #2 for free: **debug builds read
from disk at request time** (no recompile), **release builds embed +
brotli-compress** (~1.7 MB → ~0.6 MB).

External-dir mode (`--serve-web <dir>`) works without the feature using
tower-http `ServeDir`, and doubles as the release fallback if `web/dist` was
not built before `cargo build --release`.

### CLI surface

```
wgenty-code daemon [--serve-web] [--serve-web-dir <dir>]
```

- `--serve-web` (flag, embed feature): serve embedded assets; errors at
  startup if compiled without the feature and no dir given.
- `--serve-web-dir <dir>`: serve from directory (works with or without the
  feature; ignored when the flag is absent).
- Default (no flag): today's behavior — API only.

### Size budget (measured 2026-08-17)

| Artifact | Size |
|---|---|
| web/dist (raw) | 1.7 MB |
| web/dist (brotli, embedded) | ~0.6 MB |
| release binary (no feature) | ~30 MB est. |
| release binary (+ web-ui) | ~30.6 MB est. (+~2%) |

## Implementation Plan

1. **Cargo.toml**: add optional `rust-embed` + `web-ui` feature.
2. **src/daemon/routes.rs**: `/__daemon-info` handler (reads discovery state);
   static layer behind `Option<StaticLayer>` — `ServeDir` (external dir) or
   `RustEmbed` (feature) with SPA fallback to `index.html`.
3. **src/daemon/mod.rs**: thread `serve_web: Option<ServeWeb>` through
   `run()`.
4. **src/cli/mod.rs**: `--serve-web` / `--serve-web-dir` args; validation
   (flag requires feature OR dir).
5. **Build wiring**: `bundle.sh` (desktop) and CI gain
   `npm --prefix web run build` before `cargo build --release --features
   web-ui` when producing distributables.
6. **Docs**: README / docs/API.md note the URL `http://127.0.0.1:8371/`.

## Testing

- Unit: `/__daemon-info` shape; static layer 404 vs SPA fallback (routes.rs
  tests, in-process `axum::Router` ones like existing handler tests).
- Integration (`tests/integration/`): daemon with `--serve-web <test-dist>`
  serves `index.html` at `/` and a hashed asset; `/api/v1/health` unaffected;
  unknown path returns `index.html` (SPA).
- Manual: `cargo build --release --features web-ui && ./target/release/
  wgenty-code daemon --serve-web` → browser full flow (sessions, chat, WS
  live updates, approval modal).

## Alternatives Considered

- **vite preview as production server**: keeps a Node runtime dependency;
  fails goal #1 (single binary).
- **Tauri-only distribution**: desktop already works, but headless/server
  users and plain-browser users get nothing; desktop shell (menus, tray)
  remains its own reason to exist.
- **Sidecar static server (e.g. miniserve)**: two processes, port juggling,
  no singleton story — rejected.

## Open Questions

- Should `--serve-web` also imply `--spawned-by`-style keepalive semantics
  when the browser tab is the only client? (Current answer: no — browser
  tabs are regular WS clients; idle/grace shutdown policies unchanged.)
