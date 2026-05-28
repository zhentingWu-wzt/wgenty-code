#!/usr/bin/env node
import { spawn, type ChildProcess } from "node:child_process";
import { render } from "ink";
import React from "react";
import { App } from "./components/app.tsx";

const PORT = parseInt(process.env.CLAUDE_DAEMON_PORT ?? "8371", 10);
const DAEMON_CMD = process.env.CLAUDE_DAEMON_CMD ?? "cargo";
const DAEMON_ARGS = process.env.CLAUDE_DAEMON_ARGS?.split(" ") ?? [
  "run",
  "--features",
  "daemon",
  "--",
  "daemon",
  "--port",
  String(PORT),
];

async function startDaemon(): Promise<ChildProcess> {
  const proc = spawn(DAEMON_CMD, DAEMON_ARGS, {
    stdio: ["ignore", "pipe", "pipe"],
    cwd: process.env.CLAUDE_PROJECT_ROOT ?? undefined,
  });

  proc.stderr?.on("data", (data: Buffer) => {
    const text = data.toString();
    // Filter out cargo build noise
    if (!text.includes("Compiling") && !text.includes("Finished")) {
      process.stderr.write(text);
    }
  });

  // Wait for daemon to be ready
  for (let i = 0; i < 60; i++) {
    try {
      const res = await fetch(`http://127.0.0.1:${PORT}/api/v1/health`);
      if (res.ok) return proc;
    } catch {
      // not ready
    }
    await new Promise((r) => setTimeout(r, 500));
  }

  throw new Error("Daemon failed to start within 30 seconds");
}

async function main(): Promise<void> {
  let daemon: ChildProcess;

  try {
    daemon = await startDaemon();
  } catch (err) {
    console.error("Failed to start daemon:", err);
    process.exit(1);
  }

  const cleanup = () => {
    daemon.kill("SIGTERM");
    setTimeout(() => {
      if (!daemon.killed) daemon.kill("SIGKILL");
    }, 1000);
  };

  process.on("exit", cleanup);
  process.on("SIGINT", cleanup);
  process.on("SIGTERM", cleanup);

  const { waitUntilExit } = render(
    React.createElement(App, { workingDir: process.cwd() })
  );

  try {
    await waitUntilExit();
  } catch (err) {
    console.error("Ink render error:", err);
  } finally {
    cleanup();
  }
}

main().catch((err) => {
  console.error("Fatal error:", err);
  process.exit(1);
});
