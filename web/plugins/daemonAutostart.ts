import { spawn } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { resolve } from "node:path";
import type { Plugin, ViteDevServer } from "vite";

// ── Daemon autostart (global singleton) ──────────────────────────────────────
// `npm run dev` normally 401s/502s until the daemon is up. This plugin brings
// the daemon up automatically, reusing any already-running instance:
//
//   1. Probe ~/.wgenty-code/daemon.json's port AND the default port via
//      GET /api/v1/health — a healthy response means a daemon (TUI, Desktop,
//      another dev server, manual `wgenty-code daemon`) is already serving;
//      reuse it and never spawn. This is the singleton guard.
//   2. Otherwise spawn the workspace binary detached
//      (`wgenty-code daemon --port <default>`). The daemon itself fails fast
//      when the port is already bound, so even a race between two concurrent
//      dev servers resolves to exactly one daemon; the loser just exits.
//   3. Poll /api/v1/health until ready (or warn after a timeout — never crash
//      the dev server).
//   4. Hold an owner connection for the dev server's lifetime: a long-lived
//      GET /api/v1/client/heartbeat SSE (the same endpoint the TUI uses).
//      The daemon is spawned with `--spawned-by web`, so it exits once every
//      client (this owner stream, browser tabs, TUIs, …) has disconnected
//      for 30s — vite down ⇒ daemon follows. If the daemon dies mid-session,
//      the owner stream breaks and reconnects through ensureDaemon, reviving
//      it in seconds instead of a health-poll interval.
//
// Set WGENTY_AUTOSTART=0 to skip, WGENTY_BIN=<path> to override the binary.

const HEALTH_TIMEOUT_MS = 500;
const STARTUP_TIMEOUT_MS = 15_000;
const POLL_INTERVAL_MS = 250;
/** Owner-stream reconnect backoff cap: 0.5s doubling up to 5s. */
const OWNER_BACKOFF_MAX_MS = 5_000;

function defaultDaemonPort(): number {
  return Number(process.env.DAEMON_PORT ?? 8371);
}

function readDiscoveryPort(): number | null {
  // Same discovery file the proxy's `router` reads — mirrors TUI
  // discover_daemon() without touching Rust code from here.
  const discoveryPath = resolve(homedir(), ".wgenty-code", "daemon.json");
  try {
    const parsed = JSON.parse(readFileSync(discoveryPath, "utf8"));
    if (typeof parsed.port === "number" && parsed.port > 0) return parsed.port;
  } catch {
    // Missing/corrupt — fall through to the default port only.
  }
  return null;
}

async function probeHealth(port: number): Promise<boolean> {
  // /api/v1/health is unauthenticated (mirrors probe_daemon_health in Rust).
  try {
    const res = await fetch(`http://127.0.0.1:${port}/api/v1/health`, {
      signal: AbortSignal.timeout(HEALTH_TIMEOUT_MS),
    });
    return res.ok;
  } catch {
    return false;
  }
}

function findDaemonBinary(): string | null {
  const exe = process.platform === "win32" ? "wgenty-code.exe" : "wgenty-code";
  const roots = [process.cwd(), resolve(process.cwd(), ".."), resolve(process.cwd(), "..", "..")];
  const candidates = [
    process.env.WGENTY_BIN,
    ...roots.flatMap((root) => [
      resolve(root, "target", "debug", exe),
      resolve(root, "target", "release", exe),
    ]),
  ].filter((path): path is string => typeof path === "string" && path.length > 0);
  return candidates.find((path) => existsSync(path)) ?? null;
}

function candidatePorts(): number[] {
  // Discovery file's port first (what the proxy `router` reads), then the
  // default port — mirrors TUI discover_daemon() without Rust round-trips.
  const port = defaultDaemonPort();
  const discovery = readDiscoveryPort();
  return [...new Set([discovery, port].filter((p): p is number => p !== null))];
}

async function ensureDaemon(server: ViteDevServer): Promise<void> {
  const logger = server.config.logger;
  const port = defaultDaemonPort();

  // Reuse-first: a healthy daemon on the discovery or default port wins and no
  // process is spawned — the global-singleton contract.
  for (const candidate of candidatePorts()) {
    if (await probeHealth(candidate)) {
      logger.info(`daemon already running on 127.0.0.1:${candidate} — reusing`);
      return;
    }
  }

  const binary = findDaemonBinary();
  if (!binary) {
    logger.warn(
      "daemon not running and no wgenty-code binary found — build it with `cargo build` or set WGENTY_BIN",
    );
    return;
  }

  logger.info(`daemon not running — starting detached: ${binary} daemon --port ${port}`);
  const child = spawn(binary, ["daemon", "--port", String(port), "--spawned-by", "web"], {
    detached: true,
    stdio: "ignore", // Fully detached: survives `npm run dev` shutdown.
  });
  child.unref();

  let settled = false;
  // Non-zero exit usually means another instance won the port-bind race — the
  // singleton survived, just not ours. Only surface it if we never went healthy.
  child.on("exit", (code) => {
    if (!settled && code !== 0 && code !== null) {
      logger.warn(`spawned daemon exited with code ${code} (port busy or startup error)`);
    }
  });

  const deadline = Date.now() + STARTUP_TIMEOUT_MS;
  while (Date.now() < deadline) {
    await new Promise((sleep) => setTimeout(sleep, POLL_INTERVAL_MS));
    if (await probeHealth(port)) {
      settled = true;
      logger.info(`daemon ready on 127.0.0.1:${port}`);
      return;
    }
  }
  logger.warn(
    `daemon not healthy within ${STARTUP_TIMEOUT_MS / 1000}s — run \`wgenty-code daemon\` manually`,
  );
}

async function keepOwnerAlive(server: ViteDevServer): Promise<void> {
  // The dev server is the daemon's owner while it runs: hold a heartbeat
  // stream so a `--spawned-by web` daemon stays up, and revive a daemon that
  // dies mid-session (crash, kill -9, sleep/wake) by reconnecting through
  // ensureDaemon. When vite exits, this stream drops and the daemon follows
  // once its other clients are gone. WGENTY_AUTOSTART=0 disables this too.
  let stopped = false;
  let abort: AbortController | null = null;
  server.httpServer?.once("close", () => {
    stopped = true;
    abort?.abort();
  });
  let backoff = 500;
  while (!stopped) {
    const port = candidatePorts()[0] ?? defaultDaemonPort();
    abort = new AbortController();
    try {
      const res = await fetch(`http://127.0.0.1:${port}/api/v1/client/heartbeat`, {
        signal: abort.signal,
      });
      if (res.ok && res.body) {
        backoff = 500;
        // Consume the SSE until it ends — daemon exit/restart breaks it.
        const reader = res.body.getReader();
        while (!stopped) {
          const { done } = await reader.read();
          if (done) break;
        }
        await reader.cancel().catch(() => {});
      }
    } catch {
      // Stream refused/closed — the daemon may be gone.
    }
    abort = null;
    if (stopped) return;
    server.config.logger.warn("daemon owner stream broke — reviving it");
    await ensureDaemon(server);
    await new Promise((sleep) => setTimeout(sleep, backoff));
    backoff = Math.min(backoff * 2, OWNER_BACKOFF_MAX_MS);
  }
}

/** Dev-server-only plugin: bring the daemon up automatically on `vite`. */
export function daemonAutostart(): Plugin {
  return {
    name: "daemon-autostart",
    configureServer(server) {
      if (process.env.WGENTY_AUTOSTART === "0") return;
      // Fire-and-forget: never block the first page load. The owner stream
      // keeps the daemon available for the lifetime of the dev server.
      void ensureDaemon(server).then(() => keepOwnerAlive(server));
    },
  };
}
