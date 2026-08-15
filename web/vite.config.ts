/// <reference types="vitest" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { readFileSync } from "node:fs";
import { homedir } from "node:os";
import { resolve } from "node:path";

// ── Daemon connection ────────────────────────────────────────────────────────
// The daemon binds 127.0.0.1 only and writes a per-start bearer token to
// ~/.wgenty-code/daemon.token (mode 0600). Rather than ship the token to the
// browser, the dev server injects it into proxied /api requests via
// `server.proxy.configure`. The browser therefore never sees the secret.
//
// The daemon's actual port is read from ~/.wgenty-code/daemon.json on every
// proxied request (via the `router` callback), so a daemon on a non-default
// port (e.g. TUI fell back to a random port, or Desktop spawned on 8371) is
// automatically picked up. Falls back to `DAEMON_PORT` env / 8371.
const DEFAULT_DAEMON_PORT = Number(process.env.DAEMON_PORT ?? 8371);

function readDaemonToken(): string | null {
  // Best-effort read. The token is regenerated each daemon start, so a missing
  // or stale file simply means the browser will see 401s until the daemon runs.
  const tokenPath = resolve(homedir(), ".wgenty-code", "daemon.token");
  try {
    const token = readFileSync(tokenPath, "utf8").trim();
    if (token) return token;
  } catch {
    // Fall through to the discovery file.
  }
  // Fallback: the discovery file (daemon.json) also carries the token and is
  // rewritten by the live daemon's heartbeat every 30s — it survives races
  // where a shutting-down instance removed the standalone token file.
  const discoveryPath = resolve(homedir(), ".wgenty-code", "daemon.json");
  try {
    const parsed = JSON.parse(readFileSync(discoveryPath, "utf8"));
    if (typeof parsed.token === "string" && parsed.token) return parsed.token;
  } catch {
    // Missing or corrupt — no token available.
  }
  return null;
}

function readDaemonPort(): number {
  // Best-effort read from ~/.wgenty-code/daemon.json. The daemon writes this
  // file on startup and refreshes the heartbeat every 30s. If the file is
  // missing or corrupt, fall back to the default port.
  const discoveryPath = resolve(homedir(), ".wgenty-code", "daemon.json");
  try {
    const body = readFileSync(discoveryPath, "utf8");
    const parsed = JSON.parse(body);
    if (typeof parsed.port === "number" && parsed.port > 0) {
      return parsed.port;
    }
  } catch {
    // Missing or corrupt -> fall back to default.
  }
  return DEFAULT_DAEMON_PORT;
}

export default defineConfig({
  plugins: [
    react(),
    tailwindcss(),
    {
      // `GET /__daemon-info` → { port, token } read fresh from the daemon's
      // state files. The page uses this to open its LONG-LIVED SSE streams
      // directly against the daemon origin (127.0.0.1:<port>) instead of
      // through this dev server: browsers cap HTTP/1.1 at ~6 connections per
      // origin, and every permanent stream (HMR websocket, heartbeat, trace
      // SSE, session SSE) through the proxy eats one — a few tabs/turns
      // starve every other request ("send does nothing"). Short API calls
      // still go through the proxy. The token never leaves the loopback
      // machine (same trust boundary as desktop's fetch injection).
      name: "daemon-info",
      configureServer(server) {
        server.middlewares.use("/__daemon-info", (_req, res) => {
          res.setHeader("content-type", "application/json");
          res.end(
            JSON.stringify({ port: readDaemonPort(), token: readDaemonToken() }),
          );
        });
      },
    },
  ],
  server: {
    port: 5173, // 5173 is already in the daemon's CORS allow-list (src/daemon/mod.rs).
    proxy: {
      "/api": {
        // Default target; the `router` callback below overrides this per
        // request with the actual port from daemon.json.
        target: `http://127.0.0.1:${DEFAULT_DAEMON_PORT}`,
        changeOrigin: true,
        // Dynamically resolve the daemon port on every request so a daemon
        // restart on a different port is picked up without restarting Vite.
        router: () => `http://127.0.0.1:${readDaemonPort()}`,
        // Inject the bearer token on every proxied request. Read lazily so a
        // daemon restart (new token) is picked up without restarting vite -
        // the browser just needs to retry.
        configure: (proxy) => {
          proxy.on("proxyReq", (proxyReq) => {
            const token = readDaemonToken();
            if (token) {
              proxyReq.setHeader("authorization", `Bearer ${token}`);
            }
          });
        },
      },
    },
  },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.{ts,tsx}"],
    setupFiles: ["./src/test/setup.ts"],
  },
});
