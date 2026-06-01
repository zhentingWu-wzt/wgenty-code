#!/usr/bin/env node
import { spawn, type ChildProcess } from "node:child_process";
import { createInterface } from "node:readline";
import { ApiClient, AgentLoop } from "@wgenty-code/core";
import type { AgentCallbacks, ToolResult } from "@wgenty-code/core";
import {
  printWelcome,
  printUserMessage,
  printInfo,
  printError,
  printSuccess,
  printToolStart,
  printToolResult,
  startThinking,
} from "./render.ts";

// ── Configuration ────────────────────────────────────────────────────────────

const PORT = parseInt(process.env.CLAUDE_DAEMON_PORT ?? "8371", 10);
const DAEMON_CMD = process.env.CLAUDE_DAEMON_CMD ?? "cargo";
const DAEMON_ARGS = process.env.CLAUDE_DAEMON_ARGS?.split(" ") ?? [
  "run",
  "--",
  "daemon",
  "--port",
  String(PORT),
];

// ── Daemon lifecycle ─────────────────────────────────────────────────────────

async function startDaemon(): Promise<ChildProcess> {
  console.log(`  Starting daemon on port ${PORT}...`);

  const proc = spawn(DAEMON_CMD, DAEMON_ARGS, {
    stdio: ["ignore", "pipe", "pipe"],
  });

  proc.stderr?.pipe(process.stderr);

  // Wait for daemon to be ready
  let attempts = 0;
  while (attempts < 30) {
    try {
      const res = await fetch(`http://127.0.0.1:${PORT}/api/v1/health`);
      if (res.ok) {
        console.log("  Daemon started.\n");
        return proc;
      }
    } catch {
      // not ready yet
    }
    await new Promise((r) => setTimeout(r, 200));
    attempts++;
  }

  throw new Error("Daemon failed to start within 6 seconds");
}

async function stopDaemon(proc: ChildProcess): Promise<void> {
  proc.kill("SIGTERM");
  // Wait briefly for graceful shutdown
  await new Promise((r) => setTimeout(r, 500));
  if (!proc.killed) {
    proc.kill("SIGKILL");
  }
}

// ── Question UI ──────────────────────────────────────────────────────────────

function askQuestion(
  question: string,
  options: { label: string; description: string }[],
  multiSelect: boolean
): Promise<string[]> {
  return new Promise((resolve) => {
    console.log(`\n  ┌─ Question ──────────────────────────`);
    console.log(`  │ ${question}`);
    console.log(`  │`);

    options.forEach((opt, i) => {
      const desc = opt.description ? ` — ${opt.description}` : "";
      console.log(`  │  ${i + 1}. ${opt.label}${desc}`);
    });
    console.log(`  │  ${options.length + 1}. Other — enter custom answer`);
    console.log(`  └──────────────────────────────────────`);

    if (multiSelect) {
      process.stdout.write(
        "  Select options (comma-separated numbers, or type custom): "
      );
    } else {
      process.stdout.write("  Select an option (number or type custom): ");
    }

    const rl = createInterface({ input: process.stdin });
    rl.once("line", (line) => {
      rl.close();
      const trimmed = line.trim();

      if (multiSelect) {
        const indices = trimmed
          .split(",")
          .map((s) => parseInt(s.trim(), 10))
          .filter((n) => !isNaN(n) && n > 0 && n <= options.length + 1);
        const answers = indices.map((idx) => {
          if (idx === options.length + 1) return trimmed;
          return options[idx - 1].label;
        });
        resolve(answers);
      } else {
        const num = parseInt(trimmed, 10);
        if (!isNaN(num) && num > 0 && num <= options.length) {
          resolve([options[num - 1].label]);
        } else {
          resolve([trimmed]); // custom text
        }
      }
    });
  });
}

// ── Permission UI ────────────────────────────────────────────────────────────

function askPermission(
  reason: string,
  detail?: string
): Promise<"allow" | "always" | "deny"> {
  return new Promise((resolve) => {
    console.log(`\n  ┌─ Permission Required ──────────────`);
    console.log(`  │ ${reason}`);
    if (detail) console.log(`  │ ${detail}`);
    console.log(`  │`);
    console.log(`  │ [y] Allow once  [a] Always  [n] Deny`);
    console.log(`  └──────────────────────────────────────`);
    process.stdout.write("  > ");

    const rl = createInterface({ input: process.stdin });
    rl.once("line", (line) => {
      rl.close();
      switch (line.trim().toLowerCase()) {
        case "y":
        case "yes":
          resolve("allow");
          break;
        case "a":
        case "always":
          resolve("always");
          break;
        default:
          resolve("deny");
          break;
      }
    });
  });
}

// ── Main ─────────────────────────────────────────────────────────────────────

async function main(): Promise<void> {
  const args = process.argv.slice(2);
  const initialPrompt = args.length > 0 ? args.join(" ") : undefined;

  // Start the daemon
  let daemon: ChildProcess;
  try {
    daemon = await startDaemon();
  } catch (err) {
    console.error("Failed to start daemon:", err);
    process.exit(1);
  }

  // Cleanup on exit
  const cleanup = async () => {
    await stopDaemon(daemon);
    process.exit(0);
  };
  process.on("SIGINT", cleanup);
  process.on("SIGTERM", cleanup);

  // Create API client
  const client = new ApiClient({ baseUrl: `http://127.0.0.1:${PORT}` });

  // Get config for welcome message
  let model = "unknown";
  try {
    const config = await client.getConfig();
    model = config.model;
  } catch {
    // ignore
  }

  printWelcome(model);

  // Shared mutable state for callbacks
  let stopCurrentThinking: (() => void) | null = null;
  let isStreaming = false;

  // Create callbacks
  const callbacks: AgentCallbacks = {
    onContentDelta(content: string) {
      if (!isStreaming) {
        // Stop the thinking spinner on first content
        stopCurrentThinking?.();
        isStreaming = true;
      }
      process.stdout.write(content);
    },

    onReasoningDelta(_content: string) {
      // reasoning is not displayed to user
    },

    onToolStart(name: string, args: Record<string, unknown>) {
      isStreaming = false;
      printToolStart(name, args);
    },

    onToolResult(_name: string, result: ToolResult) {
      printToolResult(result.success, result.content);
    },

    async onPermissionRequired(reason: string, _sessionRule: string) {
      return askPermission(reason);
    },

    async onAskUserQuestion(question, options, multiSelect) {
      return askQuestion(question, options, multiSelect);
    },

    onStreamRetry() {
      // Readline REPL — reset output state for retry
      isStreaming = false;
      process.stdout.write("\n[stream interrupted, retrying...]\n");
    },
  };

  const agent = new AgentLoop({ client, callbacks });

  // Helper to run agent with thinking indicator
  const runWithThinking = async (input: string) => {
    isStreaming = false;
    printUserMessage(input);
    stopCurrentThinking = startThinking();
    try {
      await agent.processInput(input);
    } catch (err) {
      printError(`Error: ${err}`);
    }
    stopCurrentThinking?.();
    stopCurrentThinking = null;
    isStreaming = false;
  };

  // Process initial prompt if provided
  if (initialPrompt) {
    await runWithThinking(initialPrompt);
  }

  // Readline REPL
  const rl = createInterface({
    input: process.stdin,
    output: process.stdout,
    prompt: "  ▸ ",
  });

  rl.prompt();

  rl.on("line", async (line) => {
    const input = line.trim();
    if (!input) {
      rl.prompt();
      return;
    }

    // Built-in commands
    switch (input) {
      case "exit":
      case "quit":
      case ".exit":
      case ":q":
        console.log("\n  Goodbye!\n");
        rl.close();
        await cleanup();
        return;

      case "clear":
      case ".clear":
      case ":c":
        console.clear();
        rl.prompt();
        return;

      case "reset":
      case ".reset":
        agent.reset();
        printSuccess("Conversation reset");
        rl.prompt();
        return;

      case "history":
      case ".history":
        printInfo(
          `Conversation: ${agent.conversationHistory.length} messages`
        );
        agent.conversationHistory.forEach((msg, i) => {
          const preview = (msg.content ?? "").slice(0, 60);
          const suffix = (msg.content ?? "").length > 60 ? "..." : "";
          console.log(`  ${i + 1}. [${msg.role}] ${preview}${suffix}`);
        });
        rl.prompt();
        return;

      case "help":
      case ".help":
      case ":h":
        console.log(`
  Commands:
    exit / quit    — Exit the REPL
    clear          — Clear the screen
    reset          — Reset conversation
    history        — Show conversation history
    help           — Show this help
`);
        rl.prompt();
        return;
    }

    // Process input through agent
    await runWithThinking(input);
    rl.prompt();
  });

  rl.on("close", () => {
    cleanup();
  });
}

main().catch((err) => {
  console.error("Fatal error:", err);
  process.exit(1);
});
