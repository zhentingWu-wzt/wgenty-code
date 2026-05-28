import { useState, useRef, useCallback } from "react";
import { ApiClient, AgentLoop } from "@claude-code/core";
import type { AgentCallbacks, ToolResult } from "@claude-code/core";

export type MsgRole = "user" | "assistant" | "tool" | "system";

export interface UIMessage {
  id: number;
  role: MsgRole;
  content: string;
  /** Tool name if this is a tool call or result */
  toolName?: string;
  /** Tool args summary for tool_call messages */
  toolArgs?: string;
  /** Whether the tool succeeded (tool_result messages) */
  toolSuccess?: boolean;
  /** distinguish tool_call (invocation) from tool (result) */
  toolPhase?: "call" | "result";
  /** Number of merged calls (1 = single, >1 = merged) */
  toolCallCount?: number;
}

export type AgentStatus =
  | { type: "idle" }
  | { type: "thinking" }
  | { type: "streaming"; content: string }
  | { type: "executing"; toolName: string };

export interface UseAgentOptions {
  client: ApiClient;
}

export function useAgent({ client }: UseAgentOptions) {
  const [messages, setMessages] = useState<UIMessage[]>([]);
  const [status, setStatus] = useState<AgentStatus>({ type: "idle" });
  const [pendingQuestion, setPendingQuestion] = useState<{
    question: string;
    options: { label: string; description: string }[];
    multiSelect: boolean;
    resolve: (answers: string[]) => void;
  } | null>(null);
  const [pendingPermission, setPendingPermission] = useState<{
    reason: string;
    sessionRule: string;
    resolve: (choice: "allow" | "always" | "deny") => void;
  } | null>(null);

  const nextId = useRef(0);
  const agentRef = useRef<AgentLoop | null>(null);
  const streamingContentRef = useRef("");

  const addMsg = useCallback(
    (role: MsgRole, content: string, extra?: Partial<UIMessage>) => {
      const id = nextId.current++;
      setMessages((prev) => [...prev, { id, role, content, ...extra }]);
    },
    []
  );

  const updateLastMsg = useCallback((content: string) => {
    setMessages((prev) => {
      const updated = [...prev];
      if (updated.length > 0) {
        updated[updated.length - 1] = {
          ...updated[updated.length - 1],
          content,
        };
      }
      return updated;
    });
  }, []);

  // Throttle streaming updates: render at most every 100ms, not on every SSE byte
  const streamTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const callbacks: AgentCallbacks = {
    onContentDelta(content: string) {
      streamingContentRef.current += content;

      // Deferred render: accumulate deltas and flush periodically
      if (streamTimerRef.current === null) {
        streamTimerRef.current = setTimeout(() => {
          streamTimerRef.current = null;
          setStatus({ type: "streaming", content: streamingContentRef.current });
          updateLastMsg(streamingContentRef.current);
        }, 100);
      }
    },

    onReasoningDelta(_content: string) {
      setStatus((prev) => (prev.type === "thinking" ? prev : { type: "thinking" }));
    },

    onToolStart(name: string, args: Record<string, unknown>) {
      // Flush any pending streaming update before switching to executing
      if (streamTimerRef.current !== null) {
        clearTimeout(streamTimerRef.current);
        streamTimerRef.current = null;
        // Flush final accumulated content
        setStatus({ type: "streaming", content: streamingContentRef.current });
        updateLastMsg(streamingContentRef.current);
      }
      streamingContentRef.current = "";
      setStatus({ type: "executing", toolName: name });
      // Add tool invocation message with key args
      addMsg("tool", formatToolCallSummary(name, args), {
        toolName: name,
        toolArgs: formatToolArgs(args),
        toolPhase: "call",
        toolSuccess: undefined,
      });
    },

    onToolResult(name: string, result: ToolResult) {
      const summary = result.success
        ? formatToolResult(name, result.content ?? "Done")
        : result.error ?? "failed";
      addMsg("tool", summary, {
        toolName: name,
        toolPhase: "result",
        toolSuccess: result.success,
      });
    },

    async onPermissionRequired(
      reason: string,
      sessionRule: string
    ): Promise<"allow" | "always" | "deny"> {
      return new Promise((resolve) => {
        setPendingPermission({ reason, sessionRule, resolve });
      });
    },

    async onAskUserQuestion(
      question: string,
      options: { label: string; description: string }[],
      multiSelect: boolean
    ): Promise<string[]> {
      return new Promise((resolve) => {
        setPendingQuestion({ question, options, multiSelect, resolve });
      });
    },

    onStreamRetry() {
      // Clear partial streaming content before retry
      streamingContentRef.current = "";
      updateLastMsg("");
    },
  };

  const sendMessage = useCallback(
    async (input: string) => {
      if (!input.trim()) return;

      addMsg("user", input);
      // Add empty assistant placeholder so streaming deltas update it, not the user msg
      addMsg("assistant", "");
      streamingContentRef.current = "";
      setStatus({ type: "thinking" });

      // Reuse agent instance to preserve conversation history
      if (!agentRef.current) {
        agentRef.current = new AgentLoop({ client, callbacks });
      }
      const agent = agentRef.current;

      try {
        await agent.processInput(input);
      } catch (err) {
        addMsg("system", `Error: ${err}`);
      }

      // Flush any pending streaming update
      if (streamTimerRef.current !== null) {
        clearTimeout(streamTimerRef.current);
        streamTimerRef.current = null;
        updateLastMsg(streamingContentRef.current);
      }

      // Remove empty assistant placeholder if nothing was streamed (tool-only response)
      setMessages((prev) => {
        if (prev.length > 0) {
          const last = prev[prev.length - 1];
          if (last.role === "assistant" && last.content === "") {
            return prev.slice(0, -1);
          }
        }
        return prev;
      });
      setStatus({ type: "idle" });
    },
    [client, addMsg, updateLastMsg]
  );

  const reset = useCallback(() => {
    setMessages([]);
    setStatus({ type: "idle" });
    streamingContentRef.current = "";
    agentRef.current?.reset();
  }, []);

  const resolvePermission = useCallback(
    (choice: "allow" | "always" | "deny") => {
      if (pendingPermission) {
        pendingPermission.resolve(choice);
        setPendingPermission(null);
      }
    },
    [pendingPermission]
  );

  const resolveQuestion = useCallback(
    (answers: string[]) => {
      if (pendingQuestion) {
        pendingQuestion.resolve(answers);
        setPendingQuestion(null);
      }
    },
    [pendingQuestion]
  );

  return {
    messages,
    status,
    pendingQuestion,
    pendingPermission,
    sendMessage,
    reset,
    resolvePermission,
    resolveQuestion,
  };
}

// ── Tool display helpers ──────────────────────────────────────────────────────

/** Route tool result content through tool-specific formatters. */
function formatToolResult(name: string, content: string): string {
  if (name === "task_management") {
    const formatted = formatTaskManagementResult(content);
    if (formatted !== null) return formatted;
  }
  if (name === "TodoWrite") return content; // Already formatted by the tool itself
  return content;
}

/** Pretty-print a task_management tool result. Returns null if not parseable. */
function formatTaskManagementResult(content: string): string | null {
  try {
    const data = JSON.parse(content);
    if (!data.success) return null;

    // list operation — render a task table
    if (data.tasks && Array.isArray(data.tasks)) {
      if (data.tasks.length === 0) return "No tasks found.";
      const lines: string[] = [`Tasks (${data.count ?? data.tasks.length}):`];
      for (const t of data.tasks) {
        const s = statusIcon(t.status);
        const p = priorityLabel(t.priority);
        const id = (t.id as string).slice(0, 8);
        const tags =
          t.tags && (t.tags as string[]).length > 0
            ? ` [${(t.tags as string[]).join(", ")}]`
            : "";
        lines.push(`  ${s} ${p} [${id}] ${t.subject}${tags}`);
      }
      return lines.join("\n");
    }

    // single task result (get, complete, update)
    if (data.task) {
      const t = data.task;
      const s = statusIcon(t.status);
      const p = priorityLabel(t.priority);
      const lines: string[] = [
        `${s} ${p} [${(t.id as string).slice(0, 8)}] ${t.subject}`,
        `   Status: ${t.status} | Priority: ${t.priority}`,
      ];
      if (t.description) lines.push(`   ${t.description}`);
      if (t.tags && (t.tags as string[]).length > 0)
        lines.push(`   Tags: ${(t.tags as string[]).join(", ")}`);
      return lines.join("\n");
    }

    // create / delete — just message + task_id
    if (data.message) {
      return data.task_id
        ? `${data.message}\n   ID: ${(data.task_id as string).slice(0, 8)}`
        : data.message;
    }

    return null;
  } catch {
    return null;
  }
}

// ── Shared display helpers ────────────────────────────────────────────────────

function statusIcon(status: string): string {
  switch (status) {
    case "Pending":
      return "○";
    case "InProgress":
      return "◐";
    case "Completed":
      return "✓";
    case "Deleted":
      return "✗";
    default:
      return "?";
  }
}

function priorityLabel(priority: string): string {
  switch (priority) {
    case "Critical":
      return "🔴";
    case "High":
      return "🟠";
    case "Medium":
      return "🟡";
    case "Low":
      return "🟢";
    default:
      return "";
  }
}

/** Format a one-line summary of what the tool call is doing. */
function formatToolCallSummary(
  name: string,
  args: Record<string, unknown>
): string {
  switch (name) {
    case "file_read":
      return `Reading ${args.path ?? "file"}...`;
    case "file_write":
      return `Writing ${args.path ?? "file"}...`;
    case "file_edit":
      return `Editing ${args.path ?? "file"}...`;
    case "execute_command":
      return `Running \`${args.command ?? "command"}\`...`;
    case "search":
      return `Searching for \`${args.pattern ?? ""}\`...`;
    case "list_files":
      return `Listing ${args.path ?? "directory"}...`;
    case "git_operations":
      return `Git ${args.operation ?? "operation"}...`;
    case "task_management": {
      const op = args.operation as string | undefined;
      if (op === "create" && args.subject)
        return `Creating task "${args.subject}"...`;
      if (op === "update" && args.task_id)
        return `Updating task ${(args.task_id as string).slice(0, 8)}...`;
      if (op === "delete" && args.task_id)
        return `Deleting task ${(args.task_id as string).slice(0, 8)}...`;
      if (op === "complete" && args.task_id)
        return `Completing task ${(args.task_id as string).slice(0, 8)}...`;
      if (op === "get" && args.task_id)
        return `Getting task ${(args.task_id as string).slice(0, 8)}...`;
      if (op === "list") return `Listing tasks...`;
      return `Task: ${op ?? "..."}...`;
    }
    case "TodoWrite": {
      const newItems = args.items as Array<Record<string, string>> | undefined;
      if (newItems && newItems.length > 0) {
        const total = newItems.length;
        const done = newItems.filter(
          (i) => i.status === "completed"
        ).length;
        return `Updating todos (${done}/${total})...`;
      }
      return `Updating todos...`;
    }
    case "note_edit":
      return `Editing notes...`;
    case "view":
      return `Viewing ${args.path ?? "."}...`;
    default:
      return `${name}...`;
  }
}

/** Extract key tool args as a compact display string. */
function formatToolArgs(args: Record<string, unknown>): string {
  const parts: string[] = [];
  const path = args.path as string | undefined;
  const command = args.command as string | undefined;
  const pattern = args.pattern as string | undefined;
  const operation = args.operation as string | undefined;

  if (path) parts.push(`path: ${path}`);
  if (command) parts.push(`\`${command}\``);
  if (pattern) parts.push(`pattern: ${pattern}`);
  if (operation) parts.push(operation);

  // TodoWrite specific
  if (args.items && Array.isArray(args.items)) {
    const total = (args.items as Array<Record<string, unknown>>).length;
    const done = (args.items as Array<Record<string, unknown>>).filter(
      (i) => i.status === "completed"
    ).length;
    parts.push(`${done}/${total}`);
  }

  // task_management specific
  if (args.subject) parts.push(`"${args.subject}"`);
  if (args.task_id)
    parts.push(`id: ${(args.task_id as string).slice(0, 8)}`);
  if (args.status) parts.push(`status: ${args.status}`);
  if (args.priority) parts.push(`${args.priority}`);
  if (args.filter) {
    const f = args.filter as Record<string, unknown>;
    const fparts: string[] = [];
    if (f.status) fparts.push(`status=${f.status}`);
    if (f.priority) fparts.push(`priority=${f.priority}`);
    if (fparts.length) parts.push(`filter: ${fparts.join(",")}`);
  }

  return parts.join(" · ") || "";
}
