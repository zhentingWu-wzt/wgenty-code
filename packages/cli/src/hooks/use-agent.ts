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
        ? result.content ?? "Done"
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
    case "task_management":
      return `Task: ${args.action ?? "..."}...`;
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
  const action = args.action as string | undefined;

  if (path) parts.push(`path: ${path}`);
  if (command) parts.push(`\`${command}\``);
  if (pattern) parts.push(`pattern: ${pattern}`);
  if (operation) parts.push(operation);
  if (action) parts.push(action);

  return parts.join(" · ") || "";
}
