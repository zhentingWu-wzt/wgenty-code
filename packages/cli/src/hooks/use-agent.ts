import React, { useState, useRef, useCallback } from "react";
import { ApiClient, AgentLoop } from "@claude-code/core";
import type { AgentCallbacks, ToolResult, ChatMessage, SessionInfo } from "@claude-code/core";

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
  | { type: "streaming" }
  | { type: "executing"; toolName: string };

export interface UseAgentOptions {
  client: ApiClient;
}

/** Try to parse JSON, return null on failure. */
function tryParseJson(content?: string | null): Record<string, unknown> | null {
  if (!content) return null;
  try { return JSON.parse(content); } catch { return null; }
}

/** Find the tool name for a given tool_call_id from the messages array. */
function findToolNameForId(msgs: ChatMessage[], toolCallId?: string | null): string | undefined {
  if (!toolCallId) return undefined;
  for (const msg of msgs) {
    if (msg.role === "assistant" && msg.tool_calls) {
      for (const tc of msg.tool_calls) {
        if (tc.id === toolCallId) return tc.function.name;
      }
    }
  }
  return undefined;
}

export function useAgent({ client }: UseAgentOptions) {
  const [committedMessages, setCommittedMessages] = useState<UIMessage[]>([]);
  const [streamingContent, setStreamingContent] = useState("");
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

  const [sessionId, setSessionId] = useState<string>(() => crypto.randomUUID());
  const [sessionName, setSessionName] = useState<string>("");
  const [sessionListOpen, setSessionListOpen] = useState(false);
  const [sessions, setSessions] = useState<SessionInfo[]>([]);
  const [dirty, setDirty] = useState(false);

  const nextId = useRef(0);
  const agentRef = useRef<AgentLoop | null>(null);
  const streamingContentRef = useRef("");
  const saveRef = useRef<() => Promise<void>>(async () => {});

  const addMsg = useCallback(
    (role: MsgRole, content: string, extra?: Partial<UIMessage>) => {
      const id = nextId.current++;
      setCommittedMessages((prev) => [...prev, { id, role, content, ...extra }]);
    },
    []
  );

  // Throttle streaming updates: render at most every 100ms, not on every SSE byte
  const streamTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const callbacksRef = useRef<AgentCallbacks>(null!);

  const callbacks: AgentCallbacks = {
    onContentDelta(content: string) {
      streamingContentRef.current += content;

      // Deferred render: accumulate deltas and flush periodically
      if (streamTimerRef.current === null) {
        streamTimerRef.current = setTimeout(() => {
          streamTimerRef.current = null;
          setStatus({ type: "streaming" });
          setStreamingContent(streamingContentRef.current);
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
        const content = streamingContentRef.current;
        if (content) {
          const id = nextId.current++;
          setCommittedMessages((prev) => [...prev, { id, role: "assistant", content }]);
        }
        streamingContentRef.current = "";
        setStreamingContent("");
      }
      // Timer already fired: flush accumulated streaming content before tool call
      const pending = streamingContentRef.current;
      if (pending) {
        const id = nextId.current++;
        setCommittedMessages((prev) => [...prev, { id, role: "assistant", content: pending }]);
      }
      streamingContentRef.current = "";
      setStreamingContent("");
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
      // Clear pending timer and partial streaming content before retry
      if (streamTimerRef.current !== null) {
        clearTimeout(streamTimerRef.current);
        streamTimerRef.current = null;
      }
      streamingContentRef.current = "";
      setStreamingContent("");
    },
  };
  callbacksRef.current = callbacks;

  /** Rebuild UIMessage[] from raw ChatMessage[] for display after loading a session. */
  const rebuildUIMessages = useCallback(
    (msgs: ChatMessage[]): UIMessage[] => {
      const ui: UIMessage[] = [];
      for (const msg of msgs) {
        if (msg.role === "system") continue;

        if (msg.role === "user") {
          ui.push({ id: nextId.current++, role: "user", content: msg.content ?? "" });
        } else if (msg.role === "assistant") {
          ui.push({ id: nextId.current++, role: "assistant", content: msg.content ?? "" });
          if (msg.tool_calls) {
            for (const tc of msg.tool_calls) {
              let args: Record<string, unknown> = {};
              try { args = JSON.parse(tc.function.arguments); } catch { /* ignore */ }
              ui.push({
                id: nextId.current++,
                role: "tool",
                content: formatToolCallSummary(tc.function.name, args),
                toolName: tc.function.name,
                toolArgs: formatToolArgs(args),
                toolPhase: "call",
              });
            }
          }
        } else if (msg.role === "tool") {
          const parsed = tryParseJson(msg.content);
          const success = (parsed?.success as boolean) ?? true;
          const toolName = findToolNameForId(msgs, msg.tool_call_id);
          ui.push({
            id: nextId.current++,
            role: "tool",
            content: msg.content ?? "",
            toolName,
            toolPhase: "result",
            toolSuccess: success,
          });
        }
      }
      return ui;
    },
    [],
  );

  /** Save current session to daemon. */
  const saveCurrentSession = useCallback(async () => {
    if (!agentRef.current) return;
    const history = agentRef.current.getHistory();
    if (history.length <= 1) return;
    try {
      await client.saveSession(sessionId, sessionName || "Untitled", history);
      setDirty(false);
    } catch (err) {
      console.error("Failed to save session:", err);
    }
  }, [client, sessionId, sessionName]);

  // Keep saveRef in sync for exit handler
  React.useEffect(() => {
    saveRef.current = saveCurrentSession;
  }, [saveCurrentSession]);

  /** Load a session from daemon and restore state. */
  const loadSession = useCallback(
    async (id: string) => {
      try {
        const session = await client.loadSession(id);
        if (!agentRef.current) {
          agentRef.current = new AgentLoop({ client, callbacks: callbacksRef.current });
        }
        agentRef.current.loadHistory(session.messages);
        const ui = rebuildUIMessages(session.messages);
        nextId.current = Math.max(0, ...ui.map((m) => m.id)) + 1;
        setCommittedMessages(ui);
        setStreamingContent("");
        setSessionId(session.id);
        setSessionName(session.name);
        setDirty(false);
      } catch (err) {
      }
    },
    [client, rebuildUIMessages],
  );

  /** Refresh the session list from daemon. */
  const refreshSessions = useCallback(async () => {
    try {
      const list = await client.listSessions();
      setSessions(list);
    } catch (err) {
      console.error("Failed to refresh sessions:", err);
    }
  }, [client]);

  /** Open session modal (save current first, refresh list). */
  const openSessionList = useCallback(async () => {
    await saveCurrentSession();
    await refreshSessions();
    setSessionListOpen(true);
  }, [saveCurrentSession, refreshSessions]);

  const closeSessionList = useCallback(() => {
    setSessionListOpen(false);
  }, []);

  const reset = useCallback(async () => {
    await saveCurrentSession();
    setCommittedMessages([]);
    setStreamingContent("");
    setStatus({ type: "idle" });
    streamingContentRef.current = "";
    agentRef.current?.reset();
    setSessionId(crypto.randomUUID());
    setSessionName("");
    setDirty(false);
  }, [saveCurrentSession]);

  /** Delete a session from daemon. */
  const deleteSessionById = useCallback(
    async (id: string) => {
      try {
        await client.deleteSession(id);
        if (id === sessionId) {
          const updated = sessions.filter((s) => s.id !== id);
          if (updated.length > 0) {
            await loadSession(updated[0].id);
          } else {
            reset();
            setSessionId(crypto.randomUUID());
            setSessionName("");
          }
        }
        await refreshSessions();
      } catch {
        // silently fail
      }
    },
    [client, sessionId, sessions, loadSession, refreshSessions, reset],
  );

  /** Rename current session. */
  const renameSession = useCallback(
    async (newName: string) => {
      setSessionName(newName);
      setDirty(true);
      await saveCurrentSession();
    },
    [saveCurrentSession],
  );

  const sendMessage = useCallback(
    async (input: string) => {
      if (!input.trim()) return;

      addMsg("user", input);
      // Auto-name: use first user message as session name
      if (!sessionName) {
        const name =
          input.length > 50 ? input.slice(0, 50) + "..." : input;
        setSessionName(name);
      }
      streamingContentRef.current = "";
      setStreamingContent("");
      setStatus({ type: "thinking" });

      // Reuse agent instance to preserve conversation history
      if (!agentRef.current) {
        agentRef.current = new AgentLoop({ client, callbacks: callbacksRef.current });
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
      }
      // Commit final streaming content as assistant message
      const finalContent = streamingContentRef.current;
      if (finalContent) {
        const id = nextId.current++;
        setCommittedMessages((prev) => [...prev, { id, role: "assistant", content: finalContent }]);
        streamingContentRef.current = "";
        setStreamingContent("");
      }

      setStatus({ type: "idle" });

      // Auto-save after each round-trip
      setDirty(true);
      saveCurrentSession();
    },
    [client, addMsg, saveCurrentSession]
  );

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
    committedMessages,
    streamingContent,
    status,
    pendingQuestion,
    pendingPermission,
    sendMessage,
    reset,
    resolvePermission,
    resolveQuestion,
    // Session management
    sessionId,
    sessionName,
    sessionListOpen,
    sessions,
    loadSession,
    saveCurrentSession,
    openSessionList,
    closeSessionList,
    deleteSession: deleteSessionById,
    renameSession,
    rebuildUIMessages,
    saveRef,
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
