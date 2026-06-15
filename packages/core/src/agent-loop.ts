import { ApiClient } from "./client.ts";
import { parseSseLine } from "./sse.ts";
import type {
  ChatMessage,
  StreamChunk,
  StreamChoice,
  ToolCall,
} from "./types.ts";
import * as path from "node:path";
import * as os from "node:os";
import { promises as fs } from "node:fs";

// ── Stream result ────────────────────────────────────────────────────────────

export interface StreamResult {
  content: string;
  reasoningContent: string;
  toolCalls: ToolCall[];
  hasToolCalls: boolean;
  finishReason: string;
  /** True if the stream received a finish_reason or [DONE] marker */
  streamComplete: boolean;
}

// ── Tool result ─────────────────────────────────────────────────────────────

export interface ToolResult {
  success: boolean;
  outputType?: string;
  content?: string;
  error?: string;
}

// ── Agent loop callbacks ─────────────────────────────────────────────────────

export interface AgentCallbacks {
  /** Called for each content delta during streaming */
  onContentDelta(content: string): void;
  /** Called for each reasoning delta */
  onReasoningDelta(content: string): void;
  /** Called when a tool call is about to execute */
  onToolStart(name: string, args: Record<string, unknown>): void;
  /** Called after tool execution */
  onToolResult(name: string, result: ToolResult): void;
  /** Called when permission is needed */
  onPermissionRequired(
    reason: string,
    sessionRule: string
  ): Promise<"allow" | "always" | "deny">;
  /** Called when ask_user_question is invoked */
  onAskUserQuestion(
    question: string,
    options: { label: string; description: string }[],
    multiSelect: boolean
  ): Promise<string[]>;
  /**
   * Called when a stream attempt starts (initial or retry).
   * `attempt` is 1-based (1 = first attempt, 2 = first retry, etc.)
   */
  onStreamConnecting(attempt: number, maxRetries: number): void;
  /**
   * Called before retrying a broken stream — UI should clear partial content.
   * `reason` indicates why the retry is happening.
   */
  onStreamRetry(reason: string): void;
}

// ── Stream processor (TypeScript port of Rust StreamProcessor) ──────────────

class StreamProcessor {
  private buffer = "";
  fullContent = "";
  reasoningContent = "";
  private toolCallsAccum: Record<string, unknown>[] = [];
  hasToolCalls = false;
  private finishReason = "";
  streamComplete = false;

  feedBytes(bytes: Uint8Array): StreamEvent[] {
    this.buffer += new TextDecoder().decode(bytes);
    return this.drainBuffer();
  }

  private drainBuffer(): StreamEvent[] {
    const events: StreamEvent[] = [];
    let idx: number;
    while ((idx = this.buffer.indexOf("\n")) !== -1) {
      const line = this.buffer.slice(0, idx).trim();
      this.buffer = this.buffer.slice(idx + 1);

      const event = this.processLine(line);
      if (event) events.push(event);
    }
    return events;
  }

  private processLine(line: string): StreamEvent | null {
    // Detect daemon error events — both raw JSON and SSE-wrapped.
    // Raw: {"error":"message"}
    // SSE: data: {"error":"message"}
    const trimmed = line.trim();
    let errorPayload: string | null = null;
    if (trimmed.startsWith("data: ")) {
      errorPayload = trimmed.slice(6);
    } else if (trimmed.startsWith("{")) {
      errorPayload = trimmed;
    }
    if (errorPayload) {
      try {
        const parsed = JSON.parse(errorPayload);
        if (parsed.error) {
          return { type: "stream_error", error: parsed.error as string };
        }
      } catch {
        // Not JSON, continue to standard processing
      }
    }

    const chunk = parseSseLine(line);
    if (!chunk) return null;

    // Check for error in parsed chunk (defensive — some daemon responses may
    // include error fields inside a stream-chunk-shaped object)
    const chunkAny = chunk as unknown as Record<string, unknown>;
    if (chunkAny.error) {
      return { type: "stream_error", error: String(chunkAny.error) };
    }

    const choice = chunk.choices[0];
    if (!choice) return null;

    // Finish reason
    if (choice.finish_reason) {
      this.finishReason = choice.finish_reason;
      this.streamComplete = true;
      return { type: "done", finishReason: choice.finish_reason };
    }

    // Content delta
    if (choice.delta.content) {
      this.fullContent += choice.delta.content;
      return { type: "content", content: choice.delta.content };
    }

    // Reasoning delta
    if (choice.delta.reasoning_content) {
      this.reasoningContent += choice.delta.reasoning_content;
      return { type: "reasoning", content: choice.delta.reasoning_content };
    }

    // Tool call deltas
    if (choice.delta.tool_calls) {
      this.hasToolCalls = true;
      for (const tc of choice.delta.tool_calls) {
        const idx = tc.index;
        while (this.toolCallsAccum.length <= idx) {
          this.toolCallsAccum.push({
            id: null,
            type: "function",
            function: { name: "", arguments: "" },
          });
        }
        const entry = this.toolCallsAccum[idx] as Record<string, unknown>;
        if (tc.id) entry.id = tc.id;
        if (tc.function) {
          const func = entry.function as Record<string, string>;
          if (tc.function.name) func.name = tc.function.name;
          if (tc.function.arguments)
            func.arguments = (func.arguments || "") + tc.function.arguments;
        }

        return {
          type: "tool_call_delta",
          index: idx,
          id: tc.id ?? undefined,
          name: tc.function?.name ?? undefined,
          arguments: tc.function?.arguments ?? undefined,
        };
      }
    }

    return null;
  }

  finish(): StreamResult {
    const toolCalls: ToolCall[] = this.toolCallsAccum
      .filter((c) => c.id)
      .map((c) => {
        const func = (c as Record<string, unknown>).function as Record<
          string,
          string
        >;
        return {
          id: c.id as string,
          type: "function" as const,
          function: {
            name: func.name,
            arguments: func.arguments,
          },
        };
      });

    return {
      content: this.fullContent,
      reasoningContent: this.reasoningContent,
      toolCalls,
      hasToolCalls: this.hasToolCalls,
      finishReason: this.finishReason,
      streamComplete: this.streamComplete,
    };
  }
}

// ── Stream event ─────────────────────────────────────────────────────────────

type StreamEvent =
  | { type: "content"; content: string }
  | { type: "reasoning"; content: string }
  | {
      type: "tool_call_delta";
      index: number;
      id?: string;
      name?: string;
      arguments?: string;
    }
  | { type: "done"; finishReason: string }
  | { type: "stream_error"; error: string };

/** Build the system prompt for the agent. */
export function buildSystemPrompt(): string {
  return `You are a coding agent with access to tools for reading/writing files, executing commands, searching code, git operations, and task tracking.

## Planning

Before any non-trivial multi-step task, use \`TodoWrite\` to break it down into a checklist. \
Replace the ENTIRE list each call — it's a batch update, not CRUD. \
Mark the current task \`in_progress\` (with activeForm) before starting, \`completed\` when done. \
Only ONE in_progress at a time. Max 20 items.

Example: for "add a login page", call TodoWrite with:
\`\`\`
items: [
  {content: "Create login component", status: "pending", activeForm: ""},
  {content: "Add auth API route", status: "pending", activeForm: ""},
  {content: "Write tests", status: "pending", activeForm: ""}
]
\`\`\`
Then mark the first one in_progress: \`{content: "Create login component", status: "in_progress", activeForm: "Creating login component"}\`

Prefer tools over prose. Update TodoWrite as you progress.

## Skills (on-demand)

Use \`load_skill\` to load full skill instructions when you need detailed guidance \
for a specific task. Call \`load_skill\` with no name to list available skills.`;
}

// ── Agent loop ───────────────────────────────────────────────────────────────

export interface AgentLoopOptions {
  client: ApiClient;
  callbacks: AgentCallbacks;
  sessionId?: string;
  maxTokens?: number;
}

/** Errors that should trigger a stream retry (network, timeout, stream break) */
function isRetryableError(err: unknown): boolean {
  const msg = String(err);
  // Don't retry on HTTP API errors (4xx/5xx from the daemon itself)
  if (msg.includes("API error") && !msg.includes("timeout")) return false;
  return true;
}

export class AgentLoop {
  private client: ApiClient;
  private callbacks: AgentCallbacks;
  private sessionId: string;
  private maxTokens?: number;
  private roundsSinceTodo = 0;
  private compactedSummary = "";
  private readonly MAX_ESTIMATED_TOKENS = 50000;
  conversationHistory: ChatMessage[] = [
    { role: "system", content: buildSystemPrompt() },
  ];

  constructor(options: AgentLoopOptions) {
    this.client = options.client;
    this.callbacks = options.callbacks;
    this.sessionId = options.sessionId ?? "default";
    this.maxTokens = options.maxTokens;
  }

  /** Process a single user input. Handles the full agent loop (SSE + tools). */
  async processInput(input: string): Promise<void> {
    // Inject any completed background task results before processing new input
    await this.injectBackgroundResults();

    this.conversationHistory.push({ role: "user", content: input });

    const maxRounds = 10;
    for (let round = 0; round < maxRounds; round++) {
      // Micro-compact old tool results before sending to API
      const messages = this.microCompact();

      // Auto-compaction check — trigger if estimated tokens exceed limit
      if (this.needsCompaction(messages)) {
        await this.doAutoCompact();
        // The conversation was replaced with a summary — restart the round
        continue;
      }

      // Stream with retry on network/stream errors
      const result = await this.streamWithRetry(messages);

      if (result.hasToolCalls && result.toolCalls.length > 0) {
        // Build assistant message
        const assistantMsg: ChatMessage = {
          role: "assistant",
          content: result.content || null,
          reasoning_content: result.reasoningContent || null,
          tool_calls: result.toolCalls,
          tool_call_id: null,
        };
        this.conversationHistory.push(assistantMsg);

        // Execute each tool
        let usedTodo = false;
        for (const tc of result.toolCalls) {
          let args: Record<string, unknown>;
          try {
            args = JSON.parse(tc.function.arguments);
          } catch {
            args = {};
          }

          if (tc.function.name === "ask_user_question") {
            const toolResult = await this.handleAskUserQuestion(args);
            this.conversationHistory.push({
              role: "tool",
              content: toolResult,
              tool_call_id: tc.id,
            });
            continue;
          }

          // s06: manual compaction — handle locally, don't call the daemon
          if (tc.function.name === "compact") {
            const compactResult: ToolResult = {
              success: true,
              outputType: "text",
              content:
                "Conversation history has been compressed to save context. Full transcript archived to ~/.wgenty-code/transcripts/.",
            };
            this.callbacks.onToolStart(tc.function.name, args);
            await this.doAutoCompact();
            this.callbacks.onToolResult(tc.function.name, compactResult);
            this.conversationHistory.push({
              role: "tool",
              content: JSON.stringify(compactResult),
              tool_call_id: tc.id,
            });
            continue;
          }

          if (tc.function.name === "TodoWrite") {
            usedTodo = true;
          }

          this.callbacks.onToolStart(tc.function.name, args);

          const execResult = await this.executeToolWithPermission(
            tc.function.name,
            args
          );

          this.callbacks.onToolResult(tc.function.name, execResult);

          const toolResultStr = JSON.stringify(execResult);
          this.conversationHistory.push({
            role: "tool",
            content: toolResultStr,
            tool_call_id: tc.id,
          });
        }

        // s03: nag reminder — inject after 3 rounds without TodoWrite
        this.roundsSinceTodo = usedTodo ? 0 : this.roundsSinceTodo + 1;
        if (this.roundsSinceTodo >= 3) {
          this.conversationHistory[this.conversationHistory.length - 1] = {
            ...this.conversationHistory[this.conversationHistory.length - 1],
            content:
              this.conversationHistory[this.conversationHistory.length - 1]
                .content +
              "\n<reminder>Update your todos with TodoWrite.</reminder>",
          };
        }

        // Continue loop — model gets tool results
        continue;
      }

      // Normal response
      if (result.content) {
        this.conversationHistory.push({
          role: "assistant",
          content: result.content,
          reasoning_content: result.reasoningContent || null,
        });
      }
      break;
    }
  }

  /** Stream with retry logic. Retries up to 2 times on network/stream errors. */
  private async streamWithRetry(messages: ChatMessage[]): Promise<StreamResult> {
    const maxRetries = 2;
    let lastError = "";

    for (let attempt = 0; attempt <= maxRetries; attempt++) {
      // Notify UI that we're connecting (1-based for display)
      this.callbacks.onStreamConnecting(attempt + 1, maxRetries + 1);

      try {
        const response = await this.client.chatStream({
          messages,
          max_tokens: this.maxTokens,
        });
        const result = await this.streamResponse(response);

        // Detect incomplete stream: has tool calls but never received finish_reason
        if (result.hasToolCalls && !result.streamComplete) {
          throw new Error("Stream ended before tool calls completed");
        }

        return result;
      } catch (err) {
        lastError = String(err);

        if (!isRetryableError(err) || attempt >= maxRetries) {
          throw err;
        }

        // Build a short reason for the retry notification
        const reason = lastError.includes("timeout")
          ? "connection timed out"
          : lastError.includes("⚠️")
          ? lastError // already wrapped, keep the user-friendly text
          : "network error";
        this.callbacks.onStreamRetry(reason);
        // Exponential backoff: 2s, 4s
        await new Promise((r) => setTimeout(r, (attempt + 1) * 2000));
      }
    }

    throw new Error(`Stream failed after retries: ${lastError}`);
  }

  private async streamResponse(response: Response): Promise<StreamResult> {
    const processor = new StreamProcessor();
    const reader = response.body?.getReader();
    if (!reader) throw new Error("Response body is not readable");

    try {
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;

        for (const event of processor.feedBytes(value)) {
          switch (event.type) {
            case "content":
              this.callbacks.onContentDelta(event.content);
              break;
            case "reasoning":
              this.callbacks.onReasoningDelta(event.content);
              break;
            case "tool_call_delta":
              // silently accumulate
              break;
            case "done":
              // stream complete — finish_reason received
              break;
            case "stream_error":
              throw new Error(`Daemon error: ${event.error}`);
          }
        }
      }
    } finally {
      reader.releaseLock();
    }

    return processor.finish();
  }

  private async executeToolWithPermission(
    name: string,
    args: Record<string, unknown>
  ): Promise<ToolResult> {
    // First attempt — execute directly
    const result = await this.client.executeTool({
      tool_name: name,
      arguments: args,
      session_id: this.sessionId,
    });

    // Check if permission is required
    if (result.permission_required) {
      const choice = await this.callbacks.onPermissionRequired(
        result.permission_required.reason,
        result.permission_required.session_rule
      );

      if (choice === "deny") {
        return {
          success: false,
          error: `PERMISSION DENIED: The user rejected '${name}'.`,
        };
      }

      // Approve the rule so the daemon allows the retry
      await this.client.approveTool(
        result.permission_required.session_rule
      );

      // Retry after approval
      const retryResult = await this.client.executeTool({
        tool_name: name,
        arguments: args,
        session_id: this.sessionId,
      });

      return {
        success: retryResult.success,
        outputType: retryResult.output_type ?? undefined,
        content: retryResult.content ?? undefined,
        error: retryResult.error ?? undefined,
      };
    }

    return {
      success: result.success,
      outputType: result.output_type ?? undefined,
      content: result.content ?? undefined,
      error: result.error ?? undefined,
    };
  }

  private async handleAskUserQuestion(
    args: Record<string, unknown>
  ): Promise<string> {
    const question = (args.question as string) ?? "Select an option:";
    const rawOptions = args.options as
      | { label: string; description: string }[]
      | undefined;
    const multiSelect = (args.multiSelect as boolean) ?? false;

    const options = rawOptions ?? [];
    const answers = await this.callbacks.onAskUserQuestion(
      question,
      options,
      multiSelect
    );

    return JSON.stringify({
      success: true,
      answers: answers.map((label) => ({ label, value: label, custom: false })),
    });
  }

  /** Replace conversation history entirely (for session restore). */
  loadHistory(messages: ChatMessage[]): void {
    this.roundsSinceTodo = 0;
    this.compactedSummary = "";
    this.conversationHistory = messages;
  }

  /** Expose current conversation history (for session save). */
  getHistory(): ChatMessage[] {
    return this.conversationHistory;
  }

  /** Reset conversation history, preserving the system prompt. */
  reset(): void {
    this.roundsSinceTodo = 0;
    this.compactedSummary = "";
    this.conversationHistory = [
      { role: "system", content: buildSystemPrompt() },
    ];
  }

  /**
   * Poll the daemon for completed background task results.
   * If any exist, inject them as a user message so the agent sees them
   * on the next iteration.
   */
  private async injectBackgroundResults(): Promise<void> {
    try {
      const results = await this.client.getBackgroundResults();
      if (results && results.length > 0) {
        const notification = results
          .map(
            (r: any) =>
              `[Background task ${r.task_id} completed: ${r.success ? "SUCCESS" : "FAILED"}]\n${r.stdout || r.stderr}`,
          )
          .join("\n\n");
        this.conversationHistory.push({
          role: "user",
          content: notification,
        });
      }
    } catch {
      // Silently ignore — background results are optional
    }
  }

  // ── s06: Context compaction ──────────────────────────────────────────────

  /**
   * Micro-compaction: silently replace old tool results with short markers.
   * Keeps the last 3 tool messages as-is and always preserves read_file results.
   * Returns the compacted array without modifying conversationHistory.
   */
  private microCompact(): ChatMessage[] {
    // Build a map: tool_call_id -> tool_name from all assistant messages
    const toolCallIdToName = new Map<string, string>();
    for (const msg of this.conversationHistory) {
      if (msg.role === "assistant" && msg.tool_calls) {
        for (const tc of msg.tool_calls) {
          toolCallIdToName.set(tc.id, tc.function.name);
        }
      }
    }

    // Find indices of all tool result messages
    const toolIndices: number[] = [];
    for (let i = 0; i < this.conversationHistory.length; i++) {
      if (this.conversationHistory[i].role === "tool") {
        toolIndices.push(i);
      }
    }

    // Keep the last 3 tool messages as-is
    const keepIndices = new Set(toolIndices.slice(-3));

    const result: ChatMessage[] = [];
    for (let i = 0; i < this.conversationHistory.length; i++) {
      const msg = this.conversationHistory[i];
      if (msg.role === "tool" && !keepIndices.has(i)) {
        const toolName = msg.tool_call_id
          ? toolCallIdToName.get(msg.tool_call_id)
          : undefined;
        // Always preserve read_file results (they are reference material)
        if (toolName === "file_read" || toolName === "read_file") {
          result.push(msg);
        } else {
          result.push({
            role: "tool",
            content: `[Previous: used ${toolName || "unknown tool"}]`,
            tool_call_id: msg.tool_call_id,
          });
        }
      } else {
        result.push(msg);
      }
    }

    return result;
  }

  /**
   * Estimate whether the message list exceeds the token limit.
   * Rough estimate: total_chars / 4.
   */
  private needsCompaction(messages: ChatMessage[]): boolean {
    const totalChars = messages.reduce(
      (sum, m) => sum + (m.content?.length ?? 0),
      0,
    );
    return totalChars / 4 > this.MAX_ESTIMATED_TOKENS;
  }

  /**
   * Auto-compaction: save full transcript to disk, ask LLM to summarize,
   * then replace conversationHistory with the summary.
   */
  private async doAutoCompact(): Promise<void> {
    const timestamp = new Date().toISOString().replace(/[:.]/g, "-");
    const transcriptDir = path.join(
      os.homedir(),
      ".wgenty-code",
      "transcripts",
    );

    // Ensure transcript directory exists
    await fs.mkdir(transcriptDir, { recursive: true });

    // Save full transcript to disk as a JSON archive
    const transcriptPath = path.join(
      transcriptDir,
      `session_${timestamp}.json`,
    );
    await fs.writeFile(
      transcriptPath,
      JSON.stringify(this.conversationHistory, null, 2),
      "utf-8",
    );

    // Build a plain-text version for the summarization prompt
    const transcriptText = this.conversationHistory
      .map((m) => {
        const role = m.role;
        const content = m.content ?? "";
        return `[${role}]: ${content}`;
      })
      .join("\n\n");

    const summaryMessages: ChatMessage[] = [
      {
        role: "system",
        content:
          "You are a conversation summary assistant. Summarize the following coding assistant conversation history for an AI agent. Preserve key details: project context, files modified, decisions made, bugs found, commands executed, and any pending tasks. Keep it concise but include all important information the agent needs to continue working. Do NOT use any tools — just return the summary as plain text.",
      },
      {
        role: "user",
        content: `Summarize this conversation history:\n\n${transcriptText}`,
      },
    ];

    try {
      const summary = await this.simpleStream(summaryMessages);
      if (!summary) {
        return; // Empty summary — don't replace history
      }

      this.compactedSummary = summary;

      // Find the last user message to preserve continuity
      const lastUserMsg = [...this.conversationHistory]
        .reverse()
        .find((m) => m.role === "user");

      // Replace conversation with compressed version:
      // system prompt → summary → last user message
      this.conversationHistory = [
        { role: "system", content: buildSystemPrompt() },
        {
          role: "system",
          content: `<previous_conversation_summary>\n${summary}\n</previous_conversation_summary>`,
        },
      ];

      if (lastUserMsg) {
        this.conversationHistory.push(lastUserMsg);
      }
    } catch (err) {
      // Don't modify conversationHistory on failure — the transcript is
      // already saved, so nothing is lost.
      console.error("Auto-compaction failed:", err);
    }
  }

  /**
   * Stream messages and collect the full response without triggering UI callbacks.
   * Used internally for compaction summarization.
   */
  private async simpleStream(messages: ChatMessage[]): Promise<string> {
    const maxRetries = 1;
    let lastError = "";

    for (let attempt = 0; attempt <= maxRetries; attempt++) {
      try {
        const response = await this.client.chatStream({ messages });
        const processor = new StreamProcessor();
        const reader = response.body?.getReader();
        if (!reader) throw new Error("Response body not readable");

        try {
          while (true) {
            const { done, value } = await reader.read();
            if (done) break;
            processor.feedBytes(value);
          }
        } finally {
          reader.releaseLock();
        }

        return processor.finish().content;
      } catch (err) {
        lastError = String(err);
        if (attempt < maxRetries) {
          await new Promise((r) => setTimeout(r, 1000));
        }
      }
    }

    throw new Error(`Summary stream failed: ${lastError}`);
  }
}
