/// <reference types="vitest" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { readFileSync } from "node:fs";
import { homedir } from "node:os";
import { resolve } from "node:path";

// ── Daemon connection ────────────────────────────────────────────────────────
// The daemon binds 127.0.0.1 only and writes a per-start bearer token to
// ~/.wgenty-code/daemon.token (mode 0600). Rather than ship the token to the
// browser, the dev server injects it into proxied /api requests via
// `server.proxy.configure`. The browser therefore never sees the secret.
//
// Override the port with `DAEMON_PORT` if you start the daemon with a custom
// `--port`. Default mirrors `wgenty-code daemon` (see src/daemon/mod.rs:8).
const DAEMON_PORT = Number(process.env.DAEMON_PORT ?? 8371);
const DAEMON_TARGET = `http://127.0.0.1:${DAEMON_PORT}`;

function readDaemonToken(): string | null {
  // Best-effort read. The token is regenerated each daemon start, so a missing
  // or stale file simply means the browser will see 401s until the daemon runs.
  const tokenPath = resolve(homedir(), ".wgenty-code", "daemon.token");
  try {
    return readFileSync(tokenPath, "utf8").trim();
  } catch {
    return null;
  }
}

export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173, // 5173 is already in the daemon's CORS allow-list (src/daemon/mod.rs).
    proxy: {
      "/api": {
        target: DAEMON_TARGET,
        changeOrigin: true,
        // Inject the bearer token on every proxied request. Read lazily so a
        // daemon restart (new token) is picked up without restarting vite —
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
    environment: "node",
    include: ["src/**/*.test.ts"],
  },
});
